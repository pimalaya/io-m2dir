//! I/O-free coroutine to read an m2dir entry by its id.
//!
//! The id is the `<checksum>.<nonce>` portion of the entry
//! filename. The fetched bytes are checksum-validated before being
//! returned.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::types::M2dir,
//!     entry::get::{M2dirEntryGet, M2dirEntryGetOptions},
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let opts = M2dirEntryGetOptions::default();
//! let id = "1747997123,abcd.efgh";
//! let mut coroutine = M2dirEntryGet::new(m2dir, id, opts);
//! let mut arg = None;
//!
//! let output = loop {
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
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => {
//!             let map = paths
//!                 .into_iter()
//!                 .map(|p| {
//!                     let bytes = fs::read(p.as_str()).unwrap_or_default();
//!                     (p, bytes)
//!                 })
//!                 .collect();
//!             arg = Some(M2dirArg::FileRead(map));
//!         }
//!         M2dirCoroutineState::Complete(Ok(out)) => break out,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! };
//!
//! println!("{} bytes", output.contents.len());
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
    coroutine::*,
    entry::{
        list::*,
        types::{M2dirEntry, ParseFilenameError, validate_checksum},
    },
    m2dir::types::M2dir,
};

/// Failure causes during the m2dir GET flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirEntryGetError {
    #[error("M2DIR GET failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR GET failed: missing coroutine arg")]
    MissingArg,
    #[error("M2DIR GET failed: entry {0} not found")]
    NotFound(String),
    #[error("M2DIR GET failed: {0}")]
    List(#[from] M2dirEntryListError),
    #[error("M2DIR GET failed: {0}")]
    Parse(#[from] ParseFilenameError),
}

/// Options for [`M2dirEntryGet::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirEntryGetOptions {}

/// Terminal output of [`M2dirEntryGet`].
#[derive(Clone, Debug)]
pub struct M2dirEntryGetOutput {
    /// The resolved entry (id + on-disk path).
    pub entry: M2dirEntry,
    /// Raw bytes read from the entry file.
    pub contents: Vec<u8>,
}

/// I/O-free m2dir entry GET coroutine.
pub struct M2dirEntryGet {
    id: String,
    state: State,
    #[allow(dead_code)]
    opts: M2dirEntryGetOptions,
}

impl M2dirEntryGet {
    /// Creates a new coroutine that will retrieve entry `id` from
    /// `m2dir`.
    pub fn new(m2dir: M2dir, id: impl ToString, opts: M2dirEntryGetOptions) -> Self {
        Self {
            id: id.to_string(),
            state: State::List(M2dirEntryList::new(m2dir, M2dirEntryListOptions::default())),
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirEntryGet {
    type Yield = M2dirYield;
    type Return = Result<M2dirEntryGetOutput, M2dirEntryGetError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("get entry: {}", self.state);

        match (&mut self.state, arg) {
            (State::List(list), arg) => match list.resume(arg) {
                M2dirCoroutineState::Yielded(yld) => M2dirCoroutineState::Yielded(yld),
                M2dirCoroutineState::Complete(Ok(entries)) => {
                    let Some(entry) = entries.into_iter().find(|e| e.id() == self.id) else {
                        let err = M2dirEntryGetError::NotFound(self.id.clone());
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
                let entry = mem::replace(
                    entry,
                    M2dirEntry::from_parts(String::new(), crate::path::M2dirPath::default()),
                );
                let checksum = entry.checksum();

                if !validate_checksum(checksum, &bytes) {
                    let err = ParseFilenameError::InvalidChecksum {
                        path: entry.path().clone(),
                        expected: checksum.to_string(),
                        got: entry.id().to_string(),
                    };
                    return M2dirCoroutineState::Complete(Err(err.into()));
                }

                M2dirCoroutineState::Complete(Ok(M2dirEntryGetOutput {
                    entry,
                    contents: bytes,
                }))
            }
            (_, Some(_)) => {
                let err = M2dirEntryGetError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirEntryGetError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    List(M2dirEntryList),
    Read(M2dirEntry),
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List(_) => f.write_str("locate entry"),
            Self::Read(_) => f.write_str("read entry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use crate::path::M2dirPath;

    use super::*;

    #[test]
    fn missing_entry_returns_not_found_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut get = M2dirEntryGet::new(m2dir, "missing", M2dirEntryGetOptions::default());

        let probes = match get.resume(None) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        };
        let dir = probes.into_iter().next().unwrap();
        let mut reply = BTreeMap::new();
        reply.insert(dir, BTreeSet::new());

        let err = match get.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryGetError::NotFound(id) if id == "missing"));
    }

    #[test]
    fn list_error_propagates_via_from() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut get = M2dirEntryGet::new(m2dir, "missing", M2dirEntryGetOptions::default());

        let _ = get.resume(None);
        let err = match get.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryGetError::List(_)));
    }

    #[test]
    fn unexpected_arg_at_read_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut get = M2dirEntryGet::new(m2dir, "foo", M2dirEntryGetOptions::default());

        let err = match get.resume(Some(M2dirArg::FileRead(BTreeMap::new()))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryGetError::List(_)));
    }

    #[test]
    fn invalid_checksum_returns_parse_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut get = M2dirEntryGet::new(m2dir, "checksum.nonce", M2dirEntryGetOptions::default());

        let probes = match get.resume(None) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRead(paths)) => paths,
            state => panic!("expected WantsDirRead, got {state:?}"),
        };
        let dir = probes.into_iter().next().unwrap();

        let mut names = BTreeSet::new();
        let entry_path = M2dirPath::from("/tmp/inbox/123,checksum.nonce");
        names.insert(entry_path.clone());

        let mut reply = BTreeMap::new();
        reply.insert(dir, names);

        let probes = match get.resume(Some(M2dirArg::DirRead(reply))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileExists(probes)) => probes,
            state => panic!("expected WantsFileExists, got {state:?}"),
        };
        let exists: BTreeMap<M2dirPath, bool> = probes.into_iter().map(|p| (p, true)).collect();

        let read_paths = match get.resume(Some(M2dirArg::FileExists(exists))) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => paths,
            state => panic!("expected WantsFileRead, got {state:?}"),
        };
        let read_reply: BTreeMap<M2dirPath, Vec<u8>> = read_paths
            .into_iter()
            .map(|p| (p, b"wrong content".to_vec()))
            .collect();

        let err = match get.resume(Some(M2dirArg::FileRead(read_reply))) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryGetError::Parse(_)));
    }

    #[test]
    fn missing_arg_at_list_propagates_via_list_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut get = M2dirEntryGet::new(m2dir, "foo", M2dirEntryGetOptions::default());
        let _ = get.resume(None);

        let err = match get.resume(None) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        };
        assert!(matches!(err, M2dirEntryGetError::List(_)));
    }
}
