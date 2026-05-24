//! I/O-free coroutine to list entries inside an m2dir.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};

use log::trace;
use thiserror::Error;

use crate::{entry::M2dirEntry, m2dir::M2dir, path::M2dirPath};

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum M2dirMessageListError {
    #[error("Invalid m2dir message list arg {0:?} for state {1:?}")]
    Invalid(Option<M2dirMessageListArg>, State),
}

/// Result returned by [`M2dirMessageList::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMessageListResult {
    /// The coroutine has successfully terminated its progression.
    Ok(Vec<M2dirEntry>),
    /// The caller must read the entries of the given directories
    /// and feed back [`M2dirMessageListArg::DirRead`].
    WantsDirRead(BTreeSet<M2dirPath>),
    /// The caller must check whether the given paths exist as
    /// regular files and feed back
    /// [`M2dirMessageListArg::FileExists`].
    WantsFileExists(BTreeSet<M2dirPath>),
    /// The coroutine encountered an error.
    Err(M2dirMessageListError),
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

/// Argument fed back to [`M2dirMessageList::resume`].
#[derive(Clone, Debug)]
pub enum M2dirMessageListArg {
    /// Response to [`M2dirMessageListResult::WantsDirRead`].
    DirRead(BTreeMap<M2dirPath, BTreeSet<M2dirPath>>),
    /// Response to [`M2dirMessageListResult::WantsFileExists`].
    FileExists(BTreeMap<M2dirPath, bool>),
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

    /// Makes the listing progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<M2dirMessageListArg>>,
    ) -> M2dirMessageListResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start(m2dir), None) => {
                trace!("wants directory read of {}", m2dir.path());

                let paths = BTreeSet::from_iter([m2dir.path().clone()]);
                self.state = State::Reading;
                M2dirMessageListResult::WantsDirRead(paths)
            }
            (State::Reading, Some(M2dirMessageListArg::DirRead(entries))) => {
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
                    return M2dirMessageListResult::Ok(Vec::new());
                }

                let probes: BTreeSet<M2dirPath> = candidates.keys().cloned().collect();
                trace!("wants existence check for {} candidates", probes.len());

                self.state = State::Checking { candidates };
                M2dirMessageListResult::WantsFileExists(probes)
            }
            (State::Checking { candidates }, Some(M2dirMessageListArg::FileExists(probes))) => {
                let mut found = Vec::new();

                for (path, id) in candidates {
                    if probes.get(&path).copied().unwrap_or(false) {
                        found.push(M2dirEntry::from_parts(id, path));
                    }
                }

                trace!("found {} entries", found.len());
                M2dirMessageListResult::Ok(found)
            }
            (state, arg) => {
                let err = M2dirMessageListError::Invalid(arg, state);
                M2dirMessageListResult::Err(err)
            }
        }
    }
}
