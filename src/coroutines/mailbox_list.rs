//! I/O-free coroutine to list every m2dir inside an m2store.

use core::mem;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    m2dir::{DOT_M2DIR, M2dir},
    m2store::M2store,
    path::M2dirPath,
};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMailboxListError {
    #[error("Invalid m2dir mailbox list arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
}

/// Internal progression state of [`M2dirMailboxList`].
#[derive(Clone, Debug, Default)]
pub enum State {
    /// Reading directories; accumulating candidates and confirmed
    /// m2dirs.
    Scanning {
        pending: BTreeSet<M2dirPath>,
        found: BTreeSet<M2dir>,
    },
    /// Verifying which scanned candidates carry a `.m2dir` marker.
    CheckingMarkers {
        next_pending: BTreeSet<M2dirPath>,
        markers: BTreeMap<M2dirPath, M2dirPath>,
        found: BTreeSet<M2dir>,
    },
    #[default]
    Invalid,
}

/// I/O-free coroutine to list every valid m2dir inside an m2store.
///
/// Walks the tree depth-first. Hidden entries (whose name starts
/// with `.`) are skipped. A directory containing the `.m2dir`
/// marker is reported as an m2dir; its sub-directories are still
/// scanned because m2dirs can nest.
#[derive(Debug)]
pub struct M2dirMailboxList {
    state: State,
}

impl M2dirMailboxList {
    /// Creates a new coroutine that will list every m2dir inside
    /// `store`.
    pub fn new(store: &M2store) -> Self {
        let pending = BTreeSet::from_iter([store.path().clone()]);

        Self {
            state: State::Scanning {
                pending,
                found: BTreeSet::new(),
            },
        }
    }
}

impl M2dirCoroutine for M2dirMailboxList {
    type Yield = M2dirYield;
    type Return = Result<BTreeSet<M2dir>, M2dirMailboxListError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        match (mem::take(&mut self.state), arg) {
            (State::Scanning { pending, found }, None) => {
                let batch = pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(batch))
            }
            (State::Scanning { mut pending, found }, Some(M2dirArg::DirRead(entries))) => {
                trace!("scanned {} directories", entries.len());

                let mut markers = BTreeMap::new();
                let mut next_pending = mem::take(&mut pending);

                for (_dir, names) in entries {
                    for path in names {
                        let Some(name) = path.file_name() else {
                            continue;
                        };

                        if name.starts_with('.') {
                            continue;
                        }

                        let marker = path.join(DOT_M2DIR);
                        markers.insert(marker, path.clone());
                        next_pending.insert(path);
                    }
                }

                if markers.is_empty() {
                    if next_pending.is_empty() {
                        trace!("found {} m2dirs", found.len());
                        return M2dirCoroutineState::Complete(Ok(found));
                    }

                    let batch = next_pending;
                    self.state = State::Scanning {
                        pending: BTreeSet::new(),
                        found,
                    };
                    return M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(batch));
                }

                let probes: BTreeSet<M2dirPath> = markers.keys().cloned().collect();
                trace!("wants existence check for {} markers", probes.len());

                self.state = State::CheckingMarkers {
                    next_pending,
                    markers,
                    found,
                };
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes))
            }
            (
                State::CheckingMarkers {
                    next_pending,
                    markers,
                    mut found,
                },
                Some(M2dirArg::FileExists(probes)),
            ) => {
                for (marker, dir) in markers {
                    if probes.get(&marker).copied().unwrap_or(false) {
                        found.insert(M2dir::from(dir));
                    }
                }

                if next_pending.is_empty() {
                    trace!("found {} m2dirs", found.len());
                    return M2dirCoroutineState::Complete(Ok(found));
                }

                let batch = next_pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(batch))
            }
            (state, arg) => {
                let err = M2dirMailboxListError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
