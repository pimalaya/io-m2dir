//! Crate-local base64 encoder used by entry checksums and random
//! filename suffixes. Kept in-crate to preserve the exact byte
//! sequences produced by the original m2dir specification.

use core::fmt::{self, Write};

pub fn encode<W: Write>(input: &[u8], mut w: W) -> fmt::Result {
    let count = input.len() / 3;
    for i in 0..count {
        let n = i * 3;
        let b24 =
            ((input[n] as usize) << 16) + ((input[n + 1] as usize) << 8) + (input[n + 2] as usize);

        w.write_char(TABLE[(b24 >> 18) & 0x3f])?;
        w.write_char(TABLE[(b24 >> 12) & 0x3f])?;
        w.write_char(TABLE[(b24 >> 6) & 0x3f])?;
        w.write_char(TABLE[(b24) & 0x3f])?;
    }

    let mod_three = input.len() % 3;
    if mod_three == 1 {
        let b24 = (input[count * 3] as usize) << 16;

        w.write_char(TABLE[(b24 >> 18) & 0x3f])?;
        w.write_char(TABLE[(b24 >> 12) & 0x3f])?;
        w.write_str("==")?;
    } else if mod_three == 2 {
        let b24 = ((input[count * 3] as usize) << 16) + ((input[count * 3 + 1] as usize) << 8);

        w.write_char(TABLE[(b24 >> 18) & 0x3f])?;
        w.write_char(TABLE[(b24 >> 12) & 0x3f])?;
        w.write_char(TABLE[(b24 >> 6) & 0x3f])?;
        w.write_char('=')?;
    }
    Ok(())
}

pub const TABLE: [char; 64] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l',
    'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4',
    '5', '6', '7', '8', '9', '-', '_',
];
