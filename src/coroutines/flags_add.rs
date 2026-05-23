//! I/O-free coroutine to add flags to an m2dir entry's metadata
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
pub enum FlagsAddError {
    #[error("Invalid m2dir flags add arg {0:?} for state {1:?}")]
    Invalid(Option<FlagsAddArg>, State),
}

/// Result returned by [`FlagsAdd::resume`].
#[derive(Clone, Debug)]
pub enum FlagsAddResult {
    /// The coroutine has successfully terminated its progression.
    Ok,

    /// The caller must read the contents of the given files and feed
    /// back [`FlagsAddArg::FileRead`]. Missing files are reported as
    /// empty content.
    WantsFileRead(BTreeSet<String>),

    /// The caller must write the given files with the given contents
    /// and feed back [`FlagsAddArg::FileCreate`].
    WantsFileCreate(BTreeMap<String, Vec<u8>>),

    /// The coroutine encountered an error.
    Err(FlagsAddError),
}

/// Internal progression state of [`FlagsAdd`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Read,
    Written,
    #[default]
    Invalid,
}

/// Argument fed back to [`FlagsAdd::resume`].
#[derive(Clone, Debug)]
pub enum FlagsAddArg {
    /// Response to [`FlagsAddResult::WantsFileRead`]. Missing files
    /// may be reported as an empty byte buffer.
    FileRead(BTreeMap<String, Vec<u8>>),

    /// Response to [`FlagsAddResult::WantsFileCreate`].
    FileCreate,
}

/// I/O-free coroutine to add `flags` to entry `id`'s sidecar.
#[derive(Debug)]
pub struct FlagsAdd {
    sidecar: String,
    flags: Flags,
    state: State,
}

impl FlagsAdd {
    /// Creates a new coroutine that will add `flags` to the sidecar
    /// for entry `id` inside `m2dir`.
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

    /// Makes the flags add progress.
    pub fn resume(&mut self, arg: Option<impl Into<FlagsAddArg>>) -> FlagsAddResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                trace!("wants existing sidecar read at {}", self.sidecar);

                let paths = BTreeSet::from_iter([self.sidecar.clone()]);
                self.state = State::Read;
                FlagsAddResult::WantsFileRead(paths)
            }
            (State::Read, Some(FlagsAddArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = core::str::from_utf8(&bytes).unwrap_or("");

                let mut merged = Flags::from_sidecar(existing);
                merged.extend(self.flags.clone());

                trace!("wants sidecar write at {} ({} flags)", self.sidecar, merged.len());

                let serialized = merged.to_sidecar().into_bytes();
                let files = BTreeMap::from_iter([(self.sidecar.clone(), serialized)]);

                self.state = State::Written;
                FlagsAddResult::WantsFileCreate(files)
            }
            (State::Written, Some(FlagsAddArg::FileCreate)) => {
                trace!("flags added to {}", self.sidecar);
                FlagsAddResult::Ok
            }
            (state, arg) => {
                let err = FlagsAddError::Invalid(arg, state);
                FlagsAddResult::Err(err)
            }
        }
    }
}

