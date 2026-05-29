//! I/O-free coroutine to replace an m2dir entry's flags metadata
//! file with a new flag set.

use core::mem;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, flag::M2dirFlags, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagSetError {
    #[error("Invalid m2dir flags set arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
}

/// Internal progression state of [`M2dirFlagSet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start,
    Done,
    #[default]
    Invalid,
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
}

impl M2dirCoroutine for M2dirFlagSet {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagSetError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        match (mem::take(&mut self.state), arg) {
            (State::Start, None) => {
                self.state = State::Done;

                if self.flags.is_empty() {
                    trace!("wants flags remove at {}", self.flags_path);

                    let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths))
                } else {
                    trace!(
                        "wants flags write at {} ({} flags)",
                        self.flags_path,
                        self.flags.len(),
                    );

                    let serialized = self.flags.to_meta().into_bytes();
                    let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);

                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
                }
            }
            (State::Done, Some(M2dirArg::FileCreate | M2dirArg::FileRemove)) => {
                trace!("flags set at {}", self.flags_path);
                M2dirCoroutineState::Complete(Ok(()))
            }
            (state, arg) => {
                let err = M2dirFlagSetError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
