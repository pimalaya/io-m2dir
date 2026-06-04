//! I/O-free coroutine to list every entry inside an m2dir.
//!
//! Dotfiles, sub-directories and filenames that do not match the
//! m2dir specification (no `,` separator) are skipped. Returned
//! entries are not checksum-verified; use [`M2dirEntryGet`] when
//! validation is required.
//!
//! [`M2dirEntryGet`]: crate::entry::get::M2dirEntryGet
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::types::M2dir,
//!     entry::list::{M2dirEntryList, M2dirEntryListOptions},
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let opts = M2dirEntryListOptions::default();
//! let mut coroutine = M2dirEntryList::new(m2dir, opts);
//! let mut arg = None;
//!
//! let entries = loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => {
//!             let mut out = BTreeMap::new();
//!             for path in paths {
//!                 let mut names = Default::default();
//!                 if let Ok(rd) = fs::read_dir(path.as_str()) {
//!                     names = rd.flatten().map(|e| e.path().into()).collect();
//!                 }
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
//!         M2dirCoroutineState::Complete(Ok(entries)) => break entries,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! };
//!
//! println!("{} entries", entries.len());
//! ```

use core::{fmt, mem};

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, entry::types::M2dirEntry, m2dir::types::M2dir, path::M2dirPath};

/// Failure causes during the m2dir LIST flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirEntryListError {
    #[error("M2DIR LIST failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR LIST failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirEntryList::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirEntryListOptions {}

/// I/O-free m2dir entry LIST coroutine.
pub struct M2dirEntryList {
    m2dir: M2dir,
    state: State,
    #[allow(dead_code)]
    opts: M2dirEntryListOptions,
}

impl M2dirEntryList {
    /// Creates a new coroutine that will list every entry inside
    /// `m2dir`.
    pub fn new(m2dir: M2dir, opts: M2dirEntryListOptions) -> Self {
        Self {
            m2dir,
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirEntryList {
    type Yield = M2dirYield;
    type Return = Result<Vec<M2dirEntry>, M2dirEntryListError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("list entries: {}", self.state);

        match (&mut self.state, arg) {
            (State::Start, None) => {
                trace!("wants directory read of {}", self.m2dir.path());
                let paths = BTreeSet::from_iter([self.m2dir.path().clone()]);
                self.state = State::Reading;
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths))
            }
            (State::Reading, Some(M2dirArg::DirRead(entries))) => {
                let mut candidates = BTreeMap::new();

                for (_dir, names) in entries {
                    for path in names {
                        let Some(name) = path.file_name() else {
                            continue;
                        };

                        if name.starts_with('.') {
                            continue;
                        }

                        let Some(id) = M2dir::parse_filename_id(name) else {
                            trace!("skipping unparseable entry filename: {name}");
                            continue;
                        };

                        candidates.insert(path.clone(), id.to_string());
                    }
                }

                if candidates.is_empty() {
                    trace!("no candidate entries");
                    return M2dirCoroutineState::Complete(Ok(Vec::new()));
                }

                let probes: BTreeSet<M2dirPath> = candidates.keys().cloned().collect();
                trace!("wants existence check for {} candidates", probes.len());

                self.state = State::Checking { candidates };
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes))
            }
            (State::Checking { candidates }, Some(M2dirArg::FileExists(probes))) => {
                let candidates = mem::take(candidates);
                let mut found = Vec::new();

                for (path, id) in candidates {
                    if probes.get(&path).copied().unwrap_or(false) {
                        found.push(M2dirEntry::from_parts(id, path));
                    }
                }

                trace!("found {} entries", found.len());
                M2dirCoroutineState::Complete(Ok(found))
            }
            (_, Some(_)) => {
                let err = M2dirEntryListError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirEntryListError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    Reading,
    Checking {
        candidates: BTreeMap<M2dirPath, String>,
    },
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Reading => f.write_str("reading directory"),
            Self::Checking { .. } => f.write_str("checking candidates"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_directory_returns_empty_list() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut list = M2dirEntryList::new(m2dir, M2dirEntryListOptions::default());

        let probes = expect_wants_dir_read(&mut list, None);
        let dir = probes.into_iter().next().unwrap();
        let mut reply = BTreeMap::new();
        reply.insert(dir, BTreeSet::new());

        let entries = match list.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Ok(entries)) => entries,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert!(entries.is_empty());
    }

    #[test]
    fn skips_unparseable_filenames_and_dotfiles() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut list = M2dirEntryList::new(m2dir, M2dirEntryListOptions::default());

        let probes = expect_wants_dir_read(&mut list, None);
        let dir = probes.into_iter().next().unwrap();

        let mut names = BTreeSet::new();
        names.insert(M2dirPath::from("/tmp/inbox/.meta"));
        names.insert(M2dirPath::from("/tmp/inbox/garbage"));

        let mut reply = BTreeMap::new();
        reply.insert(dir, names);

        let entries = match list.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Ok(entries)) => entries,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert!(entries.is_empty());
    }

    #[test]
    fn missing_arg_at_reading_returns_missing_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut list = M2dirEntryList::new(m2dir, M2dirEntryListOptions::default());
        let _ = expect_wants_dir_read(&mut list, None);

        let err = expect_complete_err(&mut list, None);
        assert!(matches!(err, M2dirEntryListError::MissingArg));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut list = M2dirEntryList::new(m2dir, M2dirEntryListOptions::default());

        let err = expect_complete_err(&mut list, Some(M2dirArg::DirCreate));
        assert!(matches!(err, M2dirEntryListError::UnexpectedArg));
    }

    #[test]
    fn unexpected_arg_kind_at_reading_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut list = M2dirEntryList::new(m2dir, M2dirEntryListOptions::default());
        let _ = expect_wants_dir_read(&mut list, None);

        let err = expect_complete_err(&mut list, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirEntryListError::UnexpectedArg));
    }

    // --- utils

    fn expect_wants_dir_read(
        cor: &mut M2dirEntryList,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirEntryList, arg: Option<M2dirArg>) -> M2dirEntryListError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
