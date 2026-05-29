//! I/O-free coroutine to read an m2dir entry by its id.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    coroutines::message_list::*,
    entry::{M2dirEntry, ParseFilenameError, validate_checksum},
    m2dir::M2dir,
    path::M2dirPath,
};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageGetError {
    #[error("Invalid m2dir message get arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirMessageGetArg>, State),
    #[error("entry {0} not found in m2dir")]
    NotFound(String),
    #[error(transparent)]
    List(#[from] M2dirMessageListError),
    #[error(transparent)]
    Parse(#[from] ParseFilenameError),
}

/// Successful output of [`M2dirMessageGet`].
#[derive(Clone, Debug)]
pub struct M2dirMessageGetOk {
    pub entry: M2dirEntry,
    pub contents: Vec<u8>,
}

/// Internal progression state of [`M2dirMessageGet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(M2dirMessageList),
    Read(M2dirEntry),
    #[default]
    Invalid,
}

/// Argument fed back into [`M2dirMessageGet`].
#[derive(Clone, Debug)]
pub enum M2dirMessageGetArg {
    /// Forwarded to the inner list coroutine.
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Forwarded to the inner list coroutine.
    FileExists(BTreeMap<M2dirPath, bool>),
    /// Response to [`M2dirCoroutineState::WantsFileRead`].
    FileRead(BTreeMap<M2dirPath, Vec<u8>>),
}

/// I/O-free coroutine that locates and reads an m2dir entry by id.
///
/// The id is the `<checksum>.<nonce>` portion of the entry filename.
#[derive(Debug)]
pub struct M2dirMessageGet {
    id: String,
    state: State,
}

impl M2dirMessageGet {
    /// Creates a new coroutine that will retrieve entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString) -> Self {
        Self {
            id: id.to_string(),
            state: State::List(M2dirMessageList::new(m2dir)),
        }
    }
}

impl M2dirCoroutine for M2dirMessageGet {
    type Arg = M2dirMessageGetArg;
    type Output = M2dirMessageGetOk;
    type Error = M2dirMessageGetError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (mem::take(&mut self.state), arg) {
            (State::List(mut list), arg) => {
                let list_arg = match arg {
                    None => None,
                    Some(M2dirMessageGetArg::DirRead(entries)) => {
                        Some(M2dirMessageListArg::DirRead(entries))
                    }
                    Some(M2dirMessageGetArg::FileExists(probes)) => {
                        Some(M2dirMessageListArg::FileExists(probes))
                    }
                    Some(other) => {
                        let state = State::List(list);
                        let err = M2dirMessageGetError::Invalid(Some(other), state);
                        return M2dirCoroutineState::Err(err);
                    }
                };

                match list.resume(list_arg) {
                    M2dirCoroutineState::WantsDirRead(paths) => {
                        self.state = State::List(list);
                        M2dirCoroutineState::WantsDirRead(paths)
                    }
                    M2dirCoroutineState::WantsFileExists(paths) => {
                        self.state = State::List(list);
                        M2dirCoroutineState::WantsFileExists(paths)
                    }
                    M2dirCoroutineState::Done(entries) => {
                        let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                            let err = M2dirMessageGetError::NotFound(self.id.clone());
                            return M2dirCoroutineState::Err(err);
                        };

                        trace!("located entry at {}", entry.path());

                        let paths = BTreeSet::from_iter([entry.path().clone()]);
                        self.state = State::Read(entry);
                        M2dirCoroutineState::WantsFileRead(paths)
                    }
                    M2dirCoroutineState::Err(err) => M2dirCoroutineState::Err(err.into()),
                    other => unreachable!("M2dirMessageList yielded {other:?}"),
                }
            }
            (State::Read(entry), Some(M2dirMessageGetArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let checksum = entry.checksum();

                if !validate_checksum(checksum, &bytes) {
                    let err = ParseFilenameError::InvalidChecksum {
                        path: entry.path().clone(),
                        expected: checksum.to_string(),
                        got: entry.id().to_string(),
                    };
                    return M2dirCoroutineState::Err(err.into());
                }

                M2dirCoroutineState::Done(M2dirMessageGetOk {
                    entry,
                    contents: bytes,
                })
            }
            (state, arg) => {
                let err = M2dirMessageGetError::Invalid(arg, state);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
