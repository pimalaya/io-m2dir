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
    coroutines::message_list::*,
    entry::{Entry, ParseFilenameError, validate_checksum},
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

/// Result returned by [`M2dirMessageGet::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMessageGetResult {
    /// The coroutine has successfully terminated its progression.
    Ok { entry: Entry, contents: Vec<u8> },
    /// The caller must read the entries of the given directories
    /// and feed back [`M2dirMessageGetArg::DirRead`].
    WantsDirRead(BTreeSet<M2dirPath>),
    /// The caller must check whether the given paths exist as
    /// regular files and feed back
    /// [`M2dirMessageGetArg::FileExists`].
    WantsFileExists(BTreeSet<M2dirPath>),
    /// The caller must read the contents of the given files and
    /// feed back [`M2dirMessageGetArg::FileRead`].
    WantsFileRead(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirMessageGetError),
}

/// Internal progression state of [`M2dirMessageGet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(M2dirMessageList),
    Read(Entry),
    #[default]
    Invalid,
}

/// Argument fed back to [`M2dirMessageGet::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMessageGetArg {
    /// Response to [`M2dirMessageGetResult::WantsDirRead`].
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Response to [`M2dirMessageGetResult::WantsFileExists`].
    FileExists(BTreeMap<M2dirPath, bool>),
    /// Response to [`M2dirMessageGetResult::WantsFileRead`].
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

    /// Makes the message get progress.
    pub fn resume(&mut self, arg: Option<impl Into<M2dirMessageGetArg>>) -> M2dirMessageGetResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
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
                        return M2dirMessageGetResult::Err(err);
                    }
                };

                match list.resume(list_arg) {
                    M2dirMessageListResult::WantsDirRead(paths) => {
                        self.state = State::List(list);
                        M2dirMessageGetResult::WantsDirRead(paths)
                    }
                    M2dirMessageListResult::WantsFileExists(paths) => {
                        self.state = State::List(list);
                        M2dirMessageGetResult::WantsFileExists(paths)
                    }
                    M2dirMessageListResult::Ok(entries) => {
                        let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                            let err = M2dirMessageGetError::NotFound(self.id.clone());
                            return M2dirMessageGetResult::Err(err);
                        };

                        trace!("located entry at {}", entry.path());

                        let paths = BTreeSet::from_iter([entry.path().clone()]);
                        self.state = State::Read(entry);
                        M2dirMessageGetResult::WantsFileRead(paths)
                    }
                    M2dirMessageListResult::Err(err) => M2dirMessageGetResult::Err(err.into()),
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
                    return M2dirMessageGetResult::Err(err.into());
                }

                M2dirMessageGetResult::Ok {
                    entry,
                    contents: bytes,
                }
            }
            (state, arg) => {
                let err = M2dirMessageGetError::Invalid(arg, state);
                M2dirMessageGetResult::Err(err)
            }
        }
    }
}
