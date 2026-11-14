//! Mail and calendar protocols, each over a stream: IMAP, POP, SMTP, ManageSieve, JMAP, and
//! CalDAV/CardDAV.

pub mod dav;
pub mod imap;
pub mod jmap;
pub mod pop;
pub mod sieve;
pub mod smtp;
pub mod web_calendar;

#[cfg(test)]
mod starttls_tests;

use std::io::{BufRead, Read};

/// A text protocol's quoted string (IMAP's, ManageSieve's, Sieve's): in double quotes, with `\`
/// and `"` escaped.
pub(crate) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What [`capped_line`] read.
pub(crate) enum Capped {
    /// A line, its line break kept.
    Line(Vec<u8>),
    /// Nothing: the server closed the connection.
    Closed,
    /// A line longer than it had room for (or cut short by the connection closing).
    TooLong,
}

/// The next line a server sends, read whole when it takes at most `max` bytes.
pub(crate) fn capped_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Capped> {
    let mut buf = Vec::new();
    let n = r.take(u64::try_from(max).unwrap_or(u64::MAX)).read_until(b'\n', &mut buf)?;
    Ok(if n == 0 {
        Capped::Closed
    } else if buf.ends_with(b"\n") {
        Capped::Line(buf)
    } else {
        Capped::TooLong
    })
}
