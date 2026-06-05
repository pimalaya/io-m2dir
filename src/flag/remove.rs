//! I/O-free coroutine to remove flags from an m2dir entry's flags
//! metadata file.
//!
//! Reads the existing `.flags` payload, subtracts the caller-supplied
//! flags, and writes the remaining set back. If the result is empty
//! the file is removed.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     flag::{
//!         remove::{M2dirFlagRemove, M2dirFlagRemoveOptions},
//!         types::M2dirFlags,
//!     },
//!     m2dir::types::M2dir,
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let mut flags = M2dirFlags::default();
//! flags.insert("$seen");
//! let opts = M2dirFlagRemoveOptions::default();
//! let mut coroutine = M2dirFlagRemove::new(&m2dir, "entry-id", flags, opts);
//! let mut arg = None;
//!
//! loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => {
//!             let mut out = BTreeMap::new();
//!             for path in paths {
//!                 let bytes = fs::read(path.as_str()).unwrap_or_default();
//!                 out.insert(path, bytes);
//!             }
//!             arg = Some(M2dirArg::FileRead(out));
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => {
//!             for (path, bytes) in files {
//!                 fs::write(path.as_str(), bytes).unwrap();
//!             }
//!             arg = Some(M2dirArg::FileCreate);
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths)) => {
//!             for path in paths {
//!                 let _ = fs::remove_file(path.as_str());
//!             }
//!             arg = Some(M2dirArg::FileRemove);
//!         }
//!         M2dirCoroutineState::Complete(Ok(())) => break,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! }
//! ```

use core::{fmt, str};

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, flag::types::M2dirFlags, m2dir::types::M2dir, path::M2dirPath};

/// Failure causes during the m2dir flag REMOVE flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagRemoveError {
    #[error("M2DIR REMOVE FLAGS failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR REMOVE FLAGS failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirFlagRemove::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirFlagRemoveOptions {}

/// I/O-free m2dir flag REMOVE coroutine.
pub struct M2dirFlagRemove {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
    #[allow(dead_code)]
    opts: M2dirFlagRemoveOptions,
}

impl M2dirFlagRemove {
    /// Creates a new coroutine that will remove `flags` from the
    /// flags metadata file for entry `id` inside `m2dir`.
    pub fn new(
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
        opts: M2dirFlagRemoveOptions,
    ) -> Self {
        Self {
            flags_path: m2dir.flags_path(id.as_ref()),
            flags,
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirFlagRemove {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagRemoveError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("remove flags: {}", self.state);

        match (&self.state, arg) {
            (State::Start, None) => {
                trace!("wants existing flags read at {}", self.flags_path);
                let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                self.state = State::Read;
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths))
            }
            (State::Read, Some(M2dirArg::FileRead(contents))) => {
                let bytes = contents.into_values().next().unwrap_or_default();
                let existing = str::from_utf8(&bytes).unwrap_or("");

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
            (_, Some(_)) => {
                let err = M2dirFlagRemoveError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirFlagRemoveError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    Read,
    Done,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Read => f.write_str("read existing flags"),
            Self::Done => f.write_str("done"),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn subtracts_flags_and_writes_remainder() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut to_remove = M2dirFlags::default();
        to_remove.insert("$seen");

        let mut rm = M2dirFlagRemove::new(
            &m2dir,
            "entry",
            to_remove,
            M2dirFlagRemoveOptions::default(),
        );

        let probes = expect_wants_file_read(&mut rm, None);
        let path = probes.into_iter().next().unwrap();
        let reply = BTreeMap::from_iter([(path, b"$seen\n$forwarded\n".to_vec())]);

        let files = expect_wants_file_create(&mut rm, Some(M2dirArg::FileRead(reply)));
        let (_, bytes) = files.into_iter().next().unwrap();
        let serialized = str::from_utf8(&bytes).unwrap();
        assert!(!serialized.contains("$seen"));
        assert!(serialized.contains("$forwarded"));

        expect_complete_ok(&mut rm, Some(M2dirArg::FileCreate));
    }

    #[test]
    fn empty_remainder_removes_the_flags_file() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut to_remove = M2dirFlags::default();
        to_remove.insert("$seen");

        let mut rm = M2dirFlagRemove::new(
            &m2dir,
            "entry",
            to_remove,
            M2dirFlagRemoveOptions::default(),
        );

        let probes = expect_wants_file_read(&mut rm, None);
        let path = probes.into_iter().next().unwrap();
        let reply = BTreeMap::from_iter([(path, b"$seen\n".to_vec())]);

        let _ = expect_wants_file_remove(&mut rm, Some(M2dirArg::FileRead(reply)));

        expect_complete_ok(&mut rm, Some(M2dirArg::FileRemove));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut rm = M2dirFlagRemove::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagRemoveOptions::default(),
        );

        let err = expect_complete_err(&mut rm, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirFlagRemoveError::UnexpectedArg));
    }

    #[test]
    fn missing_arg_at_read_returns_missing_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut rm = M2dirFlagRemove::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagRemoveOptions::default(),
        );
        let _ = expect_wants_file_read(&mut rm, None);

        let err = expect_complete_err(&mut rm, None);
        assert!(matches!(err, M2dirFlagRemoveError::MissingArg));
    }

    #[test]
    fn unexpected_arg_kind_at_done_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut to_remove = M2dirFlags::default();
        to_remove.insert("$seen");

        let mut rm = M2dirFlagRemove::new(
            &m2dir,
            "entry",
            to_remove,
            M2dirFlagRemoveOptions::default(),
        );

        let probes = expect_wants_file_read(&mut rm, None);
        let path = probes.into_iter().next().unwrap();
        let reply = BTreeMap::from_iter([(path, b"$seen\n".to_vec())]);
        let _ = expect_wants_file_remove(&mut rm, Some(M2dirArg::FileRead(reply)));

        let err = expect_complete_err(&mut rm, Some(M2dirArg::DirRemove));
        assert!(matches!(err, M2dirFlagRemoveError::UnexpectedArg));
    }

    // --- utils

    fn expect_wants_file_read(
        cor: &mut M2dirFlagRemove,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => paths,
            state => panic!("expected WantsFileRead, got {state:?}"),
        }
    }

    fn expect_wants_file_create(
        cor: &mut M2dirFlagRemove,
        arg: Option<M2dirArg>,
    ) -> BTreeMap<M2dirPath, Vec<u8>> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => files,
            state => panic!("expected WantsFileCreate, got {state:?}"),
        }
    }

    fn expect_wants_file_remove(
        cor: &mut M2dirFlagRemove,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths)) => paths,
            state => panic!("expected WantsFileRemove, got {state:?}"),
        }
    }

    fn expect_complete_ok(cor: &mut M2dirFlagRemove, arg: Option<M2dirArg>) {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Ok(())) => {}
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    fn expect_complete_err(
        cor: &mut M2dirFlagRemove,
        arg: Option<M2dirArg>,
    ) -> M2dirFlagRemoveError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
