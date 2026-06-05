//! Free helpers backing m2dir entry checksums.

use core::fmt::{self, Write};

use alloc::string::String;

use base64::{Engine, engine::general_purpose::URL_SAFE};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// FNV-1a 64-bit hash; kept in-crate to preserve the exact m2dir
/// specification output for the checksum tail.
fn fnv_hash(salt: impl AsRef<[u8]>, bytes: impl AsRef<[u8]>) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in salt.as_ref().iter().chain(bytes.as_ref().iter()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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
    checksum[4..].copy_from_slice(&fnv_hash(size, bytes).to_le_bytes());
    w.write_str(&URL_SAFE.encode(checksum))
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;

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
