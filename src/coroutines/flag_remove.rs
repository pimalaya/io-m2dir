//! I/O-free coroutine to remove flags from an m2dir entry's flags
//! metadata file.

use core::mem;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, flag::M2dirFlags, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagRemoveError {
    #[error("Invalid m2dir flags remove arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
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

/// I/O-free coroutine to remove `flags` from entry `id`'s flags
/// metadata file.
#[derive(Debug)]
pub struct M2dirFlagRemove {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
}

impl M2dirFlagRemove {
    /// Creates a new coroutine that will remove `flags` from the
    /// flags metadata file for entry `id` inside `m2dir`.
    pub fn new(m2dir: &M2dir, id: impl AsRef<str>, flags: M2dirFlags) -> Self {
        let flags_path = m2dir.flags_path(id.as_ref());

        Self {
            flags_path,
            flags,
            state: State::Start,
        }
    }
}

impl M2dirCoroutine for M2dirFlagRemove {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagRemoveError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        match (mem::take(&mut self.state), arg) {
            (State::Start, None) => {
                trace!("wants existing flags read at {}", self.flags_path);

                let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                self.state = State::Read;
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths))
            }
            (State::Read, Some(M2dirArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = core::str::from_utf8(&bytes).unwrap_or("");

                let mut remaining = M2dirFlags::from_meta(existing);
                remaining.difference(&self.flags);

                self.state = State::Done;

                if remaining.is_empty() {
                    trace!("wants flags remove at {}", self.flags_path);

                    let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths))
                } else {
                    trace!(
                        "wants flags write at {} ({} flags)",
                        self.flags_path,
                        remaining.len(),
                    );

                    let serialized = remaining.to_meta().into_bytes();
                    let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);

                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
                }
            }
            (State::Done, Some(M2dirArg::FileCreate | M2dirArg::FileRemove)) => {
                trace!("flags removed from {}", self.flags_path);
                M2dirCoroutineState::Complete(Ok(()))
            }
            (state, arg) => {
                let err = M2dirFlagRemoveError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
