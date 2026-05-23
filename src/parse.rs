//! Minimal crate-local module for extracting parts of a MIME message
//! without parsing the whole structure.

use core::{
    error::Error,
    fmt::{self, Display},
    iter::Peekable,
};

use alloc::string::String;

pub fn extract_date(mime: &str) -> Option<Datetime> {
    mime.lines().find_map(|line| {
        if line.starts_with("Date:") {
            line.trim_start_matches("Date:")
                .trim()
                .split(';')
                .next()
                .and_then(|s| parse_rfc2822_datetime(s).ok())
        } else {
            None
        }
    })
}

#[derive(Debug, PartialEq)]
pub struct Datetime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub offset_minutes: i16,
}

impl Default for Datetime {
    fn default() -> Self {
        Self {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            offset_minutes: 0,
        }
    }
}

impl Display for Datetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )?;
        if self.offset_minutes == 0 {
            write!(f, "Z")
        } else {
            let sign = if self.offset_minutes.is_positive() {
                '+'
            } else {
                '-'
            };
            let hours = self.offset_minutes / 60;
            let minutes = self.offset_minutes % 60;
            write!(f, "{sign}{hours:02}{minutes:02}")
        }
    }
}

pub fn parse_rfc2822_datetime(s: &str) -> Result<Datetime, Rfc2822Error> {
    let chars = &mut s.chars().peekable();
    skip_whitespace(chars);

    // Skip day of week
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
        return Err(Rfc2822Error::DayOutOfRange(day));
    }

    let month = parse_month(chars)?;
    next_space(chars)?;

    let year = next_four_digit_number(chars)?;
    next_space(chars)?;

    let hour = next_two_digit_number(chars)?;
    next_colon(chars)?;
    if hour > 23 {
        return Err(Rfc2822Error::HourOutOfRange(hour));
    }

    let minute = next_two_digit_number(chars)?;
    next_colon(chars)?;
    if minute > 59 {
        return Err(Rfc2822Error::MinuteOutOfRange(minute));
    }

    let second = next_two_digit_number(chars)?;
    skip_whitespace(chars);
    // NOTE: leap second is allowed.
    if second > 60 {
        return Err(Rfc2822Error::SecondOutOfRange(minute));
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

fn parse_month<I>(chars: &mut Peekable<I>) -> Result<u8, Rfc2822Error>
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
        _ => Err(Rfc2822Error::InvalidMonth),
    }
}

fn parse_timezone(chars: &mut impl Iterator<Item = char>) -> Result<i16, Rfc2822Error> {
    match chars.next() {
        Some(c @ '+') | Some(c @ '-') => {
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
                _ => Err(Rfc2822Error::InvalidTimezone(s)),
            }
        }
        _ => Err(Rfc2822Error::UnexpectedEnd),
    }
}

fn next_two_digit_number(chars: &mut impl Iterator<Item = char>) -> Result<u8, Rfc2822Error> {
    Ok(10 * next_digit(chars)? + next_digit(chars)?)
}

fn next_four_digit_number(chars: &mut impl Iterator<Item = char>) -> Result<u16, Rfc2822Error> {
    Ok(1000 * next_digit(chars)? as u16
        + 100 * next_digit(chars)? as u16
        + 10 * next_digit(chars)? as u16
        + next_digit(chars)? as u16)
}

#[inline]
fn next_digit(chars: &mut impl Iterator<Item = char>) -> Result<u8, Rfc2822Error> {
    match chars.next() {
        Some(c) if c.is_ascii_digit() => Ok(c as u8 - b'0'),
        Some(_) => Err(Rfc2822Error::ExpectedDigit),
        None => Err(Rfc2822Error::UnexpectedEnd),
    }
}

#[inline]
fn next_colon(chars: &mut impl Iterator<Item = char>) -> Result<(), Rfc2822Error> {
    match chars.next() {
        Some(':') => Ok(()),
        Some(_) => Err(Rfc2822Error::ExpectedColon),
        None => Err(Rfc2822Error::UnexpectedEnd),
    }
}

#[inline]
fn next_space(chars: &mut impl Iterator<Item = char>) -> Result<(), Rfc2822Error> {
    match chars.next() {
        Some(' ') => Ok(()),
        Some(_) => Err(Rfc2822Error::ExpectedSpace),
        None => Err(Rfc2822Error::UnexpectedEnd),
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

#[derive(Debug)]
pub enum Rfc2822Error {
    UnexpectedEnd,
    ExpectedDigit,
    ExpectedColon,
    ExpectedSpace,
    InvalidMonth,
    InvalidTimezone(String),
    DayOutOfRange(u8),
    HourOutOfRange(u8),
    MinuteOutOfRange(u8),
    SecondOutOfRange(u8),
}

impl Error for Rfc2822Error {}

impl Display for Rfc2822Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error when parsing RFC 2822 datetime: ")?;
        match self {
            Rfc2822Error::UnexpectedEnd => write!(f, "unexpected end of string"),
            Rfc2822Error::ExpectedDigit => write!(f, "expected digit"),
            Rfc2822Error::ExpectedColon => write!(f, "expected colon"),
            Rfc2822Error::ExpectedSpace => write!(f, "expected space"),
            Rfc2822Error::InvalidMonth => write!(f, "invalid month"),
            Rfc2822Error::InvalidTimezone(tz) => write!(f, "invalid timezone: {tz}"),
            Rfc2822Error::DayOutOfRange(d) => write!(f, "day out of range: {d}"),
            Rfc2822Error::HourOutOfRange(h) => write!(f, "hour out of range: {h}"),
            Rfc2822Error::MinuteOutOfRange(m) => write!(f, "minute out of range: {m}"),
            Rfc2822Error::SecondOutOfRange(s) => write!(f, "second out of range: {s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parse::*;

    #[test]
    fn valid_datetime() {
        let dt = parse_rfc2822_datetime("Tue, 15 Apr 1994 08:12:31 GMT").unwrap();
        assert_eq!(
            dt,
            Datetime {
                year: 1994,
                month: 4,
                day: 15,
                hour: 8,
                minute: 12,
                second: 31,
                offset_minutes: 0,
            }
        );

        let dt = parse_rfc2822_datetime("15 Jun 1994 08:12:31 +0200").unwrap();
        assert_eq!(
            dt,
            Datetime {
                year: 1994,
                month: 6,
                day: 15,
                hour: 8,
                minute: 12,
                second: 31,
                offset_minutes: 120,
            }
        );

        let dt = parse_rfc2822_datetime("Tue, 15 Nov 1994 08:12:31 -0430").unwrap();
        assert_eq!(
            dt,
            Datetime {
                year: 1994,
                month: 11,
                day: 15,
                hour: 8,
                minute: 12,
                second: 31,
                offset_minutes: -270,
            }
        );
    }

    #[test]
    fn invalid_datetime() {
        match parse_rfc2822_datetime("Tue, 15 Xov 1994 08:12:31 GMT") {
            Err(Rfc2822Error::InvalidMonth) => {}
            r => panic!("Expected invalid month error. Got: {r:?}"),
        }

        match parse_rfc2822_datetime("15 Nov 1994 24:12:31 -0500") {
            Err(Rfc2822Error::HourOutOfRange(_)) => {}
            r => panic!("Expected hour out of range error. Got: {r:?}"),
        }

        match parse_rfc2822_datetime("15 Nov 1994 08:12:31 FOO") {
            Err(Rfc2822Error::InvalidTimezone(_)) => {}
            r => panic!("Expected invalid timezone error. Got: {r:?}"),
        }

        match parse_rfc2822_datetime("Tue, 32 Nov 1994 08:12:31 -0500") {
            Err(Rfc2822Error::DayOutOfRange(_)) => {}
            r => panic!("Expected day out of range error. Got: {r:?}"),
        }
    }
}
