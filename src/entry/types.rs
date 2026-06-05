//! Single m2dir entry: id, path, checksum helpers.

use alloc::{string::String, vec::Vec};

use thiserror::Error;

use crate::{flag::types::M2dirFlags, path::M2dirPath};

/// Errors that can occur while parsing or validating an entry
/// filename.
#[derive(Clone, Debug, Error)]
pub enum ParseFilenameError {
    /// The given path is not a regular file.
    #[error("path {0} is not a regular file")]
    NotFile(M2dirPath),
    /// The path has no final filename component.
    #[error("path {0} is missing a filename")]
    MissingFilename(M2dirPath),
    /// The filename does not match the m2dir specification.
    #[error("entry {path} does not match filename spec: {reason}")]
    InvalidFilename {
        path: M2dirPath,
        reason: &'static str,
    },
    /// The checksum embedded in the filename does not match the file
    /// contents.
    #[error("invalid checksum for {path}: expected {expected:?}, got {got:?}")]
    InvalidChecksum {
        path: M2dirPath,
        expected: String,
        got: String,
    },
}

/// A single entry inside an [`crate::m2dir::types::M2dir`].
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct M2dirEntry {
    id: String,
    path: M2dirPath,
}

impl M2dirEntry {
    /// Builds an [`M2dirEntry`] from a path and its unique id without
    /// checking the on-disk checksum. Used by coroutines that have
    /// just delivered the entry and trust their own checksum.
    pub fn from_parts(id: impl Into<String>, path: impl Into<M2dirPath>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
        }
    }

    /// Returns the path to the entry file.
    pub fn path(&self) -> &M2dirPath {
        &self.path
    }

    /// Returns the unique identifier of the entry (the
    /// `<checksum>.<nonce>` portion of the filename).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the checksum portion of the id (the chunk before the
    /// last `.`).
    pub fn checksum(&self) -> &str {
        self.id.rsplit_once('.').map(|(c, _)| c).unwrap_or(&self.id)
    }
}

/// An [`M2dirEntry`] paired with its file contents and flags
/// metadata, as produced by the bulk reads on
/// [`M2dirClient`](crate::client::M2dirClient).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct M2dirFullEntry {
    entry: M2dirEntry,
    contents: Vec<u8>,
    flags: M2dirFlags,
}

impl M2dirFullEntry {
    pub fn from_parts(entry: M2dirEntry, contents: Vec<u8>, flags: M2dirFlags) -> Self {
        Self {
            entry,
            contents,
            flags,
        }
    }

    pub fn entry(&self) -> &M2dirEntry {
        &self.entry
    }

    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    pub fn flags(&self) -> &M2dirFlags {
        &self.flags
    }
}
