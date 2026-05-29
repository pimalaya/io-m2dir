//! I/O-free coroutine to add flags to an m2dir entry's flags
//! metadata file.

use core::mem;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, flag::M2dirFlags, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagAddError {
    #[error("Invalid m2dir flags add arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
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
}

impl M2dirCoroutine for M2dirFlagAdd {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagAddError>;

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
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
            }
            (State::Written, Some(M2dirArg::FileCreate)) => {
                trace!("flags added to {}", self.flags_path);
                M2dirCoroutineState::Complete(Ok(()))
            }
            (state, arg) => {
                let err = M2dirFlagAddError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
