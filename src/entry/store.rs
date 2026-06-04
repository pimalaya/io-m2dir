//! I/O-free coroutine to store an entry in an m2dir.
//!
//! Follows the m2dir delivery protocol: write to a temporary file
//! in the same directory first, then atomically rename to the final
//! `<date>,<checksum>.<nonce>` filename.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::fs;
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::types::M2dir,
//!     entry::store::{M2dirEntryStore, M2dirEntryStoreOptions},
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let opts = M2dirEntryStoreOptions::default();
//! let bytes = b"From: a\r\n\r\nhi\r\n".to_vec();
//! let mut coroutine = M2dirEntryStore::new(m2dir, bytes, opts);
//! let mut arg = None;
//!
//! let entry = loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsPid) => {
//!             arg = Some(M2dirArg::Pid(std::process::id()));
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsRandom { len }) => {
//!             let mut bytes = vec![0u8; len];
//!             // ... fill via OS RNG ...
//!             arg = Some(M2dirArg::Random(bytes));
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => {
//!             for (path, bytes) in files {
//!                 fs::write(path.as_str(), bytes).unwrap();
//!             }
//!             arg = Some(M2dirArg::FileCreate);
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsRename(pairs)) => {
//!             for (from, to) in pairs {
//!                 fs::rename(from.as_str(), to.as_str()).unwrap();
//!             }
//!             arg = Some(M2dirArg::Rename);
//!         }
//!         M2dirCoroutineState::Complete(Ok(entry)) => break entry,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! };
//!
//! println!("delivered {}", entry.id());
//! ```

use core::{
    fmt, mem,
    sync::atomic::{AtomicU32, Ordering},
};

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, entry::types::M2dirEntry, m2dir::types::M2dir, path::M2dirPath};

const NONCE_LEN: usize = 4;

static TMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Failure causes during the m2dir STORE flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirEntryStoreError {
    #[error("M2DIR STORE failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR STORE failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirEntryStore::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirEntryStoreOptions {}

/// I/O-free m2dir entry STORE coroutine.
pub struct M2dirEntryStore {
    m2dir: M2dir,
    bytes: Vec<u8>,
    state: State,
    #[allow(dead_code)]
    opts: M2dirEntryStoreOptions,
}

impl M2dirEntryStore {
    /// Creates a new coroutine that will store `bytes` as a new
    /// entry in `m2dir`.
    pub fn new(m2dir: M2dir, bytes: Vec<u8>, opts: M2dirEntryStoreOptions) -> Self {
        Self {
            m2dir,
            bytes,
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirEntryStore {
    type Yield = M2dirYield;
    type Return = Result<M2dirEntry, M2dirEntryStoreError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("store entry: {}", self.state);

        match (&mut self.state, arg) {
            (State::Start, None) => {
                trace!("wants pid");
                self.state = State::AwaitingPid;
                M2dirCoroutineState::Yielded(M2dirYield::WantsPid)
            }
            (State::AwaitingPid, Some(M2dirArg::Pid(pid))) => {
                trace!("wants {NONCE_LEN} random bytes");
                self.state = State::AwaitingRandom { pid };
                M2dirCoroutineState::Yielded(M2dirYield::WantsRandom { len: NONCE_LEN })
            }
            (State::AwaitingRandom { pid }, Some(M2dirArg::Random(nonce))) => {
                let pid = *pid;
                let bytes = mem::take(&mut self.bytes);

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
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
            }
            (
                State::Created {
                    tmp_path,
                    final_path,
                    id,
                },
                Some(M2dirArg::FileCreate),
            ) => {
                let tmp_path = mem::take(tmp_path);
                let final_path = mem::take(final_path);
                let id = mem::take(id);
                trace!("created tmp file, wants rename to {final_path}");

                let pairs = vec![(tmp_path, final_path.clone())];
                self.state = State::Renamed { final_path, id };
                M2dirCoroutineState::Yielded(M2dirYield::WantsRename(pairs))
            }
            (State::Renamed { final_path, id }, Some(M2dirArg::Rename)) => {
                let final_path = mem::take(final_path);
                let id = mem::take(id);
                trace!("renamed tmp file to {final_path}");

                let entry = M2dirEntry::from_parts(id, final_path);
                M2dirCoroutineState::Complete(Ok(entry))
            }
            (_, Some(_)) => {
                let err = M2dirEntryStoreError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirEntryStoreError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    AwaitingPid,
    AwaitingRandom {
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
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::AwaitingPid => f.write_str("awaiting pid"),
            Self::AwaitingRandom { .. } => f.write_str("awaiting random nonce"),
            Self::Created { .. } => f.write_str("tmp file created"),
            Self::Renamed { .. } => f.write_str("renamed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_yields_full_delivery_sequence() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut store =
            M2dirEntryStore::new(m2dir, b"hi".to_vec(), M2dirEntryStoreOptions::default());

        match store.resume(None) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsPid) => {}
            state => panic!("expected WantsPid, got {state:?}"),
        }
        match store.resume(Some(M2dirArg::Pid(42))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsRandom { len }) => {
                assert_eq!(len, NONCE_LEN);
            }
            state => panic!("expected WantsRandom, got {state:?}"),
        }
        let files = match store.resume(Some(M2dirArg::Random(b"abcd".to_vec()))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => files,
            state => panic!("expected WantsFileCreate, got {state:?}"),
        };
        assert_eq!(files.len(), 1);
        let (tmp_path, _) = files.into_iter().next().unwrap();
        assert!(tmp_path.as_str().contains(".m2dir.tmp."));

        let pairs = match store.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsRename(pairs)) => pairs,
            state => panic!("expected WantsRename, got {state:?}"),
        };
        assert_eq!(pairs.len(), 1);

        let entry = match store.resume(Some(M2dirArg::Rename)) {
            M2dirCoroutineState::Complete(Ok(entry)) => entry,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert!(entry.path().as_str().starts_with("/tmp/inbox/"));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut store =
            M2dirEntryStore::new(m2dir, b"hi".to_vec(), M2dirEntryStoreOptions::default());

        let err = match store.resume(Some(M2dirArg::Pid(42))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryStoreError::UnexpectedArg));
    }

    #[test]
    fn missing_arg_at_awaiting_pid_returns_missing_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut store =
            M2dirEntryStore::new(m2dir, b"hi".to_vec(), M2dirEntryStoreOptions::default());
        let _ = store.resume(None);

        let err = match store.resume(None) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryStoreError::MissingArg));
    }

    #[test]
    fn wrong_arg_kind_at_awaiting_random_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut store =
            M2dirEntryStore::new(m2dir, b"hi".to_vec(), M2dirEntryStoreOptions::default());
        let _ = store.resume(None);
        let _ = store.resume(Some(M2dirArg::Pid(42)));

        let err = match store.resume(Some(M2dirArg::Pid(0))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryStoreError::UnexpectedArg));
    }

    #[test]
    fn wrong_arg_kind_at_renamed_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut store =
            M2dirEntryStore::new(m2dir, b"hi".to_vec(), M2dirEntryStoreOptions::default());

        let _ = store.resume(None);
        let _ = store.resume(Some(M2dirArg::Pid(42)));
        let _ = store.resume(Some(M2dirArg::Random(b"abcd".to_vec())));
        let _ = store.resume(Some(M2dirArg::FileCreate));

        let err = match store.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryStoreError::UnexpectedArg));
    }
}
