//! Pseudorandom byte generator used for m2dir entry filename
//! suffixes. Seeded from [`std::collections::hash_map::RandomState`]
//! and iterated via xorshift64*.

use core::hash::{BuildHasher, Hasher};
use core::iter;

use std::collections::hash_map::RandomState;

// Inspired by:
// https://github.com/rust-lang/rust/blob/1.55.0/library/core/src/slice/sort.rs#L559-L573
pub fn random_bytes() -> impl Iterator<Item = u8> {
    let mut state = RandomState::new().build_hasher().finish();

    // NOTE: can't start with state 0.
    if state == 0 {
        state = 0xdeadbeef;
    }

    let mut buf = 0u64;
    let mut i = 8;

    // Pick out each byte of the generated 64-bit number.
    iter::repeat_with(move || {
        if i == 8 {
            // xorshift64*
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            buf = state;
            i = 0;
        }
        let byte = buf as u8;
        buf >>= 8;
        i += 1;
        byte
    })
}
