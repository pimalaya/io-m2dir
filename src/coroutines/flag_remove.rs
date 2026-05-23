//! I/O-free coroutine to remove flags from an m2dir entry's flags
//! metadata file.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{flag::Flags, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagRemoveError {
    #[error("Invalid m2dir flags remove arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirFlagRemoveArg>, State),
}

/// Result returned by [`M2dirFlagRemove::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagRemoveResult {
    /// The coroutine has successfully terminated its progression.
    Ok,
    /// The caller must read the contents of the given files and feed
    /// back [`M2dirFlagRemoveArg::FileRead`]. Missing files are
    /// reported as empty content.
    WantsFileRead(BTreeSet<M2dirPath>),
    /// The caller must write the given files with the given contents
    /// and feed back [`M2dirFlagRemoveArg::FileCreate`].
    WantsFileCreate(BTreeMap<M2dirPath, Vec<u8>>),
    /// The caller must remove the given files and feed back
    /// [`M2dirFlagRemoveArg::FileRemove`]. Used when the new flag
    /// set is empty.
    WantsFileRemove(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirFlagRemoveError),
}

/// Internal progression state of [`M2dirFlagRemove`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Read,
    Done,
    #[default]
    Invalid,
}

/// Argument fed back to [`M2dirFlagRemove::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagRemoveArg {
    /// Response to [`M2dirFlagRemoveResult::WantsFileRead`].
    FileRead(BTreeMap<M2dirPath, Vec<u8>>),
    /// Response to [`M2dirFlagRemoveResult::WantsFileCreate`].
    FileCreate,
    /// Response to [`M2dirFlagRemoveResult::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to remove `flags` from entry `id`'s flags
/// metadata file.
#[derive(Debug)]
pub struct M2dirFlagRemove {
    flags_path: M2dirPath,
    flags: Flags,
    state: State,
}

impl M2dirFlagRemove {
    /// Creates a new coroutine that will remove `flags` from the
    /// flags metadata file for entry `id` inside `m2dir`.
    pub fn new(m2dir: &M2dir, id: impl AsRef<str>, flags: Flags) -> Self {
        let flags_path = m2dir.flags_path(id.as_ref());

        Self {
            flags_path,
            flags,
            state: State::Start,
        }
    }

    /// Makes the flags remove progress.
    pub fn resume(&mut self, arg: Option<impl Into<M2dirFlagRemoveArg>>) -> M2dirFlagRemoveResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                trace!("wants existing flags read at {}", self.flags_path);

                let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                self.state = State::Read;
                M2dirFlagRemoveResult::WantsFileRead(paths)
            }
            (State::Read, Some(M2dirFlagRemoveArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = core::str::from_utf8(&bytes).unwrap_or("");

                let mut remaining = Flags::from_meta(existing);
                remaining.difference(&self.flags);

                self.state = State::Done;

                if remaining.is_empty() {
                    trace!("wants flags remove at {}", self.flags_path);

                    let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                    M2dirFlagRemoveResult::WantsFileRemove(paths)
                } else {
                    trace!(
                        "wants flags write at {} ({} flags)",
                        self.flags_path,
                        remaining.len(),
                    );

                    let serialized = remaining.to_meta().into_bytes();
                    let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);

                    M2dirFlagRemoveResult::WantsFileCreate(files)
                }
            }
            (
                State::Done,
                Some(M2dirFlagRemoveArg::FileCreate | M2dirFlagRemoveArg::FileRemove),
            ) => {
                trace!("flags removed from {}", self.flags_path);
                M2dirFlagRemoveResult::Ok
            }
            (state, arg) => {
                let err = M2dirFlagRemoveError::Invalid(arg, state);
                M2dirFlagRemoveResult::Err(err)
            }
        }
    }
}
