//! Single m2dir directory on the filesystem.

use core::hash::{Hash, Hasher};

use alloc::{string::String, vec::Vec};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::base64;
use crate::entry::write_checksum;
use crate::parse::extract_date;

/// Marker filename written into every m2dir.
pub const DOT_M2DIR: &str = ".m2dir";

/// Metadata subdirectory inside an m2dir.
pub const META: &str = ".meta";

/// Errors that can occur while opening an existing m2dir.
#[derive(Clone, Debug, Error)]
pub enum LoadM2dirError {
    /// The given path is not a directory.
    #[error("path {0} is not a directory")]
    NotDir(PathBuf),

    /// The given directory does not contain the `.m2dir` marker.
    #[error("no valid `.m2dir` marker found in directory {0}")]
    NoDotM2dir(PathBuf),
}

/// A single m2dir directory on the filesystem.
///
/// Holds the root path and provides helpers to derive entry paths,
/// metadata paths, and a new filename for a delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct M2dir {
    path: PathBuf,
}

impl M2dir {
    /// Builds an [`M2dir`] from a path without checking the marker.
    ///
    /// Prefer [`TryFrom<PathBuf>`] for opening existing m2dirs.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the path to the m2dir directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the path to the `.m2dir` marker file.
    pub fn marker_path(&self) -> PathBuf {
        self.path.join(DOT_M2DIR)
    }

    /// Returns the path to the `.meta` directory.
    pub fn meta_dir(&self) -> PathBuf {
        self.path.join(META)
    }

    /// Returns the path to the `.flags` sidecar for the given entry
    /// id.
    pub fn flags_sidecar_path(&self, id: &str) -> PathBuf {
        self.meta_dir().join(format!("{id}.flags"))
    }

    /// Computes the filename and final on-disk path for a new entry
    /// holding `bytes`. The filename is `<date>,<checksum>.<nonce>` per
    /// the m2dir specification.
    ///
    /// The `nonce_bytes` argument should be 4 freshly-generated random
    /// bytes (from [`crate::rand::random_bytes`]).
    pub fn entry_path(&self, bytes: &[u8], nonce_bytes: &[u8]) -> (String, PathBuf) {
        let mut checksum = String::new();
        write_checksum(bytes, &mut checksum)
            .expect("base64 encoding to a string is always valid");

        let dt = core::str::from_utf8(bytes)
            .ok()
            .and_then(extract_date)
            .unwrap_or_default();

        let mut nonce = String::new();
        base64::encode(nonce_bytes, &mut nonce)
            .expect("base64 encoding to a string is always valid");

        let id = checksum.clone();
        let filename = format!("{dt},{checksum}.{nonce}");
        let path = self.path.join(&filename);

        (id, path)
    }

    /// Returns the path of a temporary file inside this m2dir, used
    /// during the write-then-rename delivery sequence.
    pub fn tmp_path(&self, pid: u32, counter: u32) -> PathBuf {
        self.path
            .join(format!(".m2dir.tmp.{pid:x}{counter:x}"))
    }

    /// Splits a filename into its `<checksum>.<nonce>` tail (used as
    /// the entry id).
    pub fn parse_filename_id(filename: &str) -> Option<&str> {
        let (_, id) = filename.rsplit_once(',')?;
        Some(id)
    }

    /// Extracts the checksum portion of an entry id (the chunk before
    /// the first `.`).
    pub fn id_checksum(id: &str) -> &str {
        id.rsplit_once('.').map(|(c, _)| c).unwrap_or(id)
    }
}

impl Hash for M2dir {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl AsRef<Path> for M2dir {
    fn as_ref(&self) -> &Path {
        self.path.as_ref()
    }
}

impl TryFrom<PathBuf> for M2dir {
    type Error = LoadM2dirError;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        if !path.is_dir() {
            return Err(LoadM2dirError::NotDir(path));
        }

        let marker = path.join(DOT_M2DIR);
        if !marker.exists() {
            return Err(LoadM2dirError::NoDotM2dir(path));
        }

        Ok(Self { path })
    }
}

impl TryFrom<&Path> for M2dir {
    type Error = LoadM2dirError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        path.to_path_buf().try_into()
    }
}

/// Splits a list of filenames into `(visible_files, dotfiles)`. Used
/// by coroutine modules that read `.meta` to filter siblings.
pub fn partition_dotfiles(paths: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut visible = Vec::new();
    let mut hidden = Vec::new();

    for path in paths {
        if path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with('.'))
        {
            hidden.push(path);
        } else {
            visible.push(path);
        }
    }

    (visible, hidden)
}
