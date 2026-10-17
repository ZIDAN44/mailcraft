//! An in-memory JMAP server (RFC 8620/8621, the parts MailCraft uses) on localhost, for tests:
//! the session, `Mailbox/get|set`, `Email/query|get|set|import`, `Identity/get`,
//! `EmailSubmission/set`, back-references, blob download and upload, Basic authentication, and
//! CORS for web pages (so the browser app can be tried against it: see
//! `serve_for_the_browser`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

pub const ACCOUNT: &str = "A1";
pub const USER: &str = "me@studio.example";
pub const PASSWORD: &str = "pw";

#[derive(Clone, Debug)]
pub struct Mb {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub role: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Em {
    pub id: String,
    pub blob: String,
    pub mailboxes: BTreeSet<String>,
    pub keywords: BTreeSet<String>,
    pub received: String,
}

#[derive(Default)]
pub struct Data {
    pub mailboxes: Vec<Mb>,
    pub emails: Vec<Em>,
    pub blobs: HashMap<String, Vec<u8>>,
    /// (email id, from, recipients)
    pub submissions: Vec<(String, String, Vec<String>)>,
    pub calls: Vec<String>,
    /// The VacationResponse singleton.
    pub vacation: Value,
    /// Open event streams (push).
    pub listeners: Vec<std::sync::mpsc::Sender<String>>,
    next: u32,
}

impl Data {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    pub fn mailbox(&mut self, name: &str, parent: Option<&str>, role: Option<&str>) -> String {
        let id = self.id("M");
        self.mailboxes.push(Mb { id: id.clone(), name: name.into(), parent: parent.map(str::to_string), role: role.map(str::to_string) });
        id
    }

    /// Tell the event streams that mail changed.
    fn notify(&mut self) {
        let change = json!({"@type": "StateChange", "changed": {ACCOUNT: {"Email": format!("s{}", self.next)}}}).to_string();
        self.listeners.retain(|l| l.send(change.clone()).is_ok());
    }

    pub fn deliver(&mut self, mailbox: &str, raw: &[u8], keywords: &[&str], received: &str) -> String {
        let blob = self.id("B");
        self.blobs.insert(blob.clone(), raw.to_vec());
        let id = self.id("E");
        self.emails.push(Em {
            id: id.clone(),
            blob,
            mailboxes: [mailbox.to_string()].into(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
            received: received.into(),
        });
        self.notify();
        id
    }

    pub fn by_name(&self, name: &str) -> Option<&Mb> {
        self.mailboxes.iter().find(|m| m.name == name)
    }

    pub fn email(&self, id: &str) -> Option<&Em> {
        self.emails.iter().find(|e| e.id == id)
    }

    fn email_json(&self, e: &Em) -> Value {
        let raw = self.blobs.get(&e.blob).cloned().unwrap_or_default();
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut headers: Vec<(String, String)> = Vec::new();
        for line in head.split("\r\n") {
            if line.starts_with([' ', '\t']) {
                if let Some(last) = headers.last_mut() {
                    last.1.push_str("\r\n");
                    last.1.push_str(line);
                }
            } else if let Some((n, v)) = line.split_once(':') {
                headers.push((n.to_string(), v.to_string()));
            }
        }
        json!({
            "id": e.id,
            "blobId": e.blob,
            "mailboxIds": e.mailboxes.iter().map(|m| (m.clone(), Value::Bool(true))).collect::<Map<_, _>>(),
            "keywords": e.keywords.iter().map(|k| (k.clone(), Value::Bool(true))).collect::<Map<_, _>>(),
            "size": raw.len(),
            "receivedAt": e.received,
            "preview": body.chars().take(60).collect::<String>(),
            "hasAttachment": text.contains("Content-Disposition: attachment"),
            "headers": headers.iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>(),
        })
    }

    fn mailbox_json(&self, m: &Mb) -> Value {
        let inside: Vec<&Em> = self.emails.iter().filter(|e| e.mailboxes.contains(&m.id)).collect();
        json!({
            "id": m.id, "name": m.name, "parentId": m.parent, "role": m.role,
            "totalEmails": inside.len(),
            "unreadEmails": inside.iter().filter(|e| !e.keywords.contains("$seen")).count(),
        })
    }

    fn method(&mut self, name: &str, args: &Value, earlier: &BTreeMap<String, Value>) -> (String, Value) {
        self.calls.push(name.to_string());
        if name.ends_with("/set") || name.ends_with("/import") {
            self.notify();
        }
        let ids = |args: &Value| -> Option<Vec<String>> {
            if let Some(r) = args.get("#ids") {
                let of = r.get("resultOf")?.as_str()?;
                return Some(earlier.get(of)?.get("ids")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
            }
            args.get("ids")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        };
        match name {
            "Mailbox/get" => {
                let want = ids(args);
                let list: Vec<Value> =
                    self.mailboxes.iter().filter(|m| want.as_ref().is_none_or(|w| w.contains(&m.id))).map(|m| self.mailbox_json(m)).collect();
                (name.into(), json!({"accountId": ACCOUNT, "state": "s", "list": list, "notFound": []}))
            }
            "Mailbox/set" => {
                let mut created = Map::new();
                for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let id = self.mailbox(v["name"].as_str().unwrap_or("?"), v["parentId"].as_str(), None);
                    created.insert(k, json!({"id": id}));
                }
                let mut not_updated = Map::new();
                for (id, v) in args.get("update").and_then(Value::as_object).cloned().unwrap_or_default() {
                    match self.mailboxes.iter_mut().find(|m| m.id == id) {
                        Some(m) => {
                            m.name = v["name"].as_str().unwrap_or(&m.name).to_string();
                            m.parent = v["parentId"].as_str().map(str::to_string);
                        }
                        None => {
                            not_updated.insert(id, json!({"type": "notFound"}));
                        }
                    }
                }
                for id in args.get("destroy").and_then(Value::as_array).cloned().unwrap_or_default() {
                    let id = id.as_str().unwrap_or_default().to_string();
                    self.mailboxes.retain(|m| m.id != id);
                    for e in &mut self.emails {
                        e.mailboxes.remove(&id);
                    }
                    self.emails.retain(|e| !e.mailboxes.is_empty());
                }
                (name.into(), json!({"accountId": ACCOUNT, "created": created, "notUpdated": not_updated}))
            }
            "Email/query" => {
                let mb = args["filter"]["inMailbox"].as_str().unwrap_or_default();
                let mut found: Vec<&Em> = self.emails.iter().filter(|e| e.mailboxes.contains(mb)).collect();
                found.sort_by(|a, b| b.received.cmp(&a.received));
                let limit = args["limit"].as_u64().unwrap_or(1000) as usize;
                let ids: Vec<&str> = found.iter().take(limit).map(|e| e.id.as_str()).collect();
                (name.into(), json!({"accountId": ACCOUNT, "queryState": "q", "ids": ids, "position": 0}))
            }
            "Email/get" => {
                let want = ids(args).unwrap_or_default();
                let list: Vec<Value> = want.iter().filter_map(|id| self.email(id)).map(|e| self.email_json(e)).collect();
                (name.into(), json!({"accountId": ACCOUNT, "state": "s", "list": list}))
            }
            "Email/set" => {
                let mut not_updated = Map::new();
                for (id, patch) in args.get("update").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let Some(e) = self.emails.iter_mut().find(|e| e.id == id) else {
                        not_updated.insert(id, json!({"type": "notFound"}));
                        continue;
                    };
                    for (path, v) in patch.as_object().cloned().unwrap_or_default() {
                        let (field, key) = path.split_once('/').unwrap_or((&path, ""));
                        let set = if field == "keywords" { &mut e.keywords } else { &mut e.mailboxes };
                        if v == Value::Bool(true) {
                            set.insert(key.to_string());
                        } else {
                            set.remove(key);
                        }
                    }
                }
                let destroy: Vec<String> = args
                    .get("destroy")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                self.emails.retain(|e| !destroy.contains(&e.id));
                (name.into(), json!({"accountId": ACCOUNT, "notUpdated": not_updated, "destroyed": destroy}))
            }
            "Email/import" => {
                let mut created = Map::new();
                for (k, v) in args.get("emails").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let blob = v["blobId"].as_str().unwrap_or_default().to_string();
                    let id = self.id("E");
                    let keys = |f: &str| v[f].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                    self.emails.push(Em {
                        id: id.clone(),
                        blob: blob.clone(),
                        mailboxes: keys("mailboxIds"),
                        keywords: keys("keywords"),
                        received: "2030-01-01T00:00:00Z".into(),
                    });
                    created.insert(k, json!({"id": id, "blobId": blob}));
                }
                (name.into(), json!({"accountId": ACCOUNT, "created": created}))
            }
            "Quota/get" => (
                name.into(),
                json!({"accountId": ACCOUNT, "list": [
                    {"id": "q1", "resourceType": "count", "used": self.emails.len(), "hardLimit": 1000, "scope": "account"},
                    {"id": "q2", "resourceType": "octets", "used": self.blobs.values().map(Vec::len).sum::<usize>(), "hardLimit": 1_000_000, "scope": "account"},
                ]}),
            ),
            "VacationResponse/get" => {
                let mut v = if self.vacation.is_object() { self.vacation.clone() } else { json!({"isEnabled": false}) };
                v["id"] = json!("singleton");
                (name.into(), json!({"accountId": ACCOUNT, "list": [v]}))
            }
            "VacationResponse/set" => {
                let patch = args["update"]["singleton"].clone();
                if patch["textBody"].as_str().is_some_and(|t| t.contains("refuse")) {
                    return (
                        name.into(),
                        json!({"accountId": ACCOUNT, "notUpdated": {"singleton": {"type": "invalidProperties", "description": "not allowed"}}}),
                    );
                }
                self.vacation = patch;
                (name.into(), json!({"accountId": ACCOUNT, "updated": {"singleton": null}}))
            }
            "Identity/get" => (name.into(), json!({"accountId": ACCOUNT, "list": [{"id": "I1", "email": USER, "name": "Me"}]})),
            "EmailSubmission/set" => {
                let mut created = Map::new();
                let mut not_created = Map::new();
                for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let rcpt: Vec<String> = v["envelope"]["rcptTo"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|r| r["email"].as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    if rcpt.iter().any(|r| r.starts_with("nobody@")) {
                        not_created.insert(k, json!({"type": "forbiddenToSend", "description": "no such recipient"}));
                        continue;
                    }
                    let from = v["envelope"]["mailFrom"]["email"].as_str().unwrap_or_default().to_string();
                    self.submissions.push((v["emailId"].as_str().unwrap_or_default().to_string(), from, rcpt));
                    created.insert(k, json!({"id": "S1"}));
                }
                (name.into(), json!({"accountId": ACCOUNT, "created": created, "notCreated": not_created}))
            }
            _ => ("error".into(), json!({"type": "unknownMethod"})),
        }
    }
}

pub struct Server {
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{CORS}Connection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Any web page may call (a test server on this computer).
const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Accept\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n";

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    serve_at(data, 0)
}

pub fn serve_at(data: Data, port: u16) -> Server {
    let l = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    let auth = format!("Basic {}", crate::imap::base64(format!("{USER}:{PASSWORD}").as_bytes()));
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let (shared, auth) = (shared.clone(), auth.clone());
            std::thread::spawn(move || handle(s, &shared, &auth));
        }
    });
    Server { base, data }
}

/// One connection: one request (`Connection: close`), or an event stream held open.
fn handle(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>, auth: &str) {
    {
        {
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).unwrap_or(0) == 0 {
                return;
            }
            let (mut len, mut authorized) = (0usize, false);
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
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            // A browser's CORS preflight carries no credentials.
            if first.starts_with("OPTIONS ") {
                respond(&mut s, 204, b"", "text/plain");
                return;
            }
            if !authorized {
                respond(&mut s, 401, b"{}", "application/json");
                return;
            }
            let path = first.split(' ').nth(1).unwrap_or("/").to_string();
            let mut d = shared.lock().unwrap();
            if path.starts_with("/events/") {
                assert!(path.contains("types=%2A") || path.contains("types=*"), "{path}");
                let (tx, rx) = std::sync::mpsc::channel::<String>();
                d.listeners.push(tx);
                drop(d);
                let _ =
                    s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{CORS}Transfer-Encoding: chunked\r\n\r\n").as_bytes());
                loop {
                    let msg = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                        Ok(m) => format!("event: state\r\ndata: {m}\r\n\r\n"),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ": keep-alive\n\nevent: ping\ndata: {\"interval\":1}\n\n".to_string(),
                        Err(_) => break,
                    };
                    if s.write_all(format!("{:x}\r\n{msg}\r\n", msg.len()).as_bytes()).is_err() {
                        break;
                    }
                }
                return;
            }
            if path == "/.well-known/jmap" {
                let session = json!({
                    "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": 100}, "urn:ietf:params:jmap:mail": {}, "urn:ietf:params:jmap:submission": {}, "urn:ietf:params:jmap:quota": {}, "urn:ietf:params:jmap:vacationresponse": {}},
                    "accounts": {ACCOUNT: {"name": USER}},
                    "primaryAccounts": {"urn:ietf:params:jmap:mail": ACCOUNT, "urn:ietf:params:jmap:submission": ACCOUNT},
                    "username": USER,
                    "apiUrl": "/api/",
                    "downloadUrl": "/download/{accountId}/{blobId}/{name}?accept={type}",
                    "uploadUrl": "/upload/{accountId}/",
                    "eventSourceUrl": "/events/?types={types}&closeafter={closeafter}&ping={ping}",
                    "state": "1",
                });
                respond(&mut s, 200, session.to_string().as_bytes(), "application/json");
            } else if path == "/api/" {
                let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let mut earlier = BTreeMap::new();
                let mut out = Vec::new();
                for call in req["methodCalls"].as_array().cloned().unwrap_or_default() {
                    let (name, args, id) =
                        (call[0].as_str().unwrap_or_default().to_string(), call[1].clone(), call[2].as_str().unwrap_or_default().to_string());
                    let (rname, result) = d.method(&name, &args, &earlier);
                    earlier.insert(id.clone(), result.clone());
                    out.push(json!([rname, result, id]));
                }
                respond(&mut s, 200, json!({"methodResponses": out, "sessionState": "1"}).to_string().as_bytes(), "application/json");
            } else if let Some(rest) = path.strip_prefix(&format!("/download/{ACCOUNT}/")) {
                let blob = rest.split('/').next().unwrap_or_default();
                match d.blobs.get(blob) {
                    Some(raw) => respond(&mut s, 200, &raw.clone(), "message/rfc822"),
                    None => respond(&mut s, 404, b"", "text/plain"),
                }
            } else if path == format!("/upload/{ACCOUNT}/") {
                let blob = d.id("B");
                d.blobs.insert(blob.clone(), body.clone());
                let r = json!({"accountId": ACCOUNT, "blobId": blob, "type": "message/rfc822", "size": body.len()});
                respond(&mut s, 201, r.to_string().as_bytes(), "application/json");
            } else {
                respond(&mut s, 404, b"", "text/plain");
            }
        }
    }
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
        format!("From: Zoe Park <zoe@studio.example>\r\nTo: {USER}\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
            .into_bytes()
    }

    #[test]
    fn a_jmap_account_syncs_changes_and_sends() {
        let mut d = Data::default();
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        d.mailbox("Sent", None, Some("sent"));
        d.mailbox("Trash", None, Some("trash"));
        let projects = d.mailbox("Projects", None, None);
        let first = d.deliver(&inbox, &mail("Storyboard", "m1"), &["$seen"], "2024-05-06T09:00:00Z");
        let second = d.deliver(&inbox, &mail("Sound mix", "m2"), &[], "2024-05-06T10:00:00Z");
        d.deliver(&projects, &mail("Plan", "m3"), &["$seen", "$label1"], "2024-05-01T10:00:00Z");
        let server = serve(d);
        let data = server.data.clone();

        let dir = std::env::temp_dir().join(format!("mailcraft-jmap-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
        let account = AccountConfig {
            id: "a1".into(),
            name: "Me".into(),
            email: USER.into(),
            username: USER.into(),
            jmap: Some(server.base.clone()),
            ..Default::default()
        };
        assert!(!account.is_local());

        // A wrong password is reported, not retried.
        pool.submit(Job::Start { account: account.clone(), password: Some(Secret("nope".into())) });
        until(&pool, "password prompt", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));

        pool.submit(Job::Start { account: account.clone(), password: Some(Secret(PASSWORD.into())) });
        let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
        let folders = seen
            .iter()
            .find_map(|e| match e {
                Event::Folders { folders, .. } => Some(folders.clone()),
                _ => None,
            })
            .unwrap();
        let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["INBOX", "Sent", "Trash", "Projects"]);
        assert_eq!((folders[0].total, folders[0].unread), (2, 1), "counts come with the list");
        let st = store.load_folder("a1", "INBOX").unwrap();
        let subjects: Vec<&str> = st.messages.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(subjects, ["Sound mix", "Storyboard"], "newest first");
        assert!(st.messages[1].flags.seen && !st.messages[0].flags.seen);
        assert_eq!(st.messages[0].from.email, "zoe@studio.example");
        assert!(st.messages[0].preview.contains("Hello about"));
        assert!(store.has_body(&st.messages[0].key), "the newest sources are downloaded");
        let plan = store.load_folder("a1", "Projects").unwrap();
        assert!(plan.messages[0].flags.has("$label1"), "keywords become categories");

        // Open a message whose source isn't cached.
        let key = st.messages[0].key.clone();
        store.remove_body(&key);
        pool.submit(Job::FetchBody { key: key.clone() });
        until(&pool, "body", |e| matches!(e, Event::Body { key: k, raw } if *k == key && raw.starts_with(b"From:")));

        // Mark read, then move: the cache at once, the server right after.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
        eventually("the server has the flag", || data.lock().unwrap().email(&second).unwrap().keywords.contains("$seen"));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: vec![key.clone()], to: "Projects".into() } });
        eventually("moved on the server", || data.lock().unwrap().email(&second).unwrap().mailboxes == [projects.clone()].into());
        until(&pool, "the move shows in Projects", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
        assert!(
            store.load_folder("a1", "Projects").unwrap().messages.iter().any(|m| m.subject == "Sound mix" && m.key.uid == key.uid),
            "same message, same number"
        );

        // Folders: create inside Projects, rename it, delete it.
        pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Projects/Caf&AOk-".into() } });
        eventually("created", || data.lock().unwrap().by_name("Café").is_some_and(|m| m.parent.as_deref() == Some(projects.as_str())));
        until(&pool, "folder list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects/Caf&AOk-")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Projects/Caf&AOk-".into(), to: "Clients".into() } });
        eventually("renamed", || data.lock().unwrap().by_name("Clients").is_some_and(|m| m.parent.is_none()));
        until(&pool, "renamed in the list", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Clients")));
        pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteFolder { path: "Clients".into() } });
        eventually("deleted", || data.lock().unwrap().by_name("Clients").is_none());

        // New mail arrives: the server pushes the change and the Inbox updates by itself (once
        // the worker is listening again after its last sync).
        eventually("listening for changes", || !data.lock().unwrap().listeners.is_empty());
        data.lock().unwrap().deliver(&inbox, &mail("Festival poster", "m4"), &[], "2024-05-07T08:00:00Z");
        until(&pool, "new mail, pushed", |e| {
            matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")
                && store.load_folder("a1", "INBOX").unwrap().messages.first().is_some_and(|m| m.subject == "Festival poster")
        });

        // Mailbox settings: the space used, automatic replies, emptying Deleted Items.
        assert!(seen.iter().any(|e| matches!(e, Event::Quota { used, limit: 1_000_000, .. } if *used > 0)), "the quota comes with a full sync");
        pool.submit(Job::GetVacation { account: "a1".into() });
        until(&pool, "replies read", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
        let away = mailcraft_model::job::Vacation {
            enabled: true,
            subject: "Away".into(),
            text: "Back on Monday".into(),
            from: Some("2026-10-15".into()),
            until: Some("2026-10-20".into()),
            ..Default::default()
        };
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
        until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
        assert_eq!(data.lock().unwrap().vacation["toDate"], "2026-10-20T23:59:59Z");
        pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { text: "refuse".into(), ..away.clone() } });
        until(&pool, "a refused change", |e| matches!(e, Event::Vacation { vacation: Some(v), error: Some(_), .. } if *v == away));
        let trash = data.lock().unwrap().by_name("Trash").unwrap().id.clone();
        for i in 0..3 {
            data.lock().unwrap().deliver(&trash, &mail(&format!("Old {i}"), &format!("t{i}")), &[], "2024-01-01T00:00:00Z");
        }
        pool.submit(Job::Apply { account: "a1".into(), op: Op::EmptyFolder { path: "Trash".into() } });
        eventually("Trash emptied", || !data.lock().unwrap().emails.iter().any(|e| e.mailboxes.contains(&trash)));

        // Delete for good.
        let gone = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "Storyboard").unwrap().key.clone();
        pool.submit(Job::Apply { account: "a1".into(), op: Op::Expunge { keys: vec![gone] } });
        eventually("destroyed", || data.lock().unwrap().email(&first).is_none());

        // Send: stored in Sent, then submitted.
        let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: e2e\r\nMessage-ID: <e2e@x>\r\n\r\nhi\r\n".to_vec();
        let send = |id: u64, to: &str| Job::Send {
            account: "a1".into(),
            id,
            raw: raw.clone(),
            from: USER.into(),
            recipients: vec![to.into()],
            sent_folder: Some("Sent".into()),
        };
        pool.submit(send(7, "zoe@studio.example"));
        until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
        until(&pool, "the copy in Sent", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
        {
            let d = data.lock().unwrap();
            let (email, from, rcpt) = d.submissions.last().unwrap().clone();
            assert_eq!((from.as_str(), rcpt), (USER, vec!["zoe@studio.example".to_string()]));
            assert!(d.email(&email).unwrap().keywords.contains("$seen"));
        }
        assert_eq!(store.load_folder("a1", "Sent").unwrap().messages[0].subject, "e2e");
        // Refused by the server: reported, and no copy left in Sent.
        let sent_before = data.lock().unwrap().emails.len();
        pool.submit(send(8, "nobody@studio.example"));
        until(&pool, "refused", |e| matches!(e, Event::SendFailed { id: 8, error, .. } if error.contains("no such recipient")));
        assert_eq!(data.lock().unwrap().emails.len(), sent_before);

        // A draft replaces its earlier copy.
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft one", "d1"), folder: "Sent".into(), replaces: None });
        until(&pool, "draft", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Sent"));
        let draft = store.load_folder("a1", "Sent").unwrap().messages.iter().find(|m| m.subject == "Draft one").unwrap().key.clone();
        assert!(store.load_folder("a1", "Sent").unwrap().find(draft.uid).unwrap().flags.draft);
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: mail("Draft two", "d2"), folder: "Sent".into(), replaces: Some(draft) });
        eventually("one draft left", || {
            let d = data.lock().unwrap();
            d.emails.iter().filter(|e| e.keywords.contains("$draft")).count() == 1
        });

        // The numbering survives a restart.
        let numbers = jmap_number(&store, &second);
        pool.submit(Job::Remove { account: "a1".into() });
        assert_eq!(numbers, Some(key.uid));
        assert!(data.lock().unwrap().calls.iter().any(|c| c == "EmailSubmission/set"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// For trying the browser app by hand: a sample JMAP server at http://127.0.0.1:8775 (or
    /// `MAILCRAFT_FAKE_JMAP_PORT`) for ten minutes, signing in as `me@studio.example` / `pw`.
    /// `cargo test -p mailcraft-net serve_for_the_browser -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn serve_for_the_browser() {
        let mut d = Data::default();
        let inbox = d.mailbox("Inbox", None, Some("inbox"));
        d.mailbox("Drafts", None, Some("drafts"));
        d.mailbox("Sent", None, Some("sent"));
        d.mailbox("Trash", None, Some("trash"));
        let projects = d.mailbox("Projects", None, None);
        d.deliver(&inbox, &mail("Storyboard", "m1"), &["$seen"], "2026-10-14T09:00:00Z");
        d.deliver(&inbox, &mail("Sound mix", "m2"), &[], "2026-10-15T06:00:00Z");
        d.deliver(&projects, &mail("Plan", "m3"), &["$seen"], "2026-10-01T10:00:00Z");
        let port = std::env::var("MAILCRAFT_FAKE_JMAP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8775);
        let server = serve_at(d, port);
        println!("JMAP at {}/.well-known/jmap ({USER} / {PASSWORD})", server.base);
        // New mail now and then, to watch push bring it in.
        for n in 1..=10 {
            std::thread::sleep(Duration::from_secs(60));
            let subject = format!("Pushed message {n}");
            server.data.lock().unwrap().deliver(&inbox, &mail(&subject, &format!("push{n}")), &[], "2026-10-15T12:00:00Z");
            println!("delivered {subject}");
        }
    }

    fn jmap_number(store: &Store, id: &str) -> Option<u32> {
        let mut st = crate::jmap::State::load(store, "a1");
        let known = st.id(1).is_some() || st.id(2).is_some();
        known.then(|| st.number(id))
    }
}
