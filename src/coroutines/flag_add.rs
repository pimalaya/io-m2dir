//! I/O-free coroutine to add flags to an m2dir entry's flags
//! metadata file.

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
pub enum M2dirFlagAddError {
    #[error("Invalid m2dir flags add arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirFlagAddArg>, State),
}

/// Result returned by [`M2dirFlagAdd::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagAddResult {
    /// The coroutine has successfully terminated its progression.
    Ok,
    /// The caller must read the contents of the given files and feed
    /// back [`M2dirFlagAddArg::FileRead`]. Missing files are reported
    /// as empty content.
    WantsFileRead(BTreeSet<M2dirPath>),
    /// The caller must write the given files with the given contents
    /// and feed back [`M2dirFlagAddArg::FileCreate`].
    WantsFileCreate(BTreeMap<M2dirPath, Vec<u8>>),
    /// The coroutine encountered an error.
    Err(M2dirFlagAddError),
}

/// Internal progression state of [`M2dirFlagAdd`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Read,
    Written,
    #[default]
    Invalid,
}

/// Argument fed back to [`M2dirFlagAdd::resume`].
#[derive(Clone, Debug)]
pub enum M2dirFlagAddArg {
    /// Response to [`M2dirFlagAddResult::WantsFileRead`]. Missing
    /// files may be reported as an empty byte buffer.
    FileRead(BTreeMap<M2dirPath, Vec<u8>>),
    /// Response to [`M2dirFlagAddResult::WantsFileCreate`].
    FileCreate,
}

/// I/O-free coroutine to add `flags` to entry `id`'s flags
/// metadata file.
#[derive(Debug)]
pub struct M2dirFlagAdd {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
}

impl M2dirFlagAdd {
    /// Creates a new coroutine that will add `flags` to the flags
    /// metadata file for entry `id` inside `m2dir`.
    pub fn new(m2dir: &M2dir, id: impl AsRef<str>, flags: M2dirFlags) -> Self {
        let flags_path = m2dir.flags_path(id.as_ref());

        Self {
            flags_path,
            flags,
            state: State::Start,
        }
    }

    /// Makes the flags add progress.
    pub fn resume(&mut self, arg: Option<impl Into<M2dirFlagAddArg>>) -> M2dirFlagAddResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                trace!("wants existing flags read at {}", self.flags_path);

                let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                self.state = State::Read;
                M2dirFlagAddResult::WantsFileRead(paths)
            }
            (State::Read, Some(M2dirFlagAddArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = core::str::from_utf8(&bytes).unwrap_or("");

                let mut merged = M2dirFlags::from_meta(existing);
                merged.extend(self.flags.clone());

                trace!(
                    "wants flags write at {} ({} flags)",
                    self.flags_path,
                    merged.len()
                );

                let serialized = merged.to_meta().into_bytes();
                let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);

                self.state = State::Written;
                M2dirFlagAddResult::WantsFileCreate(files)
            }
            (State::Written, Some(M2dirFlagAddArg::FileCreate)) => {
                trace!("flags added to {}", self.flags_path);
                M2dirFlagAddResult::Ok
            }
            (state, arg) => {
                let err = M2dirFlagAddError::Invalid(arg, state);
                M2dirFlagAddResult::Err(err)
            }
        }
    }
}
