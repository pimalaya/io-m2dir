//! I/O-free coroutine to replace an m2dir entry's flags metadata
//! file with a new flag set.
//!
//! If the new set is empty the metadata file is removed; otherwise
//! it is overwritten.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::fs;
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     flag::{
//!         set::{M2dirFlagSet, M2dirFlagSetOptions},
//!         types::M2dirFlags,
//!     },
//!     m2dir::types::M2dir,
//! };
//!
//! let m2dir = M2dir::from_path("/tmp/inbox");
//! let opts = M2dirFlagSetOptions::default();
//! let mut flags = M2dirFlags::default();
//! flags.insert("$seen");
//! let mut coroutine = M2dirFlagSet::new(&m2dir, "entry-id", flags, opts);
//! let mut arg = None;
//!
//! loop {
//!     match coroutine.resume(arg.take()) {
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

use core::fmt;

use alloc::collections::{BTreeMap, BTreeSet};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, flag::types::M2dirFlags, m2dir::types::M2dir, path::M2dirPath};

/// Failure causes during the m2dir flag SET flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirFlagSetError {
    #[error("M2DIR SET FLAGS failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR SET FLAGS failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirFlagSet::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirFlagSetOptions {}

/// I/O-free m2dir flag SET coroutine.
pub struct M2dirFlagSet {
    flags_path: M2dirPath,
    flags: M2dirFlags,
    state: State,
    #[allow(dead_code)]
    opts: M2dirFlagSetOptions,
}

impl M2dirFlagSet {
    /// Creates a new coroutine that will replace the flags metadata
    /// file for entry `id` inside `m2dir` with `flags`.
    pub fn new(
        m2dir: &M2dir,
        id: impl AsRef<str>,
        flags: M2dirFlags,
        opts: M2dirFlagSetOptions,
    ) -> Self {
        Self {
            flags_path: m2dir.flags_path(id.as_ref()),
            flags,
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirFlagSet {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirFlagSetError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("set flags: {}", self.state);

        match (&self.state, arg) {
            (State::Start, None) => {
                self.state = State::Done;

                if self.flags.is_empty() {
                    trace!("wants flags remove at {}", self.flags_path);
                    let paths = BTreeSet::from_iter([self.flags_path.clone()]);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths))
                } else {
                    trace!(
                        "wants flags write at {} ({} flags)",
                        self.flags_path,
                        self.flags.len(),
                    );
                    let serialized = self.flags.to_meta().into_bytes();
                    let files = BTreeMap::from_iter([(self.flags_path.clone(), serialized)]);
                    M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
                }
            }
            (State::Done, Some(M2dirArg::FileCreate | M2dirArg::FileRemove)) => {
                trace!("flags set at {}", self.flags_path);
                M2dirCoroutineState::Complete(Ok(()))
            }
            (_, Some(_)) => {
                let err = M2dirFlagSetError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirFlagSetError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    Done,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Done => f.write_str("done"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_flags_yields_file_create_then_completes_ok() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$seen");

        let mut set = M2dirFlagSet::new(&m2dir, "entry", flags, M2dirFlagSetOptions::default());

        let files = expect_wants_file_create(&mut set, None);
        assert_eq!(files.len(), 1);
        let (path, bytes) = files.into_iter().next().unwrap();
        assert!(path.as_str().contains("entry.flags"));
        assert_eq!(core::str::from_utf8(&bytes).unwrap(), "$seen\n");

        expect_complete_ok(&mut set, Some(M2dirArg::FileCreate));
    }

    #[test]
    fn empty_flags_yields_file_remove_then_completes_ok() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut set = M2dirFlagSet::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagSetOptions::default(),
        );

        let paths = expect_wants_file_remove(&mut set, None);
        assert_eq!(paths.len(), 1);

        expect_complete_ok(&mut set, Some(M2dirArg::FileRemove));
    }

    #[test]
    fn missing_arg_at_done_returns_missing_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$seen");

        let mut set = M2dirFlagSet::new(&m2dir, "entry", flags, M2dirFlagSetOptions::default());
        let _ = expect_wants_file_create(&mut set, None);

        let err = expect_complete_err(&mut set, None);
        assert!(matches!(err, M2dirFlagSetError::MissingArg));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut set = M2dirFlagSet::new(
            &m2dir,
            "entry",
            M2dirFlags::default(),
            M2dirFlagSetOptions::default(),
        );

        let err = expect_complete_err(&mut set, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirFlagSetError::UnexpectedArg));
    }

    #[test]
    fn unexpected_arg_kind_at_done_returns_unexpected_arg_error() {
        let m2dir = M2dir::from_path("/tmp/inbox");
        let mut flags = M2dirFlags::default();
        flags.insert("$seen");

        let mut set = M2dirFlagSet::new(&m2dir, "entry", flags, M2dirFlagSetOptions::default());
        let _ = expect_wants_file_create(&mut set, None);

        let err = expect_complete_err(&mut set, Some(M2dirArg::DirRemove));
        assert!(matches!(err, M2dirFlagSetError::UnexpectedArg));
    }

    // --- utils

    fn expect_wants_file_create(
        cor: &mut M2dirFlagSet,
        arg: Option<M2dirArg>,
    ) -> BTreeMap<M2dirPath, alloc::vec::Vec<u8>> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => files,
            state => panic!("expected WantsFileCreate, got {state:?}"),
        }
    }

    fn expect_wants_file_remove(
        cor: &mut M2dirFlagSet,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileRemove(paths)) => paths,
            state => panic!("expected WantsFileRemove, got {state:?}"),
        }
    }

    fn expect_complete_ok(cor: &mut M2dirFlagSet, arg: Option<M2dirArg>) {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Ok(())) => {}
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirFlagSet, arg: Option<M2dirArg>) -> M2dirFlagSetError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
