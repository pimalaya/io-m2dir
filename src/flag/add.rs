//! I/O-free coroutine to add flags to an m2dir entry's flags
//! metadata file.
//!
//! Reads the existing `.flags` payload (if any), unions it with the
//! caller-supplied flags, and writes the merged set back.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::{collections::BTreeMap, fs};
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     flag::{
//!         add::{M2dirFlagAdd, M2dirFlagAddOptions},
//!         types::M2dirFlags,
//!     },
//!     m2dir::types::M2dir,
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let mut flags = M2dirFlags::default();
//! flags.insert("$seen");
//! let opts = M2dirFlagAddOptions::default();
//! let mut coroutine = M2dirFlagAdd::new(&m2dir, "entry-id", flags, opts);
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

/// Failure causes during the m2dir flag ADD flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagAddError {
    #[error("M2DIR ADD FLAGS failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR ADD FLAGS failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirFlagAdd::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirFlagAddOptions {}

/// I/O-free m2dir flag ADD coroutine.
pub struct M2dirFlagAdd {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
    #[allow(dead_code)]
    opts: M2dirFlagAddOptions,
}

impl M2dirFlagAdd {
    /// Creates a new coroutine that will add `flags` to the flags
    /// metadata file for entry `id` inside `m2dir`.
    pub fn new(
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
        opts: M2dirFlagAddOptions,
    ) -> Self {
        Self {
            flags_path: m2dir.flags_path(id.as_ref()),
            flags,
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirFlagAdd {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagAddError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("add flags: {}", self.state);

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

                let mut merged = M2dirFlags::from_meta(existing);
                merged.extend(self.flags.clone());

                trace!(
                    "wants flags write at {} ({} flags)",
                    self.flags_path,
                    merged.len(),
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
            (_, Some(_)) => {
                let err = M2dirFlagAddError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirFlagAddError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    Read,
    Written,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Read => f.write_str("read existing flags"),
            Self::Written => f.write_str("written merged flags"),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn merges_with_existing_flags() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$forwarded");

        let mut add = M2dirFlagAdd::new(&m2dir, "entry", flags, M2dirFlagAddOptions::default());

        let probes = expect_wants_file_read(&mut add, None);
        let existing_path = probes.into_iter().next().unwrap();
        let mut reply = BTreeMap::new();
        reply.insert(existing_path, b"$seen\n".to_vec());

        let files = expect_wants_file_create(&mut add, Some(M2dirArg::FileRead(reply)));
        let (_, bytes) = files.into_iter().next().unwrap();
        let serialized = str::from_utf8(&bytes).unwrap();
        assert!(serialized.contains("$seen"));
        assert!(serialized.contains("$forwarded"));

        expect_complete_ok(&mut add, Some(M2dirArg::FileCreate));
    }

    #[test]
    fn empty_existing_writes_only_the_new_flags() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$seen");

        let mut add = M2dirFlagAdd::new(&m2dir, "entry", flags, M2dirFlagAddOptions::default());

        let probes = expect_wants_file_read(&mut add, None);
        let path = probes.into_iter().next().unwrap();
        let reply = BTreeMap::from_iter([(path, Vec::<u8>::new())]);

        let files = expect_wants_file_create(&mut add, Some(M2dirArg::FileRead(reply)));
        let (_, bytes) = files.into_iter().next().unwrap();
        assert_eq!(str::from_utf8(&bytes).unwrap(), "$seen\n");

        expect_complete_ok(&mut add, Some(M2dirArg::FileCreate));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut add = M2dirFlagAdd::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagAddOptions::default(),
        );

        let err = expect_complete_err(&mut add, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirFlagAddError::UnexpectedArg));
    }

    #[test]
    fn missing_arg_at_read_returns_missing_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut add = M2dirFlagAdd::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagAddOptions::default(),
        );
        let _ = expect_wants_file_read(&mut add, None);

        let err = expect_complete_err(&mut add, None);
        assert!(matches!(err, M2dirFlagAddError::MissingArg));
    }

    #[test]
    fn unexpected_arg_kind_at_written_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$seen");

        let mut add = M2dirFlagAdd::new(&m2dir, "entry", flags, M2dirFlagAddOptions::default());

        let probes = expect_wants_file_read(&mut add, None);
        let path = probes.into_iter().next().unwrap();
        let reply = BTreeMap::from_iter([(path, Vec::<u8>::new())]);
        let _ = expect_wants_file_create(&mut add, Some(M2dirArg::FileRead(reply)));

        let err = expect_complete_err(&mut add, Some(M2dirArg::DirRemove));
        assert!(matches!(err, M2dirFlagAddError::UnexpectedArg));
    }

    // --- utils

    fn expect_wants_file_read(
        cor: &mut M2dirFlagAdd,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileRead(paths)) => paths,
            state => panic!("expected WantsFileRead, got {state:?}"),
        }
    }

    fn expect_wants_file_create(
        cor: &mut M2dirFlagAdd,
        arg: Option<M2dirArg>,
    ) -> BTreeMap<M2dirPath, Vec<u8>> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => files,
            state => panic!("expected WantsFileCreate, got {state:?}"),
        }
    }

    fn expect_complete_ok(cor: &mut M2dirFlagAdd, arg: Option<M2dirArg>) {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Ok(())) => {}
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirFlagAdd, arg: Option<M2dirArg>) -> M2dirFlagAddError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
