//! I/O-free coroutine to create an m2dir mailbox.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::m2dir::M2dir;
use crate::m2store::{M2store, NewFolderError};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MailboxCreateError {
    #[error("Invalid m2dir mailbox create arg {0:?} for state {1:?}")]
    Invalid(Option<MailboxCreateArg>, State),

    #[error(transparent)]
    Resolve(#[from] NewFolderError),
}

/// Result returned by [`MailboxCreate::resume`].
#[derive(Clone, Debug)]
pub enum MailboxCreateResult {
    /// The coroutine has successfully terminated its progression.
    Ok(M2dir),

    /// The caller must recursively create the given directories and
    /// feed back [`MailboxCreateArg::DirCreate`].
    WantsDirCreate(BTreeSet<String>),

    /// The caller must write the given files with the given contents
    /// and feed back [`MailboxCreateArg::FileCreate`].
    WantsFileCreate(BTreeMap<String, Vec<u8>>),

    /// The coroutine encountered an error.
    Err(MailboxCreateError),
}

/// Internal progression state of [`MailboxCreate`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    DirCreated,
    MarkerWritten,
    #[default]
    Invalid,
}

/// Argument fed back to [`MailboxCreate::resume`] after the caller
/// performed the requested filesystem operation.
#[derive(Clone, Debug)]
pub enum MailboxCreateArg {
    /// Response to [`MailboxCreateResult::WantsDirCreate`].
    DirCreate,

    /// Response to [`MailboxCreateResult::WantsFileCreate`].
    FileCreate,
}

/// I/O-free coroutine to create a new m2dir mailbox: the folder, the
/// `.meta` sub-directory and the `.m2dir` marker file.
#[derive(Debug)]
pub struct MailboxCreate {
    m2dir: M2dir,
    state: State,
}

impl MailboxCreate {
    /// Creates a new coroutine that will create the folder named
    /// `name` inside `store`.
    pub fn new(store: &M2store, name: impl AsRef<str>) -> Result<Self, NewFolderError> {
        let path = store.resolve_folder_path(name.as_ref())?;
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

    /// Makes the mailbox creation progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MailboxCreateArg>>,
    ) -> MailboxCreateResult {
        match (core::mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                trace!("wants directory creation for {}", self.m2dir.path().display());

                let root = self.m2dir.path().to_string_lossy().into_owned();
                let meta = self.m2dir.meta_dir().to_string_lossy().into_owned();
                let paths = BTreeSet::from_iter([root, meta]);

                self.state = State::DirCreated;
                MailboxCreateResult::WantsDirCreate(paths)
            }
            (State::DirCreated, Some(MailboxCreateArg::DirCreate)) => {
                trace!("wants marker file at {}", self.m2dir.marker_path().display());

                let marker = self.m2dir.marker_path().to_string_lossy().into_owned();
                let files = BTreeMap::from_iter([(marker, Vec::new())]);

                self.state = State::MarkerWritten;
                MailboxCreateResult::WantsFileCreate(files)
            }
            (State::MarkerWritten, Some(MailboxCreateArg::FileCreate)) => {
                trace!("mailbox created at {}", self.m2dir.path().display());
                MailboxCreateResult::Ok(self.m2dir.clone())
            }
            (state, arg) => {
                let err = MailboxCreateError::Invalid(arg, state);
                MailboxCreateResult::Err(err)
            }
        }
    }
}
