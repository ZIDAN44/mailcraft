//! Calendars subscribed to by their links (issue #7): an iCal file read over HTTP(S), asked only
//! when it changed since the version kept (`If-None-Match`, `If-Modified-Since`), size capped.

use crate::http::{self, Request};
use crate::{NetError, Result};

/// Largest calendar read.
pub const MAX_CALENDAR: usize = 20 << 20;

/// What reading a calendar's link gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    /// The calendar, with the version the server named.
    Calendar { ics: String, etag: String, modified: String },
    /// Not changed since the version asked about.
    Unchanged,
}

/// Read the calendar at `url` (`http://` or `https://`; redirects followed, never from HTTPS to
/// HTTP), unless it's the version named by `etag` / `modified`.
pub fn fetch(url: &str, etag: &str, modified: &str) -> Result<Fetched> {
    let mut req = Request::get(url, MAX_CALENDAR).header("Accept", "text/calendar, */*;q=0.5");
    let header = |s: &str| !s.is_empty() && s.len() < 200 && s.chars().all(|c| c.is_ascii() && !c.is_ascii_control());
    if header(etag) {
        req = req.header("If-None-Match", etag);
    }
    if header(modified) {
        req = req.header("If-Modified-Since", modified);
    }
    let resp = http::send(&req)?;
    if resp.status == 304 {
        return Ok(Fetched::Unchanged);
    }
    if !resp.ok() {
        return Err(NetError::Refused(match resp.status {
            401 | 403 => format!("the calendar's server wants a sign-in (HTTP {}); use a link that's shared publicly", resp.status),
            404 | 410 => format!("there's no calendar at that link (HTTP {})", resp.status),
            s => format!("the calendar's server answered HTTP {s}"),
        }));
    }
    let named = |h: &str| resp.header(h).map(str::trim).filter(|v| header(v)).unwrap_or("").to_string();
    Ok(Fetched::Calendar { ics: String::from_utf8_lossy(&resp.body).into_owned(), etag: named("ETag"), modified: named("Last-Modified") })
}

#[cfg(test)]
mod tests;
