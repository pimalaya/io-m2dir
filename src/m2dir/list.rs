//! I/O-free coroutine to list every valid m2dir inside an m2store.
//!
//! Walks the tree depth-first. Hidden entries (whose name starts
//! with `.`) are skipped. A directory containing the `.m2dir`
//! marker is reported as an m2dir; its sub-directories are still
//! scanned because m2dirs can nest.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     store::M2dirStore,
//!     m2dir::list::{M2dirList, M2dirListOptions},
//! };
//!
//! let store = M2dirStore::from_path("/tmp/store");
//! let opts = M2dirListOptions::default();
//! let mut coroutine = M2dirList::new(&store, opts);
//! let mut arg = None;
//!
//! let mailboxes = loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => {
//!             let mut out = BTreeMap::new();
//!             for path in paths {
//!                 let names = fs::read_dir(path.as_str())
//!                     .map(|rd| rd.flatten().map(|e| e.path().into()).collect())
//!                     .unwrap_or_default();
//!                 out.insert(path, names);
//!             }
//!             arg = Some(M2dirArg::DirRead(out));
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes)) => {
//!             let map = probes
//!                 .into_iter()
//!                 .map(|p| {
//!                     let exists = fs::metadata(p.as_str())
//!                         .map_or(false, |m| m.is_file());
//!                     (p, exists)
//!                 })
//!                 .collect();
//!             arg = Some(M2dirArg::FileExists(map));
//!         }
//!         M2dirCoroutineState::Complete(Ok(mboxes)) => break mboxes,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! };
//!
//! println!("{} mailboxes", mailboxes.len());
//! ```

use core::{fmt, mem};

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    m2dir::types::{DOT_M2DIR, M2dir},
    path::M2dirPath,
    store::M2dirStore,
};

/// Failure causes during the m2dir LIST-MAILBOXES flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirListError {
    #[error("M2DIR LIST failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR LIST failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirList::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirListOptions {}

/// I/O-free m2dir LIST coroutine.
pub struct M2dirList {
    state: State,
    #[allow(dead_code)]
    opts: M2dirListOptions,
}

impl M2dirList {
    /// Creates a new coroutine that will list every m2dir inside
    /// `store`.
    pub fn new(store: &M2dirStore, opts: M2dirListOptions) -> Self {
        let pending = BTreeSet::from_iter([store.path().clone()]);
        Self {
            state: State::Scanning {
                pending,
                found: BTreeSet::new(),
            },
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirList {
    type Yield = M2dirYield;
    type Return = Result<BTreeSet<M2dir>, M2dirListError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("list m2dirs: {}", self.state);

        match (&mut self.state, arg) {
            (State::Scanning { pending, .. }, None) => {
                let batch = mem::take(pending);
                trace!("wants read of {} directories", batch.len());
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(batch))
            }
            (State::Scanning { pending, found }, Some(M2dirArg::DirRead(entries))) => {
                trace!("scanned {} directories", entries.len());

                let mut markers = BTreeMap::new();
                let mut next_pending = mem::take(pending);
                let found = mem::take(found);

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
                    found,
                },
                Some(M2dirArg::FileExists(probes)),
            ) => {
                let next_pending = mem::take(next_pending);
                let markers = mem::take(markers);
                let mut found = mem::take(found);

                for (marker, dir) in markers {
                    if probes.get(&marker).copied().unwrap_or(false) {
                        found.insert(M2dir::from_path(dir));
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
            (_, Some(_)) => {
                let err = M2dirListError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirListError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Scanning {
        pending: BTreeSet<M2dirPath>,
        found: BTreeSet<M2dir>,
    },
    CheckingMarkers {
        next_pending: BTreeSet<M2dirPath>,
        markers: BTreeMap<M2dirPath, M2dirPath>,
        found: BTreeSet<M2dir>,
    },
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scanning { .. } => f.write_str("scanning"),
            Self::CheckingMarkers { .. } => f.write_str("checking markers"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_returns_no_mailboxes() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut list = M2dirList::new(&store, M2dirListOptions::default());

        let batch = expect_wants_dir_read(&mut list, None);
        let dir = batch.into_iter().next().unwrap();
        let mut reply = BTreeMap::new();
        reply.insert(dir, BTreeSet::new());

        let mboxes = match list.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Ok(mboxes)) => mboxes,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert!(mboxes.is_empty());
    }

    #[test]
    fn directory_without_marker_is_skipped() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut list = M2dirList::new(&store, M2dirListOptions::default());

        let _ = expect_wants_dir_read(&mut list, None);

        let mut names = BTreeSet::new();
        names.insert(M2dirPath::from("/tmp/store/maybe"));
        let mut reply = BTreeMap::new();
        reply.insert(M2dirPath::from("/tmp/store"), names);

        let probes = expect_wants_file_exists(&mut list, Some(M2dirArg::DirRead(reply)));
        let exists: BTreeMap<M2dirPath, bool> = probes.into_iter().map(|p| (p, false)).collect();

        let _ = expect_wants_dir_read(&mut list, Some(M2dirArg::FileExists(exists)));
    }

    #[test]
    fn directory_with_marker_is_reported_as_mailbox() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut list = M2dirList::new(&store, M2dirListOptions::default());

        let _ = expect_wants_dir_read(&mut list, None);

        let mut names = BTreeSet::new();
        names.insert(M2dirPath::from("/tmp/store/inbox"));
        let mut reply = BTreeMap::new();
        reply.insert(M2dirPath::from("/tmp/store"), names);

        let probes = expect_wants_file_exists(&mut list, Some(M2dirArg::DirRead(reply)));
        let exists: BTreeMap<M2dirPath, bool> = probes.into_iter().map(|p| (p, true)).collect();

        let next_batch = expect_wants_dir_read(&mut list, Some(M2dirArg::FileExists(exists)));
        let mut reply = BTreeMap::new();
        for dir in next_batch {
            reply.insert(dir, BTreeSet::new());
        }
        let mboxes = match list.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Ok(mboxes)) => mboxes,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert!(
            mboxes
                .iter()
                .any(|m| m.path().as_str() == "/tmp/store/inbox")
        );
    }

    #[test]
    fn unexpected_arg_returns_unexpected_arg_error() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut list = M2dirList::new(&store, M2dirListOptions::default());

        let err = expect_complete_err(&mut list, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirListError::UnexpectedArg));
    }

    #[test]
    fn missing_arg_at_checking_markers_returns_missing_arg_error() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut list = M2dirList::new(&store, M2dirListOptions::default());

        let _ = expect_wants_dir_read(&mut list, None);

        let mut names = BTreeSet::new();
        names.insert(M2dirPath::from("/tmp/store/maybe"));
        let mut reply = BTreeMap::new();
        reply.insert(M2dirPath::from("/tmp/store"), names);

        let _ = expect_wants_file_exists(&mut list, Some(M2dirArg::DirRead(reply)));

        let err = expect_complete_err(&mut list, None);
        assert!(matches!(err, M2dirListError::MissingArg));
    }

    // --- utils

    fn expect_wants_dir_read(cor: &mut M2dirList, arg: Option<M2dirArg>) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        }
    }

    fn expect_wants_file_exists(cor: &mut M2dirList, arg: Option<M2dirArg>) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes)) => probes,
            state => panic!("expected WantsFileExists, got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirList, arg: Option<M2dirArg>) -> M2dirListError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
