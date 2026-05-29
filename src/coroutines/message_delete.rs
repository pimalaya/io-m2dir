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

use crate::{
    coroutine::*, coroutines::message_list::*, entry::M2dirEntry, m2dir::M2dir, path::M2dirPath,
};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageDeleteError {
    #[error("Invalid m2dir message delete arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirMessageDeleteArg>, State),
    #[error("entry {0} not found in m2dir")]
    NotFound(String),
    #[error(transparent)]
    List(#[from] M2dirMessageListError),
}

/// Internal progression state of [`M2dirMessageDelete`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(M2dirMessageList),
    ReadMeta(M2dirEntry),
    Removing,
    #[default]
    Invalid,
}

/// Argument fed back into [`M2dirMessageDelete`].
#[derive(Clone, Debug)]
pub enum M2dirMessageDeleteArg {
    /// Response to [`M2dirCoroutineState::WantsDirRead`].
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Response to [`M2dirCoroutineState::WantsFileExists`].
    FileExists(BTreeMap<M2dirPath, bool>),
    /// Response to [`M2dirCoroutineState::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to delete an entry from an m2dir.
///
/// Removes the message file and every sibling `.meta/<id>*` file.
#[derive(Debug)]
pub struct M2dirMessageDelete {
    id: String,
    meta_dir: M2dirPath,
    state: State,
}

impl M2dirMessageDelete {
    /// Creates a new coroutine that will delete entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString) -> Self {
        let meta_dir = m2dir.meta_dir();
        Self {
            id: id.to_string(),
            meta_dir,
            state: State::List(M2dirMessageList::new(m2dir)),
        }
    }
}

impl M2dirCoroutine for M2dirMessageDelete {
    type Arg = M2dirMessageDeleteArg;
    type Output = ();
    type Error = M2dirMessageDeleteError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (mem::take(&mut self.state), arg) {
            (State::List(mut list), arg) => {
                let list_arg = match arg {
                    None => None,
                    Some(M2dirMessageDeleteArg::DirRead(entries)) => {
                        Some(M2dirMessageListArg::DirRead(entries))
                    }
                    Some(M2dirMessageDeleteArg::FileExists(probes)) => {
                        Some(M2dirMessageListArg::FileExists(probes))
                    }
                    Some(other) => {
                        let state = State::List(list);
                        let err = M2dirMessageDeleteError::Invalid(Some(other), state);
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
                            let err = M2dirMessageDeleteError::NotFound(self.id.clone());
                            return M2dirCoroutineState::Err(err);
                        };

                        trace!("located entry at {}", entry.path());

                        let paths = BTreeSet::from_iter([self.meta_dir.clone()]);
                        self.state = State::ReadMeta(entry);
                        M2dirCoroutineState::WantsDirRead(paths)
                    }
                    M2dirCoroutineState::Err(err) => M2dirCoroutineState::Err(err.into()),
                    other => unreachable!("M2dirMessageList yielded {other:?}"),
                }
            }
            (State::ReadMeta(entry), Some(M2dirMessageDeleteArg::DirRead(entries))) => {
                let meta_names = entries.into_values().next().unwrap_or_default();
                let mut to_remove: Vec<M2dirPath> = Vec::new();

                to_remove.push(entry.path().clone());

                for path in meta_names {
                    let Some(name) = path.file_name() else {
                        continue;
                    };
                    if name.starts_with(&self.id) {
                        to_remove.push(path);
                    }
                }

                trace!("wants removal of {} files", to_remove.len());

                self.state = State::Removing;
                M2dirCoroutineState::WantsFileRemove(BTreeSet::from_iter(to_remove))
            }
            (State::Removing, Some(M2dirMessageDeleteArg::FileRemove)) => {
                trace!("entry deleted");
                M2dirCoroutineState::Done(())
            }
            (state, arg) => {
                let err = M2dirMessageDeleteError::Invalid(arg, state);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
