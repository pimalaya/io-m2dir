//! Root m2store directory containing one or more m2dirs.

use alloc::string::String;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::m2dir::{LoadM2dirError, M2dir};
use crate::percent::{percent_decode_bytes, percent_encode_bytes};

/// Marker filename written at the root of every m2store.
pub const DOT_M2STORE: &str = ".m2store";

/// Filename or symlink at the m2store root identifying the delivery
/// target m2dir.
pub const DOT_DELIVERY: &str = ".delivery";

/// Errors that can occur while opening an existing m2store.
#[derive(Clone, Debug, Error)]
pub enum LoadM2storeError {
    /// The given path is not a directory.
    #[error("path {0} is not a directory")]
    NotDir(PathBuf),

    /// The given directory does not contain the `.m2store` marker.
    #[error("no valid `.m2store` marker found in directory {0}")]
    NoDotM2store(PathBuf),
}

/// Errors that can occur while creating a new folder inside an
/// m2store.
#[derive(Clone, Debug, Error)]
pub enum NewFolderError {
    /// The given folder name resolves to an absolute path.
    #[error("folder path {0} must be relative")]
    AbsolutePath(PathBuf),

    /// The given folder name contains components that fall outside the
    /// m2store root (such as `..`).
    #[error("folder path {0} escapes m2store root")]
    EscapesRoot(PathBuf),
}

/// Root m2store directory holding one or more [`M2dir`]s.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct M2store {
    path: PathBuf,
}

impl M2store {
    /// Builds an [`M2store`] from a path without checking the marker.
    ///
    /// Prefer [`TryFrom<PathBuf>`] for opening existing m2stores.
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the path to the m2store root directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the path to the `.m2store` marker file.
    pub fn marker_path(&self) -> PathBuf {
        self.path.join(DOT_M2STORE)
    }

    /// Returns the path to the `.delivery` entry.
    pub fn delivery_path(&self) -> PathBuf {
        self.path.join(DOT_DELIVERY)
    }

    /// Resolves a folder name (relative path; components are percent
    /// encoded the same way as the m2dir specification) to its
    /// on-disk path inside this store.
    ///
    /// Returns an error if `name` is absolute or escapes the store
    /// root.
    pub fn resolve_folder_path(&self, name: impl AsRef<Path>) -> Result<PathBuf, NewFolderError> {
        let name = name.as_ref();
        let mut resolved = self.path.clone();

        for component in name.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(NewFolderError::AbsolutePath(name.to_path_buf()));
                }
                Component::ParentDir => {
                    return Err(NewFolderError::EscapesRoot(name.to_path_buf()));
                }
                Component::CurDir => {}
                Component::Normal(part) => {
                    let bytes = part.to_string_lossy();
                    let mut encoded = String::new();
                    percent_encode_bytes(bytes.as_bytes(), &mut encoded)
                        .expect("percent encoding to a string is always valid");
                    resolved.push(encoded);
                }
            }
        }

        Ok(resolved)
    }

    /// Decodes a path inside the store back to its UTF-8 folder name.
    pub fn decode_folder_name(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.path).ok()?;
        let raw = rel.to_string_lossy();
        percent_decode_bytes(raw.bytes()).ok()
    }
}

impl AsRef<Path> for M2store {
    fn as_ref(&self) -> &Path {
        self.path.as_ref()
    }
}

impl TryFrom<PathBuf> for M2store {
    type Error = LoadM2storeError;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        if !path.is_dir() {
            return Err(LoadM2storeError::NotDir(path));
        }

        let marker = path.join(DOT_M2STORE);
        if !marker.exists() {
            return Err(LoadM2storeError::NoDotM2store(path));
        }

        Ok(Self { path })
    }
}

impl TryFrom<&Path> for M2store {
    type Error = LoadM2storeError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        path.to_path_buf().try_into()
    }
}

/// Errors related to opening an [`M2dir`] reached via an
/// [`M2store`]. Carries both layers because callers usually do not
/// know which one failed.
#[derive(Clone, Debug, Error)]
pub enum OpenM2dirError {
    /// The m2dir at the resolved path could not be opened.
    #[error(transparent)]
    LoadM2dir(#[from] LoadM2dirError),

    /// The folder path could not be resolved inside the store.
    #[error(transparent)]
    NewFolder(#[from] NewFolderError),
}
