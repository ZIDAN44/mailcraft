//! An in-memory CalDAV / CardDAV server for tests: the well-known redirect, a principal with
//! its homes, one calendar and one address book, PROPFIND, multiget REPORTs, conditional PUT
//! and DELETE, Basic authentication. As SOGo: one uid per collection (a second is refused,
//! 409), items the user may not change (403), and a calendar-query by uid; and, when asked, a new
//! item stored over one of the same name (SOGo doesn't heed `If-None-Match`).

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use crate::xml::esc;

pub const USER: &str = "me@studio.example";
pub const PASSWORD: &str = "pw";
pub const CAL: &str = "/dav/cal/work/";
pub const BOOK: &str = "/dav/card/main/";
/// The user's scheduling outbox, when the server schedules (RFC 6638).
pub const OUTBOX: &str = "/dav/outbox/";
/// A list of tasks alone (`VTODO`), as Nextcloud's Tasks makes, when the server has one.
pub const TASKS: &str = "/dav/cal/tasks/";

#[derive(Default)]
pub struct Data {
    /// href → (etag, data)
    pub items: BTreeMap<String, (String, String)>,
    pub ctag: u32,
    pub log: Vec<String>,
    /// Items the user may read and not change or delete (SOGo's roles per classification).
    pub forbidden: Vec<String>,
    /// Store a new item over one of the same name, though asked not to (`If-None-Match: *`),
    /// as SOGo does.
    pub overwrites: bool,
    /// Says it schedules meetings itself (`calendar-auto-schedule`, RFC 6638).
    pub schedules: bool,
    /// Takes Digest sign-in only (RFC 7616, MD5 with `qop=auth`), as Baïkal does by default: its
    /// nonce, which the test may change to make the one answered stale.
    pub digest: Option<String>,
    /// Requests refused for their sign-in.
    pub refused: usize,
    /// Requests that came with a sign-in, right or wrong.
    pub offered: usize,
    /// Collections the user may not read (403 to reading them), as a calendar shared for free /
    /// busy only.
    pub unreadable: Vec<String>,
    /// People whose free/busy time the scheduling outbox gives (when it `schedules`): address →
    /// their `FREEBUSY` lines. Anyone else is an unknown calendar user.
    pub free_busy: BTreeMap<String, String>,
    /// The free/busy queries POSTed to the outbox: their `Originator` and `Recipient` headers and
    /// their body.
    pub queries: Vec<(String, String, String)>,
    /// The calendar says it takes to-dos as well as events (`VEVENT`, `VTODO`), as SOGo's and
    /// Radicale's do; else it says nothing of what it takes.
    pub cal_tasks: bool,
    /// A list of tasks alone ([`TASKS`]), as Nextcloud's; `Some(true)`: one the user may only
    /// read.
    pub task_list: Option<bool>,
    /// Require a Bearer token (Google Calendar over CalDAV): the token expected.
    pub bearer: Option<String>,
    /// Refuse the right Bearer token with 403 for a permission it lacks (`insufficient_scope`), as
    /// Google does a token from a sign-in that didn't grant calendars.
    pub bearer_forbidden: bool,
    /// Refuse it so for the address books alone, as Google does a sign-in that granted the
    /// calendars and not the contacts.
    pub bearer_lacks_contacts: bool,
    /// Or for everything but the address books: a sign-in that granted the contacts alone.
    pub bearer_lacks_calendars: bool,
    /// Refuse the right token outright (401) for the address books: not a permission it lacks.
    pub contacts_reject_token: bool,
}

/// Whether an `Authorization` value answers the fake's Digest challenge for this request.
fn digest_ok(header: &str, method: &str, path: &str, nonce: &str) -> bool {
    let Some((scheme, params)) = crate::dav::digest::challenges(header).into_iter().next() else { return false };
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or_default();
    let h = |s: &str| crate::dav::digest::hash(false, s);
    let ha1 = h(&format!("{USER}:fake:{PASSWORD}"));
    let ha2 = h(&format!("{method}:{path}"));
    let want = h(&format!("{ha1}:{nonce}:{}:{}:auth:{ha2}", get("nc"), get("cnonce")));
    scheme == "digest" && get("username") == USER && get("uri") == path && get("nonce") == nonce && get("response") == want
}

/// The uid an item's data names (its lines unfolded).
fn uid_of(data: &str) -> Option<String> {
    data.replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "")
        .lines()
        .find_map(|l| l.trim().strip_prefix("UID:").map(|u| u.trim().to_string()))
}

impl Data {
    pub fn put(&mut self, href: &str, data: &str) -> String {
        self.ctag += 1;
        let etag = format!("\"e{}\"", self.ctag);
        self.items.insert(href.to_string(), (etag.clone(), data.to_string()));
        etag
    }
}

pub struct Server {
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

fn multistatus(responses: &[String]) -> String {
    format!(
        "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\" xmlns:cs=\"http://calendarserver.org/ns/\">{}</d:multistatus>",
        responses.concat()
    )
}

fn response(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",
        esc(href)
    )
}

pub fn serve() -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(Data::default()));
    let shared = data.clone();
    let auth = format!("Basic {}", mailcraft_model::base64::encode(format!("{USER}:{PASSWORD}").as_bytes()));
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let bearer = shared.lock().unwrap().bearer.clone();
            let Some(r) = serving::read_request(&s, &auth, bearer.as_deref()) else { continue };
            let (status, mut headers, body) = shared.lock().unwrap().answer(&r);
            headers.push(("Connection", "close".into()));
            super::http::respond(&mut s, status, &headers, body.as_bytes());
        }
    });
    Server { base, data }
}

/// The account `account` in `store` has had its calendars and contacts looked for today, and
/// none found: its syncs don't look for them again, out on the network (DNS, then HTTPS at its
/// mail domain). Every test that syncs an account through the pool says so: tests never touch
/// the network, and on a busy machine that look took seconds, with whatever was queued for the
/// account after its first sync waiting for it.
pub(crate) fn looked_for_today(store: &mailcraft_store::Store, account: &str) {
    let pim = mailcraft_model::pim::Pim { dav_checked: Some(crate::worker::now()), ..Default::default() };
    store.save_account_doc(account, mailcraft_model::pim::PIM_DOC, &pim).unwrap();
}

mod serving;

#[cfg(test)]
mod scheduling;

#[cfg(test)]
mod tasks;

#[cfg(test)]
mod tests;
