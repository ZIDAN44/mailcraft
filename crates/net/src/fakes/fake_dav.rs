//! An in-memory CalDAV / CardDAV server for tests: the well-known redirect, a principal with
//! its homes, one calendar and one address book, PROPFIND, multiget REPORTs, conditional PUT
//! and DELETE, Basic authentication.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub const USER: &str = "me@studio.example";
pub const PASSWORD: &str = "pw";
pub const CAL: &str = "/dav/cal/work/";
pub const BOOK: &str = "/dav/card/main/";

#[derive(Default)]
pub struct Data {
    /// href → (etag, data)
    pub items: BTreeMap<String, (String, String)>,
    pub ctag: u32,
    pub log: Vec<String>,
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
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if line.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.trim() == auth) {
                    authorized = true;
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
            if !authorized {
                respond(&mut s, "401 Unauthorized", &[], "");
                continue;
            }
            match (method.as_str(), path.as_str()) {
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
                    let (kind, child, name) =
                        if path == "/dav/cal/" { ("<c:calendar/>", CAL, "Work") } else { ("<card:addressbook/>", BOOK, "Studio") };
                    let col = format!(
                        "<d:resourcetype><d:collection/>{kind}</d:resourcetype><d:displayname>{name}</d:displayname><cs:getctag>{}</cs:getctag><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>",
                        d.ctag
                    );
                    respond(
                        &mut s,
                        "207 Multi-Status",
                        &[],
                        &multistatus(&[response(&path, "<d:resourcetype><d:collection/></d:resourcetype>"), response(child, &col)]),
                    );
                }
                ("PROPFIND", p) if p == CAL || p == BOOK => {
                    let mut rs = vec![response(p, "<d:resourcetype><d:collection/></d:resourcetype>")];
                    for (href, (etag, _)) in d.items.iter().filter(|(h, _)| h.starts_with(p)) {
                        rs.push(response(href, &format!("<d:getetag>{}</d:getetag><d:resourcetype/>", esc(etag))));
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("REPORT", p) if p == CAL || p == BOOK => {
                    let field = if p == CAL { "c:calendar-data" } else { "card:address-data" };
                    let mut rs = Vec::new();
                    for href in body.split("<d:href>").skip(1).filter_map(|x| x.split("</d:href>").next()) {
                        if let Some((etag, data)) = d.items.get(href) {
                            rs.push(response(href, &format!("<d:getetag>{}</d:getetag><{field}>{}</{field}>", esc(etag), esc(data))));
                        }
                    }
                    respond(&mut s, "207 Multi-Status", &[], &multistatus(&rs));
                }
                ("PUT", p) => {
                    let current = d.items.get(p).map(|(e, _)| e.clone());
                    let ok = match (&current, &if_match, if_none) {
                        (Some(_), _, true) => false,
                        (Some(c), Some(m), _) => c == m,
                        (None, Some(_), _) => false,
                        _ => true,
                    };
                    if ok {
                        let etag = d.put(p, &body);
                        respond(&mut s, "201 Created", &[("ETag", etag)], "");
                    } else {
                        respond(&mut s, "412 Precondition Failed", &[], "");
                    }
                }
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

#[cfg(test)]
mod tests {
    use mailcraft_model::ical::{self, CalEvent};
    use mailcraft_model::pim::{LOCAL, Pim, StoredEvent};
    use mailcraft_model::vcard::{self, Card, Typed};

    use super::*;
    use crate::dav::{Dav, Kind, sync};

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
        let first = sync(&dav, USER, "imap.example", &pim, 100).unwrap();
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
        let second = sync(&dav, USER, "imap.example", &pim, 200).unwrap();
        assert!(second.errors.is_empty(), "{:?}", second.errors);
        assert_eq!(second.deleted.len(), 1);
        assert_eq!(second.pushed_events.len(), 2);
        let new = second.pushed_events.iter().find(|e| e.event.uid == "b@x").unwrap();
        assert!(new.href.ends_with("/b-x.ics") && !new.etag.is_empty(), "{new:?}");
        {
            let d = data.lock().unwrap();
            assert!(d.items.get(&format!("{CAL}a.ics")).is_some_and(|(_, ics)| ics.contains("Review (moved)")));
            assert!(d.items.contains_key(&format!("{CAL}b-x.ics")));
            assert!(!d.items.keys().any(|k| k.starts_with(BOOK)), "the card was deleted");
            assert!(!d.items.values().any(|(_, v)| v.contains("Only here")), "a local event stays local");
        }
        assert!(second.events.iter().any(|e| e.event.summary == "Review (moved)"), "read back after the change");

        // A stale copy isn't written over a newer one.
        let mut stale = pim.clone();
        stale.deleted.clear();
        stale.events = vec![StoredEvent { event: CalEvent { summary: "Old edit".into(), ..review.event.clone() }, dirty: true, ..review }];
        let third = sync(&dav, USER, "imap.example", &stale, 300).unwrap();
        assert_eq!(third.errors.len(), 1, "{:?}", third.errors);
        assert!(!data.lock().unwrap().items.values().any(|(_, v)| v.contains("Old edit")));
        // Nothing changed since: the calendar isn't read again.
        let mut settled = pim.clone();
        settled.deleted.clear();
        settled.calendars = third.calendars.clone().unwrap();
        settled.events.iter_mut().for_each(|e| e.dirty = false);
        settled.cards.clear();
        let fourth = sync(&dav, USER, "imap.example", &settled, 400).unwrap();
        assert_eq!(fourth.unchanged_calendars, vec![cal_id]);
        assert!(fourth.events.is_empty());
    }
}
