//! I/O-free coroutine to remove flags from an m2dir entry's metadata
//! sidecar.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::flag::Flags;
use crate::m2dir::M2dir;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum FlagsRemoveError {
    #[error("Invalid m2dir flags remove arg {0:?} for state {1:?}")]
    Invalid(Option<FlagsRemoveArg>, State),
}

/// Result returned by [`FlagsRemove::resume`].
#[derive(Clone, Debug)]
pub enum FlagsRemoveResult {
    /// The coroutine has successfully terminated its progression.
    Ok,

    /// The caller must read the contents of the given files and feed
    /// back [`FlagsRemoveArg::FileRead`]. Missing files are reported
    /// as empty content.
    WantsFileRead(BTreeSet<String>),

    /// The caller must write the given files with the given contents
    /// and feed back [`FlagsRemoveArg::FileCreate`].
    WantsFileCreate(BTreeMap<String, Vec<u8>>),

    /// The caller must remove the given files and feed back
    /// [`FlagsRemoveArg::FileRemove`]. Used when the new flag set is
    /// empty.
    WantsFileRemove(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(FlagsRemoveError),
}

/// Internal progression state of [`FlagsRemove`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Read,
    Done,
    #[default]
    Invalid,
}

/// Argument fed back to [`FlagsRemove::resume`].
#[derive(Clone, Debug)]
pub enum FlagsRemoveArg {
    /// Response to [`FlagsRemoveResult::WantsFileRead`].
    FileRead(BTreeMap<String, Vec<u8>>),

    /// Response to [`FlagsRemoveResult::WantsFileCreate`].
    FileCreate,

    /// Response to [`FlagsRemoveResult::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to remove `flags` from entry `id`'s sidecar.
#[derive(Debug)]
pub struct FlagsRemove {
    sidecar: String,
    flags: Flags,
    state: State,
}

impl FlagsRemove {
    /// Creates a new coroutine that will remove `flags` from the
    /// sidecar for entry `id` inside `m2dir`.
    pub fn new(m2dir: &M2dir, id: impl AsRef<str>, flags: Flags) -> Self {
        let sidecar = m2dir
            .flags_sidecar_path(id.as_ref())
            .to_string_lossy()
            .into_owned();

        Self {
            sidecar,
            flags,
            state: State::Start,
        }
    }

    /// Makes the flags remove progress.
    pub fn resume(&mut self, arg: Option<impl Into<FlagsRemoveArg>>) -> FlagsRemoveResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                trace!("wants existing sidecar read at {}", self.sidecar);

                let paths = BTreeSet::from_iter([self.sidecar.clone()]);
                self.state = State::Read;
                FlagsRemoveResult::WantsFileRead(paths)
            }
            (State::Read, Some(FlagsRemoveArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = core::str::from_utf8(&bytes).unwrap_or("");

                let mut remaining = Flags::from_sidecar(existing);
                remaining.difference(&self.flags);

                self.state = State::Done;

                if remaining.is_empty() {
                    trace!("wants sidecar remove at {}", self.sidecar);

                    let paths = BTreeSet::from_iter([self.sidecar.clone()]);
                    FlagsRemoveResult::WantsFileRemove(paths)
                } else {
                    trace!(
                        "wants sidecar write at {} ({} flags)",
                        self.sidecar,
                        remaining.len(),
                    );

                    let serialized = remaining.to_sidecar().into_bytes();
                    let files = BTreeMap::from_iter([(self.sidecar.clone(), serialized)]);

                    FlagsRemoveResult::WantsFileCreate(files)
                }
            }
            (State::Done, Some(FlagsRemoveArg::FileCreate | FlagsRemoveArg::FileRemove)) => {
                trace!("flags removed from {}", self.sidecar);
                FlagsRemoveResult::Ok
            }
            (state, arg) => {
                let err = FlagsRemoveError::Invalid(arg, state);
                FlagsRemoveResult::Err(err)
            }
        }
    }
}
