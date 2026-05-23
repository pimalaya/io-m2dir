//! I/O-free coroutine to read an m2dir entry by its id.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::coroutines::message_list::*;
use crate::entry::{Entry, ParseFilenameError, validate_checksum};
use crate::m2dir::M2dir;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MessageGetError {
    #[error("Invalid m2dir message get arg {0:?} for state {1:?}")]
    Invalid(Option<MessageGetArg>, State),

    #[error("entry {0} not found in m2dir")]
    NotFound(String),

    #[error(transparent)]
    List(#[from] MessageListError),

    #[error(transparent)]
    Parse(#[from] ParseFilenameError),
}

/// Result returned by [`MessageGet::resume`].
#[derive(Clone, Debug)]
pub enum MessageGetResult {
    /// The coroutine has successfully terminated its progression.
    Ok { entry: Entry, contents: Vec<u8> },

    /// The caller must read the entries of the given directories and
    /// feed back [`MessageGetArg::DirRead`].
    WantsDirRead(BTreeSet<String>),

    /// The caller must read the contents of the given files and feed
    /// back [`MessageGetArg::FileRead`].
    WantsFileRead(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(MessageGetError),
}

/// Internal progression state of [`MessageGet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(MessageList),
    Read(Entry),
    #[default]
    Invalid,
}

/// Argument fed back to [`MessageGet::resume`].
#[derive(Clone, Debug)]
pub enum MessageGetArg {
    /// Response to [`MessageGetResult::WantsDirRead`].
    DirRead(BTreeMap<String, BTreeSet<String>>),

    /// Response to [`MessageGetResult::WantsFileRead`].
    FileRead(BTreeMap<String, Vec<u8>>),
}

/// I/O-free coroutine that locates and reads an m2dir entry by id.
///
/// The id is the `<checksum>.<nonce>` portion of the entry filename.
#[derive(Debug)]
pub struct MessageGet {
    id: String,
    state: State,
}

impl MessageGet {
    /// Creates a new coroutine that will retrieve entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString) -> Self {
        Self {
            id: id.to_string(),
            state: State::List(MessageList::new(m2dir)),
        }
    }

    /// Makes the message get progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MessageGetArg>>,
    ) -> MessageGetResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::List(mut list), arg) => {
                let list_arg = match arg {
                    None => None,
                    Some(MessageGetArg::DirRead(entries)) => Some(MessageListArg::DirRead(entries)),
                    Some(other) => {
                        let state = State::List(list);
                        let err = MessageGetError::Invalid(Some(other), state);
                        return MessageGetResult::Err(err);
                    }
                };

                match list.resume(list_arg) {
                    MessageListResult::WantsDirRead(paths) => {
                        self.state = State::List(list);
                        MessageGetResult::WantsDirRead(paths)
                    }
                    MessageListResult::Ok(entries) => {
                        let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                            let err = MessageGetError::NotFound(self.id.clone());
                            return MessageGetResult::Err(err);
                        };

                        trace!("located entry at {}", entry.path().display());

                        let path = entry.path().to_string_lossy().into_owned();
                        let paths = BTreeSet::from_iter([path]);

                        self.state = State::Read(entry);
                        MessageGetResult::WantsFileRead(paths)
                    }
                    MessageListResult::Err(err) => MessageGetResult::Err(err.into()),
                }
            }
            (State::Read(entry), Some(MessageGetArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let checksum = entry.checksum();

                if !validate_checksum(checksum, &bytes) {
                    let err = ParseFilenameError::InvalidChecksum {
                        path: entry.path().to_path_buf(),
                        expected: checksum.to_string(),
                        got: entry.id().to_string(),
                    };
                    return MessageGetResult::Err(err.into());
                }

                MessageGetResult::Ok {
                    entry,
                    contents: bytes,
                }
            }
            (state, arg) => {
                let err = MessageGetError::Invalid(arg, state);
                MessageGetResult::Err(err)
            }
        }
    }
}
