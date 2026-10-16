//! An in-memory Microsoft Graph mail API on localhost, for tests: mail folders (well-known
//! names, children, paging), messages (list, get, patch, move, delete, `$value`, made from
//! MIME), `sendMail`, mailbox settings and JSON batches, with a bearer token.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub const TOKEN: &str = "test-token";
const ROOT: &str = "root";

#[derive(Clone, Debug)]
pub struct Fo {
    pub id: String,
    pub name: String,
    pub parent: String,
    /// The well-known name (`inbox`, `sentitems`…).
    pub known: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub id: String,
    pub folder: String,
    pub raw: Vec<u8>,
    pub read: bool,
    pub flagged: bool,
    pub draft: bool,
    pub received: String,
}

#[derive(Default)]
pub struct Data {
    pub folders: Vec<Fo>,
    pub messages: Vec<Msg>,
    /// MIME sources handed to `sendMail`.
    pub sent: Vec<Vec<u8>>,
    pub replies: Value,
    /// Calendars (id, name), events (with a `calendar` field) and contacts, as Graph has them.
    pub calendars: Vec<(String, String)>,
    pub events: Vec<Value>,
    pub contacts: Vec<Value>,
    /// Every request, as `METHOD path`.
    pub calls: Vec<String>,
    /// The signed-in Microsoft account has no mailbox.
    pub no_mailbox: bool,
    next: u32,
}

impl Data {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    pub fn folder(&mut self, name: &str, parent: Option<&str>, known: Option<&str>) -> String {
        let id = self.id("F");
        self.folders.push(Fo { id: id.clone(), name: name.into(), parent: parent.unwrap_or(ROOT).into(), known: known.map(str::to_string) });
        id
    }

    pub fn deliver(&mut self, folder: &str, raw: &[u8], read: bool, received: &str) -> String {
        let id = self.id("M");
        self.messages.push(Msg {
            id: id.clone(),
            folder: folder.into(),
            raw: raw.to_vec(),
            read,
            flagged: false,
            draft: false,
            received: received.into(),
        });
        id
    }

    pub fn message(&self, id: &str) -> Option<&Msg> {
        self.messages.iter().find(|m| m.id == id)
    }

    pub fn by_name(&self, name: &str) -> Option<&Fo> {
        self.folders.iter().find(|f| f.name == name)
    }

    fn resolve(&self, id: &str) -> Option<String> {
        self.folders.iter().find(|f| f.id == id || f.known.as_deref() == Some(id)).map(|f| f.id.clone())
    }

    fn folder_json(&self, f: &Fo) -> Value {
        let inside: Vec<&Msg> = self.messages.iter().filter(|m| m.folder == f.id).collect();
        json!({
            "id": f.id, "displayName": f.name, "parentFolderId": f.parent,
            "childFolderCount": self.folders.iter().filter(|c| c.parent == f.id).count(),
            "totalItemCount": inside.len(), "unreadItemCount": inside.iter().filter(|m| !m.read).count(),
        })
    }

    fn message_json(&self, m: &Msg) -> Value {
        let p = mailcraft_mime::parse(&m.raw);
        let addr = |a: &mailcraft_model::EmailAddress| json!({"emailAddress": {"name": a.name, "address": a.email}});
        json!({
            "id": m.id, "parentFolderId": m.folder, "isRead": m.read, "isDraft": m.draft,
            "flag": {"flagStatus": if m.flagged { "flagged" } else { "notFlagged" }},
            "subject": p.as_ref().map(|p| p.subject.clone()),
            "from": p.as_ref().map(|p| addr(&p.from)),
            "toRecipients": p.as_ref().map(|p| p.to.iter().map(addr).collect::<Vec<_>>()),
            "ccRecipients": [],
            "receivedDateTime": m.received,
            "bodyPreview": p.as_ref().map(|p| p.text.chars().take(60).collect::<String>()),
            "hasAttachments": false,
            "internetMessageId": p.as_ref().map(|p| format!("<{}>", p.message_id)),
        })
    }

    /// One request: (status, body, content type).
    fn route(&mut self, method: &str, target: &str, body: &[u8], base: &str) -> (u16, Vec<u8>, &'static str) {
        self.calls.push(format!("{method} {target}"));
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let q = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(str::to_string));
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let ok = |v: Value| (200, v.to_string().into_bytes(), "application/json");
        let not_found =
            || (404, json!({"error": {"code": "ErrorItemNotFound", "message": "Not found."}}).to_string().into_bytes(), "application/json");
        let input: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        // As Graph answers a Microsoft account with no mailbox (one made with a Gmail address).
        if self.no_mailbox && matches!(parts.as_slice(), ["me", "mailFolders" | "messages" | "sendMail" | "mailboxSettings", ..]) {
            let e = json!({"error": {"code": "MailboxNotEnabledForRESTAPI", "message": "The mailbox is either inactive, soft-deleted, or is hosted on-premise."}});
            return (404, e.to_string().into_bytes(), "application/json");
        }
        match (method, parts.as_slice()) {
            ("POST", ["$batch"]) => {
                let mut out = Vec::new();
                for r in input["requests"].as_array().cloned().unwrap_or_default() {
                    let b = serde_json::to_vec(&r["body"]).unwrap_or_default();
                    let (status, rb, _) = self.route(r["method"].as_str().unwrap_or("GET"), r["url"].as_str().unwrap_or("/"), &b, base);
                    let rbody: Value = serde_json::from_slice(&rb).unwrap_or(Value::Null);
                    out.push(json!({"id": r["id"], "status": status, "body": rbody}));
                }
                ok(json!({ "responses": out }))
            }
            ("GET", ["me", "mailFolders"]) => {
                self.page(self.folders.iter().filter(|f| f.parent == ROOT).map(|f| self.folder_json(f)).collect(), path, query, base)
            }
            ("POST", ["me", "mailFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), None, None);
                ok(json!({ "id": id }))
            }
            ("GET", ["me", "mailFolders", id]) => match self.resolve(id) {
                Some(id) => {
                    let f = self.folders.iter().find(|f| f.id == id).cloned();
                    f.map_or_else(not_found, |f| ok(self.folder_json(&f)))
                }
                None => not_found(),
            },
            ("PATCH", ["me", "mailFolders", id]) => match self.folders.iter_mut().find(|f| f.id == *id) {
                Some(f) => {
                    f.name = input["displayName"].as_str().unwrap_or(&f.name).to_string();
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "mailFolders", id]) => {
                let id = id.to_string();
                self.folders.retain(|f| f.id != id);
                self.messages.retain(|m| m.folder != id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "mailFolders", id, "childFolders"]) => {
                let list = self.folders.iter().filter(|f| f.parent == *id).map(|f| self.folder_json(f)).collect();
                self.page(list, path, query, base)
            }
            ("POST", ["me", "mailFolders", id, "childFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), Some(id), None);
                ok(json!({ "id": id }))
            }
            ("POST", ["me", "mailFolders", id, "move"]) => {
                let dest = input["destinationId"].as_str().unwrap_or_default().to_string();
                let dest = if dest == "msgfolderroot" { ROOT.to_string() } else { dest };
                match self.folders.iter_mut().find(|f| f.id == *id) {
                    Some(f) => {
                        f.parent = dest;
                        ok(json!({ "id": id }))
                    }
                    None => not_found(),
                }
            }
            ("GET", ["me", "mailFolders", id, "messages"]) => {
                let Some(id) = self.resolve(id) else { return not_found() };
                let mut list: Vec<&Msg> = self.messages.iter().filter(|m| m.folder == id).collect();
                if let Some(o) = q("$orderby") {
                    assert_eq!(o, "receivedDateTime%20desc");
                }
                list.sort_by(|a, b| b.received.cmp(&a.received));
                let top: usize = q("$top").and_then(|t| t.parse().ok()).unwrap_or(10);
                let list: Vec<Value> = list.into_iter().take(top).map(|m| self.message_json(m)).collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "messages"]) => {
                let raw = unbase64(body);
                let drafts = self.resolve("drafts").unwrap_or_default();
                let id = self.deliver(&drafts, &raw, true, "2030-01-01T00:00:00Z");
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                    m.draft = true;
                }
                ok(json!({ "id": id, "parentFolderId": drafts }))
            }
            ("GET", ["me", "messages", id]) => self.message(id).map_or_else(not_found, |m| ok(self.message_json(m))),
            ("PATCH", ["me", "messages", id]) => match self.messages.iter_mut().find(|m| m.id == *id) {
                Some(m) => {
                    if let Some(r) = input["isRead"].as_bool() {
                        m.read = r;
                    }
                    if let Some(f) = input["flag"]["flagStatus"].as_str() {
                        m.flagged = f == "flagged";
                    }
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "messages", id]) => {
                let id = id.to_string();
                self.messages.retain(|m| m.id != id);
                (204, Vec::new(), "text/plain")
            }
            ("POST", ["me", "messages", id, "move"]) => {
                let dest = input["destinationId"].as_str().and_then(|d| self.resolve(d));
                match (dest, self.messages.iter_mut().find(|m| m.id == *id)) {
                    (Some(d), Some(m)) => {
                        m.folder = d;
                        ok(json!({ "id": id }))
                    }
                    _ => not_found(),
                }
            }
            ("GET", ["me", "messages", id, "$value"]) => match self.message(id) {
                Some(m) => (200, m.raw.clone(), "text/plain"),
                None => not_found(),
            },
            ("POST", ["me", "sendMail"]) => {
                let raw = unbase64(body);
                if String::from_utf8_lossy(&raw).contains("nobody@") {
                    return (
                        400,
                        json!({"error": {"code": "ErrorInvalidRecipients", "message": "no such recipient"}}).to_string().into_bytes(),
                        "application/json",
                    );
                }
                let sent = self.resolve("sentitems").unwrap_or_default();
                self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
                self.sent.push(raw);
                (202, Vec::new(), "text/plain")
            }
            ("GET", ["me", "mailboxSettings", "automaticRepliesSetting"]) => ok(self.replies.clone()),
            ("PATCH", ["me", "mailboxSettings"]) => {
                let r = input["automaticRepliesSetting"].clone();
                if r["internalReplyMessage"].as_str() == Some("refuse") {
                    return (400, json!({"error": {"message": "refused"}}).to_string().into_bytes(), "application/json");
                }
                self.replies = r;
                ok(json!({}))
            }
            ("GET", ["me", "calendars"]) => {
                let list: Vec<Value> = self.calendars.iter().map(|(id, name)| json!({"id": id, "name": name, "canEdit": true})).collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendars", cal, "events"]) => {
                let list: Vec<Value> = self.events.iter().filter(|e| e["calendar"] == *cal).cloned().collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "calendars", cal, "events"]) => {
                let id = self.id("EV");
                let mut e = input.clone();
                e["id"] = json!(id);
                e["iCalUId"] = json!(format!("ical-{id}"));
                e["changeKey"] = json!("ck1");
                e["calendar"] = json!(cal);
                self.events.push(e.clone());
                (201, e.to_string().into_bytes(), "application/json")
            }
            ("PATCH", ["me", "events", id]) => match self.events.iter_mut().find(|e| e["id"] == *id) {
                Some(e) => {
                    for (k, v) in input.as_object().cloned().unwrap_or_default() {
                        e[k.as_str()] = v;
                    }
                    e["changeKey"] = json!("ck2");
                    ok(e.clone())
                }
                None => not_found(),
            },
            ("DELETE", ["me", "events", id]) => {
                self.events.retain(|e| e["id"] != *id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "contacts"]) => ok(json!({ "value": self.contacts })),
            ("POST", ["me", "contacts"]) => {
                let id = self.id("CT");
                let mut c = input.clone();
                c["id"] = json!(id);
                c["changeKey"] = json!("ck1");
                self.contacts.push(c.clone());
                (201, c.to_string().into_bytes(), "application/json")
            }
            ("DELETE", ["me", "contacts", id]) => {
                self.contacts.retain(|c| c["id"] != *id);
                (204, Vec::new(), "text/plain")
            }
            _ => not_found(),
        }
    }

    /// A listing two items a page, with `@odata.nextLink` for the rest.
    fn page(&self, list: Vec<Value>, path: &str, query: &str, base: &str) -> (u16, Vec<u8>, &'static str) {
        let skip: usize = query.split('&').find_map(|kv| kv.strip_prefix("$skip=")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let page: Vec<Value> = list.iter().skip(skip).take(2).cloned().collect();
        let mut v = json!({ "value": page });
        if skip + 2 < list.len() {
            let rest: String = query.split('&').filter(|kv| !kv.starts_with("$skip=")).collect::<Vec<_>>().join("&");
            v["@odata.nextLink"] = json!(format!("{base}{path}?{rest}&$skip={}", skip + 2));
        }
        (200, v.to_string().into_bytes(), "application/json")
    }
}

fn unbase64(body: &[u8]) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in body {
        let Some(v) = A.iter().position(|a| a == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

pub struct Server {
    /// The API address (`…/v1.0`).
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/v1.0", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(data));
    let (shared, b) = (data.clone(), base.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                continue;
            }
            let (mut len, mut authorized, mut immutable) = (0usize, false, false);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                authorized |=
                    line.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.trim() == format!("Bearer {TOKEN}"));
                immutable |= line.starts_with("Prefer: IdType=\"ImmutableId\"");
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            if !authorized {
                respond(&mut s, 401, br#"{"error": {"code": "InvalidAuthenticationToken"}}"#, "application/json");
                continue;
            }
            assert!(immutable, "ids are asked for in their immutable form");
            let mut words = first.split(' ');
            let (method, target) = (words.next().unwrap_or("GET").to_string(), words.next().unwrap_or("/").to_string());
            let target = target.strip_prefix("/v1.0").unwrap_or(&target).to_string();
            let (status, out, kind) = shared.lock().unwrap().route(&method, &target, &body, &b);
            respond(&mut s, status, &out, kind);
        }
    });
    Server { base, data }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use mailcraft_model::job::Secret;
    use mailcraft_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
    use mailcraft_store::Store;

    use super::*;

    fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let end = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < end {
            if let Some(e) = pool.wait(Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    }

    fn eventually(what: &str, mut f: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < end, "never: {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn mail(subject: &str, id: &str) -> Vec<u8> {
        format!("From: Zoe Park <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
            .into_bytes()
    }

    #[test]
    fn exchange_calendars_and_contacts_sync_both_ways() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
        use mailcraft_model::vcard::{Card, Typed};
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"));
        d.calendars.push(("CAL1".into(), "Calendar".into()));
        d.events.push(json!({"id": "OLD", "calendar": "CAL1", "iCalUId": "old", "subject": "Gone", "start": {"dateTime": "2026-10-01T09:00:00.0000000"}, "end": {"dateTime": "2026-10-01T10:00:00.0000000"}}));
        d.events.push(json!({"id": "E1", "calendar": "CAL1", "iCalUId": "standup", "subject": "Stand-up", "changeKey": "x",
            "start": {"dateTime": "2026-10-05T09:00:00.0000000"}, "end": {"dateTime": "2026-10-05T09:15:00.0000000"},
            "recurrence": {"pattern": {"type": "weekly", "interval": 1, "daysOfWeek": ["monday", "wednesday", "friday"]}, "range": {"type": "noEnd"}}}));
        d.contacts.push(json!({"id": "C1", "displayName": "Ana Lima", "emailAddresses": [{"address": "ana@contoso.example"}]}));
        let server = serve(d);
        let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
        // Here: a new event and person, and a deleted event.
        let mut pim = Pim::default();
        pim.events.push(StoredEvent {
            calendar: LOCAL.into(),
            event: CalEvent {
                uid: "local-1".into(),
                summary: "Poster review".into(),
                start: 1_792_000_000,
                end: 1_792_003_600,
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.cards.push(StoredCard {
            book: LOCAL.into(),
            card: Card {
                uid: "p1".into(),
                name: "Ben Ito".into(),
                emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        });
        pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new() });
        let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        {
            let d = server.data.lock().unwrap();
            assert!(!d.events.iter().any(|e| e["id"] == "OLD"), "deleted on the server");
            assert!(d.events.iter().any(|e| e["subject"] == "Poster review" && e["calendar"] == "CAL1"), "new event in the default calendar");
            assert!(d.contacts.iter().any(|c| c["displayName"] == "Ben Ito"));
        }
        assert_eq!(sync.deleted, ["OLD"]);
        assert_eq!(sync.pushed_events[0].calendar, "CAL1");
        assert!(sync.pushed_cards[0].href.starts_with("contact:"));
        // Read back: the recurring stand-up with its rule, both people.
        let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
        assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
        assert_eq!(standup.event.end - standup.event.start, 900);
        assert_eq!(sync.events.len(), 2);
        assert_eq!(sync.cards.len(), 2);
        assert_eq!(sync.calendars.as_ref().map(Vec::len), Some(1));
        assert!(server.data.lock().unwrap().calls.iter().all(|c| !c.contains("contacts/p1")), "a local uid is never sent as an id");
    }

    /// A Microsoft account with no mailbox (signed in with a Gmail address) said "Connecting…" for
    /// good: each refusal was retried at once. It says why and waits until asked.
    #[test]
    fn a_microsoft_account_without_a_mailbox_says_so_and_stops() {
        let server = serve(Data { no_mailbox: true, ..Default::default() });
        let dir = std::env::temp_dir().join(format!("mailcraft-graph-nomailbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut pool = crate::worker::Pool::new(Store::open(&dir).unwrap(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig { id: "a1".into(), email: "me@gmail.example".into(), graph: Some(server.base.clone()), ..Default::default() };
        pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
        let seen = until(&pool, "the error", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
        let error = seen.iter().find_map(|e| match e {
            Event::Status { status: SyncStatus::Error { error }, .. } => Some(error.clone()),
            _ => None,
        });
        assert!(error.as_deref().is_some_and(|e| e.contains("no Outlook.com or Microsoft 365 mailbox")), "{error:?}");
        std::thread::sleep(Duration::from_millis(1500));
        let tries = server.data.lock().unwrap().calls.len();
        assert!(tries <= 2, "asked once, not over and over: {tries} requests");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_exchange_account_syncs_changes_and_sends_through_graph() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"));
        d.folder("Drafts", None, Some("drafts"));
        let sent = d.folder("Sent Items", None, Some("sentitems"));
        let trash = d.folder("Deleted Items", None, Some("deleteditems"));
        d.folder("Outbox", None, Some("outbox"));
        let projects = d.folder("Projects", None, None);
        d.folder("2024", Some(&projects), None);
        d.replies = json!({"status": "disabled", "internalReplyMessage": ""});
        let first = d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
        let second = d.deliver(&inbox, &mail("Sound mix", "m2"), false, "2024-05-06T10:00:00Z");
        d.deliver(&projects, &mail("Plan", "m3"), true, "2024-05-01T10:00:00Z");
        let server = serve(d);
        let data = server.data.clone();

        let dir = std::env::temp_dir().join(format!("mailcraft-graph-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            name: "Me".into(),
            email: "me@contoso.example".into(),
            username: "me@contoso.example".into(),
            graph: Some(server.base.clone()),
            ..Default::default()
        };
        assert!(!account.is_local() && account.is_api());

        // A token Microsoft refuses means signing in again.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("expired".into())) });
        until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(TOKEN.into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let folders = seen
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::Folders { folders, .. } => Some(folders.clone()),
                _ => None,
            })
            .unwrap();
        let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            ["Deleted Items", "Drafts", "INBOX", "Projects", "Projects/2024", "Sent Items"],
            "pages followed, children listed, the Outbox left out"
        );
        let role = |p: &str| folders.iter().find(|f| f.path == p).unwrap().role;
        assert_eq!((role("Sent Items"), role("Deleted Items")), (mailcraft_model::FolderRole::Sent, mailcraft_model::FolderRole::Trash));
        let inbox_folder = folders.iter().find(|f| f.path == "INBOX").unwrap();
        assert_eq!((inbox_folder.total, inbox_folder.unread), (2, 1));
        let st = store.load_folder("a1", "INBOX").unwrap();
        let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
        assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
        assert_eq!(st.messages[0].from.email, "zoe@contoso.example");
        assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");

        // Open a message whose source isn't cached.
        let key = st.messages[0].key.clone();
        store.remove_body(&key);
        pool.submit(Job::FetchBody { key: key.clone() });
        until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

        // Read, flag, move: the same message keeps its number.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        eventually("read on the server", || data.lock().unwrap().message(&second).unwrap().read);
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true } });
        eventually("flagged on the server", || data.lock().unwrap().message(&second).unwrap().flagged);
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
        eventually("moved on the server", || data.lock().unwrap().message(&second).unwrap().folder == projects);
        until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
        assert!(
            store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.key.uid == key.uid && m.flags.flagged)
        );

        // Folders: create inside Projects, rename and move it to the top, delete it.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
        eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|f| f.parent == projects));
        until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
        eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|f| f.parent == ROOT));
        until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
        eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

        // Automatic replies.
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        let away = mailcraft_model::job::Vacation {
            enabled: true,
            subject: String::new(),
            text: "Back on Monday".into(),
            from: Some("2026-10-15".into()),
            until: Some("2026-10-20".into()),
        };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        assert_eq!(data.lock().unwrap().replies["scheduledEndDateTime"]["dateTime"], "2026-10-20T23:59:59");
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
        until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));

        // Empty Deleted Items; delete for good.
        for i in 0..3 {
            data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), true, "2024-01-01T00:00:00Z");
        }
        pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Deleted Items".into() } });
        eventually("emptied", || !data.lock().unwrap().messages.iter().any(|m| m.folder == trash));
        let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
        eventually("deleted for good", || data.lock().unwrap().message(&first).is_none());

        // Send: the Bcc goes in the MIME for Exchange; Sent Items has the copy.
        let raw = b"From: me@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
        let send = |id: u64, to: Vec<String>| Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: "me@contoso.example".into(),
            recipients: to,
            sent_folder: Some("Sent Items".into()),
        };
        pool.submit(send(7, vec!["zoe@contoso.example".into(), "boss@contoso.example".into()]));
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
        until(&pool, "the copy in Sent", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent Items"));
        let out = String::from_utf8(data.lock().unwrap().sent.last().unwrap().clone()).unwrap();
        assert!(out.starts_with("Bcc: boss@contoso.example\r\nFrom:"), "{out}");
        assert!(data.lock().unwrap().messages.iter().any(|m| m.folder == sent));
        pool.submit(send(8, vec!["nobody@contoso.example".into()]));
        until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, error, .. } if error.contains("no such recipient")));

        // A draft replaces its earlier copy.
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Drafts".into(), replaces: None });
        until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Drafts"));
        let draft = store.load_folder("a1", "Drafts").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Drafts".into(), replaces: Some(draft) });
        eventually("one draft left", || data.lock().unwrap().messages.iter().filter(|m| m.draft).count() == 1);
        assert!(data.lock().unwrap().calls.iter().any(|c| c.starts_with("POST /$batch")), "small requests go in batches");
        pool.submit(Job::Remove { account: "a1".into() });
        let _ = std::fs::remove_dir_all(dir);
    }
}
