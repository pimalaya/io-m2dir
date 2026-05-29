//! I/O-free coroutine to read an m2dir entry by its id.

use core::mem;

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    coroutines::message_list::*,
    entry::{M2dirEntry, ParseFilenameError, validate_checksum},
    m2dir::M2dir,
};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageGetError {
    #[error("Invalid m2dir message get arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
    #[error("entry {0} not found in m2dir")]
    NotFound(String),
    #[error(transparent)]
    List(#[from] M2dirMessageListError),
    #[error(transparent)]
    Parse(#[from] ParseFilenameError),
}

/// Terminal output of [`M2dirMessageGet`].
#[derive(Clone, Debug)]
pub struct M2dirMessageGetOutput {
    /// The resolved entry (id + on-disk path).
    pub entry: M2dirEntry,
    /// Raw bytes read from the entry file.
    pub contents: Vec<u8>,
}

/// Internal progression state of [`M2dirMessageGet`].
#[derive(Clone, Debug, Default)]
pub enum State {
    List(M2dirMessageList),
    Read(M2dirEntry),
    #[default]
    Invalid,
}

/// I/O-free coroutine that locates and reads an m2dir entry by id.
///
/// The id is the `<checksum>.<nonce>` portion of the entry filename.
#[derive(Debug)]
pub struct M2dirMessageGet {
    id: String,
    state: State,
}

impl M2dirMessageGet {
    /// Creates a new coroutine that will retrieve entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString) -> Self {
        Self {
            id: id.to_string(),
            state: State::List(M2dirMessageList::new(m2dir)),
        }
    }
}

impl M2dirCoroutine for M2dirMessageGet {
    type Yield = M2dirYield;
    type Return = Result<M2dirMessageGetOutput, M2dirMessageGetError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        match (mem::take(&mut self.state), arg) {
            (State::List(mut list), arg) => match list.resume(arg) {
                M2dirCoroutineState::Yielded(yld) => {
                    self.state = State::List(list);
                    M2dirCoroutineState::Yielded(yld)
                }
                M2dirCoroutineState::Complete(Ok(entries)) => {
                    let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                        let err = M2dirMessageGetError::NotFound(self.id.clone());
                        return M2dirCoroutineState::Complete(Err(err));
                    };

                    trace!("located entry at {}", entry.path());

                    let paths = BTreeSet::from_iter([entry.path().clone()]);
                    self.state = State::Read(entry);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths))
                }
                M2dirCoroutineState::Complete(Err(err)) => {
                    M2dirCoroutineState::Complete(Err(err.into()))
                }
            },
            (State::Read(entry), Some(M2dirArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let checksum = entry.checksum();

                if !validate_checksum(checksum, &bytes) {
                    let err = ParseFilenameError::InvalidChecksum {
                        path: entry.path().clone(),
                        expected: checksum.to_string(),
                        got: entry.id().to_string(),
                    };
                    return M2dirCoroutineState::Complete(Err(err.into()));
                }

                M2dirCoroutineState::Complete(Ok(M2dirMessageGetOutput {
                    entry,
                    contents: bytes,
                }))
            }
            (state, arg) => {
                let err = M2dirMessageGetError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
