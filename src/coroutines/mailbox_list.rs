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
    Invalid(Option<M2dirMailboxListArg>, State),
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

/// Argument fed back into [`M2dirMailboxList`].
#[derive(Clone, Debug)]
pub enum M2dirMailboxListArg {
    /// Response to [`M2dirCoroutineState::WantsDirRead`].
    ///
    /// Maps each requested directory path to the set of entry paths
    /// found inside it.
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Response to [`M2dirCoroutineState::WantsFileExists`].
    ///
    /// Maps each probed marker path to whether it exists as a
    /// regular file.
    FileExists(BTreeMap<M2dirPath, bool>),
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
    type Arg = M2dirMailboxListArg;
    type Output = BTreeSet<M2dir>;
    type Error = M2dirMailboxListError;

    fn resume(&mut self, arg: Option<Self::Arg>) -> M2dirCoroutineState<Self::Output, Self::Error> {
        match (mem::take(&mut self.state), arg) {
            (State::Scanning { pending, found }, None) => {
                let batch = pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirCoroutineState::WantsDirRead(batch)
            }
            (
                State::Scanning { mut pending, found },
                Some(M2dirMailboxListArg::DirRead(entries)),
            ) => {
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
                        return M2dirCoroutineState::Done(found);
                    }

                    let batch = next_pending;
                    self.state = State::Scanning {
                        pending: BTreeSet::new(),
                        found,
                    };
                    return M2dirCoroutineState::WantsDirRead(batch);
                }

                let probes: BTreeSet<M2dirPath> = markers.keys().cloned().collect();
                trace!("wants existence check for {} markers", probes.len());

                self.state = State::CheckingMarkers {
                    next_pending,
                    markers,
                    found,
                };
                M2dirCoroutineState::WantsFileExists(probes)
            }
            (
                State::CheckingMarkers {
                    next_pending,
                    markers,
                    mut found,
                },
                Some(M2dirMailboxListArg::FileExists(probes)),
            ) => {
                for (marker, dir) in markers {
                    if probes.get(&marker).copied().unwrap_or(false) {
                        found.insert(M2dir::from(dir));
                    }
                }

                if next_pending.is_empty() {
                    trace!("found {} m2dirs", found.len());
                    return M2dirCoroutineState::Done(found);
                }

                let batch = next_pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirCoroutineState::WantsDirRead(batch)
            }
            (state, arg) => {
                let err = M2dirMailboxListError::Invalid(arg, state);
                M2dirCoroutineState::Err(err)
            }
        }
    }
}
