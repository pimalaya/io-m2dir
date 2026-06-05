//! No-std fallback `Date:`-header parser used when the `client`
//! feature is off (which would otherwise pull in `mail-parser` and its
//! `std` dependency). Kept here only because mail-parser is not
//! `no_std`-compatible; client builds use mail-parser instead.

use core::{
    fmt::{self, Write},
    iter::Peekable,
};

use alloc::string::String;

/// Returns the formatted `YYYY-MM-DDTHH:MM:SS[Z|±HHMM]` date from the
/// MIME message's `Date:` header, or [`None`] if missing/unparseable.
pub(crate) fn extract_date(bytes: &[u8]) -> Option<String> {
    let mime = core::str::from_utf8(bytes).ok()?;
    let value = mime.lines().find_map(|line| {
        let rest = line.strip_prefix("Date:")?;
        rest.trim().split(';').next()
    })?;
    let dt = parse_rfc2822_datetime(value).ok()?;
    let mut s = String::new();
    fmt_datetime(&dt, &mut s).ok()?;
    Some(s)
}

struct Datetime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    offset_minutes: i16,
}

fn fmt_datetime(dt: &Datetime, w: &mut String) -> fmt::Result {
    write!(
        w,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
    )?;
    if dt.offset_minutes == 0 {
        w.write_char('Z')
    } else {
        let sign = if dt.offset_minutes.is_positive() {
            '+'
        } else {
            '-'
        };
        let abs = dt.offset_minutes.unsigned_abs();
        write!(w, "{sign}{:02}{:02}", abs / 60, abs % 60)
    }
}

fn parse_rfc2822_datetime(s: &str) -> Result<Datetime, ()> {
    let chars = &mut s.chars().peekable();
    skip_whitespace(chars);

    // Skip optional day of week ("Mon, ", "Tue, ", ...)
    if let Some(c) = chars.peek()
        && c.is_alphabetic()
    {
        for c in chars.by_ref() {
            if c == ',' {
                break;
            }
        }
        skip_whitespace(chars);
    }

    let day = next_two_digit_number(chars)?;
    next_space(chars)?;
    if !(1..=31).contains(&day) {
        return Err(());
    }

    let month = parse_month(chars)?;
    next_space(chars)?;

    let year = next_four_digit_number(chars)?;
    next_space(chars)?;

    let hour = next_two_digit_number(chars)?;
    next_colon(chars)?;
    if hour > 23 {
        return Err(());
    }

    let minute = next_two_digit_number(chars)?;
    next_colon(chars)?;
    if minute > 59 {
        return Err(());
    }

    // NOTE: leap second is allowed (60).
    let second = next_two_digit_number(chars)?;
    skip_whitespace(chars);
    if second > 60 {
        return Err(());
    }

    let offset_minutes = parse_timezone(chars)?;

    Ok(Datetime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        offset_minutes,
    })
}

fn parse_month<I>(chars: &mut Peekable<I>) -> Result<u8, ()>
where
    I: Iterator<Item = char>,
{
    match (chars.next(), chars.next(), chars.next()) {
        (Some('J'), Some('a'), Some('n')) => Ok(1),
        (Some('F'), Some('e'), Some('b')) => Ok(2),
        (Some('M'), Some('a'), Some('r')) => Ok(3),
        (Some('A'), Some('p'), Some('r')) => Ok(4),
        (Some('M'), Some('a'), Some('y')) => Ok(5),
        (Some('J'), Some('u'), Some('n')) => Ok(6),
        (Some('J'), Some('u'), Some('l')) => Ok(7),
        (Some('A'), Some('u'), Some('g')) => Ok(8),
        (Some('S'), Some('e'), Some('p')) => Ok(9),
        (Some('O'), Some('c'), Some('t')) => Ok(10),
        (Some('N'), Some('o'), Some('v')) => Ok(11),
        (Some('D'), Some('e'), Some('c')) => Ok(12),
        _ => Err(()),
    }
}

fn parse_timezone(chars: &mut impl Iterator<Item = char>) -> Result<i16, ()> {
    match chars.next() {
        Some(c @ ('+' | '-')) => {
            let sign = if c == '+' { 1 } else { -1 };
            let hour = next_two_digit_number(chars)? as i16;
            let minute = next_two_digit_number(chars)? as i16;
            Ok(sign * (hour * 60 + minute))
        }
        Some(c) => {
            let mut s = String::from(c);
            for c in chars {
                s.push(c);
            }
            match s.as_str() {
                "Z" | "UTC" | "GMT" => Ok(0),
                "EDT" => Ok(-4 * 60),
                "EST" | "CDT" => Ok(-5 * 60),
                "CST" | "MDT" => Ok(-6 * 60),
                "MST" | "PDT" => Ok(-6 * 60),
                "PST" => Ok(-7 * 60),
                _ => Err(()),
            }
        }
        None => Err(()),
    }
}

fn next_two_digit_number(chars: &mut impl Iterator<Item = char>) -> Result<u8, ()> {
    Ok(10 * next_digit(chars)? + next_digit(chars)?)
}

fn next_four_digit_number(chars: &mut impl Iterator<Item = char>) -> Result<u16, ()> {
    Ok(1000 * next_digit(chars)? as u16
        + 100 * next_digit(chars)? as u16
        + 10 * next_digit(chars)? as u16
        + next_digit(chars)? as u16)
}

#[inline]
fn next_digit(chars: &mut impl Iterator<Item = char>) -> Result<u8, ()> {
    match chars.next() {
        Some(c) if c.is_ascii_digit() => Ok(c as u8 - b'0'),
        _ => Err(()),
    }
}

#[inline]
fn next_colon(chars: &mut impl Iterator<Item = char>) -> Result<(), ()> {
    match chars.next() {
        Some(':') => Ok(()),
        _ => Err(()),
    }
}

#[inline]
fn next_space(chars: &mut impl Iterator<Item = char>) -> Result<(), ()> {
    match chars.next() {
        Some(' ') => Ok(()),
        _ => Err(()),
    }
}

fn skip_whitespace(chars: &mut Peekable<impl Iterator<Item = char>>) {
    while let Some(c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_from_full_message() {
        let mime = "Date: Tue, 15 Apr 1994 08:12:31 GMT\r\nFrom: a@b\r\n\r\nbody";
        assert_eq!(
            extract_date(mime.as_bytes()).as_deref(),
            Some("1994-04-15T08:12:31Z"),
        );
    }

    #[test]
    fn formats_positive_offset() {
        let mime = "Date: 15 Jun 1994 08:12:31 +0200\r\n";
        assert_eq!(
            extract_date(mime.as_bytes()).as_deref(),
            Some("1994-06-15T08:12:31+0200"),
        );
    }

    #[test]
    fn formats_negative_offset() {
        let mime = "Date: 15 Nov 1994 08:12:31 -0430\r\n";
        assert_eq!(
            extract_date(mime.as_bytes()).as_deref(),
            Some("1994-11-15T08:12:31-0430"),
        );
    }

    #[test]
    fn missing_header_is_none() {
        let mime = "From: a@b\r\n\r\nbody";
        assert_eq!(extract_date(mime.as_bytes()), None);
    }

    #[test]
    fn invalid_date_is_none() {
        let mime = "Date: not a date\r\n";
        assert_eq!(extract_date(mime.as_bytes()), None);
    }
}
