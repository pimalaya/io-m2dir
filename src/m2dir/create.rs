//! I/O-free coroutine to create an m2dir: the folder, its
//! `.meta` sub-directory and the `.m2dir` marker file.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::fs;
//!
//! use io_m2dir::{
//!     coroutine::{M2dirArg, M2dirCoroutine, M2dirCoroutineState, M2dirYield},
//!     m2dir::create::{M2dirCreate, M2dirCreateOptions},
//!     store::M2dirStore,
//! };
//!
//! let store = M2dirStore::from_path("/tmp/store");
//! let opts = M2dirCreateOptions::default();
//! let mut coroutine = M2dirCreate::new(&store, "inbox", opts).unwrap();
//! let mut arg = None;
//!
//! loop {
//!     match coroutine.resume(arg.take()) {
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsDirCreate(paths)) => {
//!             for path in paths {
//!                 fs::create_dir_all(path.as_str()).unwrap();
//!             }
//!             arg = Some(M2dirArg::DirCreate);
//!         }
//!         M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => {
//!             for (path, bytes) in files {
//!                 fs::write(path.as_str(), bytes).unwrap();
//!             }
//!             arg = Some(M2dirArg::FileCreate);
//!         }
//!         M2dirCoroutineState::Complete(Ok(_)) => break,
//!         M2dirCoroutineState::Complete(Err(err)) => panic!("{err}"),
//!         state => panic!("unexpected state {state:?}"),
//!     }
//! }
//! ```

use core::fmt;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{
    coroutine::*,
    m2dir::types::M2dir,
    store::{M2dirStore, M2dirStoreError},
};

/// Failure causes during the m2dir CREATE flow.
#[derive(Clone, Debug, Error)]
pub enum M2dirCreateError {
    #[error("M2DIR CREATE failed: unexpected coroutine arg")]
    UnexpectedArg,
    #[error("M2DIR CREATE failed: missing coroutine arg")]
    MissingArg,
    #[error(transparent)]
    Resolve(#[from] M2dirStoreError),
}

/// Options for [`M2dirCreate::new`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct M2dirCreateOptions {}

/// I/O-free m2dir CREATE coroutine.
pub struct M2dirCreate {
    m2dir: M2dir,
    state: State,
    #[allow(dead_code)]
    opts: M2dirCreateOptions,
}

impl M2dirCreate {
    /// Creates a new coroutine that will create the folder named
    /// `name` inside `store`.
    pub fn new(
        store: &M2dirStore,
        name: &str,
        opts: M2dirCreateOptions,
    ) -> Result<Self, M2dirStoreError> {
        let path = store.resolve_folder_path(name)?;
        let m2dir = M2dir::from_path(path);

        Ok(Self {
            m2dir,
            state: State::Start,
            opts,
        })
    }

    /// Returns the [`M2dir`] this coroutine targets.
    pub fn m2dir(&self) -> &M2dir {
        &self.m2dir
    }
}

impl M2dirCoroutine for M2dirCreate {
    type Yield = M2dirYield;
    type Return = Result<M2dir, M2dirCreateError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        trace!("create m2dir: {}", self.state);

        match (&self.state, arg) {
            (State::Start, None) => {
                trace!("wants directory creation for {}", self.m2dir.path());

                let root = self.m2dir.path().clone();
                let meta = self.m2dir.meta_dir();
                let paths = BTreeSet::from_iter([root, meta]);

                self.state = State::DirCreated;
                M2dirCoroutineState::Yielded(M2dirYield::WantsDirCreate(paths))
            }
            (State::DirCreated, Some(M2dirArg::DirCreate)) => {
                trace!("wants marker file at {}", self.m2dir.marker_path());

                let marker = self.m2dir.marker_path();
                let files = BTreeMap::from_iter([(marker, Vec::new())]);

                self.state = State::MarkerWritten;
                M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files))
            }
            (State::MarkerWritten, Some(M2dirArg::FileCreate)) => {
                trace!("m2dir created at {}", self.m2dir.path());
                M2dirCoroutineState::Complete(Ok(self.m2dir.clone()))
            }
            (_, Some(_)) => {
                let err = M2dirCreateError::UnexpectedArg;
                M2dirCoroutineState::Complete(Err(err))
            }
            (_, None) => {
                let err = M2dirCreateError::MissingArg;
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}

enum State {
    Start,
    DirCreated,
    MarkerWritten,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::DirCreated => f.write_str("directory created"),
            Self::MarkerWritten => f.write_str("marker written"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::path::M2dirPath;

    use super::*;

    #[test]
    fn success_returns_the_new_m2dir() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut create =
            M2dirCreate::new(&store, "inbox", M2dirCreateOptions::default()).expect("valid name");

        let paths = expect_wants_dir_create(&mut create, None);
        assert!(paths.iter().any(|p| p.as_str() == "/tmp/store/inbox"));
        assert!(paths.iter().any(|p| p.as_str() == "/tmp/store/inbox/.meta"));

        let files = expect_wants_file_create(&mut create, Some(M2dirArg::DirCreate));
        assert!(
            files
                .keys()
                .any(|p| p.as_str() == "/tmp/store/inbox/.m2dir")
        );

        let m2dir = match create.resume(Some(M2dirArg::FileCreate)) {
            M2dirCoroutineState::Complete(Ok(m2dir)) => m2dir,
            state => panic!("expected Complete(Ok), got {state:?}"),
        };
        assert_eq!(m2dir.path().as_str(), "/tmp/store/inbox");
    }

    #[test]
    fn escaping_name_returns_resolve_error_at_construction() {
        let store = M2dirStore::from_path("/tmp/store");
        let result = M2dirCreate::new(&store, "../escape", M2dirCreateOptions::default());
        assert!(matches!(result, Err(M2dirStoreError::EscapesRoot(_))));
    }

    #[test]
    fn unexpected_arg_at_start_returns_unexpected_arg_error() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut create = M2dirCreate::new(&store, "inbox", M2dirCreateOptions::default()).unwrap();

        let err = expect_complete_err(&mut create, Some(M2dirArg::DirCreate));
        assert!(matches!(err, M2dirCreateError::UnexpectedArg));
    }

    #[test]
    fn missing_arg_at_dir_created_returns_missing_arg_error() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut create = M2dirCreate::new(&store, "inbox", M2dirCreateOptions::default()).unwrap();
        let _ = expect_wants_dir_create(&mut create, None);

        let err = expect_complete_err(&mut create, None);
        assert!(matches!(err, M2dirCreateError::MissingArg));
    }

    #[test]
    fn unexpected_arg_kind_at_marker_written_returns_unexpected_arg_error() {
        let store = M2dirStore::from_path("/tmp/store");
        let mut create = M2dirCreate::new(&store, "inbox", M2dirCreateOptions::default()).unwrap();
        let _ = expect_wants_dir_create(&mut create, None);
        let _ = expect_wants_file_create(&mut create, Some(M2dirArg::DirCreate));

        let err = expect_complete_err(&mut create, Some(M2dirArg::DirCreate));
        assert!(matches!(err, M2dirCreateError::UnexpectedArg));
    }

    // --- utils

    fn expect_wants_dir_create(
        cor: &mut M2dirCreate,
        arg: Option<M2dirArg>,
    ) -> BTreeSet<M2dirPath> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsDirCreate(paths)) => paths,
            state => panic!("expected WantsDirCreate, got {state:?}"),
        }
    }

    fn expect_wants_file_create(
        cor: &mut M2dirCreate,
        arg: Option<M2dirArg>,
    ) -> BTreeMap<M2dirPath, Vec<u8>> {
        match cor.resume(arg) {
            M2dirCoroutineState::Yielded(M2dirYield::WantsFileCreate(files)) => files,
            state => panic!("expected WantsFileCreate, got {state:?}"),
        }
    }

    fn expect_complete_err(cor: &mut M2dirCreate, arg: Option<M2dirArg>) -> M2dirCreateError {
        match cor.resume(arg) {
            M2dirCoroutineState::Complete(Err(err)) => err,
            state => panic!("expected Complete(Err), got {state:?}"),
        }
    }
}
