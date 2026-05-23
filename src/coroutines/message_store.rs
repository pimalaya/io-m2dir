//! I/O-free coroutine to store a message in an m2dir.

use core::{
    mem,
    sync::atomic::{AtomicU32, Ordering},
};

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use std::{path::PathBuf, process};

use log::trace;
use thiserror::Error;

use crate::entry::Entry;
use crate::m2dir::M2dir;
use crate::rand::random_bytes;

static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MessageStoreError {
    #[error("Invalid m2dir message store arg {0:?} for state {1:?}")]
    Invalid(Option<MessageStoreArg>, State),
}

/// Result returned by [`MessageStore::resume`].
#[derive(Clone, Debug)]
pub enum MessageStoreResult {
    /// The coroutine has successfully terminated its progression.
    Ok(Entry),

    /// The caller must write the given files with the given contents
    /// and feed back [`MessageStoreArg::FileCreate`].
    WantsFileCreate(BTreeMap<String, Vec<u8>>),

    /// The caller must rename each `(from, to)` pair and feed back
    /// [`MessageStoreArg::Rename`].
    WantsRename(Vec<(String, String)>),

    /// The coroutine encountered an error.
    Err(MessageStoreError),
}

/// Internal progression state of [`MessageStore`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start(Vec<u8>),
    Created,
    Renamed,
    #[default]
    Invalid,
}

/// Argument fed back to [`MessageStore::resume`].
#[derive(Clone, Debug)]
pub enum MessageStoreArg {
    /// Response to [`MessageStoreResult::WantsFileCreate`].
    FileCreate,

    /// Response to [`MessageStoreResult::WantsRename`].
    Rename,
}

/// I/O-free coroutine to store a message in an m2dir.
///
/// Follows the m2dir delivery protocol: write to a temporary file
/// in the same directory first, then atomically rename to
/// `<date>,<checksum>.<nonce>`.
#[derive(Debug)]
pub struct MessageStore {
    id: String,
    tmp_path: String,
    final_path: String,
    state: State,
}

impl MessageStore {
    /// Creates a new coroutine that will store `bytes` as a new entry
    /// in `m2dir`.
    pub fn new(m2dir: &M2dir, bytes: Vec<u8>) -> Self {
        let nonce_bytes: Vec<u8> = random_bytes().take(4).collect();
        let (id, final_path) = m2dir.entry_path(&bytes, &nonce_bytes);

        let counter = TMP_COUNTER.fetch_add(1, Ordering::AcqRel);
        let tmp_path = m2dir.tmp_path(process::id(), counter);

        Self {
            id,
            tmp_path: tmp_path.to_string_lossy().into_owned(),
            final_path: final_path.to_string_lossy().into_owned(),
            state: State::Start(bytes),
        }
    }

    /// Makes the message store progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MessageStoreArg>>,
    ) -> MessageStoreResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start(bytes), None) => {
                trace!("wants tmp file create at {}", self.tmp_path);

                let files = BTreeMap::from_iter([(self.tmp_path.clone(), bytes)]);
                self.state = State::Created;
                MessageStoreResult::WantsFileCreate(files)
            }
            (State::Created, Some(MessageStoreArg::FileCreate)) => {
                trace!("created tmp file, wants rename to {}", self.final_path);

                let pairs = vec![(self.tmp_path.clone(), self.final_path.clone())];
                self.state = State::Renamed;
                MessageStoreResult::WantsRename(pairs)
            }
            (State::Renamed, Some(MessageStoreArg::Rename)) => {
                trace!("renamed tmp file to {}", self.final_path);

                let entry = Entry::from_parts(self.id.clone(), PathBuf::from(&self.final_path));
                MessageStoreResult::Ok(entry)
            }
            (state, arg) => {
                let err = MessageStoreError::Invalid(arg, state);
                MessageStoreResult::Err(err)
            }
        }
    }
}
