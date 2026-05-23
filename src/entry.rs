//! Single m2dir message entry: id, path, checksum helpers.

use core::fmt::{self, Write};

use alloc::string::String;

use thiserror::Error;

use crate::{base64, fnv, path::M2dirPath};

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

/// A single message entry inside an [`crate::m2dir::M2dir`].
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entry {
    id: String,
    path: M2dirPath,
}

impl Entry {
    /// Builds an [`Entry`] from a path and its unique id without
    /// checking the on-disk checksum. Used by coroutines that have
    /// just delivered the entry and trust their own checksum.
    pub fn from_parts(id: impl Into<String>, path: impl Into<M2dirPath>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
        }
    }

    /// Returns the path to the message file.
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

#[cfg(test)]
mod tests {
    use crate::entry::*;

    #[test]
    fn checksum_is_deterministic() {
        let mut a = String::new();
        write_checksum(b"Some content", &mut a).unwrap();
        let mut b = String::new();
        write_checksum(b"Some content", &mut b).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
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
