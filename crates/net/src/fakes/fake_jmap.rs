//! An in-memory JMAP server (RFC 8620/8621, the parts MailCraft uses) on localhost, for tests:
//! the session, `Mailbox/get|set`, `Email/query|get|set|import`, `Identity/get`,
//! `EmailSubmission/set`, back-references, blob download and upload, Basic authentication, and
//! CORS for web pages (so the browser app can be tried against it: see
//! `serve_for_the_browser`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

pub const ACCOUNT: &str = "A1";
pub const USER: &str = "me@studio.example";
pub const PASSWORD: &str = "pw";

/// The account these tests sign in with (`id`, as [`USER`]), on the fake at `base`.
pub fn account(id: &str, base: &str) -> mailcraft_model::AccountConfig {
    mailcraft_model::AccountConfig { id: id.into(), email: USER.into(), username: USER.into(), jmap: Some(base.into()), ..Default::default() }
}

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
    /// What the client did that the fake checks it never does, each answered 400: the test
    /// fails on any left here when the server is dropped (see [`Server`]'s `Drop`).
    pub violations: Vec<String>,
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
            let conds = f.get("conditions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            return match op {
                "AND" => conds.iter().all(|c| self.matches(e, c)),
                "OR" => conds.iter().any(|c| self.matches(e, c)),
                _ => !conds.iter().any(|c| self.matches(e, c)),
            };
        }
        let word = |k: &str| f.get(k).and_then(Value::as_str);
        let listed = word("inMailbox").is_none_or(|m| e.mailboxes.contains(m))
            && word("hasKeyword").is_none_or(|k| e.keywords.contains(k))
            && word("notKeyword").is_none_or(|k| !e.keywords.contains(k))
            // When it came (issue #179): UTCDates compare as text.
            && word("after").is_none_or(|t| e.received.as_str() >= t)
            && word("before").is_none_or(|t| e.received.as_str() < t);
        // The message's words read only when a condition asks for some.
        if !listed || ["text", "from", "to", "subject", "body"].iter().all(|k| word(k).is_none()) {
            return listed;
        }
        let raw = String::from_utf8_lossy(self.blobs.get(&e.blob).map(Vec::as_slice).unwrap_or_default()).to_lowercase();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let has = |hay: &str, k: &str| word(k).is_none_or(|w| hay.contains(&w.to_lowercase()));
        // A header's lines (`name:` first), when its condition is there.
        let in_header = |n: &str| {
            word(n).is_none() || {
                let lines: Vec<&str> = head.split("\r\n").filter(|l| l.strip_prefix(n).is_some_and(|r| r.starts_with(':'))).collect();
                has(&lines.join(" "), n)
            }
        };
        has(&raw, "text") && in_header("from") && in_header("to") && in_header("subject") && has(body, "body")
    }

    fn email_json(&self, e: &Em) -> Value {
        let raw = self.blobs.get(&e.blob).map(Vec::as_slice).unwrap_or_default();
        let text = String::from_utf8_lossy(raw).to_string();
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

    /// One method call: (the response's name, its result).
    fn method(&mut self, name: &str, args: &Value, earlier: &BTreeMap<String, Value>) -> (String, Value) {
        self.calls.push(name.to_string());
        if name.ends_with("/set") || name.ends_with("/import") {
            self.notify();
        }
        match name {
            "Mailbox/get" => self.mailbox_get(name, ids(args, earlier)),
            "Mailbox/set" => self.mailbox_set(name, args),
            "Email/query" => self.email_query(name, args),
            "Email/get" => self.email_get(name, ids(args, earlier)),
            "Email/set" => self.email_set(name, args),
            "Email/import" => self.email_import(name, args),
            "Quota/get" => self.quota_get(name),
            "VacationResponse/get" => self.vacation_get(name),
            "VacationResponse/set" => self.vacation_set(name, args),
            "Identity/get" => self.identity_get(name),
            "Identity/set" => self.identity_set(name, args),
            "EmailSubmission/set" => self.submission_set(name, args),
            _ => ("error".into(), json!({"type": "unknownMethod"})),
        }
    }
}

/// The ids a method's arguments name: given, or (`#ids`) an earlier call's result's.
fn ids(args: &Value, earlier: &BTreeMap<String, Value>) -> Option<Vec<String>> {
    if let Some(r) = args.get("#ids") {
        let of = r.get("resultOf")?.as_str()?;
        return Some(earlier.get(of)?.get("ids")?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
    }
    args.get("ids")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

pub struct Server {
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

impl Drop for Server {
    /// A test whose client broke what the fake checks fails, even where it took the refusal.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let left = self.data.lock().map(|d| d.violations.clone()).unwrap_or_default();
        assert!(left.is_empty(), "the client broke what the fake checks: {left:?}");
    }
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
    let auth = format!("Basic {}", mailcraft_model::base64::encode(format!("{USER}:{PASSWORD}").as_bytes()));
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let (shared, auth) = (shared.clone(), auth.clone());
            std::thread::spawn(move || serving::handle(s, &shared, &auth));
        }
    });
    Server { base, data }
}

mod account;
mod mail;
mod serving;
#[cfg(test)]
mod tests;
