//! I/O-free coroutine to delete an m2dir mailbox.

use alloc::collections::BTreeSet;

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMailboxDeleteError {
    #[error("Invalid m2dir mailbox delete arg: {0:?}")]
    Invalid(Option<M2dirMailboxDeleteArg>),
}

/// Argument fed back into [`M2dirMailboxDelete`] after the caller
/// performed the requested filesystem operation.
#[derive(Clone, Debug)]
pub enum M2dirMailboxDeleteArg {
    /// Response to [`M2dirCoroutineState::WantsDirRemove`].
    DirRemove,
}

/// I/O-free coroutine to delete an m2dir mailbox and all its
/// contents.
#[derive(Debug)]
pub struct M2dirMailboxDelete {
    wants_dir_remove: Option<BTreeSet<M2dirPath>>,
}

impl M2dirMailboxDelete {
    /// Creates a new coroutine that will recursively remove the
    /// m2dir at `path`.
    pub fn new(path: impl Into<M2dirPath>) -> Self {
        let paths = BTreeSet::from_iter([path.into()]);
        Self {
            wants_dir_remove: Some(paths),
        }
    }
}

impl M2dirCoroutine for M2dirMailboxDelete {
    type Arg = M2dirMailboxDeleteArg;
    type Output = ();
    type Error = M2dirMailboxDeleteError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (self.wants_dir_remove.take(), arg) {
            (Some(paths), None) => {
                trace!("wants filesystem I/O to remove {} directories", paths.len());
                M2dirCoroutineState::WantsDirRemove(paths)
            }
            (None, Some(M2dirMailboxDeleteArg::DirRemove)) => {
                trace!("resume after removing m2dir");
                M2dirCoroutineState::Done(())
            }
            (_, arg) => {
                let err = M2dirMailboxDeleteError::Invalid(arg);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
