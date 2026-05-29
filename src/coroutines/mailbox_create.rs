//! I/O-free coroutine to create an m2dir mailbox.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    m2dir::M2dir,
    m2store::{M2store, NewFolderError},
};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMailboxCreateError {
    #[error("Invalid m2dir mailbox create arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirMailboxCreateArg>, State),
    #[error(transparent)]
    Resolve(#[from] NewFolderError),
}

/// Internal progression state of [`M2dirMailboxCreate`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    DirCreated,
    MarkerWritten,
    #[default]
    Invalid,
}

/// Argument fed back into [`M2dirMailboxCreate`] after the caller
/// performed the requested filesystem operation.
#[derive(Clone, Debug)]
pub enum M2dirMailboxCreateArg {
    /// Response to [`M2dirCoroutineState::WantsDirCreate`].
    DirCreate,
    /// Response to [`M2dirCoroutineState::WantsFileCreate`].
    FileCreate,
}

/// I/O-free coroutine to create a new m2dir mailbox: the folder, the
/// `.meta` sub-directory and the `.m2dir` marker file.
#[derive(Debug)]
pub struct M2dirMailboxCreate {
    m2dir: M2dir,
    state: State,
}

impl M2dirMailboxCreate {
    /// Creates a new coroutine that will create the folder named
    /// `name` inside `store`.
    pub fn new(store: &M2store, name: &str) -> Result<Self, NewFolderError> {
        let path = store.resolve_folder_path(name)?;
        let m2dir = M2dir::from_path(path);

        Ok(Self {
            m2dir,
            state: State::Start,
        })
    }

    /// Returns the [`M2dir`] this coroutine targets.
    pub fn m2dir(&self) -> &M2dir {
        &self.m2dir
    }
}

impl M2dirCoroutine for M2dirMailboxCreate {
    type Arg = M2dirMailboxCreateArg;
    type Output = M2dir;
    type Error = M2dirMailboxCreateError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (mem::take(&mut self.state), arg) {
            (State::Start, None) => {
                trace!("wants directory creation for {}", self.m2dir.path());

                let root = self.m2dir.path().clone();
                let meta = self.m2dir.meta_dir();
                let paths = BTreeSet::from_iter([root, meta]);

                self.state = State::DirCreated;
                M2dirCoroutineState::WantsDirCreate(paths)
            }
            (State::DirCreated, Some(M2dirMailboxCreateArg::DirCreate)) => {
                trace!("wants marker file at {}", self.m2dir.marker_path());

                let marker = self.m2dir.marker_path();
                let files = BTreeMap::from_iter([(marker, Vec::new())]);

                self.state = State::MarkerWritten;
                M2dirCoroutineState::WantsFileCreate(files)
            }
            (State::MarkerWritten, Some(M2dirMailboxCreateArg::FileCreate)) => {
                trace!("mailbox created at {}", self.m2dir.path());
                M2dirCoroutineState::Done(self.m2dir.clone())
            }
            (state, arg) => {
                let err = M2dirMailboxCreateError::Invalid(arg, state);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
