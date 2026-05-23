//! Crate-local FNV-1a 64-bit hash used by entry checksums. Kept
//! in-crate to preserve the exact m2dir specification output.

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub fn hash(salt: impl AsRef<[u8]>, bytes: impl AsRef<[u8]>) -> u64 {
    let mut hash = FNV_OFFSET;

    for byte in salt.as_ref().iter().chain(bytes.as_ref().iter()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    hash
}
