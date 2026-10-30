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
    /// File a sent message's copy in Sent Items a moment after `sendMail`, as Graph does.
    pub file_late: bool,
    /// Other people whose calendars are shared with this account: (address, may edit). Their
    /// events have `calendar` = `user:<address>`.
    pub others: Vec<(String, bool)>,
    /// Messages whose source can't be had (the server fails on it).
    pub unreadable: Vec<String>,
    /// Per-batch concurrency cap: entries past this index get a 429 with Retry-After.
    pub at_once: Option<usize>,
    /// Number of upcoming top-level requests to refuse with 429.
    pub throttle: u32,
    /// The one bearer token taken, when not [`TOKEN`] (a test renews it as tokens expire).
    pub token: Option<String>,
    /// The organisation's directory (issue #9): (name, address), searched through `/users` and
    /// `/me/people`.
    pub directory: Vec<(String, String)>,
    /// The token the directory takes (its own permissions), when the mail's isn't enough; the
    /// mail's endpoints refuse it, as a token for other permissions is refused.
    pub directory_token: Option<String>,
    /// `/users` refuses the search (a guest, a tenant hiding its users): `/me/people` is asked.
    pub users_forbidden: bool,
    /// The bearer token and headers of the request being answered.
    bearer: String,
    headers: Vec<(String, String)>,
    /// Set per request from its `Prefer` header; without `IdType="ImmutableId"` message ids
    /// are returned in the other (prefixed) form.
    asked_immutable: bool,
    late: Vec<(std::time::Instant, Vec<u8>)>,
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
        let id = plain_id(id);
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
            "id": if self.asked_immutable { m.id.clone() } else { format!("{REGULAR}{}", m.id) }, "parentFolderId": m.folder, "isRead": m.read, "isDraft": m.draft,
            "flag": {"flagStatus": if m.flagged { "flagged" } else { "notFlagged" }},
            "subject": p.as_ref().map(|p| p.subject.clone()),
            "from": p.as_ref().map(|p| addr(&p.from)),
            "toRecipients": p.as_ref().map(|p| p.to.iter().map(addr).collect::<Vec<_>>()),
            "ccRecipients": [],
            "receivedDateTime": m.received,
            "bodyPreview": p.as_ref().map(|p| p.text.chars().take(60).collect::<String>()),
            "hasAttachments": p.as_ref().is_some_and(|p| !p.attachments.is_empty()),
            "internetMessageId": p.as_ref().map(|p| format!("<{}>", p.message_id)),
        })
    }

    /// One request: (status, body, content type).
    fn route(&mut self, method: &str, target: &str, body: &[u8], base: &str) -> (u16, Vec<u8>, &'static str) {
        self.calls.push(format!("{method} {target}"));
        let now = std::time::Instant::now();
        for (_, raw) in self.late.extract_if(.., |(at, _)| *at <= now).collect::<Vec<_>>() {
            let sent = self.resolve("sentitems").unwrap_or_default();
            self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
        }
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
        let directory = matches!(parts.as_slice(), ["users"] | ["me", "people"]);
        let forbidden = || {
            (
                403,
                json!({"error": {"code": "Authorization_RequestDenied", "message": "Insufficient privileges to complete the operation."}})
                    .to_string()
                    .into_bytes(),
                "application/json",
            )
        };
        if let Some(dt) = self.directory_token.clone() {
            // Each token is good for its own permissions only.
            if directory != (self.bearer == dt) {
                return forbidden();
            }
        }
        if directory {
            let header = |k: &str| self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone()).unwrap_or_default();
            let search = crate::oauth::unpct(&q("$search").unwrap_or_default());
            // The words searched for: what's inside the quotes, `displayName:`/`mail:` dropped.
            let words: Vec<String> = search
                .split('"')
                .skip(1)
                .step_by(2)
                .map(|w| w.rsplit(':').next().unwrap_or("").trim().to_lowercase())
                .filter(|w| !w.is_empty())
                .collect();
            let hit = |n: &str, a: &str| {
                let (n, a) = (n.to_lowercase(), a.to_lowercase());
                words.iter().any(|w| a.starts_with(w.as_str()) || n.split(' ').any(|p| p.starts_with(w.as_str())))
            };
            let top: usize = q("$top").and_then(|t| t.parse().ok()).unwrap_or(10);
            let found: Vec<&(String, String)> = self.directory.iter().filter(|(n, a)| hit(n, a)).take(top).collect();
            if parts == ["users"] {
                if self.users_forbidden {
                    return forbidden();
                }
                if !search.is_empty() && header("ConsistencyLevel") != "eventual" {
                    let e = json!({"error": {"code": "Request_UnsupportedQuery", "message": "Request with $search query parameter only works through MSGraph with a special request header: 'ConsistencyLevel: eventual'"}});
                    return (400, e.to_string().into_bytes(), "application/json");
                }
                let value: Vec<Value> = found.iter().map(|(n, a)| json!({"displayName": n, "mail": a, "userPrincipalName": a})).collect();
                return ok(json!({"@odata.count": value.len(), "value": value}));
            }
            // The relevant people: the directory's only when asked for.
            let with_directory = header("X-PeopleQuery-QuerySources").contains("Directory");
            let value: Vec<Value> = found
                .iter()
                .filter(|_| with_directory)
                .map(|(n, a)| json!({"displayName": n, "scoredEmailAddresses": [{"address": a, "relevanceScore": 8}], "personType": {"class": "Person", "subclass": "OrganizationUser"}}))
                .collect();
            return ok(json!({"value": value}));
        }
        match (method, parts.as_slice()) {
            ("POST", ["$batch"]) => {
                let mut out = Vec::new();
                for (i, r) in input["requests"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                    if self.at_once.is_some_and(|n| i >= n) {
                        let mut refused = serde_json::Map::new();
                        refused.insert("id".into(), r["id"].clone());
                        refused.insert("status".into(), json!(429));
                        refused.insert("headers".into(), json!({ "retry-after": 1 }));
                        refused.insert("body".into(), json!({"error": {"code": "TooManyRequests"}}));
                        out.push(Value::Object(refused));
                        continue;
                    }
                    // Graph reads Prefer from each batch entry's own headers.
                    let prefer = r.pointer("/headers/Prefer").and_then(Value::as_str).unwrap_or_default();
                    self.asked_immutable = prefer.split(',').any(|part| part.trim() == "IdType=\"ImmutableId\"");
                    let b = serde_json::to_vec(&r["body"]).unwrap_or_default();
                    let (status, rb, _) = self.route(r["method"].as_str().unwrap_or("GET"), r["url"].as_str().unwrap_or("/"), &b, base);
                    self.asked_immutable = true;
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
                // A search: its words (KQL, the whole in quotes), or read and flag conditions.
                if let Some(s) = q("$search") {
                    let s = crate::oauth::unpct(&s);
                    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&s).replace("\\\"", "\"");
                    list.retain(|m| words_match(&inner, &m.raw));
                }
                if let Some(f) = q("$filter") {
                    let f = crate::oauth::unpct(&f);
                    for cond in f.split(" and ") {
                        match cond {
                            "isRead eq false" => list.retain(|m| !m.read),
                            "isRead eq true" => list.retain(|m| m.read),
                            "flag/flagStatus eq 'flagged'" => list.retain(|m| m.flagged),
                            "flag/flagStatus ne 'flagged'" => list.retain(|m| !m.flagged),
                            "hasAttachments eq true" => list.clear(),
                            _ => {}
                        }
                    }
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
            // With its body and files' list (a message's text first, issue #24).
            ("GET", ["me", "messages", id]) if query.contains("$expand=attachments") => match self.message(id) {
                Some(m) => {
                    let mut v = self.message_json(m);
                    let p = mailcraft_mime::parse(&m.raw);
                    v["body"] = json!({"contentType": "text", "content": p.as_ref().map(|p| p.text.clone())});
                    v["attachments"] = json!(p
                        .map(|p| p
                            .attachments
                            .iter()
                            .map(|a| json!({"@odata.type": "#microsoft.graph.fileAttachment", "name": a.name, "contentType": a.mime, "size": a.size, "isInline": a.inline}))
                            .collect::<Vec<_>>())
                        .unwrap_or_default());
                    ok(v)
                }
                None => not_found(),
            },
            ("GET", ["me", "messages", id]) => self.message(id).map_or_else(not_found, |m| ok(self.message_json(m))),
            ("PATCH", ["me", "messages", id]) => match self.messages.iter_mut().find(|m| m.id == plain_id(id)) {
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
                let id = plain_id(id).to_string();
                self.messages.retain(|m| m.id != id);
                (204, Vec::new(), "text/plain")
            }
            ("POST", ["me", "messages", id, "move"]) => {
                let dest = input["destinationId"].as_str().and_then(|d| self.resolve(d));
                match (dest, self.messages.iter_mut().find(|m| m.id == plain_id(id))) {
                    (Some(d), Some(m)) => {
                        m.folder = d;
                        ok(json!({ "id": id }))
                    }
                    _ => not_found(),
                }
            }
            ("GET", ["me", "messages", id, "$value"]) if self.unreadable.iter().any(|u| u == plain_id(id)) => {
                (500, json!({"error": {"code": "ErrorInternalServerError", "message": "can't read it"}}).to_string().into_bytes(), "application/json")
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
                if self.file_late {
                    self.late.push((std::time::Instant::now() + std::time::Duration::from_secs(1), raw.clone()));
                } else {
                    let sent = self.resolve("sentitems").unwrap_or_default();
                    self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
                }
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
            // Someone else's calendar, by their address: read when shared, changed when they allow it.
            ("GET" | "POST" | "PATCH" | "DELETE", ["users", who, ..]) => {
                let who = who.replace("%40", "@").to_ascii_lowercase();
                let Some(edit) = self.others.iter().find(|(e, _)| *e == who).map(|(_, e)| *e) else {
                    return (404, json!({"error": {"code": "ErrorItemNotFound"}}).to_string().into_bytes(), "application/json");
                };
                let denied = || {
                    (
                        403,
                        json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}).to_string().into_bytes(),
                        "application/json",
                    )
                };
                let cal = format!("user:{who}");
                match (method, parts.get(2..).unwrap_or_default()) {
                    ("GET", ["calendar"]) => ok(json!({"id": cal, "canEdit": edit})),
                    ("GET", ["calendar", "events"]) => {
                        ok(json!({"value": self.events.iter().filter(|e| e["calendar"] == cal.as_str()).cloned().collect::<Vec<_>>()}))
                    }
                    (_, _) if !edit => denied(),
                    ("POST", ["calendar", "events"]) => {
                        let id = self.id("EV");
                        let mut e = input.clone();
                        e["id"] = json!(id);
                        e["iCalUId"] = json!(format!("ical-{id}"));
                        e["changeKey"] = json!("ck1");
                        e["calendar"] = json!(cal);
                        self.events.push(e.clone());
                        (201, e.to_string().into_bytes(), "application/json")
                    }
                    ("PATCH", ["events", id]) => match self.events.iter_mut().find(|e| e["id"] == *id && e["calendar"] == cal.as_str()) {
                        Some(e) => {
                            for (k, v) in input.as_object().cloned().unwrap_or_default() {
                                e[k.as_str()] = v;
                            }
                            ok(e.clone())
                        }
                        None => not_found(),
                    },
                    ("DELETE", ["events", id]) => {
                        self.events.retain(|e| !(e["id"] == *id && e["calendar"] == cal.as_str()));
                        (204, Vec::new(), "text/plain")
                    }
                    _ => not_found(),
                }
            }
            // `/me/events` reaches only this mailbox's own events, as Graph's does.
            ("PATCH", ["me", "events", id]) => {
                match self.events.iter_mut().find(|e| e["id"] == *id && !e["calendar"].as_str().unwrap_or("").starts_with("user:")) {
                    Some(e) => {
                        for (k, v) in input.as_object().cloned().unwrap_or_default() {
                            e[k.as_str()] = v;
                        }
                        e["changeKey"] = json!("ck2");
                        ok(e.clone())
                    }
                    None => not_found(),
                }
            }
            ("DELETE", ["me", "events", id]) => {
                self.events.retain(|e| e["id"] != *id || e["calendar"].as_str().unwrap_or("").starts_with("user:"));
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

/// Whether a message meets a KQL (Graph's `$search`) or AQS (EWS's `QueryString`) query, as
/// far as MailCraft writes them: words and `"phrases"`, `from:` `to:` `subject:` `body:`, `NOT`;
/// a word as a part of what's searched, case aside.
pub(crate) fn words_match(query: &str, raw: &[u8]) -> bool {
    let p = mailcraft_mime::parse(raw);
    let field = |f: &str| -> String {
        let Some(p) = &p else { return String::new() };
        let addr = |a: &mailcraft_model::EmailAddress| format!("{} {}", a.name, a.email);
        match f {
            "from" => addr(&p.from),
            "to" => p.to.iter().map(addr).collect::<Vec<_>>().join(" "),
            "subject" => p.subject.clone(),
            "body" => p.text.clone(),
            _ => format!("{} {} {}", addr(&p.from), p.subject, p.text),
        }
        .to_lowercase()
    };
    // Tokens, a quoted part kept whole.
    let mut tokens: Vec<String> = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    for c in query.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut not = false;
    for t in tokens {
        if t == "NOT" {
            not = true;
            continue;
        }
        let (f, w) = match t.split_once(':') {
            Some((f, w)) if ["from", "to", "subject", "body"].contains(&f) => (f, w),
            _ => ("", t.as_str()),
        };
        if field(f).contains(&w.to_lowercase()) == not {
            return false;
        }
        not = false;
    }
    true
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

/// Prefix that turns a stored id into its non-immutable spelling.
const REGULAR: &str = "AAMk-";

/// Strip [`REGULAR`] if present.
fn plain_id(id: &str) -> &str {
    id.strip_prefix(REGULAR).unwrap_or(id)
}

fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n", body.len());
    if status == 429 {
        head.push_str("Retry-After: 1\r\n");
    }
    head.push_str("Connection: close\r\n\r\n");
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
            let (bearer, directory_bearer) = {
                let d = shared.lock().unwrap();
                (format!("Bearer {}", d.token.clone().unwrap_or_else(|| TOKEN.into())), d.directory_token.clone().map(|t| format!("Bearer {t}")))
            };
            let (mut used, mut headers) = (String::new(), Vec::new());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim();
                    if k.eq_ignore_ascii_case("authorization") && (v == bearer || directory_bearer.as_deref() == Some(v)) {
                        authorized = true;
                        used = v.trim_start_matches("Bearer ").to_string();
                    }
                    headers.push((k.trim().to_string(), v.to_string()));
                }
                immutable |= line.starts_with("Prefer: IdType=\"ImmutableId\"");
            }
            let mut body = vec![0u8; len];
            let _ = reader.read_exact(&mut body);
            if !authorized {
                respond(&mut s, 401, br#"{"error": {"code": "InvalidAuthenticationToken"}}"#, "application/json");
                continue;
            }
            let directory = first.contains(" /v1.0/users?") || first.contains(" /v1.0/me/people?");
            assert!(immutable || directory, "ids are asked for in their immutable form");
            let mut words = first.split(' ');
            let (method, target) = (words.next().unwrap_or("GET").to_string(), words.next().unwrap_or("/").to_string());
            let target = target.strip_prefix("/v1.0").unwrap_or(&target).to_string();
            let (status, out, kind) = {
                let mut d = shared.lock().unwrap();
                if d.throttle > 0 {
                    d.throttle -= 1;
                    (429, br#"{"error": {"code": "TooManyRequests"}}"#.to_vec(), "application/json")
                } else {
                    d.asked_immutable |= !directory;
                    d.bearer = used;
                    d.headers = headers;
                    d.route(&method, &target, &body, &b)
                }
            };
            respond(&mut s, status, &out, kind);
        }
    });
    Server { base, data }
}

#[cfg(test)]
mod tests;
