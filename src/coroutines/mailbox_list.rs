//! I/O-free coroutine to list every m2dir inside an m2store.

use core::mem;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{
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

/// Result returned by [`M2dirMailboxList::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMailboxListResult {
    /// The coroutine has successfully terminated its progression.
    Ok(BTreeSet<M2dir>),
    /// The caller must read the entries of the given directories and
    /// feed back [`M2dirMailboxListArg::DirRead`].
    WantsDirRead(BTreeSet<M2dirPath>),
    /// The caller must check whether the given paths exist as
    /// regular files and feed back
    /// [`M2dirMailboxListArg::FileExists`].
    WantsFileExists(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirMailboxListError),
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

/// Argument fed back to [`M2dirMailboxList::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMailboxListArg {
    /// Response to [`M2dirMailboxListResult::WantsDirRead`].
    ///
    /// Maps each requested directory path to the set of entry paths
    /// found inside it.
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Response to [`M2dirMailboxListResult::WantsFileExists`].
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

    /// Makes the listing progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<M2dirMailboxListArg>>,
    ) -> M2dirMailboxListResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Scanning { pending, found }, None) => {
                let batch = pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirMailboxListResult::WantsDirRead(batch)
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
                        return M2dirMailboxListResult::Ok(found);
                    }

                    let batch = next_pending;
                    self.state = State::Scanning {
                        pending: BTreeSet::new(),
                        found,
                    };
                    return M2dirMailboxListResult::WantsDirRead(batch);
                }

                let probes: BTreeSet<M2dirPath> = markers.keys().cloned().collect();
                trace!("wants existence check for {} markers", probes.len());

                self.state = State::CheckingMarkers {
                    next_pending,
                    markers,
                    found,
                };
                M2dirMailboxListResult::WantsFileExists(probes)
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
                    return M2dirMailboxListResult::Ok(found);
                }

                let batch = next_pending;
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning {
                    pending: BTreeSet::new(),
                    found,
                };
                M2dirMailboxListResult::WantsDirRead(batch)
            }
            (state, arg) => {
                let err = M2dirMailboxListError::Invalid(arg, state);
                M2dirMailboxListResult::Err(err)
            }
        }
    }
}
