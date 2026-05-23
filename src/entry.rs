//! Single m2dir message entry: id, path, checksum helpers.

use core::fmt::{self, Write};

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::base64;
use crate::fnv;

/// Errors that can occur while parsing or validating an entry
/// filename.
#[derive(Clone, Debug, Error)]
pub enum ParseFilenameError {
    /// The given path is not a regular file.
    #[error("path {0} is not a regular file")]
    NotFile(PathBuf),

    /// The path has no final filename component.
    #[error("path {0} is missing a filename")]
    MissingFilename(PathBuf),

    /// The filename does not match the m2dir specification.
    #[error("entry {path} does not match filename spec: {reason}")]
    InvalidFilename {
        path: PathBuf,
        reason: &'static str,
    },

    /// The checksum embedded in the filename does not match the file
    /// contents.
    #[error("invalid checksum for {path}: expected {expected:?}, got {got:?}")]
    InvalidChecksum {
        path: PathBuf,
        expected: String,
        got: String,
    },
}

/// A single message entry inside an [`M2dir`](crate::m2dir::M2dir).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    id: String,
    path: PathBuf,
}

impl Entry {
    /// Builds an [`Entry`] from a path and its unique id without
    /// checking the on-disk checksum. Used by coroutines that have
    /// just delivered the entry and trust their own checksum.
    pub fn from_parts(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
        }
    }

    /// Returns the path to the message file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the unique identifier of the entry (the
    /// `<checksum>.<nonce>` portion of the filename).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the checksum portion of the id (the chunk before the
    /// first `.`).
    pub fn checksum(&self) -> &str {
        self.id.rsplit_once('.').map(|(c, _)| c).unwrap_or(&self.id)
    }

    /// Parses an entry id from its on-disk filename without reading
    /// the file contents. Returns the id (`<checksum>.<nonce>`) and
    /// the date prefix.
    pub fn parse_filename(filename: &str) -> Result<(&str, &str), ParseFilenameErrorKind> {
        let (date, id) = filename
            .rsplit_once(',')
            .ok_or(ParseFilenameErrorKind::MissingDelimiter)?;
        Ok((date, id))
    }

    /// Validates that `contents` matches the checksum embedded in
    /// `filename`. Returns the parsed id on success, or an error
    /// describing the mismatch.
    pub fn validate(path: &Path, contents: &[u8]) -> Result<String, ParseFilenameError> {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| ParseFilenameError::MissingFilename(path.to_path_buf()))?;

        let (_, id) = filename
            .rsplit_once(',')
            .ok_or_else(|| ParseFilenameError::InvalidFilename {
                path: path.to_path_buf(),
                reason: "missing delimiter `,`",
            })?;

        let checksum = id.rsplit_once('.').map(|(c, _)| c).unwrap_or(id);

        if !validate_checksum(checksum, contents) {
            let mut expected = String::new();
            write_checksum(contents, &mut expected).ok();
            return Err(ParseFilenameError::InvalidChecksum {
                path: path.to_path_buf(),
                expected,
                got: id.to_string(),
            });
        }

        Ok(id.to_string())
    }
}

/// Lightweight variant of [`ParseFilenameError`] that does not carry
/// the originating path. Returned by [`Entry::parse_filename`] which
/// has access only to the filename string.
#[derive(Clone, Copy, Debug, Error)]
pub enum ParseFilenameErrorKind {
    #[error("missing `,` delimiter in entry filename")]
    MissingDelimiter,
}

/// Validates the checksum for a given set of bytes against a provided
/// checksum string.
pub fn validate_checksum(checksum: &str, bytes: impl AsRef<[u8]>) -> bool {
    let mut gold = String::new();
    write_checksum(bytes, &mut gold).is_ok() && gold == checksum
}

/// Writes a base64-encoded checksum derived from `bytes` to `w`. The
/// checksum is 12 bytes: a little-endian length prefix followed by an
/// FNV-1a-64 hash of `length || bytes`.
pub fn write_checksum<B: AsRef<[u8]>, W: Write>(bytes: B, mut w: W) -> fmt::Result {
    let mut checksum = [0u8; 12];
    let bytes: &[u8] = bytes.as_ref();
    let size: [u8; 4] = (bytes.len() as u32).to_le_bytes();

    checksum[..4].copy_from_slice(&size);
    checksum[4..].copy_from_slice(&fnv::hash(size, bytes).to_le_bytes());
    base64::encode(&checksum, &mut w)?;
    Ok(())
}

/// Filters a list of filenames to those whose name starts with `id`,
/// preserving order.
pub fn filter_sidecar_paths<I, S>(id: &str, paths: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    paths
        .into_iter()
        .filter_map(|path| {
            let path = path.as_ref();
            let name = path.rsplit('/').next()?;
            if name.starts_with(id) {
                Some(path.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::entry::*;

    #[test]
    fn checksum_matches_spec() {
        let mut got = String::new();
        write_checksum(b"Some content", &mut got).unwrap();
        assert_eq!(got, "DAAAAGh5pqOOxdeD");

        let mut got = String::new();
        write_checksum(b"Some other content", &mut got).unwrap();
        assert_eq!(got, "EgAAAFhc88xwPkT+");
    }

    #[test]
    fn checksum_roundtrips() {
        let bytes = b"hello world";
        let mut sum = String::new();
        write_checksum(bytes, &mut sum).unwrap();
        assert!(validate_checksum(&sum, bytes));
        assert!(!validate_checksum(&sum, b"other"));
    }
}
