//! I/O-free coroutine to delete an m2dir entry and every metadata
//! sibling carrying the same id prefix (`.flags`, `.notes`, …).
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::types::M2dir,
//!     entry::delete::{M2dirEntryDelete, M2dirEntryDeleteOptions},
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let opts = M2dirEntryDeleteOptions::default();
//! let mut coroutine = M2dirEntryDelete::new(m2dir, "id", opts);
//! let mut arg = None;
//!
//! loop {
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
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths)) => {
//!             for path in paths { let _ = fs::remove_file(path.as_str()); }
//!             arg = Some(M2dirArg::FileRemove);
//!         }
//!         M2dirCoroutineState::Complete(Ok(())) => break,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! }
//! ```

use core::{fmt, mem};

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*, entry::list::*, entry::types::M2dirEntry, m2dir::types::M2dir, path::M2dirPath,
};

/// Failure causes during the m2dir DELETE-ENTRY flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirEntryDeleteError {
    #[error("M2DIR DELETE failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR DELETE failed: missing coroutine arg")]
    MissingArg,
    #[error("M2DIR DELETE failed: entry {0} not found")]
    NotFound(String),
    #[error("M2DIR DELETE failed: {0}")]
    List(#[from] M2dirEntryListError),
}

/// Options for [`M2dirEntryDelete::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirEntryDeleteOptions {}

/// I/O-free m2dir entry DELETE coroutine.
pub struct M2dirEntryDelete {
    id: String,
    meta_dir: M2dirPath,
    state: State,
    #[allow(dead_code)]
    opts: M2dirEntryDeleteOptions,
}

impl M2dirEntryDelete {
    /// Creates a new coroutine that will delete entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString, opts: M2dirEntryDeleteOptions) -> Self {
        let meta_dir = m2dir.meta_dir();
        Self {
            id: id.to_string(),
            meta_dir,
            state: State::List(M2dirEntryList::new(m2dir, M2dirEntryListOptions::default())),
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirEntryDelete {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirEntryDeleteError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("delete entry: {}", self.state);

        match (&mut self.state, arg) {
            (State::List(list), arg) => match list.resume(arg) {
                M2dirCoroutineState::Yielded(yld) => M2dirCoroutineState::Yielded(yld),
                M2dirCoroutineState::Complete(Ok(entries)) => {
                    let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                        let err = M2dirEntryDeleteError::NotFound(self.id.clone());
                        return M2dirCoroutineState::Complete(Err(err));
                    };

                    trace!("located entry at {}", entry.path());

                    let paths = BTreeSet::from_iter([self.meta_dir.clone()]);
                    self.state = State::ReadMeta(entry);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths))
                }
                M2dirCoroutineState::Complete(Err(err)) => {
                    M2dirCoroutineState::Complete(Err(err.into()))
                }
            },
            (State::ReadMeta(entry), Some(M2dirArg::DirRead(entries))) => {
                let meta_names = entries.into_values().next().unwrap_or_default();
                let entry = mem::replace(
                    entry,
                    M2dirEntry::from_parts(String::new(), M2dirPath::default()),
                );

                let mut to_remove: Vec<M2dirPath> = Vec::new();
                to_remove.push(entry.path().clone());

                for path in meta_names {
                    let Some(name) = path.file_name() else {
                        continue;
                    };
                    if name.starts_with(&self.id) {
                        to_remove.push(path);
                    }
                }

                trace!("wants removal of {} files", to_remove.len());
                self.state = State::Removing;
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(BTreeSet::from_iter(
                    to_remove,
                )))
            }
            (State::Removing, Some(M2dirArg::FileRemove)) => {
                trace!("entry deleted");
                M2dirCoroutineState::Complete(Ok(()))
            }
            (_, Some(_)) => {
                let err = M2dirEntryDeleteError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirEntryDeleteError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    List(M2dirEntryList),
    ReadMeta(M2dirEntry),
    Removing,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List(_) => f.write_str("locate entry"),
            Self::ReadMeta(_) => f.write_str("read .meta directory"),
            Self::Removing => f.write_str("removing files"),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::*;

    #[test]
    fn missing_entry_returns_not_found_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut delete =
            M2dirEntryDelete::new(m2dir, "missing", M2dirEntryDeleteOptions::default());

        let probes = match delete.resume(None) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        };
        let dir = probes.into_iter().next().unwrap();
        let mut reply = BTreeMap::new();
        reply.insert(dir, BTreeSet::new());

        let err = match delete.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryDeleteError::NotFound(id) if id == "missing"));
    }

    #[test]
    fn list_error_propagates_via_from() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut delete = M2dirEntryDelete::new(m2dir, "x", M2dirEntryDeleteOptions::default());

        let _ = delete.resume(None);
        let err = match delete.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryDeleteError::List(_)));
    }

    #[test]
    fn unexpected_arg_at_list_initial_returns_list_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut delete = M2dirEntryDelete::new(m2dir, "x", M2dirEntryDeleteOptions::default());

        let err = match delete.resume(Some(M2dirArg::FileRead(BTreeMap::new()))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryDeleteError::List(_)));
    }

    #[test]
    fn missing_arg_at_list_propagates_via_list_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut delete = M2dirEntryDelete::new(m2dir, "x", M2dirEntryDeleteOptions::default());
        let _ = delete.resume(None);

        let err = match delete.resume(None) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryDeleteError::List(_)));
    }

    #[test]
    fn read_meta_unexpected_arg_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut delete = M2dirEntryDelete::new(m2dir, "any", M2dirEntryDeleteOptions::default());

        let probes = match delete.resume(None) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        };
        let dir = probes.into_iter().next().unwrap();
        let mut names = BTreeSet::new();
        let entry_path = M2dirPath::from("/tmp/inbox/123,checksum.nonce");
        names.insert(entry_path.clone());
        let mut reply = BTreeMap::new();
        reply.insert(dir, names);

        let probes = match delete.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes)) => probes,
            state => panic!("expected WantsFileExists, got {state:?}"),
        };
        let exists: BTreeMap<M2dirPath, bool> = probes.into_iter().map(|p| (p, true)).collect();

        delete.id = "checksum.nonce".into();

        let _ = match delete.resume(Some(M2dirArg::FileExists(exists))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        };

        let err = match delete.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryDeleteError::UnexpectedArg));
    }
}
