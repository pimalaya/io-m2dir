//! Free helpers backing the m2dir filename date prefix.

use alloc::string::String;

/// Extracts the `Date:` header from a MIME message and formats it as
/// `YYYY-MM-DDTHH:MM:SS[Z|±HHMM]` for the m2dir filename prefix.
///
/// Returns [`None`] if the header is missing or unparseable. Without
/// the `client` feature there is no parser available and this returns
/// [`None`] unconditionally; callers fall back to a default date.
#[cfg(feature = "client")]
pub(crate) fn extract_date(bytes: &[u8]) -> Option<String> {
    use core::fmt::Write;

    use mail_parser::MessageParser;

    let msg = MessageParser::new().with_date_headers().parse(bytes)?;
    let dt = msg.date()?;

    let mut s = String::new();
    let _ = write!(
        s,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
    );
    if dt.tz_hour == 0 && dt.tz_minute == 0 {
        s.push('Z');
    } else {
        let sign = if dt.tz_before_gmt { '-' } else { '+' };
        let _ = write!(s, "{sign}{:02}{:02}", dt.tz_hour, dt.tz_minute);
    }
    Some(s)
}

#[cfg(not(feature = "client"))]
pub(crate) fn extract_date(bytes: &[u8]) -> Option<String> {
    crate::m2dir::parse::extract_date(bytes)
}
