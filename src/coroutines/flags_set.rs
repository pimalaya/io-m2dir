//! I/O-free coroutine to replace an m2dir entry's metadata sidecar
//! with a new flag set.

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
pub enum FlagsSetError {
    #[error("Invalid m2dir flags set arg {0:?} for state {1:?}")]
    Invalid(Option<FlagsSetArg>, State),
}

/// Result returned by [`FlagsSet::resume`].
#[derive(Clone, Debug)]
pub enum FlagsSetResult {
    /// The coroutine has successfully terminated its progression.
    Ok,

    /// The caller must write the given files with the given contents
    /// and feed back [`FlagsSetArg::FileCreate`].
    WantsFileCreate(BTreeMap<String, Vec<u8>>),

    /// The caller must remove the given files and feed back
    /// [`FlagsSetArg::FileRemove`]. Used when the new flag set is
    /// empty.
    WantsFileRemove(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(FlagsSetError),
}

/// Internal progression state of [`FlagsSet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Done,
    #[default]
    Invalid,
}

/// Argument fed back to [`FlagsSet::resume`].
#[derive(Clone, Debug)]
pub enum FlagsSetArg {
    /// Response to [`FlagsSetResult::WantsFileCreate`].
    FileCreate,

    /// Response to [`FlagsSetResult::WantsFileRemove`].
    FileRemove,
}

/// I/O-free coroutine to replace an entry's flag sidecar with
/// `flags`.
///
/// If `flags` is empty the sidecar is removed; otherwise it is
/// overwritten.
#[derive(Debug)]
pub struct FlagsSet {
    sidecar: String,
    flags: Flags,
    state: State,
}

impl FlagsSet {
    /// Creates a new coroutine that will replace the sidecar for
    /// entry `id` inside `m2dir` with `flags`.
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

    /// Makes the flags set progress.
    pub fn resume(&mut self, arg: Option<impl Into<FlagsSetArg>>) -> FlagsSetResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start, None) => {
                self.state = State::Done;

                if self.flags.is_empty() {
                    trace!("wants sidecar remove at {}", self.sidecar);

                    let paths = BTreeSet::from_iter([self.sidecar.clone()]);
                    FlagsSetResult::WantsFileRemove(paths)
                } else {
                    trace!(
                        "wants sidecar write at {} ({} flags)",
                        self.sidecar,
                        self.flags.len(),
                    );

                    let serialized = self.flags.to_sidecar().into_bytes();
                    let files = BTreeMap::from_iter([(self.sidecar.clone(), serialized)]);

                    FlagsSetResult::WantsFileCreate(files)
                }
            }
            (State::Done, Some(FlagsSetArg::FileCreate | FlagsSetArg::FileRemove)) => {
                trace!("flags set at {}", self.sidecar);
                FlagsSetResult::Ok
            }
            (state, arg) => {
                let err = FlagsSetError::Invalid(arg, state);
                FlagsSetResult::Err(err)
            }
        }
    }
}
