//! An in-memory CalDAV / CardDAV server for tests: the well-known redirect, a principal with
//! its homes, one calendar and one address book, PROPFIND, multiget REPORTs, conditional PUT
//! and DELETE, Basic authentication. As SOGo: one uid per collection (a second is refused,
//! 409), items the user may not change (403), and a calendar-query by uid; and, when asked, a new
//! item stored over one of the same name (SOGo doesn't heed `If-None-Match`).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub const USER: &str = "me@studio.example";
pub const PASSWORD: &str = "pw";
pub const CAL: &str = "/dav/cal/work/";
pub const TASKS: &str = "/dav/cal/tasks/";
pub const NOTES: &str = "/dav/cal/notes/";
pub const BOOK: &str = "/dav/card/main/";

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
    /// Collections the user may not read (403 to reading them), as a calendar shared for free /
    /// busy only.
    pub unreadable: Vec<String>,
    /// Publishes dedicated `VTODO` (`/dav/cal/tasks/`) and `VJOURNAL` (`/dav/cal/notes/`)
    /// collections beside the main calendar, as Nextxch does.
    pub with_tasks_and_notes: bool,
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
    data.replace(
        "
 ", "",
    )
    .replace(
        "
 ", "",
    )
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

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn respond(s: &mut impl Write, status: &str, headers: &[(&str, String)], body: &str) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body.as_bytes());
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
    let auth = format!("Basic {}", crate::imap::base64(format!("{USER}:{PASSWORD}").as_bytes()));
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let (mut len, mut authorized, mut if_match, mut if_none) = (0usize, false, None::<String>, false);
            let mut given = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if let Some((_, v)) = line.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
                    authorized = v.trim() == auth;
                    given = v.trim().to_string();
                }
                if let Some(v) = line.strip_prefix("If-Match:") {
                    if_match = Some(v.trim().to_string());
                }
                if lower.starts_with("if-none-match:") {
                    if_none = true;
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            let body = String::from_utf8_lossy(&body).to_string();
            let mut parts = first.split_whitespace();
            let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
            let mut d = shared.lock().unwrap();
            d.log.push(format!("{method} {path}"));
            if let Some(nonce) = d.digest.clone() {
                authorized = digest_ok(&given, &method, &path, &nonce);
                if !authorized {
                    // The right password with a nonce no longer current: stale, as RFC 7616 says.
                    let old = crate::dav::digest::challenges(&given)
                        .into_iter()
                        .next()
                        .and_then(|(_, p)| p.into_iter().find(|(k, _)| k == "nonce"))
                        .map(|(_, v)| v);
                    let stale = old.is_some_and(|o| o != nonce && digest_ok(&given, &method, &path, &o));
                    let challenge = format!(
                        "Basic realm=\"fake\", Digest realm=\"fake\", qop=\"auth\", nonce=\"{nonce}\"{}",
                        if stale { ", stale=true" } else { "" }
                    );
                    d.refused += 1;
                    respond(&mut s, "401 Unauthorized", &[("WWW-Authenticate", challenge)], "");
                    continue;
                }
            }
            if !authorized {
                respond(&mut s, "401 Unauthorized", &[], "");
                continue;
            }
            if matches!(method.as_str(), "PROPFIND" | "REPORT") && d.unreadable.iter().any(|p| path.starts_with(p.as_str())) {
                respond(&mut s, "403 Forbidden", &[], "");
                continue;
            }
            match (method.as_str(), path.as_str()) {
                ("OPTIONS", _) => {
                    let dav = if d.schedules { "1, 2, access-control, calendar-access, calendar-auto-schedule" } else { "1, 2, 3, calendar-access" };
                    respond(&mut s, "200 OK", &[("DAV", dav.into())], "");
                }
                ("PROPFIND", "/.well-known/caldav" | "/.well-known/carddav") => respond(&mut s, "301 Moved", &[("Location", "/dav/".into())], ""),
                ("PROPFIND", "/dav/") => respond(
                    &mut s,
                    "207 Multi-Status",
                    &[],
                    &multistatus(&[response("/dav/", "<d:current-user-principal><d:href>/dav/principals/me/</d:href></d:current-user-principal>")]),
                ),
                ("PROPFIND", "/dav/principals/me/") => {
                    let props = "<c:calendar-home-set><d:href>/dav/cal/</d:href></c:calendar-home-set><card:addressbook-home-set><d:href>/dav/card/</d:href></card:addressbook-home-set>";
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&[response("/dav/principals/me/", props)]));
                }
                ("PROPFIND", "/dav/cal/" | "/dav/card/") => {
                    let mut rs = vec![response(&path, "<d:resourcetype><d:collection/></d:resourcetype>")];
                    if path == "/dav/cal/" && d.with_tasks_and_notes {
                        for (href, name, comps) in [
                            (CAL, "Work", "<c:comp name=\"VEVENT\"/><c:comp name=\"VTODO\"/><c:comp name=\"VJOURNAL\"/>"),
                            (TASKS, "Tasks", "<c:comp name=\"VTODO\"/>"),
                            (NOTES, "Notes", "<c:comp name=\"VJOURNAL\"/>"),
                        ] {
                            let col = format!(
                                "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>{name}</d:displayname><cs:getctag>{}</cs:getctag><c:supported-calendar-component-set>{comps}</c:supported-calendar-component-set><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>",
                                d.ctag
                            );
                            rs.push(response(href, &col));
                        }
                    } else {
                        let (kind, child, name) =
                            if path == "/dav/cal/" { ("<c:calendar/>", CAL, "Work") } else { ("<card:addressbook/>", BOOK, "Studio") };
                        let col = format!(
                            "<d:resourcetype><d:collection/>{kind}</d:resourcetype><d:displayname>{name}</d:displayname><cs:getctag>{}</cs:getctag><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>",
                            d.ctag
                        );
                        rs.push(response(child, &col));
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("PROPFIND", p) if p == CAL || p == TASKS || p == NOTES || p == BOOK => {
                    let mut rs = vec![response(p, "<d:resourcetype><d:collection/></d:resourcetype>")];
                    for (href, (etag, _)) in d.items.iter().filter(|(h, _)| h.starts_with(p)) {
                        rs.push(response(href, &format!("<d:getetag>{}</d:getetag><d:resourcetype/>", esc(etag))));
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("REPORT", p) if p == CAL && body.contains("calendar-query") => {
                    // By uid: its text-match finds the uid inside longer ones too, as RFC 4791's does.
                    let wanted =
                        body.split("<c:text-match").nth(1).and_then(|x| x.split_once('>')).and_then(|(_, x)| x.split("</c:text-match>").next());
                    let mut rs = Vec::new();
                    for (href, (etag, data)) in d.items.iter().filter(|(h, _)| h.starts_with(p)) {
                        if wanted.is_some_and(|w| uid_of(data).is_some_and(|u| u.contains(w))) {
                            rs.push(response(href, &format!("<d:getetag>{}</d:getetag>", esc(etag))));
                        }
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("REPORT", p) if p == CAL || p == TASKS || p == NOTES || p == BOOK => {
                    let field = if p == BOOK { "card:address-data" } else { "c:calendar-data" };
                    let mut rs = Vec::new();
                    for href in body.split("<d:href>").skip(1).filter_map(|x| x.split("</d:href>").next()) {
                        if let Some((etag, data)) = d.items.get(href) {
                            rs.push(response(href, &format!("<d:getetag>{}</d:getetag><{field}>{}</{field}>", esc(etag), esc(data))));
                        }
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("PUT", p) if d.forbidden.iter().any(|f| f == p) => respond(&mut s, "403 Forbidden", &[], ""),
                ("PUT", p)
                    if !d.items.contains_key(p)
                        && uid_of(&body).is_some_and(|u| {
                            let folder = p.rsplit_once('/').map_or("", |(f, _)| f);
                            d.items.iter().any(|(h, (_, v))| h.rsplit_once('/').map_or("", |(f, _)| f) == folder && uid_of(v).as_ref() == Some(&u))
                        }) =>
                {
                    respond(&mut s, "409 Conflict", &[], "<D:error xmlns:D=\"DAV:\">Event UID already in use.</D:error>");
                }
                ("PUT", p) => {
                    let current = d.items.get(p).map(|(e, _)| e.clone());
                    let ok = match (&current, &if_match, if_none) {
                        (Some(_), _, true) => d.overwrites,
                        (Some(c), Some(m), _) => c == m,
                        (None, Some(_), _) => false,
                        _ => true,
                    };
                    if ok {
                        let etag = d.put(p, &body);
                        if d.with_tasks_and_notes && p.starts_with(NOTES) {
                            // Nextxch's unified notes bridge rewrites the stored VJOURNAL right after PUT.
                            let _ = d.put(p, &body);
                        }
                        respond(&mut s, "201 Created", &[("ETag", etag)], "");
                    } else {
                        respond(&mut s, "412 Precondition Failed", &[], "");
                    }
                }
                ("DELETE", p) if d.forbidden.iter().any(|f| f == p) => respond(&mut s, "403 Forbidden", &[], ""),
                ("DELETE", p) => {
                    if d.items.remove(p).is_some() {
                        d.ctag += 1;
                        respond(&mut s, "204 No Content", &[], "");
                    } else {
                        respond(&mut s, "404 Not Found", &[], "");
                    }
                }
                _ => respond(&mut s, "404 Not Found", &[], ""),
            }
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

#[cfg(test)]
mod scheduling;

#[cfg(test)]
mod tests {
    use mailcraft_model::ical::{self, CalEvent};
    use mailcraft_model::pim::{LOCAL, Pim, StoredEvent};
    use mailcraft_model::vcard::{self, Card, Typed};
    use mailcraft_model::zone::Zone;

    use super::*;
    use crate::dav::{Dav, Kind, Where, sync};

    /// Found from the address, as an account with nothing given by hand. The tests' syncs never
    /// look for anything there, out on the network: their calendars and contacts are given, or
    /// were looked for already (`dav_checked`), the contacts none.
    const AT: Where<'static> = Where { email: USER, username: "", imap_host: "imap.example", caldav: None, carddav: None, mails: true };

    fn event(uid: &str, summary: &str, start: i64) -> CalEvent {
        CalEvent { uid: uid.into(), summary: summary.into(), start, end: start + 3600, ..Default::default() }
    }

    #[test]
    fn finds_reads_writes_and_deletes() {
        let server = serve();
        let data = server.data.clone();
        {
            let mut d = data.lock().unwrap();
            d.put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Review", 1_800_000_000)], None, 0));
            d.put(
                &format!("{BOOK}z.vcf"),
                &vcard::write(&Card {
                    uid: "z".into(),
                    name: "Zoe Park".into(),
                    emails: vec![Typed { value: "zoe@studio.example".into(), kind: String::new() }],
                    ..Default::default()
                }),
            );
        }
        let dav = Dav::new(USER, PASSWORD);
        assert!(matches!(
            Dav::new(USER, "nope").home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)),
            Err(crate::NetError::Auth(_))
        ));
        // Discovery: the well-known address redirects to the principal, which names the homes.
        let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
        assert_eq!(home, format!("{}/dav/cal/", server.base));
        let book_home = dav.home_from(Kind::Contacts, &format!("{}/.well-known/carddav", server.base)).unwrap().unwrap();

        // A first sync reads everything.
        let mut pim = Pim { caldav_home: Some(home), carddav_home: Some(book_home), ..Default::default() };
        pim.ensure_defaults();
        let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
        let cal_id = format!("{}{CAL}", server.base);
        assert_eq!(first.calendars.as_ref().map(|c| (c.len(), c[0].name.as_str(), c[0].id.as_str())), Some((1, "Work", cal_id.as_str())));
        assert_eq!(first.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
        assert_eq!(first.cards.iter().map(|c| c.card.display()).collect::<Vec<_>>(), ["Zoe Park"]);

        // The engine would merge this; mirror it, then change things here.
        pim.calendars = first.calendars.clone().unwrap();
        pim.books = first.books.clone().unwrap();
        pim.events = first.events.clone();
        pim.cards = first.cards.clone();
        let review = pim.events[0].clone();
        pim.events[0].event.summary = "Review (moved)".into();
        pim.events[0].dirty = true;
        pim.events.push(StoredEvent { calendar: cal_id.clone(), event: event("b@x", "New / one", 1_800_100_000), dirty: true, ..Default::default() });
        pim.events.push(StoredEvent { calendar: LOCAL.into(), event: event("l", "Only here", 1_800_000_000), dirty: false, ..Default::default() });
        pim.deleted.push(mailcraft_model::pim::Tombstone { href: pim.cards[0].href.clone(), etag: pim.cards[0].etag.clone(), ..Default::default() });
        let second = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(second.deleted.len(), 1);
        assert_eq!(second.pushed_events.len(), 2);
        let new = second.pushed_events.iter().find(|e| e.event.uid == "b@x").unwrap();
        let name = new.href.rsplit('/').next().unwrap_or_default().to_string();
        assert!(name.starts_with("b-x~") && name.ends_with(".ics") && !new.etag.is_empty(), "{new:?}");
        {
            let d = data.lock().unwrap();
            assert!(d.items.get(&format!("{CAL}a.ics")).is_some_and(|(_, ics)| ics.contains("Review (moved)")));
            assert!(d.items.contains_key(&format!("{CAL}{name}")));
            assert!(!d.items.keys().any(|k| k.starts_with(BOOK)), "the card was deleted");
            assert!(!d.items.values().any(|(_, v)| v.contains("Only here")), "a local event stays local");
        }
        assert!(second.events.iter().any(|e| e.event.summary == "Review (moved)"), "read back after the change");

        // A stale copy isn't written over a newer one: the server's comes in its place (it was
        // sent again, and refused again, at every sync).
        let mut stale = pim.clone();
        stale.deleted.clear();
        stale.events = vec![StoredEvent { event: CalEvent { summary: "Old edit".into(), ..review.event.clone() }, dirty: true, ..review.clone() }];
        let third = sync(&dav, &AT, &stale, 300, Zone::UTC).unwrap();
        assert_eq!(third.errors.len(), 1, "{:?}", third.errors);
        assert!(!data.lock().unwrap().items.values().any(|(_, v)| v.contains("Old edit")));
        assert_eq!(third.refused, ["a"]);
        assert!(third.events.iter().any(|e| e.event.uid == "a" && e.event.summary == "Review (moved)"), "the server's copy read");
        // The server has what was changed here already (SOGo put a guest's answer in the
        // organizer's copy itself): its copy is kept without a word.
        let mut same = pim.clone();
        same.deleted.clear();
        same.events =
            vec![StoredEvent { event: CalEvent { summary: "Review (moved)".into(), ..review.event.clone() }, dirty: true, ..review.clone() }];
        let quiet = sync(&dav, &AT, &same, 300, Zone::UTC).unwrap();
        assert_eq!((quiet.errors.len(), quiet.refused.len()), (0, 1), "{:?}", quiet.errors);
        // Nothing changed since: the calendar isn't read again.
        let mut settled = pim.clone();
        settled.deleted.clear();
        settled.calendars = third.calendars.clone().unwrap();
        settled.events.iter_mut().for_each(|e| e.dirty = false);
        settled.cards.clear();
        let fourth = sync(&dav, &AT, &settled, 400, Zone::UTC).unwrap();
        assert_eq!(fourth.unchanged_calendars, vec![cal_id]);
        assert!(fourth.events.is_empty());
    }

    /// Two new events whose long uids differ only after the part a file name keeps each get an
    /// item of their own; an event already on the server keeps its address.
    #[test]
    fn long_uids_get_items_of_their_own() {
        let server = serve();
        let data = server.data.clone();
        let dav = Dav::new(USER, PASSWORD);
        let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
        let cal_id = format!("{}{CAL}", server.base);
        let mut pim = Pim { caldav_home: Some(home), carddav_home: Some(String::new()), dav_checked: Some(100), ..Default::default() };
        pim.ensure_defaults();
        let long = "x".repeat(125);
        let old_href = format!("{cal_id}{}.ics", "y".repeat(120));
        let etag = data
            .lock()
            .unwrap()
            .put(&format!("{CAL}{}.ics", "y".repeat(120)), &ical::write(&[event(&format!("{long}-old"), "Old", 1_800_000_000)], None, 0));
        let new = |uid: &str| StoredEvent { calendar: cal_id.clone(), event: event(uid, uid, 1_800_000_000), dirty: true, ..Default::default() };
        pim.events = vec![
            new(&format!("{long}-one")),
            new(&format!("{long}-two")),
            StoredEvent { href: old_href.clone(), etag, ..new(&format!("{long}-old")) },
        ];
        let out = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let hrefs: Vec<&str> = out.pushed_events.iter().map(|e| e.href.as_str()).collect();
        assert_eq!(hrefs.len(), 3);
        assert_ne!(hrefs[0], hrefs[1], "{hrefs:?}");
        assert!(hrefs.contains(&old_href.as_str()), "an item already stored keeps its address: {hrefs:?}");
        let d = data.lock().unwrap();
        for uid in ["one", "two"] {
            let on_server = d.items.values().any(|(_, v)| ical::parse(v, 0).events.iter().any(|e| e.uid == format!("{long}-{uid}")));
            assert!(on_server, "{uid} is on the server");
        }
    }

    /// A calendar or address book the user may not read (403: a calendar shared for free / busy
    /// only) is neither the whole sync's failure nor a password refused, as it was (found through
    /// a proxy in front of Baïkal, issue #61): the rest syncs, and it's kept as it is, and said.
    #[test]
    fn a_collection_the_user_may_not_read_is_kept_and_the_rest_synced() {
        let server = serve();
        {
            let mut d = server.data.lock().unwrap();
            d.put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Review", 1_800_000_000)], None, 0));
            d.unreadable.push(BOOK.into());
        }
        let home = |k: &str| Some(format!("{}/dav/{k}/", server.base));
        let mut pim = Pim { caldav_home: home("cal"), carddav_home: home("card"), dav_checked: Some(100), ..Default::default() };
        pim.ensure_defaults();
        let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
        assert_eq!(out.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
        let book = format!("{}{BOOK}", server.base);
        assert_eq!((out.unchanged_books.clone(), out.synced_books.len()), (vec![book], 0));
        assert!(out.errors.iter().any(|e| e.contains("couldn't be read") && e.contains("403")), "{:?}", out.errors);
    }

    /// Issue #7: an address given by hand, as a user copies it from the provider's help: the
    /// server's own address, the home, or one calendar in it (whose parent is the home); one with
    /// no calendars, or not https across the network, is said; a wrong password is refused.
    #[test]
    fn an_address_given_by_hand_finds_the_home() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        let cal_home = format!("{}/dav/cal/", server.base);
        let book_home = format!("{}/dav/card/", server.base);
        for given in [
            server.base.clone(),
            format!("{}/", server.base),
            cal_home.clone(),
            format!("{}{CAL}", server.base),
            format!("{}/dav/cal/work", server.base),
        ] {
            assert_eq!(dav.home_given(Kind::Calendars, &given).unwrap(), cal_home, "{given}");
        }
        assert_eq!(dav.home_given(Kind::Contacts, &format!("{}{BOOK}", server.base)).unwrap(), book_home);
        let none = dav.home_given(Kind::Calendars, &format!("{}/nothing/here", server.base)).unwrap_err();
        assert!(none.to_string().contains("no calendars were found"), "{none}");
        assert!(matches!(Dav::new(USER, "nope").home_given(Kind::Calendars, &cal_home), Err(crate::NetError::Auth(_))));
        assert!(dav.home_given(Kind::Calendars, "http://calendar.example/dav/").unwrap_err().to_string().contains("https"));
        assert!(dav.home_given(Kind::Calendars, "").is_err());

        // In a sync: the given calendar's home is found, its events read; contacts are found at
        // the address given for them too (from the account's address, they'd be looked for on
        // the network). A given address that holds nothing is said once and remembered as none,
        // not an error that stops the sync.
        let at = Where { caldav: Some(&format!("{}{CAL}", server.base)), carddav: Some(&format!("{}{BOOK}", server.base)), ..AT };
        let mut pim = Pim::default();
        pim.ensure_defaults();
        server.data.lock().unwrap().put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Planning", 1_800_000_000)], None, 0));
        let out = sync(&dav, &at, &pim, 100, Zone::UTC).unwrap();
        assert_eq!(out.caldav_home.as_deref(), Some(cal_home.as_str()));
        assert_eq!(out.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Planning"]);
        let wrong = format!("{}/nothing/", server.base);
        let out = sync(&dav, &Where { caldav: Some(&wrong), ..at }, &pim, 100, Zone::UTC).unwrap();
        assert!(out.caldav_home.is_none() && out.errors.iter().any(|e| e.contains("no calendars")), "{:?}", out.errors);
        assert!(matches!(sync(&Dav::new(USER, "nope"), &at, &pim, 100, Zone::UTC), Err(crate::NetError::Auth(_))));
    }

    /// From the review: nothing goes to a server other than the one the homes are on (items
    /// left from an old calendar server aren't sent there with this one's password); a server
    /// that can't be reached isn't taken for one with nothing there; plain http goes nowhere but
    /// this computer, whatever points there.
    #[test]
    fn only_the_found_server_is_written_to() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        let home = format!("{}/dav/cal/", server.base);
        let mut pim = Pim { caldav_home: Some(home.clone()), carddav_home: Some(String::new()), dav_checked: Some(100), ..Default::default() };
        pim.ensure_defaults();
        let elsewhere = "http://127.0.0.2:9/cal/work/";
        pim.events.push(StoredEvent {
            calendar: elsewhere.into(),
            href: format!("{elsewhere}x.ics"),
            etag: "\"1\"".into(),
            event: event("x", "Old server", 1_800_000_000),
            dirty: true,
        });
        pim.deleted.push(mailcraft_model::pim::Tombstone { href: format!("{elsewhere}gone.ics"), ..Default::default() });
        let out = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
        assert!(out.errors.is_empty() && out.pushed_events.is_empty() && out.deleted.is_empty(), "{:?}", out.errors);
        assert!(!server.data.lock().unwrap().log.iter().any(|l| l.contains("x.ics") || l.contains("gone")));

        // Nothing listening: said as it is (and tried again), not "no calendars".
        let closed = "http://127.0.0.1:9/cal/";
        let e = dav.home_given(Kind::Calendars, closed).unwrap_err();
        assert!(!matches!(e, crate::NetError::No(_)), "{e:?}");
        assert!(sync(&dav, &Where { caldav: Some(closed), ..AT }, &Pim::default(), 100, Zone::UTC).is_err());

        // Plain http to another computer: refused before anything is sent.
        assert!(matches!(dav.home_from(Kind::Calendars, "http://calendar.example/dav/"), Err(crate::NetError::Tls(_))));
    }

    /// A calendar on the server, read, as the engine keeps it.
    fn read(server: &Server, dav: &Dav) -> Pim {
        let mut pim = Pim {
            caldav_home: Some(format!("{}/dav/cal/", server.base)),
            carddav_home: Some(String::new()),
            dav_checked: Some(100),
            ..Default::default()
        };
        pim.ensure_defaults();
        let first = sync(dav, &AT, &pim, 100, Zone::UTC).unwrap();
        pim.calendars = first.calendars.unwrap();
        pim.events = first.events;
        pim
    }

    /// As a real Mailcow's SOGo does: an event the user may see and not change (a role for its
    /// classification, in a calendar where they may change others) is refused with 403. It was
    /// taken for a wrong password, and the whole sync of calendars and contacts stopped, at
    /// every sync. Now the change and the deletion are given up and the server's copy read
    /// again; the sync goes on.
    #[test]
    fn a_change_the_server_forbids_is_given_up() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        {
            let mut d = server.data.lock().unwrap();
            d.put(&format!("{CAL}conf.ics"), &ical::write(&[event("conf", "Interview", 1_800_000_000)], None, 0));
            d.put(&format!("{CAL}mine.ics"), &ical::write(&[event("mine", "Lunch", 1_800_100_000)], None, 0));
            d.forbidden = vec![format!("{CAL}conf.ics"), format!("{CAL}mine.ics")];
        }
        let mut pim = read(&server, &dav);
        let conf = pim.events.iter().position(|e| e.event.uid == "conf").unwrap();
        pim.events[conf].event.summary = "Bob was here".into();
        pim.events[conf].dirty = true;
        let mine = pim.events.iter().position(|e| e.event.uid == "mine").unwrap();
        let gone = pim.events.remove(mine);
        pim.deleted.push(mailcraft_model::pim::Tombstone { href: gone.href.clone(), etag: gone.etag.clone(), ..Default::default() });
        pim.events.push(StoredEvent {
            calendar: pim.calendars[0].id.clone(),
            event: event("new", "Standup", 1_800_200_000),
            dirty: true,
            ..Default::default()
        });
        let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
        assert_eq!(out.refused, ["conf"]);
        assert_eq!(out.deleted, std::slice::from_ref(&gone.href), "the deletion is given up");
        assert_eq!(out.errors.len(), 2, "both said: {:?}", out.errors);
        assert!(out.errors.iter().any(|e| e.contains("Interview") || e.contains("Bob was here")), "{:?}", out.errors);
        let back: Vec<&str> = out.events.iter().map(|e| e.event.summary.as_str()).collect();
        assert!(back.contains(&"Interview") && back.contains(&"Lunch"), "the server's copies read again: {back:?}");
        assert_eq!(out.pushed_events.iter().map(|e| e.event.uid.as_str()).collect::<Vec<_>>(), ["new"], "the rest is stored");
        let d = server.data.lock().unwrap();
        assert!(!d.items.values().any(|(_, v)| v.contains("Bob was here")));
    }

    /// SOGo puts a meeting in the calendar of each of its users invited, under the organizer's
    /// item name, before the invitation's mail comes. The invitation taken in here (a new item,
    /// named by its uid) was refused (409, "Event UID already in use") at every sync, and stayed
    /// apart from the server's copy. It goes over the server's copy now, so there's one, when it
    /// has the user's answer; otherwise (and when the server's is a newer version) the server's
    /// comes in its place. Stored over, an answer given on another device was undone (a real
    /// Mailcow: accepted on the phone, back to "needs action" when the laptop woke up).
    #[test]
    fn a_meeting_the_server_put_in_the_calendar_already_is_stored_over() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        let meeting = |uid: &str, sequence: u32, summary: &str, answer: &str| CalEvent {
            sequence,
            attendees: vec![ical::Attendee { email: USER.into(), partstat: answer.into(), ..Default::default() }],
            ..event(uid, summary, 1_800_000_000)
        };
        {
            let mut d = server.data.lock().unwrap();
            d.put(&format!("{CAL}organizers-name.ics"), &ical::write(&[meeting("m1", 0, "Planning", "NEEDS-ACTION")], None, 0));
            d.put(&format!("{CAL}updated.ics"), &ical::write(&[meeting("m2", 2, "Review (moved)", "NEEDS-ACTION")], None, 0));
            d.put(&format!("{CAL}phone.ics"), &ical::write(&[meeting("m3", 0, "Standup", "ACCEPTED")], None, 0));
            // Another whose uid holds this one's: not taken for it.
            d.put(&format!("{CAL}longer.ics"), &ical::write(&[meeting("m1-and-more", 0, "Other", "NEEDS-ACTION")], None, 0));
        }
        let mut pim = Pim {
            caldav_home: Some(format!("{}/dav/cal/", server.base)),
            carddav_home: Some(String::new()),
            dav_checked: Some(100),
            ..Default::default()
        };
        pim.ensure_defaults();
        let cal = format!("{}{CAL}", server.base);
        // Answered here: m1. Taken in from the mail only: m2 (older) and m3 (answered elsewhere).
        let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
        pim.events.push(new(meeting("m1", 0, "Planning", "TENTATIVE")));
        pim.events.push(new(meeting("m2", 1, "Review", "NEEDS-ACTION")));
        pim.events.push(new(meeting("m3", 0, "Standup", "NEEDS-ACTION")));
        let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let m1 = out.pushed_events.iter().find(|e| e.event.uid == "m1").expect("stored");
        assert_eq!(m1.href, format!("{cal}organizers-name.ics"), "over the server's copy");
        assert_eq!(out.refused, ["m2", "m3"], "the server's is newer, or has all this one has");
        assert!(out.events.iter().any(|e| e.event.uid == "m2" && e.event.summary == "Review (moved)"), "read in its place");
        assert!(out.events.iter().any(|e| e.event.uid == "m3" && e.event.attendees[0].partstat == "ACCEPTED"), "read in its place");
        let d = server.data.lock().unwrap();
        let with = |uid: &str| d.items.values().filter(|(_, v)| uid_of(v).as_deref() == Some(uid)).count();
        assert_eq!((with("m1"), with("m2"), with("m3"), with("m1-and-more")), (1, 1, 1, 1));
        assert!(d.items[&format!("{CAL}organizers-name.ics")].1.contains("PARTSTAT=TENTATIVE"));
        assert!(d.items[&format!("{CAL}updated.ics")].1.contains("Review (moved)"));
        assert!(d.items[&format!("{CAL}phone.ics")].1.contains("PARTSTAT=ACCEPTED"), "the answer given elsewhere stays");
    }

    /// SOGo stores a new item over one of the same name, though asked not to (If-None-Match),
    /// and puts a meeting in each invited user's calendar under the organizer's item name, which
    /// MailCraft's (and so the invitation taken in here) names by its uid. On a real Mailcow, bob
    /// accepted on his phone, his laptop woke up, took the invitation in from the mail and stored
    /// it over the calendar's copy: back to "needs action". The calendar is asked for the meeting
    /// first now; its copy comes in, unless this one has an answer the server's lacks.
    #[test]
    fn a_meeting_stored_under_its_own_name_already_isnt_written_over() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        let ann = Some(ical::Attendee { email: "ann@x".into(), ..Default::default() });
        let meeting = |uid: &str, answer: &str| CalEvent {
            organizer: ann.clone(),
            attendees: vec![ical::Attendee { email: USER.into(), partstat: answer.into(), ..Default::default() }],
            ..event(uid, "Planning", 1_800_000_000)
        };
        {
            let mut d = server.data.lock().unwrap();
            d.overwrites = true;
            d.put(&format!("{CAL}p1.ics"), &ical::write(&[meeting("p1", "ACCEPTED")], None, 0));
            d.put(&format!("{CAL}p2.ics"), &ical::write(&[meeting("p2", "NEEDS-ACTION")], None, 0));
        }
        let mut pim = Pim {
            caldav_home: Some(format!("{}/dav/cal/", server.base)),
            carddav_home: Some(String::new()),
            dav_checked: Some(100),
            ..Default::default()
        };
        pim.ensure_defaults();
        let cal = format!("{}{CAL}", server.base);
        let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
        // p1: taken in from the mail, answered elsewhere since. p2: answered here.
        pim.events.push(new(meeting("p1", "NEEDS-ACTION")));
        pim.events.push(new(meeting("p2", "DECLINED")));
        let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.refused, ["p1"]);
        assert!(out.events.iter().any(|e| e.event.uid == "p1" && e.event.attendees[0].partstat == "ACCEPTED"), "the server's copy read");
        assert_eq!(out.pushed_events.iter().map(|e| e.event.uid.as_str()).collect::<Vec<_>>(), ["p2"]);
        let d = server.data.lock().unwrap();
        assert!(d.items[&format!("{CAL}p1.ics")].1.contains("PARTSTAT=ACCEPTED"), "the answer given elsewhere stays");
        assert!(d.items[&format!("{CAL}p2.ics")].1.contains("PARTSTAT=DECLINED"));
    }

    /// SOGo writes a guest's answer into the organizer's copy (and the other guests'), without a
    /// new SEQUENCE. A change made here meanwhile (the meeting's notes, a version on) was refused
    /// with 412 and the server's copy kept: on a real Mailcow, the agenda ann wrote right after
    /// taking bob's answer in was gone, without a word. It's stored now, with the answers the
    /// server has. A change to the same version as the server's is still given way to, said
    /// unless the server's copy has all of it.
    #[test]
    fn a_change_crossing_a_guests_answer_keeps_both() {
        let server = serve();
        let dav = Dav::new(USER, PASSWORD);
        let guests = |bob: &str, carol: &str| {
            vec![
                ical::Attendee { email: "bob@x".into(), partstat: bob.into(), ..Default::default() },
                ical::Attendee { email: "carol@x".into(), partstat: carol.into(), ..Default::default() },
            ]
        };
        let base = CalEvent { attendees: guests("NEEDS-ACTION", "NEEDS-ACTION"), ..event("m", "Budget", 1_800_000_000) };
        server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(std::slice::from_ref(&base), None, 0));
        let pim = read(&server, &dav);
        // bob answers: the server writes it in, the same version.
        let answered = CalEvent { attendees: guests("ACCEPTED", "NEEDS-ACTION"), ..base.clone() };
        server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(std::slice::from_ref(&answered), None, 0));
        // The organizer writes the agenda (a new version) and takes carol's answer in.
        let mut mine = pim.clone();
        mine.events[0].event.description = "Agenda: numbers".into();
        mine.events[0].event.sequence = 1;
        mine.events[0].event.attendees = guests("NEEDS-ACTION", "DECLINED");
        mine.events[0].dirty = true;
        let out = sync(&dav, &AT, &mine, 200, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.refused, ["m"], "the stored copy is read back");
        let back = out.events.iter().find(|e| e.event.uid == "m").expect("read back");
        assert_eq!(back.event.description, "Agenda: numbers");
        let answers: Vec<&str> = back.event.attendees.iter().map(|a| a.partstat.as_str()).collect();
        assert_eq!(answers, ["ACCEPTED", "DECLINED"], "both answers");
        // The same version changed here and there: the server's is kept, and the change that
        // isn't in it is said (it used to go without a word when the title and times matched).
        let mut same = read(&server, &dav);
        server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(&[CalEvent { sequence: 1, ..back.event.clone() }], None, 0));
        same.events[0].event.description = "Agenda: other numbers".into();
        same.events[0].dirty = true;
        let out = sync(&dav, &AT, &same, 300, Zone::UTC).unwrap();
        assert_eq!((out.errors.len(), out.refused.len()), (1, 1), "{:?}", out.errors);
        assert!(server.data.lock().unwrap().items[&format!("{CAL}m.ics")].1.contains("Agenda: numbers"));
        // An event without guests changed elsewhere (another program needn't give it a new
        // SEQUENCE): not written over, the server's kept and the change here said.
        server.data.lock().unwrap().put(&format!("{CAL}solo.ics"), &ical::write(&[event("solo", "Dentist", 1_800_100_000)], None, 0));
        let mut solo = read(&server, &dav);
        server.data.lock().unwrap().put(&format!("{CAL}solo.ics"), &ical::write(&[event("solo", "Dentist (moved)", 1_800_100_000)], None, 0));
        solo.events.retain(|e| e.event.uid == "solo");
        solo.events[0].event.description = "Bring the card".into();
        solo.events[0].event.sequence = 1;
        solo.events[0].dirty = true;
        let out = sync(&dav, &AT, &solo, 400, Zone::UTC).unwrap();
        assert_eq!((out.errors.len(), out.refused.as_slice()), (1, ["solo".to_string()].as_slice()), "{:?}", out.errors);
        assert!(server.data.lock().unwrap().items[&format!("{CAL}solo.ics")].1.contains("Dentist (moved)"));
    }

    #[test]
    fn tasks_and_notes_sync_over_dedicated_caldav_collections() {
        use mailcraft_model::pim::{Note, StoredNote, StoredTask, Task, TaskPriority, TaskStatus, Tombstone};

        let server = serve();
        {
            let mut d = server.data.lock().unwrap();
            d.with_tasks_and_notes = true;
            d.put(
                &format!("{TASKS}server-t.ics"),
                &ical::write_calendar(
                    &[],
                    &[Task {
                        uid: "server-t".into(),
                        title: "Server task".into(),
                        status: TaskStatus::InProgress,
                        percent: 50,
                        priority: TaskPriority::High,
                        ..Default::default()
                    }],
                    100,
                ),
            );
            d.put(
                &format!("{NOTES}server-n.ics"),
                &ical::write_journal_calendar(
                    &Note { uid: "server-n".into(), text: "Server note\nFrom webmail".into(), color: 1, modified: 100, files: Vec::new() },
                    100,
                ),
            );
        }
        let dav = Dav::new(USER, PASSWORD);
        let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
        let mut pim = Pim {
            caldav_home: Some(home),
            carddav_home: Some(String::new()),
            dav_checked: Some(100),
            tasks: vec![StoredTask {
                task: Task { uid: "local-t".into(), title: "Local task".into(), due: Some(86_400 * 20), ..Default::default() },
                dirty: true,
                ..Default::default()
            }],
            notes: vec![StoredNote {
                note: Note { uid: "local-n".into(), text: "Local note\nLine two".into(), color: 2, modified: 120, files: Vec::new() },
                dirty: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let first = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        assert_eq!(first.calendars.as_ref().map(Vec::len), Some(1), "task and note collections stay out of event calendars");
        assert_eq!(first.pushed_tasks.len(), 1);
        assert_eq!(first.pushed_notes.len(), 1);
        let tasks = first.tasks.clone().expect("tasks synced");
        let notes = first.notes.clone().expect("notes synced");
        assert_eq!(tasks.len(), 2);
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().any(|n| n.note.uid == "local-n" && n.note.color == 2 && n.note.text == "Local note\nLine two"));

        // Edit the note and delete the server task: even with a bridge-bumped etag, both go cleanly.
        pim.tasks = tasks;
        pim.notes = notes;
        let gone = pim.tasks.iter().find(|t| t.task.uid == "server-t").unwrap().clone();
        pim.tasks.retain(|t| t.task.uid != "server-t");
        pim.deleted.push(Tombstone { href: gone.href, etag: gone.etag, ..Default::default() });
        let ln = pim.notes.iter_mut().find(|n| n.note.uid == "local-n").unwrap();
        ln.etag = "\"stale\"".into();
        ln.note.text = "Local note\nUpdated body".into();
        ln.dirty = true;

        let second = sync(&dav, &AT, &pim, 300, Zone::UTC).unwrap();
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(second.tasks.as_ref().map(Vec::len), Some(1));
        assert_eq!(second.notes.as_ref().unwrap().iter().find(|n| n.note.uid == "local-n").unwrap().note.text, "Local note\nUpdated body");
    }
}
