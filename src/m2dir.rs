//! Single m2dir directory on the filesystem.

use core::hash::{Hash, Hasher};

use alloc::{
    format,
    string::{String, ToString},
};

use thiserror::Error;

use crate::{base64, entry::write_checksum, parse::extract_date, path::M2dirPath};

/// Marker filename written into every m2dir.
pub const DOT_M2DIR: &str = ".m2dir";

/// Metadata subdirectory inside an m2dir.
pub const META: &str = ".meta";

/// Errors that can occur while opening an existing m2dir.
#[derive(Clone, Debug, Error)]
pub enum LoadM2dirError {
    /// The given path is not a directory.
    #[error("path {0} is not a directory")]
    NotDir(M2dirPath),

    /// The given directory does not contain the `.m2dir` marker.
    #[error("no valid `.m2dir` marker found in directory {0}")]
    NoDotM2dir(M2dirPath),
}

/// A single m2dir directory on the filesystem.
///
/// Holds the root path and provides helpers to derive entry paths,
/// metadata paths, and a new filename for a delivery.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct M2dir {
    path: M2dirPath,
}

impl M2dir {
    /// Builds an [`M2dir`] from a path without checking the marker.
    pub fn from_path(path: impl Into<M2dirPath>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the path to the m2dir directory.
    pub fn path(&self) -> &M2dirPath {
        &self.path
    }

    /// Returns the path to the `.m2dir` marker file.
    pub fn marker_path(&self) -> M2dirPath {
        self.path.join(DOT_M2DIR)
    }

    /// Returns the path to the `.meta` directory.
    pub fn meta_dir(&self) -> M2dirPath {
        self.path.join(META)
    }

    /// Returns the path to the `.flags` metadata file for the given
    /// entry id.
    pub fn flags_path(&self, id: &str) -> M2dirPath {
        self.meta_dir().join(&format!("{id}.flags"))
    }

    /// Computes the filename and final on-disk path for a new entry
    /// holding `bytes`. The filename is `<date>,<checksum>.<nonce>`
    /// per the m2dir specification.
    ///
    /// `nonce_bytes` should be 4 freshly-generated random bytes
    /// supplied by the caller.
    pub fn entry_path(&self, bytes: &[u8], nonce_bytes: &[u8]) -> (String, M2dirPath) {
        let mut checksum = String::new();
        write_checksum(bytes, &mut checksum).expect("base64 encoding to a string is always valid");

        let dt = core::str::from_utf8(bytes)
            .ok()
            .and_then(extract_date)
            .unwrap_or_default();

        let mut nonce = String::new();
        base64::encode(nonce_bytes, &mut nonce)
            .expect("base64 encoding to a string is always valid");

        let id = format!("{checksum}.{nonce}");
        let filename = format!("{dt},{id}");
        let path = self.path.join(&filename);

        (id, path)
    }

    /// Returns the path of a temporary file inside this m2dir, used
    /// during the write-then-rename delivery sequence.
    pub fn tmp_path(&self, pid: u32, counter: u32) -> M2dirPath {
        self.path.join(&format!(".m2dir.tmp.{pid:x}{counter:x}"))
    }

    /// Splits a filename into its `<checksum>.<nonce>` tail (used as
    /// the entry id).
    pub fn parse_filename_id(filename: &str) -> Option<&str> {
        let (_, id) = filename.rsplit_once(',')?;
        Some(id)
    }
}

impl Hash for M2dir {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl AsRef<M2dirPath> for M2dir {
    fn as_ref(&self) -> &M2dirPath {
        &self.path
    }
}

impl AsRef<str> for M2dir {
    fn as_ref(&self) -> &str {
        self.path.as_str()
    }
}

impl From<M2dirPath> for M2dir {
    fn from(path: M2dirPath) -> Self {
        Self { path }
    }
}

impl From<String> for M2dir {
    fn from(path: String) -> Self {
        Self { path: path.into() }
    }
}

impl From<&str> for M2dir {
    fn from(path: &str) -> Self {
        Self {
            path: path.to_string().into(),
        }
    }
}
