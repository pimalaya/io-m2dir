//! I/O-free coroutine to replace an m2dir entry's flags metadata
//! file with a new flag set.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{flag::M2dirFlags, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagSetError {
    #[error("Invalid m2dir flags set arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirFlagSetArg>, State),
}

/// Result returned by [`M2dirFlagSet::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagSetResult {
    /// The coroutine has successfully terminated its progression.
    Ok,
    /// The caller must write the given files with the given contents
    /// and feed back [`M2dirFlagSetArg::FileCreate`].
    WantsFileCreate(BTreeMap<M2dirPath, Vec<u8>>),
    /// The caller must remove the given files and feed back
    /// [`M2dirFlagSetArg::FileRemove`]. Used when the new flag set
    /// is empty.
    WantsFileRemove(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirFlagSetError),
}

/// Internal progression state of [`M2dirFlagSet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Done,
    #[default]
    Invalid,
}

/// Argument fed back to [`M2dirFlagSet::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagSetArg {
    /// Response to [`M2dirFlagSetResult::WantsFileCreate`].
    FileCreate,
    /// Response to [`M2dirFlagSetResult::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to replace an entry's flags metadata file
/// with `flags`.
///
/// If `flags` is empty the metadata file is removed; otherwise it
/// is overwritten.
#[derive(Debug)]
pub struct M2dirFlagSet {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
}

impl M2dirFlagSet {
    /// Creates a new coroutine that will replace the flags metadata
    /// file for entry `id` inside `m2dir` with `flags`.
    pub fn new(m2dir: &M2dir, id: impl AsRef<str>, flags: M2dirFlags) -> Self {
        let flags_path = m2dir.flags_path(id.as_ref());

        Self {
            flags_path,
            flags,
            state: State::Start,
        }
    }

    /// Makes the flags set progress.
    pub fn resume(&mut self, arg: Option<impl Into<M2dirFlagSetArg>>) -> M2dirFlagSetResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                self.state = State::Done;

                if self.flags.is_empty() {
                    trace!("wants flags remove at {}", self.flags_path);

                    let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                    M2dirFlagSetResult::WantsFileRemove(paths)
                } else {
                    trace!(
                        "wants flags write at {} ({} flags)",
                        self.flags_path,
                        self.flags.len(),
                    );

                    let serialized = self.flags.to_meta().into_bytes();
                    let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);

                    M2dirFlagSetResult::WantsFileCreate(files)
                }
            }
            (State::Done, Some(M2dirFlagSetArg::FileCreate | M2dirFlagSetArg::FileRemove)) => {
                trace!("flags set at {}", self.flags_path);
                M2dirFlagSetResult::Ok
            }
            (state, arg) => {
                let err = M2dirFlagSetError::Invalid(arg, state);
                M2dirFlagSetResult::Err(err)
            }
        }
    }
}
