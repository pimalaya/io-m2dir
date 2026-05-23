//! I/O-free coroutine to delete an m2dir entry and its metadata
//! siblings.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::coroutines::message_list::*;
use crate::entry::Entry;
use crate::m2dir::M2dir;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MessageDeleteError {
    #[error("Invalid m2dir message delete arg {0:?} for state {1:?}")]
    Invalid(Option<MessageDeleteArg>, State),

    #[error("entry {0} not found in m2dir")]
    NotFound(String),

    #[error(transparent)]
    List(#[from] MessageListError),
}

/// Result returned by [`MessageDelete::resume`].
#[derive(Clone, Debug)]
pub enum MessageDeleteResult {
    /// The coroutine has successfully terminated its progression.
    Ok,

    /// The caller must read the entries of the given directories and
    /// feed back [`MessageDeleteArg::DirRead`].
    WantsDirRead(BTreeSet<String>),

    /// The caller must remove the given files and feed back
    /// [`MessageDeleteArg::FileRemove`].
    WantsFileRemove(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(MessageDeleteError),
}

/// Internal progression state of [`MessageDelete`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(MessageList),
    ReadMeta(Entry),
    Removing,
    #[default]
    Invalid,
}

/// Argument fed back to [`MessageDelete::resume`].
#[derive(Clone, Debug)]
pub enum MessageDeleteArg {
    /// Response to [`MessageDeleteResult::WantsDirRead`].
    DirRead(BTreeMap<String, BTreeSet<String>>),

    /// Response to [`MessageDeleteResult::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to delete an entry from an m2dir.
///
/// Removes the message file and every sibling `.meta/<id>*` sidecar.
#[derive(Debug)]
pub struct MessageDelete {
    id: String,
    meta_dir: String,
    state: State,
}

impl MessageDelete {
    /// Creates a new coroutine that will delete entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString) -> Self {
        let meta_dir = m2dir.meta_dir().to_string_lossy().into_owned();
        Self {
            id: id.to_string(),
            meta_dir,
            state: State::List(MessageList::new(m2dir)),
        }
    }

    /// Makes the message deletion progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MessageDeleteArg>>,
    ) -> MessageDeleteResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::List(mut list), arg) => {
                let list_arg = match arg {
                    None => None,
                    Some(MessageDeleteArg::DirRead(entries)) => {
                        Some(MessageListArg::DirRead(entries))
                    }
                    Some(other) => {
                        let state = State::List(list);
                        let err = MessageDeleteError::Invalid(Some(other), state);
                        return MessageDeleteResult::Err(err);
                    }
                };

                match list.resume(list_arg) {
                    MessageListResult::WantsDirRead(paths) => {
                        self.state = State::List(list);
                        MessageDeleteResult::WantsDirRead(paths)
                    }
                    MessageListResult::Ok(entries) => {
                        let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                            let err = MessageDeleteError::NotFound(self.id.clone());
                            return MessageDeleteResult::Err(err);
                        };

                        trace!("located entry at {}", entry.path().display());

                        let paths = BTreeSet::from_iter([self.meta_dir.clone()]);
                        self.state = State::ReadMeta(entry);
                        MessageDeleteResult::WantsDirRead(paths)
                    }
                    MessageListResult::Err(err) => MessageDeleteResult::Err(err.into()),
                }
            }
            (State::ReadMeta(entry), Some(MessageDeleteArg::DirRead(entries))) => {
                let meta_names = entries.into_values().next().unwrap_or_default();
                let mut to_remove = Vec::new();

                to_remove.push(entry.path().to_string_lossy().into_owned());

                for path_str in meta_names {
                    let name = match path_str.rsplit('/').next() {
                        Some(name) => name,
                        None => continue,
                    };
                    if name.starts_with(&self.id) {
                        to_remove.push(path_str);
                    }
                }

                trace!("wants removal of {} files", to_remove.len());

                self.state = State::Removing;
                MessageDeleteResult::WantsFileRemove(BTreeSet::from_iter(to_remove))
            }
            (State::Removing, Some(MessageDeleteArg::FileRemove)) => {
                trace!("entry deleted");
                MessageDeleteResult::Ok
            }
            (state, arg) => {
                let err = MessageDeleteError::Invalid(arg, state);
                MessageDeleteResult::Err(err)
            }
        }
    }
}
