//! What the page downloads for the user: the pictures a message links to, and calendars
//! subscribed to by their links (issue #7).

use mailcraft_engine::model::Event;
use mailcraft_jmap::Http as _;

use super::{Fetch, Rcs, emit};

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;
/// Largest calendar read in the browser.
const MAX_CALENDAR: usize = 20 << 20;

/// Download a picture a message links to (once the user asked to see pictures).
pub(super) async fn fetch_url(sh: Rcs, url: String) {
    let r = Fetch.send(mailcraft_jmap::Request::get(&url, MAX_PICTURE)).await;
    let e = match r {
        Ok(resp) if (200..300).contains(&resp.status) => Event::Fetched { url, data: Some(resp.body), error: None },
        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
    };
    emit(&sh, e);
}

/// A calendar subscribed to by its link (issue #7), read by the page: only from servers that
/// let web pages read their calendars (CORS); the desktop app reads any.
pub(super) async fn fetch_calendar(sh: Rcs, account: String, calendar: String, url: String) {
    let r = Fetch.send(mailcraft_jmap::Request::get(&url, MAX_CALENDAR)).await;
    let (ics, error) = match r {
        Ok(resp) if (200..300).contains(&resp.status) => (Some(String::from_utf8_lossy(&resp.body).into_owned()), None),
        Ok(resp) => (None, Some(format!("the calendar's server answered HTTP {}", resp.status))),
        Err(_) => (None, Some("the calendar's server doesn't let web pages read it; the desktop app can".to_string())),
    };
    let e = Event::CalendarFetched { account, calendar, ics, etag: String::new(), modified: String::new(), unchanged: false, error };
    emit(&sh, e);
}
