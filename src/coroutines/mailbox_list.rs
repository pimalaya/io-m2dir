//! I/O-free coroutine to list every m2dir inside an m2store.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
};
use std::{collections::HashSet, path::PathBuf};

use log::trace;
use thiserror::Error;

use crate::m2dir::{DOT_M2DIR, M2dir};
use crate::m2store::M2store;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MailboxListError {
    #[error("Invalid m2dir mailbox list arg {0:?} for state {1:?}")]
    Invalid(Option<MailboxListArg>, State),
}

/// Result returned by [`MailboxList::resume`].
#[derive(Clone, Debug)]
pub enum MailboxListResult {
    /// The coroutine has successfully terminated its progression.
    Ok(HashSet<M2dir>),

    /// The caller must read the entries of the given directories and
    /// feed back [`MailboxListArg::DirRead`].
    WantsDirRead(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(MailboxListError),
}

/// Internal progression state of [`MailboxList`].
#[derive(Clone, Debug, Default)]
pub enum State {
    /// Directories yet to scan, and the accumulator of found m2dirs.
    Scanning {
        pending: BTreeSet<String>,
        found: HashSet<M2dir>,
    },
    #[default]
    Invalid,
}

/// Argument fed back to [`MailboxList::resume`].
#[derive(Clone, Debug)]
pub enum MailboxListArg {
    /// Response to [`MailboxListResult::WantsDirRead`].
    ///
    /// Maps each requested directory path to the set of entry paths
    /// found inside it.
    DirRead(BTreeMap<String, BTreeSet<String>>),
}

/// I/O-free coroutine to list every valid m2dir inside an m2store.
///
/// Walks the tree depth-first. Hidden entries (whose name starts with
/// `.`) are skipped. A directory containing the `.m2dir` marker is
/// reported as an m2dir; its sub-directories are still scanned
/// because m2dirs can nest.
#[derive(Debug)]
pub struct MailboxList {
    state: State,
}

impl MailboxList {
    /// Creates a new coroutine that will list every m2dir inside
    /// `store`.
    pub fn new(store: &M2store) -> Self {
        let root = store.path().to_string_lossy().into_owned();
        let pending = BTreeSet::from_iter([root]);

        Self {
            state: State::Scanning {
                pending,
                found: HashSet::new(),
            },
        }
    }

    /// Makes the listing progress.
    pub fn resume(
        &mut self,
        arg: Option<impl Into<MailboxListArg>>,
    ) -> MailboxListResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (
                State::Scanning {
                    mut pending,
                    mut found,
                },
                arg,
            ) => {
                if let Some(MailboxListArg::DirRead(entries)) = arg {
                    trace!("scanned {} directories", entries.len());

                    for (_dir, names) in entries {
                        for path_str in names {
                            let path = PathBuf::from(&path_str);

                            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                                Some(name) => name,
                                None => continue,
                            };

                            if file_name.starts_with('.') {
                                continue;
                            }

                            if !path.is_dir() {
                                continue;
                            }

                            // A folder is an m2dir iff it carries the
                            // `.m2dir` marker. Either way we still
                            // recurse into its children.
                            if path.join(DOT_M2DIR).exists() {
                                found.insert(M2dir::from_path(path.clone()));
                            }

                            pending.insert(path_str);
                        }
                    }
                }

                if pending.is_empty() {
                    trace!("found {} m2dirs", found.len());
                    return MailboxListResult::Ok(found);
                }

                // Take every pending dir for the next batch read.
                let batch = mem::take(&mut pending);
                trace!("wants read of {} directories", batch.len());

                self.state = State::Scanning { pending, found };
                MailboxListResult::WantsDirRead(batch)
            }
            (state, arg) => {
                let err = MailboxListError::Invalid(arg, state);
                MailboxListResult::Err(err)
            }
        }
    }
}
