//! JMAP (RFC 8620 core, RFC 8621 mail): MailCraft's second transport, behind the same
//! [`Job`](mailcraft_model::Job)/[`Event`](mailcraft_model::Event) interface as IMAP. Portable:
//! it runs over any [`Http`] client, the desktop's own (through `mailcraft-net`, which waits on
//! each request) or the browser's `fetch` (the web app).
//!
//! Every request is one HTTPS POST of a batch of method calls to the session's API URL, signed
//! in with Basic authentication (user name and password) or an OAuth bearer token. Message
//! sources are blobs, downloaded and uploaded through the session's URL templates.
//!
//! The cache and the engine key messages the IMAP way (folder, UIDVALIDITY, UID). JMAP ids are
//! strings that stay the same across mailboxes, so each account numbers them as they are first
//! seen ([`State`]) and the folder's "UIDVALIDITY" is a constant. Folders are named by their
//! path of mailbox names (`Projects/Short film`, the Inbox is `INBOX`) like IMAP's, so moves,
//! favourites and rules work unchanged. (The Microsoft Graph transport shares this numbering.)
//!
//! A folder sync asks for the newest [`SyncOptions::window`] ids with their keywords, a page of
//! the server's `maxObjectsInGet` per round trip (the mailbox's counts with the first), then the
//! headers of the ones not cached yet. Mail that falls out of the window leaves the cache, unless
//! a search of the server found it ([`find_on_server`]): that stays while the mailbox has it.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod challenge;
pub mod sse;
pub mod url;

use std::collections::{BTreeMap, HashMap};

use mailcraft_mime as mime;
use mailcraft_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, SharedSummary, Summary, sort_newest_first, utf7};
use mailcraft_store::{FolderState, Store, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use url::{Url, base64, pct};

mod mailboxes;
mod remote;
mod search;
mod state;
mod sync;
pub use mailboxes::{Mailbox, folders, list_folders, name_special};
pub use remote::{append, apply_remote, fetch_body, send};
use search::*;
pub use search::*;
use state::*;
pub use state::*;
pub use sync::{sync_folder, sync_folder_as};

const CORE: &str = "urn:ietf:params:jmap:core";
const MAIL: &str = "urn:ietf:params:jmap:mail";
const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
const QUOTA: &str = "urn:ietf:params:jmap:quota";
const VACATION: &str = "urn:ietf:params:jmap:vacationresponse";

/// Largest JSON response accepted.
const MAX_JSON: usize = 32 << 20;
/// Largest message source downloaded.
const MAX_BLOB: usize = 64 << 20;
/// The constant "UIDVALIDITY" of JMAP folders (the numbering is MailCraft's own).
pub const VALIDITY: u32 = 1;
/// The Inbox's folder path, as on IMAP.
const INBOX: &str = "INBOX";
/// Deepest mailbox nesting followed.
const MAX_DEPTH: usize = 32;

/// What can go wrong, in the categories the callers act on.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The network failed (reconnect).
    #[error("network: {0}")]
    Io(String),
    #[error("secure connection: {0}")]
    Tls(String),
    #[error("unexpected server response: {0}")]
    Protocol(String),
    #[error("the server refused: {0}")]
    No(String),
    #[error("the server rejected the command: {0}")]
    Bad(String),
    #[error("sign-in failed: {0}")]
    Auth(String),
    #[error("the server closed the connection: {0}")]
    Closed(String),
    #[error("sending failed: {0}")]
    Send(String),
    #[error("{0}")]
    Store(#[from] StoreError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An HTTP request.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Largest body accepted.
    pub max: usize,
}

impl Request {
    pub fn get(url: &str, max: usize) -> Request {
        Request { method: "GET".into(), url: url.into(), headers: Vec::new(), body: Vec::new(), max }
    }

    pub fn post(url: &str, content_type: &str, body: Vec<u8>, max: usize) -> Request {
        Request { method: "POST".into(), url: url.into(), headers: vec![("Content-Type".into(), content_type.into())], body, max }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// An HTTP response. `headers`: what the client could read of them (a browser shows a page only
/// those the server lets it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// How the server asks to be signed in to: every `WWW-Authenticate` header, read as one list.
    pub fn challenges(&self) -> Vec<challenge::Challenge> {
        let all: Vec<&str> = self.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("www-authenticate")).map(|(_, v)| v.as_str()).collect();
        challenge::parse(&all.join(", "))
    }
}

/// The HTTP client JMAP runs over. Implementations follow redirects for GET, never from https
/// to http, and cap the body at [`Request::max`].
#[allow(async_fn_in_trait)]
pub trait Http {
    async fn send(&self, req: Request) -> Result<Response>;
}

/// The session URL for what the user typed: a bare host or origin gets the well-known path
/// (RFC 8620 §2.2).
pub fn session_url(input: &str) -> String {
    let s = input.trim().trim_end_matches('/');
    let s = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let after_scheme = s.split_once("://").map_or("", |(_, r)| r);
    if after_scheme.contains('/') { s } else { format!("{s}/.well-known/jmap") }
}

/// A signed-in JMAP session.
pub struct Client<H> {
    http: H,
    auth: String,
    api: String,
    download: String,
    upload: String,
    /// The push template (RFC 8620 §7.3), when the server offers one.
    events: Option<String>,
    account: String,
    submission: bool,
    quota: bool,
    vacation: bool,
    max_get: usize,
}

fn str_at<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| Error::Protocol(format!("the JMAP session has no `{key}`")))
}

fn json_body(resp: &Response) -> Result<Value> {
    serde_json::from_slice(&resp.body).map_err(|e| Error::Protocol(format!("JMAP: {e}")))
}

/// Turn an HTTP failure into the error the caller knows how to handle. `auth`: the
/// `Authorization` the request carried.
fn check(resp: &Response, auth: &str) -> Result<()> {
    match resp.status {
        200..=299 => Ok(()),
        401 => Err(Error::Auth(refusal(resp, auth))),
        403 => Err(Error::Auth("the server refused the user name or password".into())),
        // Busy or down: reconnect with back-off.
        429 | 500..=599 => Err(Error::Closed(format!("HTTP {}", resp.status))),
        s => {
            // RFC 7807 problem details, when there are any.
            let detail = serde_json::from_slice::<Value>(&resp.body)
                .ok()
                .and_then(|v| v.get("detail").or_else(|| v.get("type")).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            Err(Error::Bad(format!("HTTP {s} {detail}").trim().to_string()))
        }
    }
}

/// Why the server said 401, from how it asks to be signed in to (RFC 9110 §11.6.1). A server
/// that wants bearer tokens only (a sign-in through the browser) takes no password, however
/// right it is: [`NO_PASSWORDS`](mailcraft_model::NO_PASSWORDS) says so.
fn refusal(resp: &Response, auth: &str) -> String {
    let challenges = resp.challenges();
    let offers = |scheme: &str| challenges.iter().find(|c| c.scheme == scheme);
    if auth.starts_with("Bearer ") {
        // RFC 6750 §3.1: what was wrong with the token.
        return match offers("bearer").and_then(|c| c.param("error")).filter(|e| e.len() <= 64) {
            Some(e) => format!("the server refused the sign-in ({e})"),
            None => "the server refused the sign-in".into(),
        };
    }
    if challenges.is_empty() || offers("basic").is_some() {
        return "the server refused the user name or password".into();
    }
    if offers("bearer").is_some() {
        return mailcraft_model::NO_PASSWORDS.into();
    }
    let schemes: Vec<&str> = challenges.iter().map(|c| c.scheme.as_str()).take(4).collect();
    format!("the server takes no passwords, nor any sign-in MailCraft knows ({})", schemes.join(", "))
}

/// `{accountId}`-style template variables (RFC 6570 level 1), percent-encoded.
fn expand(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), &pct(v));
    }
    out
}

impl<H: Http> Client<H> {
    /// Fetch the session and check it offers mail. `bearer` signs in with an OAuth access token.
    pub async fn connect(http: H, url: &str, user: &str, secret: &str, bearer: bool) -> Result<Client<H>> {
        let auth = if bearer { format!("Bearer {secret}") } else { format!("Basic {}", base64(format!("{user}:{secret}").as_bytes())) };
        let session = session_url(url);
        let resp = http.send(Request::get(&session, 1 << 20).header("Authorization", &auth).header("Accept", "application/json")).await?;
        check(&resp, &auth)?;
        let v = json_body(&resp)?;
        let caps = v.get("capabilities").cloned().unwrap_or(Value::Null);
        if caps.get(MAIL).is_none() {
            return Err(Error::Protocol("this JMAP server doesn't offer mail".into()));
        }
        let account = v
            .get("primaryAccounts")
            .and_then(|p| p.get(MAIL))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("no mail account in the JMAP session".into()))?;
        let base = Url::parse(&session)?;
        let resolve = |key: &str| -> Result<String> {
            let u = base.join(str_at(&v, key)?)?;
            // Never from HTTPS down to plain HTTP (the password goes with every request).
            if base.https && !u.https {
                return Err(Error::Tls(format!("the JMAP {key} isn't https")));
            }
            Ok(u.full())
        };
        let max_get = caps.get(CORE).and_then(|c| c.get("maxObjectsInGet")).and_then(Value::as_u64).unwrap_or(500);
        Ok(Client {
            api: resolve("apiUrl")?,
            // Templates keep their braces: resolve only the part before them.
            download: template_url(&base, str_at(&v, "downloadUrl")?)?,
            upload: template_url(&base, str_at(&v, "uploadUrl")?)?,
            events: v.get("eventSourceUrl").and_then(Value::as_str).and_then(|t| template_url(&base, t).ok()),
            account: account.to_string(),
            submission: caps.get(SUBMISSION).is_some(),
            quota: caps.get(QUOTA).is_some(),
            vacation: caps.get(VACATION).is_some(),
            max_get: usize::try_from(max_get).unwrap_or(500).clamp(1, 1000),
            auth,
            http,
        })
    }

    /// The `Authorization` header value every request carries.
    pub fn authorization(&self) -> &str {
        &self.auth
    }

    /// Carry a new OAuth access token from now on (the old one ran out).
    pub fn set_bearer(&mut self, token: &str) {
        self.auth = format!("Bearer {token}");
    }

    /// The push address (RFC 8620 §7.3) for every type of change, when the server pushes.
    pub fn events_url(&self) -> Option<String> {
        self.events.as_ref().map(|t| expand(t, &[("types", "*"), ("closeafter", "no"), ("ping", "30")]))
    }

    /// Run a batch of method calls; the results in order. A method error fails the batch.
    async fn call(&self, using: &[&str], calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        let method_calls: Vec<Value> = calls.iter().enumerate().map(|(i, (name, args))| json!([name, args, format!("c{i}")])).collect();
        let body = json!({ "using": using, "methodCalls": method_calls });
        let bytes = serde_json::to_vec(&body).map_err(|e| Error::Protocol(e.to_string()))?;
        let resp = self
            .http
            .send(
                Request::post(&self.api, "application/json", bytes, MAX_JSON)
                    .header("Authorization", &self.auth)
                    .header("Accept", "application/json"),
            )
            .await?;
        check(&resp, &self.auth)?;
        let v = json_body(&resp)?;
        let responses = v.get("methodResponses").and_then(Value::as_array).ok_or_else(|| Error::Protocol("JMAP: no methodResponses".into()))?;
        let mut out = Vec::with_capacity(calls.len());
        for i in 0..calls.len() {
            let id = format!("c{i}");
            let r = responses
                .iter()
                .filter_map(Value::as_array)
                .find(|r| r.get(2).and_then(Value::as_str) == Some(id.as_str()))
                .ok_or_else(|| Error::Protocol(format!("JMAP: no answer to {}", calls.get(i).map_or("", |c| c.0))))?;
            let name = r.first().and_then(Value::as_str).unwrap_or_default();
            let args = r.get(1).cloned().unwrap_or(Value::Null);
            if name == "error" {
                let kind = args.get("type").and_then(Value::as_str).unwrap_or("error");
                let desc = args.get("description").and_then(Value::as_str).unwrap_or_default();
                return Err(Error::No(format!("{kind} {desc}").trim().to_string()));
            }
            out.push(args);
        }
        Ok(out)
    }

    async fn mail(&self, calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        self.call(&[CORE, MAIL], calls).await
    }

    pub async fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let r = self.mail(vec![("Mailbox/get", json!({"accountId": self.account, "ids": null}))]).await?;
        Ok(list(r.first(), "list").iter().filter_map(Mailbox::from_json).collect())
    }

    /// The newest `limit` ids in a mailbox with their keywords, and its (total, unread) counts.
    /// A page of the server's `maxObjectsInGet` at a time (by position): asked at once, a folder
    /// kept no more than that however many it was to keep.
    async fn newest(&self, mailbox: &str, limit: usize) -> Result<(Vec<(String, Flags)>, Option<(u32, u32)>)> {
        let limit = limit.clamp(1, 100_000);
        let (mut ids, mut counts, mut seen, mut total) = (Vec::new(), None, 0usize, None);
        for page in 0..1000 {
            if seen >= limit {
                break;
            }
            let want = (limit - seen).min(self.max_get);
            let mut calls = vec![
                (
                    "Email/query",
                    json!({"accountId": self.account, "filter": {"inMailbox": mailbox}, "sort": [{"property": "receivedAt", "isAscending": false}],
                           "position": seen, "limit": want, "calculateTotal": page == 0}),
                ),
                (
                    "Email/get",
                    json!({"accountId": self.account, "#ids": {"resultOf": "c0", "name": "Email/query", "path": "/ids"}, "properties": ["id", "keywords"]}),
                ),
            ];
            if page == 0 {
                calls.push(("Mailbox/get", json!({"accountId": self.account, "ids": [mailbox], "properties": ["totalEmails", "unreadEmails"]})));
            }
            let r = self.mail(calls).await?;
            let order: Vec<&str> = list(r.first(), "ids").iter().filter_map(Value::as_str).collect();
            let got: HashMap<&str, Flags> =
                list(r.get(1), "list").iter().filter_map(|e| Some((e.get("id")?.as_str()?, keywords(e.get("keywords"))))).collect();
            ids.extend(order.iter().filter_map(|id| got.get(id).map(|f| (id.to_string(), f.clone()))));
            if page == 0 {
                counts = list(r.get(2), "list").first().map(|m| (count(m, "totalEmails"), count(m, "unreadEmails")));
                total = r.first().and_then(|q| q.get("total")).and_then(Value::as_u64).and_then(|t| usize::try_from(t).ok());
            }
            seen += order.len();
            // The end: nothing more, all there is, or (with no total) a short page.
            if order.is_empty() || total.map_or(order.len() < want, |t| seen >= t) {
                break;
            }
        }
        Ok((ids, counts))
    }

    /// Headers and details of messages, for the list.
    async fn headers(&self, ids: &[String]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(self.max_get.min(200)) {
            let r = self
                .mail(vec![(
                    "Email/get",
                    json!({"accountId": self.account, "ids": chunk, "properties": ["id", "keywords", "size", "receivedAt", "preview", "hasAttachment", "headers"]}),
                )])
                .await?;
            out.extend(list(r.first(), "list").iter().cloned());
        }
        Ok(out)
    }

    /// The blob ids of messages' sources.
    async fn blobs(&self, ids: &[String]) -> Result<HashMap<String, String>> {
        let r = self.mail(vec![("Email/get", json!({"accountId": self.account, "ids": ids, "properties": ["id", "blobId"]}))]).await?;
        Ok(list(r.first(), "list")
            .iter()
            .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("blobId")?.as_str()?.to_string())))
            .collect())
    }

    pub async fn download(&self, blob: &str) -> Result<Vec<u8>> {
        let url = expand(&self.download, &[("accountId", &self.account), ("blobId", blob), ("type", "message/rfc822"), ("name", "message.eml")]);
        let resp = self.http.send(Request::get(&url, MAX_BLOB).header("Authorization", &self.auth)).await?;
        check(&resp, &self.auth)?;
        Ok(resp.body)
    }

    pub async fn upload(&self, raw: &[u8]) -> Result<String> {
        let url = expand(&self.upload, &[("accountId", &self.account)]);
        let resp = self.http.send(Request::post(&url, "message/rfc822", raw.to_vec(), 1 << 20).header("Authorization", &self.auth)).await?;
        check(&resp, &self.auth)?;
        let v = json_body(&resp)?;
        Ok(str_at(&v, "blobId")?.to_string())
    }

    /// Add a message to a mailbox from its source.
    pub async fn import(&self, raw: &[u8], mailbox: &str, keywords: &[&str]) -> Result<String> {
        let blob = self.upload(raw).await?;
        let kw: Map<String, Value> = keywords.iter().map(|k| (k.to_string(), Value::Bool(true))).collect();
        let r = self
            .mail(vec![(
                "Email/import",
                json!({"accountId": self.account, "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": kw}}}),
            )])
            .await?;
        set_error(r.first(), "notCreated")?;
        r.first()
            .and_then(|v| v.get("created"))
            .and_then(|c| c.get("m"))
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Protocol("JMAP: the import made no message".into()))
    }

    /// Update and destroy messages (`Email/set`).
    async fn set_emails(&self, update: Map<String, Value>, destroy: Vec<String>) -> Result<()> {
        if update.is_empty() && destroy.is_empty() {
            return Ok(());
        }
        let r = self.mail(vec![("Email/set", json!({"accountId": self.account, "update": update, "destroy": destroy}))]).await?;
        set_error(r.first(), "notUpdated")?;
        set_error(r.first(), "notDestroyed")
    }

    async fn set_mailboxes(&self, args: Value) -> Result<Value> {
        let mut a = args;
        if let Some(o) = a.as_object_mut() {
            o.insert("accountId".into(), Value::String(self.account.clone()));
        }
        let r = self.mail(vec![("Mailbox/set", a)]).await?;
        for k in ["notCreated", "notUpdated", "notDestroyed"] {
            set_error(r.first(), k)?;
        }
        Ok(r.into_iter().next().unwrap_or(Value::Null))
    }

    /// Send a message already stored on the server, from the identity for `from`.
    async fn submit(&self, email: &str, from: &str, recipients: &[String]) -> Result<()> {
        if !self.submission {
            return Err(Error::Send("this JMAP server doesn't send mail".into()));
        }
        let using = [CORE, MAIL, SUBMISSION];
        let r = self.call(&using, vec![("Identity/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let identities = list(r.first(), "list");
        let found = identities
            .iter()
            .find(|i| i.get("email").and_then(Value::as_str).is_some_and(|e| e.eq_ignore_ascii_case(from)))
            .or_else(|| identities.first())
            .and_then(|i| i.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // None yet (Stalwart makes none for a new account): one made for the address, as RFC 8621
        // §6 lets a client; a server that won't says why.
        let identity = match found {
            Some(id) => id,
            None => {
                let r = self.call(&using, vec![("Identity/set", json!({"accountId": self.account, "create": {"i": {"email": from}}}))]).await?;
                set_error(r.first(), "notCreated")
                    .map_err(|e| Error::Send(format!("the JMAP account has no identity to send from, and none could be made: {e}")))?;
                r.first()
                    .and_then(|v| v.get("created")?.get("i")?.get("id")?.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| Error::Send("the JMAP account has no identity to send from".into()))?
            }
        };
        let identity = identity.as_str();
        let rcpt: Vec<Value> = recipients.iter().map(|r| json!({"email": r})).collect();
        let r = self
            .call(
                &using,
                vec![(
                    "EmailSubmission/set",
                    json!({"accountId": self.account, "create": {"s": {"identityId": identity, "emailId": email, "envelope": {"mailFrom": {"email": from}, "rcptTo": rcpt}}}}),
                )],
            )
            .await?;
        set_error(r.first(), "notCreated").map_err(|e| Error::Send(e.to_string()))
    }

    /// The addresses the account's mail is sent from, as the server keeps them (RFC 8621 §6:
    /// its identities), each with its name and where replies to it go; none for a server
    /// that doesn't send mail. At most a hundred, each held to what an address is.
    pub async fn identities(&self) -> Result<Vec<mailcraft_model::Identity>> {
        if !self.submission {
            return Ok(Vec::new());
        }
        let r = self.call(&[CORE, MAIL, SUBMISSION], vec![("Identity/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let text = |v: Option<&Value>, most: usize| v.and_then(Value::as_str).unwrap_or_default().trim().chars().take(most).collect::<String>();
        let found = list(r.first(), "list").iter().take(100).map(|i| {
            let reply_to = i.get("replyTo").and_then(Value::as_array).and_then(|l| l.first()).and_then(|a| a.get("email"));
            mailcraft_model::Identity {
                email: text(i.get("email"), 254),
                name: text(i.get("name"), 200),
                reply_to: text(reply_to, 254),
                signature: None,
            }
        });
        Ok(found.filter(|i| mailcraft_model::addr::is_valid_email(&i.email)).collect())
    }

    /// Space used and the limit in bytes for the whole account (RFC 9425); `None` without quotas.
    pub async fn quota(&self) -> Result<Option<(u64, u64)>> {
        if !self.quota {
            return Ok(None);
        }
        let r = self.call(&[CORE, QUOTA], vec![("Quota/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let octets = list(r.first(), "list")
            .iter()
            .filter(|q| q.get("resourceType").and_then(Value::as_str) == Some("octets"))
            .filter(|q| q.get("scope").and_then(Value::as_str).is_none_or(|s| s == "account"))
            .find_map(|q| Some((q.get("used")?.as_u64()?, q.get("hardLimit").and_then(Value::as_u64).unwrap_or(0))));
        Ok(octets)
    }

    /// The automatic replies (RFC 8621 §8); an error when the server offers none.
    pub async fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        if !self.vacation {
            return Err(Error::No("this server doesn't offer automatic replies".into()));
        }
        let r = self.call(&[CORE, VACATION], vec![("VacationResponse/get", json!({"accountId": self.account, "ids": ["singleton"]}))]).await?;
        let v = list(r.first(), "list").first().cloned().unwrap_or(Value::Null);
        let day =
            |k: &str| v.get(k).and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| mailcraft_model::job::is_day(d)).map(str::to_string);
        Ok(mailcraft_model::job::Vacation {
            enabled: v.get("isEnabled").and_then(Value::as_bool).unwrap_or(false),
            subject: v.get("subject").and_then(Value::as_str).unwrap_or_default().to_string(),
            text: v.get("textBody").and_then(Value::as_str).unwrap_or_default().to_string(),
            from: day("fromDate"),
            until: day("toDate"),
            ..Default::default()
        })
    }

    pub async fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        if !self.vacation {
            return Err(Error::No("this server doesn't offer automatic replies".into()));
        }
        // Dates are whole days: from the start of the first to the end of the last.
        let from = v.from.as_ref().map(|d| format!("{d}T00:00:00Z"));
        let to = v.until.as_ref().map(|d| format!("{d}T23:59:59Z"));
        let patch = json!({
            "isEnabled": v.enabled,
            "subject": if v.subject.trim().is_empty() { Value::Null } else { Value::String(v.subject.clone()) },
            "textBody": v.text,
            "fromDate": from,
            "toDate": to,
        });
        let r =
            self.call(&[CORE, VACATION], vec![("VacationResponse/set", json!({"accountId": self.account, "update": {"singleton": patch}}))]).await?;
        set_error(r.first(), "notUpdated")
    }

    /// Destroy every message in a mailbox, in batches.
    async fn empty(&self, mailbox: &str) -> Result<()> {
        for _ in 0..1000 {
            let r = self
                .mail(vec![("Email/query", json!({"accountId": self.account, "filter": {"inMailbox": mailbox}, "limit": self.max_get.min(500)}))])
                .await?;
            let ids: Vec<String> = list(r.first(), "ids").iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            if ids.is_empty() {
                return Ok(());
            }
            self.set_emails(Map::new(), ids).await?;
        }
        Err(Error::No("the folder didn't empty".into()))
    }
}

/// A download or upload template resolved against the session URL, braces intact.
fn template_url(base: &Url, t: &str) -> Result<String> {
    let cut = t.find('{').unwrap_or(t.len());
    let (head, rest) = t.split_at(cut);
    let u = base.join(head)?;
    if base.https && !u.https {
        return Err(Error::Tls("a JMAP address isn't https".into()));
    }
    Ok(format!("{}{rest}", u.full()))
}

fn list<'a>(v: Option<&'a Value>, key: &str) -> &'a [Value] {
    v.and_then(|v| v.get(key)).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn count(v: &Value, key: &str) -> u32 {
    v.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0)
}

/// The first `notCreated`/`notUpdated`/`notDestroyed` reason, as an error.
fn set_error(v: Option<&Value>, key: &str) -> Result<()> {
    match v.and_then(|v| v.get(key)).and_then(Value::as_object).and_then(|m| m.values().next()) {
        Some(e) => {
            let kind = e.get("type").and_then(Value::as_str).unwrap_or("refused");
            let desc = e.get("description").and_then(Value::as_str).unwrap_or_default();
            Err(Error::No(format!("{kind} {desc}").trim().to_string()))
        }
        None => Ok(()),
    }
}

/// JMAP keywords as the IMAP flags the model knows (RFC 8621 §4.1.1).
fn keywords(v: Option<&Value>) -> Flags {
    let names: Vec<String> = v
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, on)| on.as_bool() == Some(true))
                .take(64)
                .map(|(k, _)| match k.to_ascii_lowercase().as_str() {
                    "$seen" => "\\Seen".to_string(),
                    "$flagged" => "\\Flagged".to_string(),
                    "$answered" => "\\Answered".to_string(),
                    "$draft" => "\\Draft".to_string(),
                    _ => k.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    Flags::from_imap(names.iter().map(String::as_str))
}

/// The JMAP keyword for a flag; `None` for `\Deleted`, which JMAP doesn't have (messages are
/// destroyed instead).
fn flag_keyword(f: Flag) -> Option<&'static str> {
    match f {
        Flag::Seen => Some("$seen"),
        Flag::Flagged => Some("$flagged"),
        Flag::Answered => Some("$answered"),
        Flag::Forwarded => Some("$forwarded"),
        Flag::Deleted => None,
    }
}

/// `2024-05-01T09:30:00Z` (RFC 3339, as JMAP's UTCDate) in Unix seconds.
pub fn utc_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut t = mailcraft_model::dates::days_from_civil(y, u32::try_from(mo).ok()?, u32::try_from(d).ok()?) * 86_400 + h * 3600 + mi * 60 + se;
    // Fractions are skipped; an offset other than Z is applied.
    let tail = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(off) = tail.strip_prefix('+').map(|o| (1, o)).or_else(|| tail.strip_prefix('-').map(|o| (-1, o))) {
        let (sign, o) = off;
        let oh = o.get(0..2).and_then(|x| x.parse::<i64>().ok())?;
        let om = o.get(3..5).and_then(|x| x.parse::<i64>().ok())?;
        t -= sign * (oh * 3600 + om * 60);
    }
    Some(t)
}

/// Run a future whose every wait is already over, as the desktop's blocking HTTP client's are:
/// it completes on the first poll. (A future that does wait is polled again until done.)
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests;
