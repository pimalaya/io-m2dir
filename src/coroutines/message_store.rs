//! I/O-free coroutine to store a message in an m2dir.

use core::{
    mem,
    sync::atomic::{AtomicU32, Ordering},
};

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use log::trace;
use thiserror::Error;

use crate::coroutine::*;
use crate::entry::M2dirEntry;
use crate::m2dir::M2dir;
use crate::path::M2dirPath;

const NONCE_LEN: usize = 4;

static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageStoreError {
    #[error("Invalid m2dir message store arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirMessageStoreArg>, State),
}

/// Internal progression state of [`M2dirMessageStore`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start(Vec<u8>),
    AwaitingPid(Vec<u8>),
    AwaitingRandom {
        bytes: Vec<u8>,
        pid: u32,
    },
    Created {
        tmp_path: M2dirPath,
        final_path: M2dirPath,
        id: String,
    },
    Renamed {
        final_path: M2dirPath,
        id: String,
    },
    #[default]
    Invalid,
}

/// Argument fed back into [`M2dirMessageStore`].
#[derive(Clone, Debug)]
pub enum M2dirMessageStoreArg {
    /// Response to [`M2dirCoroutineState::WantsPid`].
    Pid(u32),
    /// Response to [`M2dirCoroutineState::WantsRandom`].
    Random(Vec<u8>),
    /// Response to [`M2dirCoroutineState::WantsFileCreate`].
    FileCreate,
    /// Response to [`M2dirCoroutineState::WantsRename`].
    Rename,
}

/// I/O-free coroutine to store a message in an m2dir.
///
/// Follows the m2dir delivery protocol: write to a temporary file
/// in the same directory first, then atomically rename to
/// `<date>,<checksum>.<nonce>`.
#[derive(Debug)]
pub struct M2dirMessageStore {
    m2dir: M2dir,
    state: State,
}

impl M2dirMessageStore {
    /// Creates a new coroutine that will store `bytes` as a new
    /// entry in `m2dir`.
    pub fn new(m2dir: M2dir, bytes: Vec<u8>) -> Self {
        Self {
            m2dir,
            state: State::Start(bytes),
        }
    }
}

impl M2dirCoroutine for M2dirMessageStore {
    type Arg = M2dirMessageStoreArg;
    type Output = M2dirEntry;
    type Error = M2dirMessageStoreError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (mem::take(&mut self.state), arg) {
            (State::Start(bytes), None) => {
                trace!("wants pid");
                self.state = State::AwaitingPid(bytes);
                M2dirCoroutineState::WantsPid
            }
            (State::AwaitingPid(bytes), Some(M2dirMessageStoreArg::Pid(pid))) => {
                trace!("wants {NONCE_LEN} random bytes");
                self.state = State::AwaitingRandom { bytes, pid };
                M2dirCoroutineState::WantsRandom { len: NONCE_LEN }
            }
            (State::AwaitingRandom { bytes, pid }, Some(M2dirMessageStoreArg::Random(nonce))) => {
                let (id, final_path) = self.m2dir.entry_path(&bytes, &nonce);
                let counter = TMP_COUNTER.fetch_add(1, Ordering::AcqRel);
                let tmp_path = self.m2dir.tmp_path(pid, counter);

                trace!("wants tmp file create at {tmp_path}");

                let files = BTreeMap::from_iter([(tmp_path.clone(), bytes)]);
                self.state = State::Created {
                    tmp_path,
                    final_path,
                    id,
                };
                M2dirCoroutineState::WantsFileCreate(files)
            }
            (
                State::Created {
                    tmp_path,
                    final_path,
                    id,
                },
                Some(M2dirMessageStoreArg::FileCreate),
            ) => {
                trace!("created tmp file, wants rename to {final_path}");

                let pairs = vec![(tmp_path, final_path.clone())];
                self.state = State::Renamed { final_path, id };
                M2dirCoroutineState::WantsRename(pairs)
            }
            (State::Renamed { final_path, id }, Some(M2dirMessageStoreArg::Rename)) => {
                trace!("renamed tmp file to {final_path}");

                let entry = M2dirEntry::from_parts(id, final_path);
                M2dirCoroutineState::Done(entry)
            }
            (state, arg) => {
                let err = M2dirMessageStoreError::Invalid(arg, state);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
