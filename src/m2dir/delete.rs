//! I/O-free coroutine to delete an m2dir and every entry it
//! contains.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::fs;
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::delete::{M2dirDelete, M2dirDeleteOptions},
//!     path::M2dirPath,
//! };
//!
//! let opts = M2dirDeleteOptions::default();
//! let path: M2dirPath = "/tmp/inbox".into();
//! let mut coroutine = M2dirDelete::new(path, opts);
//! let mut arg = None;
//!
//! loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsDirRemove(paths)) => {
//!             for path in paths {
//!                 let _ = fs::remove_dir_all(path.as_str());
//!             }
//!             arg = Some(M2dirArg::DirRemove);
//!         }
//!         M2dirCoroutineState::Complete(Ok(())) => break,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! }
//! ```

use core::fmt;

use alloc::collections::BTreeSet;

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, path::M2dirPath};

/// Failure causes during the m2dir DELETE flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirDeleteError {
    #[error("M2DIR DELETE failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR DELETE failed: missing coroutine arg")]
    MissingArg,
}

/// Options for [`M2dirDelete::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirDeleteOptions {}

/// I/O-free m2dir DELETE coroutine.
pub struct M2dirDelete {
    path: M2dirPath,
    state: State,
    #[allow(dead_code)]
    opts: M2dirDeleteOptions,
}

impl M2dirDelete {
    /// Creates a new coroutine that will recursively remove the
    /// m2dir at `path`.
    pub fn new(path: impl Into<M2dirPath>, opts: M2dirDeleteOptions) -> Self {
        Self {
            path: path.into(),
            state: State::Start,
            opts,
        }
    }
}

impl M2dirCoroutine for M2dirDelete {
    type Yield = M2dirYield;
    type Return = Result<(), M2dirDeleteError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("delete m2dir: {}", self.state);

        match (&self.state, arg) {
            (State::Start, None) => {
                let paths = BTreeSet::from_iter([self.path.clone()]);
                trace!("wants directory removal at {}", self.path);
                self.state = State::Removed;
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirRemove(paths))
            }
            (State::Removed, Some(M2dirArg::DirRemove)) => {
                trace!("m2dir removed at {}", self.path);
                M2dirCoroutineState::Complete(Ok(()))
            }
            (_, Some(_)) => {
                let err = M2dirDeleteError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirDeleteError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    Removed,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Removed => f.write_str("removed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_returns_ok() {
        let mut delete = M2dirDelete::new("/tmp/inbox", M2dirDeleteOptions::default());

        let paths = expect_wants_dir_remove(&mut delete, None);
        assert_eq!(paths.len(), 1);
        assert!(paths.contains(&M2dirPath::from("/tmp/inbox")));

        expect_complete_ok(&mut delete, Some(M2dirArg::DirRemove));
    }

    #[test]
    fn missing_arg_at_removed_state_returns_missing_arg_error() {
        let mut delete = M2dirDelete::new("/tmp/inbox", M2dirDeleteOptions::default());
        let _ = expect_wants_dir_remove(&mut delete, None);

        let err = expect_complete_err(&mut delete, None);
        assert!(matches!(err, M2dirDeleteError::MissingArg));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let mut delete = M2dirDelete::new("/tmp/inbox", M2dirDeleteOptions::default());

        let err = expect_complete_err(&mut delete, Some(M2dirArg::DirRemove));
        assert!(matches!(err, M2dirDeleteError::UnexpectedArg));
    }

    #[test]
    fn unexpected_arg_kind_at_removed_state_returns_unexpected_arg_error() {
        let mut delete = M2dirDelete::new("/tmp/inbox", M2dirDeleteOptions::default());
        let _ = expect_wants_dir_remove(&mut delete, None);

        let err = expect_complete_err(&mut delete, Some(M2dirArg::FileCreate));
        assert!(matches!(err, M2dirDeleteError::UnexpectedArg));
    }

    #[test]
    fn first_yield_carries_the_target_path() {
        let mut delete = M2dirDelete::new("/tmp/some/deep/inbox", M2dirDeleteOptions::default());

        let paths = expect_wants_dir_remove(&mut delete, None);
        assert_eq!(
            paths.iter().next().map(M2dirPath::as_str),
            Some("/tmp/some/deep/inbox")
        );
    }

    // --- utils

    fn expect_wants_dir_remove(
        cor: &mut M2dirDelete,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirRemove(paths)) => paths,
            state => panic!("expected WantsDirRemove, got {state:?}"),
        }
    }

    fn expect_complete_ok(cor: &mut M2dirDelete, arg: Option<M2dirArg>) {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Ok(())) => {}
            state => panic!("expected Complete(Ok), got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirDelete, arg: Option<M2dirArg>) -> M2dirDeleteError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
