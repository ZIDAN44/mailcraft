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
/// The most objects one `/get` takes (the session's `maxObjectsInGet`).
const MAX_GET: usize = 100;

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
    /// The next so many `Mailbox/get` calls fail (a server that didn't answer).
    pub fail_mailbox_get: u32,
    /// The account starts with no identity to send from (Stalwart's new accounts); those made
    /// with `Identity/set` are listed (their addresses).
    pub no_identity: bool,
    pub made_identities: Vec<String>,
    /// Signs in with these OAuth access tokens only, not the password: a refusal asks for a
    /// bearer token (`WWW-Authenticate: Bearer`), as a server behind a single sign-on does.
    pub tokens: Option<Vec<String>>,
    /// Where its OIDC provider is: `/.well-known/openid-configuration` redirects there.
    pub issuer: Option<String>,
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

    /// Whether an email meets an `Email/query` filter (RFC 8621 §4.4.1): the conditions and
    /// operators MailCraft asks, the words checked in the raw message.
    fn matches(&self, e: &Em, f: &Value) -> bool {
        if let Some(op) = f.get("operator").and_then(Value::as_str) {
            let conds = f.get("conditions").and_then(Value::as_array).cloned().unwrap_or_default();
            return match op {
                "AND" => conds.iter().all(|c| self.matches(e, c)),
                "OR" => conds.iter().any(|c| self.matches(e, c)),
                _ => !conds.iter().any(|c| self.matches(e, c)),
            };
        }
        let raw = String::from_utf8_lossy(&self.blobs.get(&e.blob).cloned().unwrap_or_default()).to_lowercase();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let header = |n: &str| head.split("\r\n").filter(|l| l.starts_with(&format!("{n}:"))).collect::<Vec<_>>().join(" ");
        let has = |hay: &str, k: &str| f.get(k).and_then(Value::as_str).is_none_or(|w| hay.contains(&w.to_lowercase()));
        f.get("inMailbox").and_then(Value::as_str).is_none_or(|m| e.mailboxes.contains(m))
            && f.get("hasKeyword").and_then(Value::as_str).is_none_or(|k| e.keywords.contains(k))
            && f.get("notKeyword").and_then(Value::as_str).is_none_or(|k| !e.keywords.contains(k))
            && has(&raw, "text")
            && has(&header("from"), "from")
            && has(&header("to"), "to")
            && has(&header("subject"), "subject")
            && has(body, "body")
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
            "Mailbox/get" if self.fail_mailbox_get > 0 => {
                self.fail_mailbox_get -= 1;
                ("error".into(), json!({"type": "serverUnavailable"}))
            }
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
                let mut found: Vec<&Em> = self.emails.iter().filter(|e| self.matches(e, &args["filter"])).collect();
                found.sort_by(|a, b| b.received.cmp(&a.received));
                let limit = args["limit"].as_u64().unwrap_or(1000) as usize;
                let position = args["position"].as_u64().unwrap_or(0) as usize;
                let ids: Vec<&str> = found.iter().skip(position).take(limit).map(|e| e.id.as_str()).collect();
                let mut answer = json!({"accountId": ACCOUNT, "queryState": "q", "ids": ids, "position": position});
                if args["calculateTotal"] == Value::Bool(true) {
                    answer["total"] = json!(found.len());
                }
                (name.into(), answer)
            }
            "Email/get" => {
                let want = ids(args).unwrap_or_default();
                // As a server refuses more than its maxObjectsInGet (RFC 8620 §5.1).
                if want.len() > MAX_GET {
                    return ("error".into(), json!({"type": "requestTooLarge"}));
                }
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
            "Identity/get" => {
                let mut list: Vec<Value> = if self.no_identity { Vec::new() } else { vec![json!({"id": "I1", "email": USER, "name": "Me"})] };
                list.extend(self.made_identities.iter().enumerate().map(|(i, e)| json!({"id": format!("M{i}"), "email": e, "name": ""})));
                (name.into(), json!({"accountId": ACCOUNT, "list": list}))
            }
            "Identity/set" => {
                let mut created = Map::new();
                for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
                    let email = v["email"].as_str().unwrap_or_default().to_string();
                    created.insert(k, json!({"id": format!("M{}", self.made_identities.len())}));
                    self.made_identities.push(email);
                }
                (name.into(), json!({"accountId": ACCOUNT, "created": created}))
            }
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
const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Accept\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Expose-Headers: WWW-Authenticate\r\n";

/// A 401 that says how to sign in.
fn refuse(s: &mut impl Write, challenge: &str) {
    let body = b"{}";
    let head = format!(
        "HTTP/1.1 401 X\r\nWWW-Authenticate: {challenge}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{CORS}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

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
            let (mut len, mut given) = (0usize, None);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if let Some((k, v)) = line.split_once(':')
                    && k.eq_ignore_ascii_case("authorization")
                {
                    given = Some(v.trim().to_string());
                }
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            // A browser's CORS preflight carries no credentials.
            if first.starts_with("OPTIONS ") {
                respond(&mut s, 204, b"", "text/plain");
                return;
            }
            let path = first.split(' ').nth(1).unwrap_or("/").to_string();
            let mut d = shared.lock().unwrap();
            // Where its sign-in is, for anyone who asks (RFC 8414 §3: the provider's own address).
            if path == "/.well-known/openid-configuration"
                && let Some(issuer) = d.issuer.clone()
            {
                drop(d);
                let head = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {issuer}/.well-known/openid-configuration\r\nContent-Length: 0\r\n{CORS}Connection: close\r\n\r\n"
                );
                let _ = s.write_all(head.as_bytes());
                return;
            }
            let authorized = match (&d.tokens, given.as_deref()) {
                (Some(tokens), Some(g)) => g.strip_prefix("Bearer ").is_some_and(|t| tokens.iter().any(|ok| ok == t)),
                (None, Some(g)) => g == auth,
                (_, None) => false,
            };
            if !authorized {
                // RFC 6750 §3: an `error` only when a token was sent.
                let challenge = match (&d.tokens, given.as_deref()) {
                    (Some(_), Some(g)) if g.starts_with("Bearer ") => r#"Bearer realm="fake", error="invalid_token", scope="openid profile email""#,
                    (Some(_), _) => r#"Bearer realm="fake", scope="openid profile email""#,
                    (None, _) => r#"Basic realm="fake""#,
                };
                drop(d);
                refuse(&mut s, challenge);
                return;
            }
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
                    "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": MAX_GET}, "urn:ietf:params:jmap:mail": {}, "urn:ietf:params:jmap:submission": {}, "urn:ietf:params:jmap:quota": {}, "urn:ietf:params:jmap:vacationresponse": {}},
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
mod tests;
