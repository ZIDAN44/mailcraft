//! Calendars and address books on the account's server: CalDAV (RFC 4791) and CardDAV
//! (RFC 6352) over WebDAV (RFC 4918), found as RFC 6764 describes (an SRV record and the path
//! its TXT record gives, then the `/.well-known/` addresses), or at an address the user gave
//! (issue #7); signed in with the account's password, or one of its own.
//!
//! A sync first sends what changed here (deletions, then edits and new items, each guarded by
//! the item's etag so a newer copy on the server is never overwritten), then reads every
//! collection whose version (`getctag`) moved: the items' etags, and the data of those that are
//! new or changed (`multiget`). The engine merges the result ([`DavSync`]).
//!
//! Server answers are hostile: sizes are capped and XML is read without entities beyond XML's
//! own.

use std::collections::HashMap;

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{BookInfo, CalendarInfo, DavSync, LOCAL, Pim, StoredCard, StoredEvent};
use mailcraft_model::vcard;
use quick_xml::Reader;
use quick_xml::events::Event as X;

use crate::http::{self, Request, Response, Url};
use crate::{NetError, Result};

/// Largest answer read.
const MAX_BODY: usize = 32 << 20;
/// Items fetched per `multiget`.
const BATCH: usize = 50;
const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// One `<response>` of a multistatus: the resource and the properties found (200).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub href: String,
    /// Text properties by local name (`displayname`, `getetag`, `getctag`, `calendar-data`…).
    pub props: HashMap<String, String>,
    /// Hrefs inside properties (`current-user-principal`, `calendar-home-set`…).
    pub hrefs: HashMap<String, Vec<String>>,
    /// The kinds in `resourcetype` (`collection`, `calendar`, `addressbook`).
    pub kinds: Vec<String>,
    /// `supported-calendar-component-set` (`VEVENT`, `VTODO`…).
    pub comps: Vec<String>,
    /// The user may write to it (a `write` privilege, or `all`); `None` when not said.
    pub writable: Option<bool>,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_ascii_lowercase()
}

/// Read a `207 Multi-Status` body.
pub fn multistatus(xml: &str) -> Vec<Item> {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut out: Vec<Item> = Vec::new();
    let mut cur: Option<Item> = None;
    // The properties of the propstat being read, kept only if its status is 200.
    let mut pending = Item::default();
    let mut status = String::new();
    let mut text = String::new();
    let mut privileges: Vec<String> = Vec::new();
    while let Ok(ev) = r.read_event() {
        // `<x/>` is a start and an end at once.
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(e) | X::Empty(e) if stack.len() < 64 => {
                let name = local(e.name().as_ref());
                let parent = stack.last().cloned().unwrap_or_default();
                if parent == "resourcetype" {
                    pending.kinds.push(name.clone());
                }
                if name == "comp" && parent == "supported-calendar-component-set" {
                    for a in e.attributes().flatten() {
                        if local(a.key.as_ref()) == "name" {
                            pending.comps.push(a.value.to_ascii_uppercase());
                        }
                    }
                }
                if stack.iter().any(|s| s == "privilege") {
                    privileges.push(name.clone());
                }
                match name.as_str() {
                    "response" => {
                        cur = Some(Item::default());
                    }
                    "propstat" => {
                        pending = Item::default();
                        status.clear();
                    }
                    _ => {}
                }
                text.clear();
                if !empty {
                    stack.push(name);
                } else {
                    end_tag(&name, &stack, &mut cur, &mut pending, &mut status, &mut privileges, &text, &mut out);
                }
            }
            X::Text(t) => {
                if text.len() < MAX_BODY {
                    text.push_str(&t);
                }
            }
            X::CData(t) => {
                if text.len() < MAX_BODY {
                    text.push_str(&t);
                }
            }
            X::GeneralRef(g) => {
                let name = g.to_string();
                let c = match name.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n => n
                        .strip_prefix("#x")
                        .or_else(|| n.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                if let Some(c) = c {
                    text.push(c);
                }
            }
            X::End(e) => {
                let name = local(e.name().as_ref());
                if stack.last() == Some(&name) {
                    stack.pop();
                }
                end_tag(&name, &stack, &mut cur, &mut pending, &mut status, &mut privileges, &text, &mut out);
                text.clear();
            }
            X::Eof => break,
            _ => {}
        }
        if out.len() > 100_000 {
            break;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn end_tag(
    name: &str,
    stack: &[String],
    cur: &mut Option<Item>,
    pending: &mut Item,
    status: &mut String,
    privileges: &mut Vec<String>,
    text: &str,
    out: &mut Vec<Item>,
) {
    let parent = stack.last().map(String::as_str).unwrap_or("");
    match name {
        "href" if parent == "response" => {
            if let Some(c) = cur.as_mut() {
                c.href = text.trim().to_string();
            }
        }
        "href" => {
            // An href inside a property: file it under the property.
            if let Some(prop) = stack.iter().rev().find(|s| s.as_str() != "href" && s.as_str() != "prop") {
                pending.hrefs.entry(prop.clone()).or_default().push(text.trim().to_string());
            }
        }
        "status" if parent == "propstat" => *status = text.to_string(),
        "current-user-privilege-set" => {
            pending.writable = Some(privileges.iter().any(|p| p == "write" || p == "all" || p == "write-content" || p == "bind"));
            privileges.clear();
        }
        "propstat" => {
            let ok = status.split_whitespace().nth(1).is_none_or(|c| c.starts_with('2'));
            if ok && let Some(c) = cur.as_mut() {
                let p = std::mem::take(pending);
                c.props.extend(p.props);
                for (k, v) in p.hrefs {
                    c.hrefs.entry(k).or_default().extend(v);
                }
                c.kinds.extend(p.kinds);
                c.comps.extend(p.comps);
                c.writable = c.writable.or(p.writable);
            }
        }
        "response" => {
            if let Some(c) = cur.take() {
                out.push(c);
            }
        }
        _ => {
            if parent == "prop" && !text.trim().is_empty() {
                pending.props.insert(name.to_string(), text.trim().to_string());
            }
        }
    }
}

/// Where an account's calendars and contacts are looked for: from its address and mail server,
/// or at the addresses the user gave.
#[derive(Clone, Copy, Debug, Default)]
pub struct Where<'a> {
    pub email: &'a str,
    pub imap_host: &'a str,
    pub caldav: Option<&'a str>,
    pub carddav: Option<&'a str>,
}

/// What a collection holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Calendars,
    Contacts,
}

impl Kind {
    fn well_known(self) -> &'static str {
        match self {
            Kind::Calendars => "/.well-known/caldav",
            Kind::Contacts => "/.well-known/carddav",
        }
    }
    fn srv(self) -> &'static str {
        match self {
            Kind::Calendars => "_caldavs._tcp",
            Kind::Contacts => "_carddavs._tcp",
        }
    }
    fn home(self) -> &'static str {
        match self {
            Kind::Calendars => "calendar-home-set",
            Kind::Contacts => "addressbook-home-set",
        }
    }
}

/// A signed-in DAV session.
pub struct Dav {
    auth: String,
}

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        Dav { auth: format!("Basic {}", crate::imap::base64(format!("{user}:{password}").as_bytes())) }
    }

    /// One request, following redirects (WebDAV methods keep their method and body).
    fn request(&self, method: &str, url: &str, depth: Option<&str>, body: &str, extra: &[(&str, &str)]) -> Result<(Response, String)> {
        let mut url = url.to_string();
        for _ in 0..6 {
            let mut req = if body.is_empty() {
                Request { method: method.into(), url: url.clone(), max: MAX_BODY, ..Default::default() }
            } else {
                Request {
                    method: method.into(),
                    url: url.clone(),
                    headers: vec![("Content-Type".into(), "application/xml; charset=utf-8".into())],
                    body: body.as_bytes().to_vec(),
                    max: MAX_BODY,
                }
            };
            req = req.header("Authorization", &self.auth);
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            for (k, v) in extra {
                req = req.header(k, v);
            }
            let resp = http::send(&req)?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = resp.header("Location")
            {
                let from = Url::parse(&url)?;
                let next = from.join(loc)?;
                if from.https && !next.https {
                    return Err(NetError::Tls("redirected from https to http".into()));
                }
                url = format!("{}{}", next.origin(), next.path);
                continue;
            }
            if matches!(resp.status, 401 | 403) {
                return Err(NetError::Auth("the calendar server refused the user name or password".into()));
            }
            return Ok((resp, url));
        }
        Err(NetError::Protocol("too many redirects".into()))
    }

    fn propfind(&self, url: &str, depth: &str, props: &str) -> Result<(Vec<Item>, String)> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"{DAV}\" xmlns:c=\"{CALDAV}\" xmlns:card=\"{CARDDAV}\" xmlns:cs=\"http://calendarserver.org/ns/\" xmlns:a=\"http://apple.com/ns/ical/\"><d:prop>{props}</d:prop></d:propfind>"
        );
        let (resp, at) = self.request("PROPFIND", url, Some(depth), &body, &[])?;
        if resp.status != 207 {
            return Err(NetError::No(format!("PROPFIND answered HTTP {}", resp.status)));
        }
        Ok((multistatus(&String::from_utf8_lossy(&resp.body)), at))
    }

    /// The home of the user's calendars or address books, found from the account's domain.
    pub fn discover(&self, kind: Kind, email: &str, imap_host: &str) -> Result<Option<String>> {
        let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
        let srv = if domain.is_empty() {
            None
        } else {
            let name = format!("{}.{domain}", kind.srv());
            crate::dns::srv(&name).map(|(host, port)| (host, port, crate::dns::txt_path(&name)))
        };
        let mut starts = srv_starts(kind, srv);
        let service = if kind == Kind::Calendars { "caldav" } else { "carddav" };
        for host in [domain.clone(), format!("{service}.{domain}"), imap_host.replacen("imap.", &format!("{service}."), 1), imap_host.to_string()] {
            if !host.is_empty() && !host.contains(['/', ' ']) {
                let u = format!("https://{host}{}", kind.well_known());
                if !starts.contains(&u) {
                    starts.push(u);
                }
            }
        }
        // A refused sign-in while looking (Google's servers want OAuth) means "not here".
        for start in starts {
            if let Ok(Some(h)) = self.home_from(kind, &start) {
                return Ok(Some(h));
            }
        }
        Ok(None)
    }

    /// The home at an address the user gave: the server's own address (its principal found from
    /// there or its `/.well-known/` one), the home itself, or one calendar or address book in it
    /// (Posteo's settings give `https://posteo.de:8443/calendars/<user>/default`), whose parent is
    /// the home. A refused sign-in is said; an address that holds none says so too.
    pub fn home_given(&self, kind: Kind, given: &str) -> Result<String> {
        let start = given_url(given).ok_or_else(|| NetError::No(format!("`{}` isn't an https:// address", given.trim())))?;
        let mut tries = vec![start.clone()];
        if let Ok(u) = Url::parse(&start)
            && (u.path.is_empty() || u.path == "/")
        {
            tries.push(format!("{}{}", u.origin(), kind.well_known()));
        }
        for t in &tries {
            match self.home_from(kind, t) {
                Ok(Some(h)) => return Ok(h),
                Err(e @ NetError::Auth(_)) => return Err(e),
                _ => {}
            }
        }
        // The address is a home, or a collection in one.
        if self.collections(kind, &start).is_ok_and(|c| !c.is_empty()) {
            return Ok(start);
        }
        if let Some(parent) = parent_of(&start)
            && self.collections(kind, &parent).is_ok_and(|c| !c.is_empty())
        {
            return Ok(parent);
        }
        let what = if kind == Kind::Calendars { "calendars" } else { "address books" };
        Err(NetError::No(format!("no {what} were found at {start}")))
    }

    /// From a starting address: the principal, then its home.
    pub fn home_from(&self, kind: Kind, start: &str) -> Result<Option<String>> {
        let (items, at) = self.propfind(start, "0", "<d:current-user-principal/>")?;
        let Some(principal) = items.iter().find_map(|i| i.hrefs.get("current-user-principal").and_then(|h| h.first())).cloned() else {
            return Ok(None);
        };
        let principal = absolute(&at, &principal)?;
        let prop = if kind == Kind::Calendars { "<c:calendar-home-set/>" } else { "<card:addressbook-home-set/>" };
        let (items, at) = self.propfind(&principal, "0", prop)?;
        let home = items.iter().find_map(|i| i.hrefs.get(kind.home()).and_then(|h| h.first())).cloned();
        home.map(|h| absolute(&at, &h)).transpose()
    }

    /// The collections in a home (calendars that hold events, or address books).
    pub fn collections(&self, kind: Kind, home: &str) -> Result<Vec<Item>> {
        let props = "<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/>";
        let (items, at) = self.propfind(home, "1", props)?;
        let want = if kind == Kind::Calendars { "calendar" } else { "addressbook" };
        let mut out = Vec::new();
        for mut i in items.into_iter().filter(|i| i.kinds.iter().any(|k| k == want)) {
            if kind == Kind::Calendars && !i.comps.is_empty() && !i.comps.iter().any(|c| c == "VEVENT") {
                continue;
            }
            i.href = absolute(&at, &i.href)?;
            out.push(i);
        }
        Ok(out)
    }

    /// The items in a collection and their etags.
    pub fn etags(&self, collection: &str) -> Result<Vec<(String, String)>> {
        let (items, at) = self.propfind(collection, "1", "<d:getetag/><d:resourcetype/>")?;
        let mut out = Vec::new();
        for i in items {
            if i.kinds.iter().any(|k| k == "collection") {
                continue;
            }
            let Some(etag) = i.props.get("getetag") else { continue };
            out.push((absolute(&at, &i.href)?, etag.clone()));
        }
        Ok(out)
    }

    /// The data of items (`calendar-data` or `address-data`), by absolute href.
    pub fn multiget(&self, kind: Kind, collection: &str, hrefs: &[String]) -> Result<Vec<(String, String, String)>> {
        let mut out = Vec::new();
        for chunk in hrefs.chunks(BATCH) {
            let refs: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(&path_of(h)))).collect();
            let body = match kind {
                Kind::Calendars => format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-multiget xmlns:d=\"{DAV}\" xmlns:c=\"{CALDAV}\"><d:prop><d:getetag/><c:calendar-data/></d:prop>{refs}</c:calendar-multiget>"
                ),
                Kind::Contacts => format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><card:addressbook-multiget xmlns:d=\"{DAV}\" xmlns:card=\"{CARDDAV}\"><d:prop><d:getetag/><card:address-data/></d:prop>{refs}</card:addressbook-multiget>"
                ),
            };
            let (resp, at) = self.request("REPORT", collection, Some("1"), &body, &[])?;
            if resp.status != 207 {
                return Err(NetError::No(format!("REPORT answered HTTP {}", resp.status)));
            }
            let field = if kind == Kind::Calendars { "calendar-data" } else { "address-data" };
            for i in multistatus(&String::from_utf8_lossy(&resp.body)) {
                if let Some(data) = i.props.get(field) {
                    out.push((absolute(&at, &i.href)?, i.props.get("getetag").cloned().unwrap_or_default(), data.clone()));
                }
            }
        }
        Ok(out)
    }

    /// Store an item: a new one (`etag` `None`) only if nothing is there, else only if it is
    /// unchanged on the server. Returns the new etag (empty when the server doesn't say).
    pub fn put(&self, url: &str, content_type: &str, data: &str, etag: Option<&str>) -> Result<String> {
        let cond = match etag {
            Some(e) if !e.is_empty() => ("If-Match", e.to_string()),
            _ => ("If-None-Match", "*".to_string()),
        };
        let mut req = Request {
            method: "PUT".into(),
            url: url.into(),
            headers: vec![("Content-Type".into(), content_type.into())],
            body: data.as_bytes().to_vec(),
            max: 1 << 20,
        };
        req = req.header("Authorization", &self.auth).header(cond.0, &cond.1);
        let resp = http::send(&req)?;
        match resp.status {
            200..=299 => Ok(resp.header("ETag").unwrap_or_default().to_string()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            412 => Err(NetError::No("changed on the server meanwhile".into())),
            s => Err(NetError::No(format!("the server didn't store it (HTTP {s})"))),
        }
    }

    pub fn delete(&self, url: &str, etag: &str) -> Result<()> {
        let mut req = Request { method: "DELETE".into(), url: url.into(), max: 1 << 20, ..Default::default() }.header("Authorization", &self.auth);
        if !etag.is_empty() {
            req = req.header("If-Match", etag);
        }
        let resp = http::send(&req)?;
        match resp.status {
            200..=299 | 404 | 410 => Ok(()),
            401 | 403 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Err(NetError::No(format!("the server didn't delete it (HTTP {s})"))),
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// An href (absolute, or a path) made absolute against the address it came from.
fn absolute(base: &str, href: &str) -> Result<String> {
    let u = Url::parse(base)?.join(href.trim())?;
    Ok(format!("{}{}", u.origin(), u.path))
}

/// Where to look first for a service its SRV record names (`host`, `port`, and its TXT record's
/// path): that path, its `/.well-known/` address, then the server's own root (Posteo publishes
/// `path=/` on port 8443, where `/.well-known/caldav` isn't found).
fn srv_starts(kind: Kind, srv: Option<(String, u16, Option<String>)>) -> Vec<String> {
    let Some((host, port, path)) = srv else { return Vec::new() };
    let base = if port == 443 { format!("https://{host}") } else { format!("https://{host}:{port}") };
    let mut starts: Vec<String> = Vec::new();
    for p in [path.as_deref(), Some(kind.well_known()), Some("/")].into_iter().flatten() {
        let u = format!("{base}{p}");
        if !starts.contains(&u) {
            starts.push(u);
        }
    }
    starts
}

/// An address the user typed, as one to ask: `posteo.de:8443` and `https://posteo.de:8443/` both
/// read as https; plain http only to this computer (a test server), never a password in the clear
/// across the network.
pub fn given_url(given: &str) -> Option<String> {
    let g = given.trim();
    if g.is_empty() || g.len() > 2048 || g.contains(char::is_whitespace) {
        return None;
    }
    let lower = g.to_ascii_lowercase();
    let full = if lower.starts_with("https://") || lower.starts_with("http://") { g.to_string() } else { format!("https://{g}") };
    let u = Url::parse(&full).ok()?;
    let local = matches!(u.host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
    if !u.https && !local {
        return None;
    }
    Some(format!("{}{}", u.origin(), if u.path.is_empty() { "/" } else { &u.path }))
}

/// The collection an address is in (`…/calendars/me/default` → `…/calendars/me/`); none above a
/// server's root.
fn parent_of(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let trimmed = u.path.trim_end_matches('/');
    let cut = trimmed.rfind('/')?;
    let parent = trimmed.get(..=cut)?;
    (!parent.is_empty() && parent != u.path).then(|| format!("{}{parent}", u.origin()))
}

/// The path of an absolute address (multiget names items by path).
fn path_of(url: &str) -> String {
    Url::parse(url).map(|u| u.path).unwrap_or_else(|_| url.to_string())
}

/// The longest uid used as a file name as it is.
const NAME_KEEP: usize = 100;

/// A file name for a new item (one already on the server keeps the address it has). A short
/// uid of plain characters is the name; any other keeps its readable start and adds `~` and a
/// hash of the whole uid, so two uids alike in what's kept still get names of their own (a plain
/// name never has a `~`).
fn resource_name(uid: &str, ext: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if !uid.is_empty() && uid.len() <= NAME_KEEP && uid.chars().all(plain) {
        return format!("{uid}.{ext}");
    }
    let safe: String = uid.chars().map(|c| if plain(c) { c } else { '-' }).take(NAME_KEEP).collect();
    format!("{safe}~{:016x}.{ext}", mailcraft_store::fnv64(uid))
}

/// Sync an account's calendars and address books with its server. `pim` is what the engine
/// last saved; nothing is written here: the result is for the engine to merge.
pub fn sync(dav: &Dav, at: &Where, pim: &Pim, now: i64, local: mailcraft_model::zone::Zone) -> Result<DavSync> {
    let mut out = DavSync::default();
    // Where the collections live (looked up once; again a day after finding none). An address
    // the user gave is followed instead of looking, and what's wrong with it is said.
    let retry = pim.dav_checked.is_none_or(|t| now - t > 86_400);
    // A given address that holds nothing is said once and remembered as none, as one that wasn't
    // found is (looked at again a day on, or when it's changed); a refused sign-in stops the sync.
    let find = |kind: Kind, given: Option<&str>, errors: &mut Vec<String>| -> Result<Option<String>> {
        match given.filter(|g| !g.trim().is_empty()) {
            Some(g) => match dav.home_given(kind, g) {
                Ok(h) => Ok(Some(h)),
                Err(e @ NetError::Auth(_)) => Err(e),
                Err(e) => {
                    errors.push(e.to_string());
                    Ok(None)
                }
            },
            None => dav.discover(kind, at.email, at.imap_host),
        }
    };
    out.caldav_home = match &pim.caldav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Calendars, at.caldav, &mut out.errors)?,
        _ => None,
    };
    out.carddav_home = match &pim.carddav_home {
        Some(h) if !h.is_empty() => Some(h.clone()),
        _ if retry => find(Kind::Contacts, at.carddav, &mut out.errors)?,
        _ => None,
    };
    out.checked = now;

    // 1. Deletions made here.
    for t in &pim.deleted {
        match dav.delete(&t.href, &t.etag) {
            Ok(()) => out.deleted.push(t.href.clone()),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(e.to_string()),
        }
    }

    // 2. Changes made here, per item (an event with its changed instances).
    let mut uids: Vec<&str> = pim.events.iter().filter(|e| e.dirty && e.calendar != LOCAL).map(|e| e.event.uid.as_str()).collect();
    uids.sort_unstable();
    uids.dedup();
    for uid in uids {
        let parts: Vec<&StoredEvent> = pim.events.iter().filter(|e| e.event.uid == uid).collect();
        let Some(master) = parts.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| parts.first()) else { continue };
        let url = if master.href.is_empty() {
            format!("{}/{}", master.calendar.trim_end_matches('/'), resource_name(uid, "ics"))
        } else {
            master.href.clone()
        };
        let events: Vec<CalEvent> = parts.iter().map(|e| e.event.clone()).collect();
        let data = ical::write(&events, None, now);
        let etag = (!master.href.is_empty()).then_some(master.etag.as_str());
        match dav.put(&url, "text/calendar; charset=utf-8", &data, etag) {
            Ok(new) => {
                for p in parts {
                    out.pushed_events.push(StoredEvent { href: url.clone(), etag: new.clone(), dirty: false, ..(*p).clone() });
                }
            }
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", master.event.summary)),
        }
    }
    for c in pim.cards.iter().filter(|c| c.dirty && c.book != LOCAL) {
        let url = if c.href.is_empty() { format!("{}/{}", c.book.trim_end_matches('/'), resource_name(&c.card.uid, "vcf")) } else { c.href.clone() };
        let etag = (!c.href.is_empty()).then_some(c.etag.as_str());
        match dav.put(&url, "text/vcard; charset=utf-8", &vcard::write(&c.card), etag) {
            Ok(new) => out.pushed_cards.push(StoredCard { href: url, etag: new, dirty: false, ..c.clone() }),
            Err(e @ NetError::Auth(_)) => return Err(e),
            Err(e) => out.errors.push(format!("{}: {e}", c.card.display())),
        }
    }

    // 3. What the server has.
    if let Some(home) = &out.caldav_home {
        let mut calendars = Vec::new();
        for (n, c) in dav.collections(Kind::Calendars, home)?.into_iter().enumerate() {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.calendars.iter().find(|k| k.id == c.href);
            let info = CalendarInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Calendar".into()),
                color: known.map_or(u8::try_from(n % 8).unwrap_or(0), |k| k.color),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
            };
            let pushed_here = out.pushed_events.iter().any(|e| e.calendar == c.href);
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_calendars.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> =
                    pim.events.iter().filter(|e| e.calendar == c.href).map(|e| (e.href.as_str(), e.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> = listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str())).map(|(h, _)| h.clone()).collect();
                // Unchanged: the engine keeps what it has.
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Calendars, &c.href, &fetch)? {
                    for event in ical::parse_in(&data, local).events {
                        out.events.push(StoredEvent { calendar: c.href.clone(), href: href.clone(), etag: etag.clone(), event, dirty: false });
                    }
                }
                out.synced_calendars.push(c.href.clone());
            }
            calendars.push(info);
        }
        out.calendars = Some(calendars);
    }
    if let Some(home) = &out.carddav_home {
        let mut books = Vec::new();
        for c in dav.collections(Kind::Contacts, home)? {
            let ctag = c.props.get("getctag").or_else(|| c.props.get("sync-token")).cloned().unwrap_or_default();
            let known = pim.books.iter().find(|k| k.id == c.href);
            let info = BookInfo {
                id: c.href.clone(),
                name: c.props.get("displayname").cloned().unwrap_or_else(|| "Contacts".into()),
                read_only: c.writable == Some(false),
                ctag: ctag.clone(),
            };
            let pushed_here = out.pushed_cards.iter().any(|x| x.book == c.href);
            if !ctag.is_empty() && known.is_some_and(|k| k.ctag == ctag) && !pushed_here {
                out.unchanged_books.push(c.href.clone());
            } else {
                let have: HashMap<&str, &str> = pim.cards.iter().filter(|x| x.book == c.href).map(|x| (x.href.as_str(), x.etag.as_str())).collect();
                let listed = dav.etags(&c.href)?;
                let fetch: Vec<String> = listed.iter().filter(|(h, t)| have.get(h.as_str()) != Some(&t.as_str())).map(|(h, _)| h.clone()).collect();
                out.kept.extend(listed.iter().filter(|(h, _)| !fetch.contains(h)).map(|(h, _)| h.clone()));
                for (href, etag, data) in dav.multiget(Kind::Contacts, &c.href, &fetch)? {
                    for card in vcard::parse(&data) {
                        out.cards.push(StoredCard { book: c.href.clone(), href: href.clone(), etag: etag.clone(), card, dirty: false });
                    }
                }
                out.synced_books.push(c.href.clone());
            }
            books.push(info);
        }
        out.books = Some(books);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
