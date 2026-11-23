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
use mailcraft_model::pim::{BookInfo, CalendarInfo, DavSync, LOCAL, Pim, StoredCard, StoredEvent, StoredTask, TaskList};
use mailcraft_model::vcard;

use crate::http::{self, Request, Response, Url};
use crate::{NetError, Result};

/// Largest answer read.
const MAX_BODY: usize = 32 << 20;
/// Items fetched per `multiget`.
const BATCH: usize = 50;
const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// Build an error message from a Bearer session's 403 response: what Google said, not a guess.
/// Handles (a) insufficient_scope in WWW-Authenticate, (b) API not enabled in XML body,
/// (c) anything else.
fn bearer_403_message(headers: &[(String, String)], body: &[u8]) -> String {
    // (a) insufficient_scope: WWW-Authenticate header with error and optionally scope
    if let Some((_, auth_value)) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate"))
        && auth_value.contains("error=\"insufficient_scope\"")
    {
        // Extract the scope if present: scope="..."
        if let Some(scope_start) = auth_value.find("scope=\"") {
            let after = &auth_value[scope_start + 7..];
            if let Some(end) = after.find('"') {
                let scope = &after[..end];
                return format!("sign in again to allow access (scope missing: {})", scope);
            }
        }
        return "sign in again to allow access".to_string();
    }

    // (b) API not enabled: XML body with <internalReason>...</internalReason>
    let body_str = String::from_utf8_lossy(body);
    if let Some(start) = body_str.find("<internalReason>") {
        let after = &body_str[start + 16..];
        if let Some(end) = after.find("</internalReason>") {
            let reason = &after[..end];
            // Cap to reasonable length, respecting char boundaries (hostile input)
            const MAX_REASON: usize = 500;
            let capped = if reason.len() <= MAX_REASON {
                reason
            } else {
                // Find a char boundary at or before MAX_REASON
                let mut boundary = MAX_REASON;
                while boundary > 0 && !reason.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                &reason[..boundary]
            };
            return capped.to_string();
        }
    }

    // (c) anything else: generic message with HTTP status, still suggesting sign-in
    "the calendar or contacts server refused access; sign in again (HTTP 403)".to_string()
}

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
    /// Whether to sync contacts (CardDAV). Google OAuth accounts lack the contacts scope.
    pub contacts: bool,
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

/// How a DAV session signs in.
enum Auth {
    /// Basic or Digest authentication with username and password.
    Password { user: String, password: String, basic: String },
    /// OAuth Bearer token.
    Bearer(String),
}

/// A signed-in DAV session: Basic sign-in, or Digest once the server asks for it, or Bearer token.
pub struct Dav {
    auth: Auth,
    /// The server's Digest challenge once it gave one (Baïkal's default): every request after
    /// answers it, rather than being refused first. Only for Password auth.
    digest: std::sync::Mutex<Option<digest::Challenge>>,
    /// Set while the account's server is being looked for: who may have the password then.
    /// Only for Password auth.
    guard: std::sync::Mutex<Option<guard::Guard>>,
}

impl Dav {
    pub fn new(user: &str, password: &str) -> Dav {
        let basic = format!("Basic {}", mailcraft_model::base64::encode(format!("{user}:{password}").as_bytes()));
        Dav {
            auth: Auth::Password { user: user.into(), password: password.into(), basic },
            digest: std::sync::Mutex::default(),
            guard: std::sync::Mutex::default(),
        }
    }

    /// A session that signs in with an OAuth Bearer token (Google Calendar over CalDAV).
    pub fn bearer(token: &str) -> Dav {
        Dav { auth: Auth::Bearer(format!("Bearer {token}")), digest: std::sync::Mutex::default(), guard: std::sync::Mutex::default() }
    }

    /// Whether this session signs in with a Bearer token.
    pub fn is_bearer(&self) -> bool {
        matches!(self.auth, Auth::Bearer(_))
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

    /// Send a request signed in: with Bearer token, or with Basic / the server's Digest challenge.
    /// Password auth: refused with a Digest challenge it can answer, it's sent again answering it
    /// (twice at most: once more for a nonce gone stale, or another realm a redirect led to);
    /// refused again, it's the password.
    fn send(&self, req: &Request) -> Result<Response> {
        match &self.auth {
            Auth::Bearer(token) => http::send(&req.clone().header("Authorization", token)),
            Auth::Password { user, password, basic } => self.send_with_password(req, user, password, basic),
        }
    }

    /// Send with password auth: check guard, try Basic or Digest.
    fn send_with_password(&self, req: &Request, user: &str, password: &str, basic: &str) -> Result<Response> {
        if let Some(resp) = self.unsigned(req)? {
            return Ok(resp);
        }
        let uri = Url::parse(&req.url).map(|u| u.path).unwrap_or_else(|_| "/".into());
        let mut tries = 0;
        loop {
            let (auth, answered) = {
                let mut held = self.digest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                match held.as_mut() {
                    Some(c) => (c.authorization(user, password, &req.method, &uri), Some(c.realm.clone())),
                    None => (basic.to_string(), None),
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
        for _ in 0..=http::MAX_REDIRECTS {
            // Plain http only to this computer (a test server): the password never crosses the
            // network in the clear, wherever a redirect or a server's href points.
            if Url::parse(&url).is_ok_and(|u| !u.https && !http::is_loopback(&u.host)) {
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
            if let Some(next) = http::next_hop(&Url::parse(&url)?, &resp)? {
                url = format!("{}{}", next.origin(), next.path);
                continue;
            }
            // Only 401 is the sign-in refused for Password auth: a 403 is this request's (a
            // collection the user may not read), and asking for the password again wouldn't
            // change it. For Bearer auth, a 403 is insufficient OAuth scope: the user must sign
            // in again to allow calendar access.
            if resp.status == 401 {
                return Err(NetError::Auth("the calendar server refused the user name or password".into()));
            }
            if resp.status == 403 && self.is_bearer() {
                return Err(NetError::Auth(bearer_403_message(&resp.headers, &resp.body)));
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
        // What it answered at the address itself (a 403 or 404 where it didn't let these
        // credentials look) is said with the `No`: "no calendars were found" alone didn't say
        // why (#147).
        let mut failed = None;
        let mut answered: Option<String> = None;
        for t in &tries {
            match self.home_from(kind, t) {
                Ok(Some(h)) => return Ok(h),
                Err(e @ NetError::Auth(_)) => return Err(e),
                Ok(None) => {}
                Err(NetError::No(e)) => {
                    log::info!("no principal at {t}: {e}");
                    if t == &start {
                        answered.get_or_insert(e);
                    }
                }
                Err(e) => failed = Some(e),
            }
        }
        // The address is a home, or a collection in one.
        let mut holds = |url: &str| match self.collections(kind, url) {
            Ok(c) => !c.is_empty(),
            Err(NetError::No(e)) => {
                log::info!("no collections at {url}: {e}");
                if url == start {
                    answered.get_or_insert(e);
                }
                false
            }
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
        Err(NetError::No(match answered {
            Some(a) => format!("no {what} were found at {start} ({a})"),
            None => format!("no {what} were found at {start}"),
        }))
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
        let all = self.all_collections(kind, home)?;
        Ok(all.into_iter().filter(|i| kind != Kind::Calendars || holds_events(i)).collect())
    }

    /// Every collection of a kind in a home: for calendars, those of events and the task lists
    /// alike (issue #201), each to be taken as what it holds ([`holds_events`], [`holds_tasks`]).
    pub fn all_collections(&self, kind: Kind, home: &str) -> Result<Vec<Item>> {
        let props = "<d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/><d:owner/><d:current-user-principal/>";
        let (mut items, at) = self.propfind(home, "1", props)?;
        let want = if kind == Kind::Calendars { "calendar" } else { "addressbook" };
        mark_foreign(&mut items, &at, home);
        let mut out = Vec::new();
        for mut i in items.into_iter().filter(|i| i.kinds.iter().any(|k| k == want)) {
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
            let refs: String = chunk.iter().map(|h| format!("<d:href>{}</d:href>", crate::xml::esc(&path_of(h)))).collect();
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
            s => {
                // Where, and on which condition: what tells a wrong address (404, issue #147's
                // Zoho) from a copy the server says changed (412) in the log (never the item).
                log::info!("calendars and contacts: PUT {url} ({}) answered HTTP {s}", cond.0);
                Ok(Err(Refused::from_status(s)))
            }
        }
    }

    /// Delete an item (when it still has `etag`, if given): why the server didn't (`Err` inside).
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
            crate::xml::esc(&ical::stored_uid(uid))
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

/// Whether a calendar collection holds events: it says it takes `VEVENT`, or says nothing of
/// what it takes (RFC 4791 §5.2.3: then anything).
pub fn holds_events(c: &Item) -> bool {
    c.comps.is_empty() || c.comps.iter().any(|x| x == "VEVENT")
}

/// Whether a calendar collection is a task list (issue #201): it says it takes `VTODO`. One that
/// says nothing isn't taken for one: SOGo lists its calendars' `.ics` and `.xml` views beside
/// them as calendars that say nothing, and tasks would go there.
pub fn holds_tasks(c: &Item) -> bool {
    c.comps.iter().any(|x| x == "VTODO")
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
    if !u.https && !http::is_loopback(&u.host) {
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

pub(crate) mod digest;
mod free_busy;
pub use free_busy::schedule_answers;
pub(crate) mod guard;
mod multistatus;
pub use multistatus::multistatus;
mod syncing;
pub use syncing::sync;
#[cfg(test)]
use syncing::{NAME_KEEP, resource_name};

#[cfg(test)]
mod tests;
