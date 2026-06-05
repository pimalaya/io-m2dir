//! Root m2store directory containing one or more m2dirs.

use alloc::string::{String, ToString};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use thiserror::Error;

use crate::path::M2dirPath;

/// Bytes percent-encoded by m2dir folder names: the path separators
/// (`/`, `\`), the escape char (`%`), plus all control bytes; non-ASCII
/// codepoints are always encoded by `utf8_percent_encode`.
const M2DIR_PCT: &AsciiSet = &CONTROLS.add(b'%').add(b'/').add(b'\\');

/// Marker filename written at the root of every m2store.
pub const DOT_M2STORE: &str = ".m2store";

/// Filename or symlink at the m2store root identifying the delivery
/// target m2dir.
pub const DOT_DELIVERY: &str = ".delivery";

/// Errors that can occur while operating on an m2store.
#[derive(Clone, Debug, Error)]
pub enum M2dirStoreError {
    /// The given path is not a directory.
    #[error("path {0} is not a directory")]
    NotDir(M2dirPath),
    /// The given directory does not contain the `.m2store` marker.
    #[error("no valid `.m2store` marker found in directory {0}")]
    NoDotM2store(M2dirPath),
    /// The given folder name resolves to an absolute path.
    #[error("folder path {0} must be relative")]
    AbsolutePath(String),
    /// The given folder name contains components that fall outside
    /// the m2store root (such as `..`).
    #[error("folder path {0} escapes m2store root")]
    EscapesRoot(String),
}

/// Root m2store directory holding one or more [`crate::m2dir::types::M2dir`]s.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct M2dirStore {
    path: M2dirPath,
}

impl M2dirStore {
    /// Builds an [`M2dirStore`] from a path without checking the marker.
    pub fn from_path(path: impl Into<M2dirPath>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the path to the m2store root directory.
    pub fn path(&self) -> &M2dirPath {
        &self.path
    }

    /// Returns the path to the `.m2store` marker file.
    pub fn marker_path(&self) -> M2dirPath {
        self.path.join(DOT_M2STORE)
    }

    /// Returns the path to the `.delivery` entry.
    pub fn delivery_path(&self) -> M2dirPath {
        self.path.join(DOT_DELIVERY)
    }

    /// Resolves a folder name (relative path, components percent
    /// encoded per the m2dir specification) to its on-disk path
    /// inside this store.
    ///
    /// Returns an error if `name` is absolute or escapes the store
    /// root.
    pub fn resolve_folder_path(&self, name: &str) -> Result<M2dirPath, M2dirStoreError> {
        if name.starts_with('/') || name.starts_with('\\') {
            return Err(M2dirStoreError::AbsolutePath(name.to_string()));
        }

        let mut resolved = self.path.clone();

        for raw in name.split(|c| c == '/' || c == '\\') {
            match raw {
                "" | "." => {}
                ".." => {
                    return Err(M2dirStoreError::EscapesRoot(name.to_string()));
                }
                part => {
                    let encoded = utf8_percent_encode(part, M2DIR_PCT).to_string();
                    resolved.push(&encoded);
                }
            }
        }

        Ok(resolved)
    }

    /// Decodes a path inside the store back to its UTF-8 folder name.
    pub fn decode_folder_name(&self, path: &M2dirPath) -> Option<String> {
        let rel = path.strip_prefix(&self.path)?;
        percent_decode_str(rel)
            .decode_utf8()
            .ok()
            .map(|s| s.into_owned())
    }
}

impl AsRef<M2dirPath> for M2dirStore {
    fn as_ref(&self) -> &M2dirPath {
        &self.path
    }
}

impl AsRef<str> for M2dirStore {
    fn as_ref(&self) -> &str {
        self.path.as_str()
    }
}

impl From<M2dirPath> for M2dirStore {
    fn from(path: M2dirPath) -> Self {
        Self { path }
    }
}
