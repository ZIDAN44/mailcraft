//! An in-memory Microsoft Graph mail API on localhost, for tests: mail folders (well-known
//! names, children, paging), messages (list, get, patch, move, delete, `$value`, made from
//! MIME), `sendMail`, mailbox settings and JSON batches, with a bearer token.

use std::io::{BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::http;

pub const TOKEN: &str = "test-token";
const ROOT: &str = "root";

/// The account these tests sign in with (`id`, `me@contoso.example`), on the fake at `base`.
pub fn account(id: &str, base: &str) -> mailcraft_model::AccountConfig {
    mailcraft_model::AccountConfig { id: id.into(), email: "me@contoso.example".into(), graph: Some(base.into()), ..Default::default() }
}

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
    /// Its follow-up marked complete (`flagStatus` `complete`).
    pub complete: bool,
    /// When its flag is due (`flag.dueDateTime`, in UTC), as written.
    pub due: Option<String>,
    /// Its single-value extended properties (a flag's reminder), by id as written.
    pub props: Vec<(String, String)>,
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
    /// People's free/busy times (issue #140): (address, a `scheduleItem` as Graph gives it, in
    /// UTC). Anyone asked about who has none here and isn't in [`Data::others`] or the
    /// directory isn't found.
    pub schedules: Vec<(String, Value)>,
    /// Someone's `workingHours`, as `getSchedule` gives it (issue #185).
    pub working_hours: Vec<(String, Value)>,
    /// The bodies of the `getSchedule` requests, in order.
    pub schedule_asks: Vec<Value>,
    /// Per-batch concurrency cap: entries past this index get a 429 with Retry-After.
    pub at_once: Option<usize>,
    /// Number of upcoming top-level requests to refuse with 429.
    pub throttle: u32,
    /// The one bearer token taken, when not [`TOKEN`] (a test renews it as tokens expire).
    pub token: Option<String>,
    /// Once a request whose target holds the first is answered, the token taken becomes the
    /// second: one running out, or revoked, in the middle of a sync.
    pub token_after: Option<(String, String)>,
    /// The organisation's directory (issue #9): (name, address), searched through `/users` and
    /// `/me/people`.
    pub directory: Vec<(String, String)>,
    /// The token the directory takes (its own permissions), when the mail's isn't enough; the
    /// mail's endpoints refuse it, as a token for other permissions is refused.
    pub directory_token: Option<String>,
    /// `/users` refuses the search (a guest, a tenant hiding its users): `/me/people` is asked.
    pub users_forbidden: bool,
    /// A series' occurrences, as `/me/events/{id}/instances` lists them: (series id, its
    /// occurrences' `{id, start}`).
    pub instances: Vec<(String, Vec<Value>)>,
    /// Reminders answered (issue #40): (`dismissReminder` or `snoozeReminder`, the event's or
    /// occurrence's id, the request's body).
    pub reminders: Vec<(String, String, Value)>,
    /// What the client did that the fake checks it never does, each answered 400: the test
    /// fails on any left here when the server is dropped (see [`Server`]'s `Drop`).
    pub violations: Vec<String>,
    /// The bearer token and headers of the request being answered.
    bearer: String,
    headers: Vec<(String, String)>,
    /// Set per request from its `Prefer` header, and per `$batch` entry from the entry's own;
    /// without `IdType="ImmutableId"` message ids are returned in the other (prefixed) form.
    /// Only a batch entry gets them so: any other request for mail without it is refused.
    asked_immutable: bool,
    /// Set per request: it asked for the messages' single-value extended properties.
    asked_props: bool,
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
            complete: false,
            due: None,
            props: Vec::new(),
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
        let mut v = self.message_fields(m);
        // Named back as Graph does: its own case.
        if self.asked_props && !m.props.is_empty() {
            v["singleValueExtendedProperties"] =
                json!(m.props.iter().map(|(id, value)| json!({"id": id.to_lowercase(), "value": value})).collect::<Vec<_>>());
        }
        v
    }

    fn message_fields(&self, m: &Msg) -> Value {
        let p = mailcraft_mime::parse(&m.raw);
        let addr = |a: &mailcraft_model::EmailAddress| json!({"emailAddress": {"name": a.name, "address": a.email}});
        json!({
            "id": if self.asked_immutable { m.id.clone() } else { format!("{REGULAR}{}", m.id) }, "parentFolderId": m.folder, "isRead": m.read, "isDraft": m.draft,
            "flag": {
                "flagStatus": if m.flagged { "flagged" } else if m.complete { "complete" } else { "notFlagged" },
                "dueDateTime": m.due.as_ref().map(|d| json!({"dateTime": d, "timeZone": "UTC"})),
            },
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
    fn route(&mut self, method: &str, target: &str, body: &[u8], base: &str) -> Answer {
        self.calls.push(format!("{method} {target}"));
        let now = std::time::Instant::now();
        for (_, raw) in self.late.extract_if(.., |(at, _)| *at <= now).collect::<Vec<_>>() {
            let sent = self.resolve("sentitems").unwrap_or_default();
            self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let input: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        // Extended properties come only when asked for, by their ids (as Graph has them).
        let expand = crate::oauth::unpct(query);
        self.asked_props = expand.contains("$expand=singleValueExtendedProperties($filter=id eq '");
        // As Graph answers a Microsoft account with no mailbox (one made with a Gmail address).
        if self.no_mailbox && matches!(parts.as_slice(), ["me", "mailFolders" | "messages" | "sendMail" | "mailboxSettings", ..]) {
            return error(404, "MailboxNotEnabledForRESTAPI", "The mailbox is either inactive, soft-deleted, or is hosted on-premise.");
        }
        let directory = matches!(parts.as_slice(), ["users"] | ["me", "people"]);
        if let Some(dt) = self.directory_token.clone() {
            // Each token is good for its own permissions only.
            if directory != (self.bearer == dt) {
                return forbidden();
            }
        }
        if directory {
            return self.directory_answer(&parts, query);
        }
        match (method, parts.as_slice()) {
            ("POST", ["$batch"]) => self.batch(&input, base),
            (_, ["me", "mailFolders", ..]) => self.on_mail_folders(method, &parts, &input, path, query, base),
            (_, ["me", "messages", ..]) => self.on_messages(method, &parts, &input, body, query),
            ("POST", ["me", "sendMail"]) => self.send_mail(body),
            (_, ["me", "mailboxSettings", ..]) => self.on_mailbox_settings(method, &parts, &input),
            ("GET" | "POST" | "PATCH" | "DELETE", ["users", who, ..]) => self.on_others(method, who, &parts, &input),
            ("POST", ["me", "calendar", "getSchedule"]) => self.schedule(&input),
            (_, ["me", "calendars" | "events" | "contacts", ..]) => self.on_pim(method, &parts, &input, query),
            _ => not_found(),
        }
    }

    /// A request breaking what the fake checks of the client: kept for the test to fail on, and
    /// answered 400 with why (not asserted here: that would kill the server, not fail the test).
    fn refuse(&mut self, why: String) -> Answer {
        let refused = error(400, "BadRequest", &format!("the fake refuses it: {why}"));
        self.violations.push(why);
        refused
    }

    /// A listing two items a page, with `@odata.nextLink` for the rest.
    fn page(&self, list: Vec<Value>, path: &str, query: &str, base: &str) -> Answer {
        let skip: usize = query.split('&').find_map(|kv| kv.strip_prefix("$skip=")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let page: Vec<Value> = list.iter().skip(skip).take(2).cloned().collect();
        let mut v = json!({ "value": page });
        if skip + 2 < list.len() {
            let rest: String = query.split('&').filter(|kv| !kv.starts_with("$skip=")).collect::<Vec<_>>().join("&");
            v["@odata.nextLink"] = json!(format!("{base}{path}?{rest}&$skip={}", skip + 2));
        }
        ok(v)
    }
}

/// An answer: (status, body, content type).
type Answer = (u16, Vec<u8>, &'static str);

fn ok(v: Value) -> Answer {
    answer(200, &v)
}

/// An answer in JSON.
fn answer(status: u16, v: &Value) -> Answer {
    (status, v.to_string().into_bytes(), "application/json")
}

/// An error as Graph answers one: its code and its message, each left out when empty.
fn error(status: u16, code: &str, message: &str) -> Answer {
    let mut e = serde_json::Map::new();
    if !code.is_empty() {
        e.insert("code".into(), json!(code));
    }
    if !message.is_empty() {
        e.insert("message".into(), json!(message));
    }
    answer(status, &json!({ "error": e }))
}

fn not_found() -> Answer {
    error(404, "ErrorItemNotFound", "Not found.")
}

fn forbidden() -> Answer {
    error(403, "Authorization_RequestDenied", "Insufficient privileges to complete the operation.")
}

/// A query's parameter, as given (still percent-encoded).
fn param(query: &str, k: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(str::to_string))
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

/// A MIME source as Graph takes it, in base64; nothing when it isn't base64.
fn unbase64(body: &[u8]) -> Vec<u8> {
    crate::ews::unb64(&String::from_utf8_lossy(body)).unwrap_or_default()
}

pub struct Server {
    /// The API address (`…/v1.0`).
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

/// Prefix that turns a stored id into its non-immutable spelling.
const REGULAR: &str = "AAMk-";

/// Strip [`REGULAR`] if present.
fn plain_id(id: &str) -> &str {
    id.strip_prefix(REGULAR).unwrap_or(id)
}

/// An answer; the connection is closed after it.
fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let mut headers = vec![("Content-Type", kind.to_string())];
    if status == 429 {
        headers.push(("Retry-After", "1".into()));
    }
    headers.push(("Connection", "close".into()));
    http::respond(s, &format!("{status} X"), &headers, body);
}

/// Whether a `Prefer` header's value asks for ids in their immutable form, among its other
/// preferences.
fn asks_immutable(prefer: &str) -> bool {
    prefer.split(',').any(|part| part.trim() == "IdType=\"ImmutableId\"")
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
            let Some(r) = s.try_clone().ok().and_then(|c| http::read_request(&mut BufReader::new(c))) else { continue };
            let (bearer, directory_bearer) = {
                let d = shared.lock().unwrap();
                (format!("Bearer {}", d.token.clone().unwrap_or_else(|| TOKEN.into())), d.directory_token.clone().map(|t| format!("Bearer {t}")))
            };
            let used = r.headers_named("authorization").find(|v| *v == bearer || directory_bearer.as_deref() == Some(*v));
            let Some(used) = used.map(|v| v.trim_start_matches("Bearer ").to_string()) else {
                let (status, out, kind) = error(401, "InvalidAuthenticationToken", "");
                respond(&mut s, status, &out, kind);
                continue;
            };
            let immutable = r.headers_named("prefer").any(asks_immutable);
            let directory = r.target.starts_with("/v1.0/users?") || r.target.starts_with("/v1.0/me/people?");
            if !immutable && !directory {
                let (status, out, kind) =
                    shared.lock().unwrap().refuse(format!("ids not asked for in their immutable form: {} {}", r.method, r.target));
                respond(&mut s, status, &out, kind);
                continue;
            }
            let (method, body, headers) = (r.method, r.body, r.headers);
            let target = r.target.strip_prefix("/v1.0").unwrap_or(&r.target).to_string();
            let (status, out, kind) = {
                let mut d = shared.lock().unwrap();
                if d.throttle > 0 {
                    d.throttle -= 1;
                    error(429, "TooManyRequests", "")
                } else {
                    d.asked_immutable = immutable;
                    d.bearer = used;
                    d.headers = headers;
                    let answer = d.route(&method, &target, &body, &b);
                    if let Some((_, next)) = d.token_after.take_if(|(after, _)| target.contains(after.as_str())) {
                        d.token = Some(next);
                    }
                    answer
                }
            };
            respond(&mut s, status, &out, kind);
        }
    });
    Server { base, data }
}

mod calendars;
mod directory;
mod mail;
#[cfg(test)]
mod tests;
