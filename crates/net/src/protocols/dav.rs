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
    /// Someone else's, listed in the user's home ([`Dav::collections`]: its `owner` isn't the
    /// user's).
    pub foreign: bool,
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
    /// The account's user name when it isn't its address (a meeting it organizes may name it).
    pub username: &'a str,
    /// Its other addresses (issue #81): a meeting may name one of them.
    pub others: &'a [mailcraft_model::Identity],
    pub imap_host: &'a str,
    pub caldav: Option<&'a str>,
    pub carddav: Option<&'a str>,
    /// MailCraft mails the account's meeting messages. Not for an account with calendars and
    /// contacts only (issue #61): a server that schedules sends them.
    pub mails: bool,
}

impl Where<'_> {
    /// An address of the account's own (an organizer's, say), `mailto:` and case aside.
    pub fn is_me(&self, address: &str) -> bool {
        let a = address.trim();
        let a = a.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map_or(a, |_| a.get(7..).unwrap_or("")).trim();
        let mine = [self.email, self.username].into_iter().chain(self.others.iter().map(|i| i.email.as_str()));
        !a.is_empty() && mine.into_iter().any(|m| m.contains('@') && m.trim().eq_ignore_ascii_case(a))
    }
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

/// A signed-in DAV session: Basic sign-in, or Digest once the server asks for it.
pub struct Dav {
    user: String,
    password: String,
    basic: String,
    /// The server's Digest challenge once it gave one (Baïkal's default): every request after
    /// answers it, rather than being refused first.
    digest: std::sync::Mutex<Option<digest::Challenge>>,
    /// Set while the account's server is being looked for: who may have the password then.
    guard: std::sync::Mutex<Option<guard::Guard>>,
}

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        let basic = format!("Basic {}", crate::imap::base64(format!("{user}:{password}").as_bytes()));
        Dav { user: user.into(), password: password.into(), basic, digest: std::sync::Mutex::default(), guard: std::sync::Mutex::default() }
    }

    /// Looking for the server of the account `email` (its mail server `mail_host`) from here on,
    /// until [`Dav::found`]: the password goes only to a server of the account's that asks for it.
    pub(crate) fn looking_for(&self, email: &str, mail_host: &str) {
        *self.guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(guard::Guard::new(email, mail_host));
    }

    /// The looking is over: the servers asked from here on are the ones found, or given.
    pub(crate) fn found(&self) {
        *self.guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    /// While looking: the answer to `req` sent without the password, when that's all this host
    /// gets (it isn't the account's, or it didn't ask for one). `None` when the password goes.
    fn unsigned(&self, req: &Request) -> Result<Option<Response>> {
        let host = Url::parse(&req.url).map(|u| u.host).unwrap_or_default();
        {
            let held = self.guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match held.as_ref() {
                None => return Ok(None),
                Some(g) if g.own(&host) && g.asked(&host) => return Ok(None),
                Some(_) => {}
            }
        }
        let resp = http::send(req)?;
        let mut held = self.guard.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match held.as_mut() {
            Some(g) if resp.status == 401 && g.own(&host) && guard::asks_for_password(&resp.headers) => {
                g.note_asked(&host);
                // Asked for Digest: answered with that from the first, not Basic (the password
                // itself) to a server that takes only Digest.
                if let Some(c) = digest::challenge(&resp.headers) {
                    *self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(c);
                }
                Ok(None)
            }
            _ => Ok(Some(resp)),
        }
    }

    /// Answering the Digest challenge an earlier session was given, from its first request.
    pub(crate) fn with_digest(self, challenge: Option<digest::Challenge>) -> Dav {
        *self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = challenge;
        self
    }

    /// The Digest challenge being answered, to keep for the next session.
    pub(crate) fn digest(&self) -> Option<digest::Challenge> {
        self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Send a request signed in: with Basic, or the server's Digest challenge. Refused with a
    /// Digest challenge it can answer, it's sent again answering it (twice at most: once more
    /// for a nonce gone stale, or another realm a redirect led to); refused again, it's the
    /// password.
    fn send(&self, req: &Request) -> Result<Response> {
        if let Some(resp) = self.unsigned(req)? {
            return Ok(resp);
        }
        let uri = Url::parse(&req.url).map(|u| u.path).unwrap_or_else(|_| "/".into());
        let mut tries = 0;
        loop {
            let (auth, answered) = {
                let mut held = self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match held.as_mut() {
                    Some(c) => (c.authorization(&self.user, &self.password, &req.method, &uri), Some(c.realm.clone())),
                    None => (self.basic.clone(), None),
                }
            };
            let resp = http::send(&req.clone().header("Authorization", &auth))?;
            if resp.status != 401 || tries >= 2 {
                return Ok(resp);
            }
            let Some(c) = digest::challenge(&resp.headers) else { return Ok(resp) };
            if answered.as_ref().is_some_and(|realm| *realm == c.realm) && !c.stale {
                return Ok(resp);
            }
            *self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(c);
            tries += 1;
        }
    }

    /// One request, following redirects (WebDAV methods keep their method and body).
    fn request(&self, method: &str, url: &str, depth: Option<&str>, body: &str, extra: &[(&str, &str)]) -> Result<(Response, String)> {
        let mut url = url.to_string();
        for _ in 0..6 {
            // Plain http only to this computer (a test server): the password never crosses the
            // network in the clear, wherever a redirect or a server's href points.
            if Url::parse(&url).is_ok_and(|u| !u.https && !local_host(&u.host)) {
                return Err(NetError::Tls(format!("{url} isn't https: MailCraft won't send the calendar password there")));
            }
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
            if let Some(d) = depth {
                req = req.header("Depth", d);
            }
            for (k, v) in extra {
                req = req.header(k, v);
            }
            let resp = self.send(&req)?;
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
            // Only 401 is the sign-in refused: a 403 is this request's (a collection the user may
            // not read), and asking for the password again wouldn't change it.
            if resp.status == 401 {
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
        self.looking_for(email, imap_host);
        let home = starts.iter().find_map(|start| self.home_from(kind, start).ok().flatten());
        self.found();
        Ok(home)
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
        // A server that answers but has nothing there says `No`; anything else (no connection, a
        // timeout, a certificate refused) is said as it is, and tried again at the next sync.
        let mut failed = None;
        for t in &tries {
            match self.home_from(kind, t) {
                Ok(Some(h)) => return Ok(h),
                Err(e @ NetError::Auth(_)) => return Err(e),
                Ok(None) | Err(NetError::No(_)) => {}
                Err(e) => failed = Some(e),
            }
        }
        // The address is a home, or a collection in one.
        let mut holds = |url: &str| match self.collections(kind, url) {
            Ok(c) => !c.is_empty(),
            Err(NetError::No(_)) => false,
            Err(e) => {
                failed.get_or_insert(e);
                false
            }
        };
        if holds(&start) {
            return Ok(start);
        }
        if let Some(parent) = parent_of(&start)
            && holds(&parent)
        {
            return Ok(parent);
        }
        if let Some(e) = failed {
            return Err(e);
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

    /// The collections in a home (calendars that hold events, or address books). One whose
    /// `owner` is known and is neither the home's owner nor the signed-in user is marked
    /// someone else's (SOGo lists the calendars subscribed to in the user's home, issue #30).
    pub fn collections(&self, kind: Kind, home: &str) -> Result<Vec<Item>> {
        let props = "<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/><d:owner/><d:current-user-principal/>";
        let (mut items, at) = self.propfind(home, "1", props)?;
        let want = if kind == Kind::Calendars { "calendar" } else { "addressbook" };
        mark_foreign(&mut items, &at, home);
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

    /// Whether the server sends meeting invitations and answers itself as events are stored
    /// (RFC 6638 §2: `calendar-auto-schedule` in the `DAV` header of its `OPTIONS` answer).
    pub fn schedules(&self, url: &str) -> Result<bool> {
        let (resp, _) = self.request("OPTIONS", url, None, "", &[])?;
        Ok(resp.ok() && schedules_in(&resp.headers))
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
        self.store(url, content_type, data, etag)?.map_err(|r| NetError::No(r.to_string()))
    }

    /// [`Dav::put`], telling why the server didn't store it (`Err` inside): only a refused
    /// sign-in, or no answer, is an error of the whole sync.
    pub fn store(&self, url: &str, content_type: &str, data: &str, etag: Option<&str>) -> Result<std::result::Result<String, Refused>> {
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
        req = req.header(cond.0, &cond.1);
        let resp = self.send(&req)?;
        match resp.status {
            200..=299 => Ok(Ok(resp.header("ETag").unwrap_or_default().to_string())),
            401 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Ok(Err(Refused::from_status(s))),
        }
    }

    pub fn delete(&self, url: &str, etag: &str) -> Result<()> {
        self.remove(url, etag)?.map_err(|r| NetError::No(r.to_string()))
    }

    /// [`Dav::delete`], telling why the server didn't delete it (`Err` inside).
    pub fn remove(&self, url: &str, etag: &str) -> Result<std::result::Result<(), Refused>> {
        let mut req = Request { method: "DELETE".into(), url: url.into(), max: 1 << 20, ..Default::default() };
        if !etag.is_empty() {
            req = req.header("If-Match", etag);
        }
        let resp = self.send(&req)?;
        match resp.status {
            200..=299 | 404 | 410 => Ok(Ok(())),
            401 => Err(NetError::Auth("the calendar server refused the user name or password".into())),
            s => Ok(Err(Refused::from_status(s))),
        }
    }

    /// The items of a calendar that may have this uid (a calendar-query, RFC 4791 §7.8; its
    /// text-match finds the uid inside longer ones too, so what's found is to be read to be
    /// sure).
    pub fn find_uid(&self, calendar: &str, uid: &str) -> Result<Vec<String>> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-query xmlns:d=\"{DAV}\" xmlns:c=\"{CALDAV}\"><d:prop><d:getetag/></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"><c:prop-filter name=\"UID\"><c:text-match collation=\"i;octet\">{}</c:text-match></c:prop-filter></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>",
            xml_escape(uid)
        );
        let (resp, at) = self.request("REPORT", calendar, Some("1"), &body, &[])?;
        if resp.status != 207 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for i in multistatus(&String::from_utf8_lossy(&resp.body)) {
            if !i.kinds.iter().any(|k| k == "collection") && out.len() < 20 {
                out.push(absolute(&at, &i.href)?);
            }
        }
        Ok(out)
    }
}

/// Whether `DAV` headers say `calendar-auto-schedule` (each may list several, comma-separated).
pub fn schedules_in(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("DAV"))
        .any(|(_, v)| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("calendar-auto-schedule")))
}

/// Why a server didn't store or delete an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not allowed (403): SOGo's roles let a user change some events of a calendar shared with
    /// them (its public ones, say) and not others.
    Forbidden,
    /// The calendar holds an item with this uid already (409): SOGo puts a meeting in the
    /// calendars of its users it invites itself.
    UidTaken,
    /// Changed on the server since it was read (412).
    Changed,
    Other(u16),
}

impl Refused {
    fn from_status(s: u16) -> Refused {
        match s {
            403 => Refused::Forbidden,
            409 => Refused::UidTaken,
            412 => Refused::Changed,
            s => Refused::Other(s),
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Forbidden => write!(f, "the server doesn't allow this change (HTTP 403)"),
            Refused::UidTaken => write!(f, "the calendar has this event already (HTTP 409)"),
            Refused::Changed => write!(f, "changed on the server meanwhile"),
            Refused::Other(s) => write!(f, "the server refused it (HTTP {s})"),
        }
    }
}

/// Mark the collections of a home listing (read at `at`) that are someone else's: an `owner`
/// that is neither the home's owner nor the signed-in user (`current-user-principal`). Nothing
/// is marked when the server names neither.
fn mark_foreign(items: &mut [Item], at: &str, home: &str) {
    let path = |h: &str| absolute(at, h).map(|u| path_of(&u).trim_end_matches('/').to_string()).unwrap_or_default();
    let home_path = path(home);
    let mut mine: Vec<String> = items.iter().flat_map(|i| i.hrefs.get("current-user-principal").into_iter().flatten()).map(|h| path(h)).collect();
    mine.extend(items.iter().filter(|i| path(&i.href) == home_path).flat_map(|i| i.hrefs.get("owner").into_iter().flatten()).map(|h| path(h)));
    mine.retain(|m| !m.is_empty());
    for i in items.iter_mut().filter(|i| path(&i.href) != home_path) {
        let owner = i.hrefs.get("owner").and_then(|o| o.first()).map(|h| path(h)).filter(|o| !o.is_empty());
        i.foreign = !mine.is_empty() && owner.is_some_and(|o| !mine.contains(&o));
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
    if !u.https && !local_host(&u.host) {
        return None;
    }
    Some(format!("{}{}", u.origin(), if u.path.is_empty() { "/" } else { &u.path }))
}

/// This computer, by name or address.
fn local_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
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

pub(crate) mod digest;
pub(crate) mod guard;
mod syncing;
pub use syncing::sync;
#[cfg(test)]
use syncing::{NAME_KEEP, resource_name};

#[cfg(test)]
mod tests;
