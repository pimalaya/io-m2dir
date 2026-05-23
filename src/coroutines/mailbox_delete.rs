//! I/O-free coroutine to delete an m2dir mailbox.

use alloc::collections::BTreeSet;

use log::trace;
use thiserror::Error;

use crate::path::M2dirPath;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMailboxDeleteError {
    #[error("Invalid m2dir mailbox delete arg: {0:?}")]
    Invalid(Option<M2dirMailboxDeleteArg>),
}

/// Result returned by [`M2dirMailboxDelete::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMailboxDeleteResult {
    /// The coroutine has successfully terminated its progression.
    Ok,
    /// The caller must recursively remove the given directories and
    /// feed back [`M2dirMailboxDeleteArg::DirRemove`].
    WantsDirRemove(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirMailboxDeleteError),
}

/// Argument fed back to [`M2dirMailboxDelete::resume`] after the
/// caller performed the requested filesystem operation.
#[derive(Clone, Debug)]
pub enum M2dirMailboxDeleteArg {
    /// Response to [`M2dirMailboxDeleteResult::WantsDirRemove`].
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

    /// Makes the mailbox deletion progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<M2dirMailboxDeleteArg>>,
    ) -> M2dirMailboxDeleteResult {
        match (self.wants_dir_remove.take(), arg.map(Into::into)) {
            (Some(paths), None) => {
                trace!("wants filesystem I/O to remove {} directories", paths.len());
                M2dirMailboxDeleteResult::WantsDirRemove(paths)
            }
            (None, Some(M2dirMailboxDeleteArg::DirRemove)) => {
                trace!("resume after removing m2dir");
                M2dirMailboxDeleteResult::Ok
            }
            (_, arg) => {
                let err = M2dirMailboxDeleteError::Invalid(arg);
                M2dirMailboxDeleteResult::Err(err)
            }
        }
    }
}
