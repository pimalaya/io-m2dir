//! I/O-free coroutine to list entries inside an m2dir.

use core::mem;

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use std::{collections::HashSet, path::PathBuf};

use log::trace;
use thiserror::Error;

use crate::entry::Entry;
use crate::m2dir::M2dir;

/// Errors that can occur during the coroutine progression.
#[derive(Clone, Debug, Error)]
pub enum MessageListError {
    #[error("Invalid m2dir message list arg {0:?} for state {1:?}")]
    Invalid(Option<MessageListArg>, State),
}

/// Result returned by [`MessageList::resume`].
#[derive(Clone, Debug)]
pub enum MessageListResult {
    /// The coroutine has successfully terminated its progression.
    Ok(Vec<Entry>),

    /// The caller must read the entries of the given directories and
    /// feed back [`MessageListArg::DirRead`].
    WantsDirRead(BTreeSet<String>),

    /// The coroutine encountered an error.
    Err(MessageListError),
}

/// Internal progression state of [`MessageList`].
#[derive(Clone, Debug, Default)]
pub enum State {
    Start(M2dir),
    Read,
    #[default]
    Invalid,
}

/// Argument fed back to [`MessageList::resume`].
#[derive(Clone, Debug)]
pub enum MessageListArg {
    /// Response to [`MessageListResult::WantsDirRead`].
    DirRead(BTreeMap<String, BTreeSet<String>>),
}

/// I/O-free coroutine to list every entry inside an [`M2dir`].
///
/// Dotfiles and sub-directories are skipped. Filenames that do not
/// match the m2dir specification (no `,` separator) are also skipped.
/// Returned entries are not checksum-verified; use
/// [`MessageGet`](crate::coroutines::message_get::MessageGet) when
/// validation is required.
#[derive(Debug)]
pub struct MessageList {
    state: State,
}

impl MessageList {
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
        arg: Option<impl Into<MessageListArg>>,
    ) -> MessageListResult {
        match (mem::take(&mut self.state), arg.map(Into::into)) {
            (State::Start(m2dir), None) => {
                trace!("wants directory read of {}", m2dir.path().display());

                let path = m2dir.path().to_string_lossy().into_owned();
                let paths = BTreeSet::from_iter([path]);

                self.state = State::Read;
                MessageListResult::WantsDirRead(paths)
            }
            (State::Read, Some(MessageListArg::DirRead(entries))) => {
                let names = entries.into_values().next().unwrap_or_default();
                let mut found = Vec::new();

                for path_str in names {
                    let path = PathBuf::from(&path_str);

                    let name = match path.file_name().and_then(|n| n.to_str()) {
                        Some(name) => name,
                        None => continue,
                    };

                    if name.starts_with('.') {
                        continue;
                    }

                    if !path.is_file() {
                        continue;
                    }

                    let Some(id) = M2dir::parse_filename_id(name) else {
                        trace!("skipping unparseable entry filename: {name}");
                        continue;
                    };

                    found.push(Entry::from_parts(id, path));
                }

                trace!("found {} entries", found.len());
                MessageListResult::Ok(found)
            }
            (state, arg) => {
                let err = MessageListError::Invalid(arg, state);
                MessageListResult::Err(err)
            }
        }
    }
}
