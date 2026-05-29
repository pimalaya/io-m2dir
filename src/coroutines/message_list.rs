//! I/O-free coroutine to list entries inside an m2dir.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{coroutine::*, entry::M2dirEntry, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageListError {
    #[error("Invalid m2dir message list arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirArg>, State),
}

/// Internal progression state of [`M2dirMessageList`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start(M2dir),
    Reading,
    Checking {
        candidates: BTreeMap<M2dirPath, String>,
    },
    #[default]
    Invalid,
}

/// I/O-free coroutine to list every entry inside an [`M2dir`].
///
/// Dotfiles and sub-directories are skipped. Filenames that do not
/// match the m2dir specification (no `,` separator) are also
/// skipped. Returned entries are not checksum-verified; use
/// [`M2dirMessageGet`](crate::coroutines::message_get::M2dirMessageGet)
/// when validation is required.
#[derive(Clone, Debug)]
pub struct M2dirMessageList {
    state: State,
}

impl M2dirMessageList {
    /// Creates a new coroutine that will list every entry inside
    /// `m2dir`.
    pub fn new(m2dir: M2dir) -> Self {
        Self {
            state: State::Start(m2dir),
        }
    }
}

impl M2dirCoroutine for M2dirMessageList {
    type Yield = M2dirYield;
    type Return = Result<Vec<M2dirEntry>, M2dirMessageListError>;

    fn resume(&mut self, arg: Option<M2dirArg>) -> M2dirCoroutineState<Self::Yield, Self::Return> {
        match (mem::take(&mut self.state), arg) {
            (State::Start(m2dir), None) => {
                trace!("wants directory read of {}", m2dir.path());

                let paths = BTreeSet::from_iter([m2dir.path().clone()]);
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
                let mut found = Vec::new();

                for (path, id) in candidates {
                    if probes.get(&path).copied().unwrap_or(false) {
                        found.push(M2dirEntry::from_parts(id, path));
                    }
                }

                trace!("found {} entries", found.len());
                M2dirCoroutineState::Complete(Ok(found))
            }
            (state, arg) => {
                let err = M2dirMessageListError::Invalid(arg, state);
                M2dirCoroutineState::Complete(Err(err))
            }
        }
    }
}
