//! I/O-free coroutine to delete an m2dir mailbox.

use alloc::{collections::BTreeSet, string::String};
use std::path::Path;

use log::trace;
use thiserror::Error;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MailboxDeleteError {
    #[error("Invalid m2dir mailbox delete arg: {0:?}")]
    Invalid(Option<MailboxDeleteArg>),
}

/// Result returned by [`MailboxDelete::resume`].
#[derive(Clone, Debug)]
pub enum MailboxDeleteResult {
    /// The coroutine has successfully terminated its progression.
    Ok,

    /// The caller must recursively remove the given directories and
    /// feed back [`MailboxDeleteArg::DirRemove`].
    WantsDirRemove(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(MailboxDeleteError),
}

/// Argument fed back to [`MailboxDelete::resume`] after the caller
/// performed the requested filesystem operation.
#[derive(Clone, Debug)]
pub enum MailboxDeleteArg {
    /// Response to [`MailboxDeleteResult::WantsDirRemove`].
    DirRemove,
}

/// I/O-free coroutine to delete an m2dir mailbox and all its
/// contents.
#[derive(Debug)]
pub struct MailboxDelete {
    wants_dir_remove: Option<BTreeSet<String>>,
}

impl MailboxDelete {
    /// Creates a new coroutine that will recursively remove the
    /// m2dir at `path`.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_string_lossy().into_owned();
        let paths = BTreeSet::from_iter([path]);
        Self {
            wants_dir_remove: Some(paths),
        }
    }

    /// Makes the mailbox deletion progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MailboxDeleteArg>>,
    ) -> MailboxDeleteResult {
        match (self.wants_dir_remove.take(), arg.map(Into::into)) {
            (Some(paths), None) => {
                trace!("wants filesystem I/O to remove {} directories", paths.len());
                MailboxDeleteResult::WantsDirRemove(paths)
            }
            (None, Some(MailboxDeleteArg::DirRemove)) => {
                trace!("resume after removing m2dir");
                MailboxDeleteResult::Ok
            }
            (_, arg) => {
                let err = MailboxDeleteError::Invalid(arg);
                MailboxDeleteResult::Err(err)
            }
        }
    }
}
