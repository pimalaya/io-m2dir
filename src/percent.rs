//! Crate-local percent-encoding helpers used by m2store folder name
//! encoding. Kept in-crate to preserve the original m2dir
//! specification output.

use core::fmt::{self, Write};

use alloc::{
    string::{FromUtf8Error, String},
    vec::Vec,
};

fn percent_encode_byte(byte: u8) -> &'static str {
    let index = byte as usize * 3;

    &"\
      %00%01%02%03%04%05%06%07%08%09%0A%0B%0C%0D%0E%0F\
      %10%11%12%13%14%15%16%17%18%19%1A%1B%1C%1D%1E%1F\
      %20%21%22%23%24%25%26%27%28%29%2A%2B%2C%2D%2E%2F\
      %30%31%32%33%34%35%36%37%38%39%3A%3B%3C%3D%3E%3F\
      %40%41%42%43%44%45%46%47%48%49%4A%4B%4C%4D%4E%4F\
      %50%51%52%53%54%55%56%57%58%59%5A%5B%5C%5D%5E%5F\
      %60%61%62%63%64%65%66%67%68%69%6A%6B%6C%6D%6E%6F\
      %70%71%72%73%74%75%76%77%78%79%7A%7B%7C%7D%7E%7F\
      %80%81%82%83%84%85%86%87%88%89%8A%8B%8C%8D%8E%8F\
      %90%91%92%93%94%95%96%97%98%99%9A%9B%9C%9D%9E%9F\
      %A0%A1%A2%A3%A4%A5%A6%A7%A8%A9%AA%AB%AC%AD%AE%AF\
      %B0%B1%B2%B3%B4%B5%B6%B7%B8%B9%BA%BB%BC%BD%BE%BF\
      %C0%C1%C2%C3%C4%C5%C6%C7%C8%C9%CA%CB%CC%CD%CE%CF\
      %D0%D1%D2%D3%D4%D5%D6%D7%D8%D9%DA%DB%DC%DD%DE%DF\
      %E0%E1%E2%E3%E4%E5%E6%E7%E8%E9%EA%EB%EC%ED%EE%EF\
      %F0%F1%F2%F3%F4%F5%F6%F7%F8%F9%FA%FB%FC%FD%FE%FF\
      "[index..index + 3]
}

pub fn percent_encode_bytes<W: Write>(bytes: &[u8], mut w: W) -> fmt::Result {
    bytes.iter().try_for_each(|byte| {
        if matches!(byte, b'%' | b'/' | b'\\') || !byte.is_ascii() {
            write!(w, "{}", percent_encode_byte(*byte))
        } else {
            write!(w, "{}", char::from(*byte))
        }
    })
}

pub fn percent_decode_bytes(bytes: impl IntoIterator<Item = u8>) -> Result<String, FromUtf8Error> {
    let mut decoded = Vec::new();
    let mut iter = bytes.into_iter();
    while let Some(byte) = iter.next() {
        match byte {
            b'%' => match iter.next() {
                Some(h) => match iter.next() {
                    Some(l) => match (char::from(h).to_digit(16), char::from(l).to_digit(16)) {
                        (Some(h), Some(l)) => decoded.push(h as u8 * 0x10 + l as u8),
                        _ => {
                            decoded.push(b'%');
                            decoded.push(h);
                            decoded.push(l);
                        }
                    },
                    None => {
                        decoded.push(b'%');
                        decoded.push(h);
                    }
                },
                None => decoded.push(b'%'),
            },
            _ => decoded.push(byte),
        }
    }
    String::from_utf8(decoded)
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::percent::*;

    #[test]
    fn decode_success() {
        let s = "test%20test";
        assert_eq!(
            percent_decode_bytes(s.bytes()).unwrap(),
            "test test".to_string()
        );

        let s = "test%2test";
        assert_eq!(
            percent_decode_bytes(s.bytes()).unwrap(),
            "test%2test".to_string()
        );

        let s = "%C3%A6%C3%A5%C3%B8%E1%BA%BD%C3%AF%C3%A9%C3%A0";
        assert_eq!(
            percent_decode_bytes(s.bytes()).unwrap(),
            "æåøẽïéà".to_string()
        );
    }

    #[test]
    #[should_panic]
    fn decode_fail() {
        let s = "test%21test";
        assert_eq!(
            percent_decode_bytes(s.bytes()).unwrap(),
            "test test".to_string()
        );
    }

    #[test]
    fn encode() {
        let s = "encode this ø";
        let want = "encode this %C3%B8";
        let mut got = String::new();
        percent_encode_bytes(s.as_bytes(), &mut got).unwrap();
        assert_eq!(want, got);

        let s = "æåøẽïéà";
        let want = "%C3%A6%C3%A5%C3%B8%E1%BA%BD%C3%AF%C3%A9%C3%A0";
        let mut got = String::new();
        percent_encode_bytes(s.as_bytes(), &mut got).unwrap();
        assert_eq!(want, got);
    }
}
