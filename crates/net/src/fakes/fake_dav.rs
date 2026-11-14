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
    /// Requests that came with a sign-in, right or wrong.
    pub offered: usize,
    /// Collections the user may not read (403 to reading them), as a calendar shared for free /
    /// busy only.
    pub unreadable: Vec<String>,
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
    let auth = format!("Basic {}", mailcraft_model::base64::encode(format!("{USER}:{PASSWORD}").as_bytes()));
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
            if !given.is_empty() {
                d.offered += 1;
            }
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
                respond(&mut s, "401 Unauthorized", &[("WWW-Authenticate", "Basic realm=\"fake\"".to_string())], "");
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
mod tests;
