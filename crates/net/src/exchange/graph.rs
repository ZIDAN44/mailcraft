//! Microsoft Graph mail: the transport for Exchange Online (Microsoft 365, Outlook.com), behind
//! the same [`Job`](mailcraft_model::Job)/[`Event`](mailcraft_model::Event) interface as IMAP
//! and JMAP. It works where an organisation has switched IMAP off.
//!
//! Every request is HTTPS with an OAuth bearer token (`Mail.ReadWrite`, `Mail.Send`,
//! `MailboxSettings.ReadWrite`), against the documented REST API: mail folders, messages,
//! their MIME source (`$value`), `sendMail` with a MIME body, and mailbox settings for automatic
//! replies. Ids are asked for in their immutable form (`Prefer: IdType="ImmutableId"`), so a
//! message keeps its id when it moves, as JMAP's do: the numbering and folder paths are the
//! JMAP transport's ([`jmap::State`], [`jmap::folders`]). Several small requests go as one
//! `$batch` (four per batch; see [`BATCH`]).

use std::collections::{HashMap, HashSet};

use mailcraft_mime as mime;
use mailcraft_model::{Flags, Folder, FolderRole, MessageKey, Op, Summary};
use mailcraft_store::Store;
use serde_json::{Value, json};

use crate::http::{self, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, WordQuery};
use crate::{NetError, Result};

/// The Graph endpoint accounts use.
pub const DEFAULT_URL: &str = "https://graph.microsoft.com/v1.0";
/// Largest JSON response accepted.
const MAX_JSON: usize = 32 << 20;
/// Largest message source downloaded.
const MAX_MIME: usize = 64 << 20;
/// Entries per `$batch`. Graph executes a batch's entries concurrently and Exchange Online's
/// per-app, per-mailbox concurrency limit is 4, so larger batches mostly came back as 429s.
const BATCH: usize = 4;
/// Attempts per request (or batch entry) while the server keeps answering 429.
const BUSY_ROUNDS: u32 = 4;
/// Upper bound in seconds on a single back-off, whatever `Retry-After` says.
const BUSY_WAIT_CAP: u64 = 20;
/// Up to this many uncached messages are fetched individually (with internet headers); above
/// it the listing's own fields are used.
const FEW: usize = 20;
/// Pages followed for one listing.
const MAX_PAGES: usize = 200;
/// Folders listed at most.
const MAX_FOLDERS: usize = 5000;
const FOLDER_FIELDS: &str = "id,displayName,parentFolderId,childFolderCount,totalItemCount,unreadItemCount";
const HEADER_FIELDS: &str = "id,isRead,isDraft,flag,subject,from,sender,toRecipients,ccRecipients,replyTo,receivedDateTime,sentDateTime,bodyPreview,hasAttachments,internetMessageId,internetMessageHeaders";
/// `$select` for folder listings. `internetMessageHeaders` is left out on purpose: it is
/// expensive for the server to produce per item, and listing pages that include it are slow
/// enough to hit gateway timeouts. [`fill_from_source`] recovers threading later.
const LIST_FIELDS: &str = "id,subject,from,sender,toRecipients,ccRecipients,replyTo,internetMessageId,receivedDateTime,importance,hasAttachments,bodyPreview,isRead,flag,isDraft";
/// Sent on every call, including inside each `$batch` entry, where the outer header is ignored.
const PREFER: &str = "IdType=\"ImmutableId\", outlook.timezone=\"UTC\"";

/// A signed-in Graph session.
pub struct Client {
    base: String,
    auth: String,
}

/// Turn an HTTP failure into the error the worker knows how to handle.
fn check(resp: &Response) -> Result<()> {
    status_error(resp.status, &resp.body)
}

/// What a Microsoft account without a mailbox Graph can reach gets (a personal account made
/// with a Gmail or other address, a mailbox on premises).
pub(crate) const NO_MAILBOX: &str = "this Microsoft account has no Outlook.com or Microsoft 365 mailbox that MailCraft can reach. \
If its mail is with another provider (such as Gmail), add it with that provider's settings instead";

fn status_error(status: u16, body: &[u8]) -> Result<()> {
    status_error_of(status, serde_json::from_slice::<Value>(body).ok().as_ref())
}

/// [`status_error`] of an answer already read (a `$batch` entry's body).
fn status_error_of(status: u16, body: Option<&Value>) -> Result<()> {
    let error = body.and_then(|v| v.get("error"));
    let field = |k: &str| error.and_then(|e| e.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
    let detail = || {
        let message = field("message");
        if message.is_empty() { field("code") } else { message }
    };
    if matches!(field("code").as_str(), "MailboxNotEnabledForRESTAPI" | "MailboxNotSupportedForRESTAPI") {
        return Err(NetError::No(NO_MAILBOX.into()));
    }
    match status {
        200..=299 => Ok(()),
        401 => Err(NetError::Auth("Microsoft refused the sign-in; sign in again".into())),
        429 | 500..=599 => Err(NetError::Closed(format!("HTTP {status}"))),
        404 => Err(NetError::No(format!("{} ({})", crate::ews::GONE, format!("HTTP 404 {}", detail()).trim()))),
        400 if field("code").starts_with(crate::ews::INVALID_ID) => {
            Err(NetError::No(format!("{} ({})", crate::ews::NO_SUCH_ID, format!("HTTP 400 {}", detail()).trim())))
        }
        s => Err(NetError::No(format!("HTTP {s} {}", detail()).trim().to_string())),
    }
}

fn json_body(body: &[u8]) -> Result<Value> {
    serde_json::from_slice(body).map_err(|e| NetError::Protocol(format!("Graph: {e}")))
}

pub(crate) fn seg(id: &str) -> String {
    http::pct(id)
}

impl Client {
    /// Check the token works (and that the address is the mailbox's).
    pub fn connect(base: &str, token: &str) -> Result<Client> {
        let base = base.trim().trim_end_matches('/').to_string();
        let url = http::Url::parse(&base)?;
        if !url.https && !http::is_loopback(&url.host) {
            return Err(NetError::Tls("the Graph address isn't https".into()));
        }
        let c = Client { base, auth: format!("Bearer {token}") };
        c.get("/me/mailFolders/inbox?$select=id")?;
        Ok(c)
    }

    /// Carry a new access token from now on (the old one ran out).
    pub fn set_token(&mut self, token: &str) {
        self.auth = format!("Bearer {token}");
    }

    fn request(&self, mut r: Request) -> Result<Value> {
        for (name, value) in [("Authorization", self.auth.as_str()), ("Accept", "application/json"), ("Prefer", PREFER)] {
            r = r.header(name, value);
        }
        let resp = send_through_limits(&r)?;
        check(&resp)?;
        if resp.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        json_body(&resp.body)
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value> {
        self.request(Request::get(&format!("{}{path}", self.base), MAX_JSON))
    }

    pub(crate) fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let bytes = serde_json::to_vec(body).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.request(Request::post(&format!("{}{path}", self.base), "application/json", bytes, MAX_JSON))
    }

    pub(crate) fn send_raw(&self, method: &str, path: &str, content_type: &str, body: Vec<u8>) -> Result<Value> {
        let mut r = Request::post(&format!("{}{path}", self.base), content_type, body, MAX_JSON);
        r.method = method.into();
        self.request(r)
    }

    /// Every item of a listing, following `@odata.nextLink` (on the same server only).
    pub(crate) fn all(&self, path: &str, limit: usize) -> Result<Vec<Value>> {
        let origin = http::Url::parse(&self.base)?.origin();
        let mut out = Vec::new();
        let mut url = format!("{}{path}", self.base);
        for _ in 0..MAX_PAGES {
            let v = self.request(Request::get(&url, MAX_JSON))?;
            out.extend(v.get("value").and_then(Value::as_array).cloned().unwrap_or_default());
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
            match v.get("@odata.nextLink").and_then(Value::as_str) {
                Some(next) if http::Url::parse(next).is_ok_and(|u| u.origin() == origin) => url = next.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Run requests through `$batch`, [`BATCH`] at a time, returning (status, body) per request
    /// in input order. Entries answered 429 are resubmitted on their own after the longest
    /// `Retry-After` among them, up to [`BUSY_ROUNDS`] times; whatever is still 429 is returned
    /// as such.
    fn batch(&self, requests: &[(&str, String, Option<Value>)]) -> Result<Vec<(u16, Value)>> {
        let mut out = Vec::with_capacity(requests.len());
        for (n, chunk) in requests.chunks(BATCH).enumerate() {
            let mut results: HashMap<usize, (u16, Value)> = HashMap::new();
            let mut todo: Vec<usize> = (0..chunk.len()).collect();
            let mut round = 0;
            while !todo.is_empty() && round < BUSY_ROUNDS {
                round += 1;
                let entries: Vec<Value> = todo.iter().filter_map(|&i| chunk.get(i).map(|req| batch_entry(i, req))).collect();
                let reply = self.post("/$batch", &json!({ "requests": entries }))?;
                let by_id: HashMap<&str, &Value> = reply
                    .get("responses")
                    .and_then(Value::as_array)
                    .map(|rs| rs.iter().filter_map(|r| Some((r.get("id")?.as_str()?, r))).collect())
                    .unwrap_or_default();
                let mut busy = Vec::new();
                let mut pause = 0;
                for &i in &todo {
                    let key = (i + 1).to_string();
                    let Some(r) = by_id.get(key.as_str()) else {
                        return Err(NetError::Protocol(format!("Graph: no answer in batch {n}")));
                    };
                    let Some(status) = r["status"].as_u64().and_then(|s| u16::try_from(s).ok()).filter(|s| *s != 0) else {
                        return Err(NetError::Protocol(format!("Graph: no answer in batch {n}")));
                    };
                    if status == 429 {
                        busy.push(i);
                        pause = pause.max(backoff_secs(entry_retry_after(r).as_deref(), round));
                    }
                    results.insert(i, (status, r.get("body").cloned().unwrap_or(Value::Null)));
                }
                todo = busy;
                if !todo.is_empty() && round < BUSY_ROUNDS {
                    std::thread::sleep(std::time::Duration::from_secs(pause));
                }
            }
            out.extend((0..chunk.len()).map(|i| results.remove(&i).unwrap_or((429, Value::Null))));
        }
        Ok(out)
    }

    /// Run requests as batches and fail on the first refusal.
    fn batch_ok(&self, requests: &[(&str, String, Option<Value>)]) -> Result<()> {
        for (status, body) in self.batch(requests)? {
            // Already gone is fine for a delete or a move.
            if status == 404 {
                continue;
            }
            status_error_of(status, Some(&body))?;
        }
        Ok(())
    }

    /// The mail folders, with roles for the well-known ones; the Outbox and Exchange's own
    /// bookkeeping folders are left out.
    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let roles = [
            ("inbox", FolderRole::Inbox),
            ("drafts", FolderRole::Drafts),
            ("sentitems", FolderRole::Sent),
            ("deleteditems", FolderRole::Trash),
            ("junkemail", FolderRole::Junk),
            ("archive", FolderRole::Archive),
        ];
        let hidden = ["outbox", "conversationhistory", "syncissues", "recoverableitemsdeletions"];
        let wk: Vec<(&str, String, Option<Value>)> =
            roles.iter().map(|(n, _)| *n).chain(hidden).map(|n| ("GET", format!("/me/mailFolders/{n}?$select=id"), None)).collect();
        let ids: Vec<Option<String>> = self
            .batch(&wk)?
            .into_iter()
            .map(|(s, b)| if s == 200 { b.get("id").and_then(Value::as_str).map(str::to_string) } else { None })
            .collect();
        let role_of: HashMap<String, FolderRole> = roles.iter().zip(&ids).filter_map(|((_, r), id)| Some((id.clone()?, *r))).collect();
        let skip: Vec<String> = ids.iter().skip(roles.len()).flatten().cloned().collect();

        let mut out: Vec<Mailbox> = Vec::new();
        let mut listed: HashSet<String> = HashSet::new();
        let mut pending: Vec<Value> = self.all(&format!("/me/mailFolders?$top=100&$select={FOLDER_FIELDS}"), MAX_FOLDERS)?;
        while let Some(f) = pending.pop() {
            let Some(id) = f.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
            if skip.contains(&id) || out.len() >= MAX_FOLDERS || !listed.insert(id.clone()) {
                continue;
            }
            if f.get("childFolderCount").and_then(Value::as_u64).unwrap_or(0) > 0 {
                pending.extend(self.all(&format!("/me/mailFolders/{}/childFolders?$top=100&$select={FOLDER_FIELDS}", seg(&id)), MAX_FOLDERS)?);
            }
            let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
            out.push(Mailbox {
                name: f.get("displayName").and_then(Value::as_str).unwrap_or_default().chars().take(255).collect(),
                parent: f.get("parentFolderId").and_then(Value::as_str).map(str::to_string),
                role: role_of.get(&id).copied().unwrap_or(FolderRole::Other),
                total: count("totalItemCount"),
                unread: count("unreadItemCount"),
                id,
            });
        }
        Ok(out)
    }

    /// The newest `window` messages of a folder with their list details ([`LIST_FIELDS`]), and
    /// its (total, unread).
    fn newest(&self, folder: &str, window: usize) -> Result<(Vec<Value>, (u32, u32))> {
        let page = window.clamp(1, 1000);
        let mut entries = self.all(
            &format!(
                "/me/mailFolders/{}/messages?$top={page}&$orderby=receivedDateTime%20desc&$select={LIST_FIELDS}&{}",
                seg(folder),
                reminder_expand()
            ),
            window.min(100_000),
        )?;
        entries.retain(|m| m["id"].is_string());
        let f = self.get(&format!("/me/mailFolders/{}?$select=totalItemCount,unreadItemCount", seg(folder)))?;
        let count = |k: &str| f.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        Ok((entries, (count("totalItemCount"), count("unreadItemCount"))))
    }

    /// Of these messages, those still in a folder, with their flags (through `$batch`).
    fn still_in(&self, folder: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let reqs: Vec<(&str, String, Option<Value>)> = ids
            .iter()
            .map(|id| ("GET", format!("/me/messages/{}?$select=id,isRead,isDraft,flag,parentFolderId&{}", seg(id), reminder_expand()), None))
            .collect();
        Ok(self
            .batch(&reqs)?
            .into_iter()
            .filter(|(s, m)| *s == 200 && m.get("parentFolderId").and_then(Value::as_str).is_none_or(|p| p == folder))
            .filter_map(|(_, m)| Some((m.get("id")?.as_str()?.to_string(), flags(&m))))
            .collect())
    }

    /// Messages' details, for the list.
    fn headers(&self, ids: &[String]) -> Result<Vec<Value>> {
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids.iter().map(|id| ("GET", format!("/me/messages/{}?$select={HEADER_FIELDS}&{}", seg(id), reminder_expand()), None)).collect();
        Ok(self.batch(&reqs)?.into_iter().filter(|(s, _)| *s == 200).map(|(_, b)| b).collect())
    }

    /// A message's MIME source.
    pub fn download(&self, id: &str) -> Result<Vec<u8>> {
        let r = Request::get(&format!("{}/me/messages/{}/$value", self.base, seg(id)), MAX_MIME)
            .header("Authorization", &self.auth)
            .header("Prefer", "IdType=\"ImmutableId\"");
        let resp = send_through_limits(&r)?;
        check(&resp)?;
        Ok(resp.body)
    }

    /// A message from its MIME source, as a draft (Graph files it in Drafts), moved to `folder`
    /// when that's somewhere else. Its id.
    pub fn import(&self, raw: &[u8], folder: Option<&str>) -> Result<String> {
        let v = self.send_raw("POST", "/me/messages", "text/plain", mailcraft_model::base64::encode(raw).into_bytes())?;
        let id = v.get("id").and_then(Value::as_str).ok_or_else(|| NetError::Protocol("Graph: the message wasn't made".into()))?.to_string();
        if let Some(f) = folder {
            let parent = v.get("parentFolderId").and_then(Value::as_str).unwrap_or_default();
            if parent != f {
                let moved = self.post(&format!("/me/messages/{}/move", seg(&id)), &json!({"destinationId": f}))?;
                return Ok(moved.get("id").and_then(Value::as_str).unwrap_or(&id).to_string());
            }
        }
        Ok(id)
    }

    /// Send a MIME message; Exchange files the copy in Sent Items.
    pub fn send_mime(&self, raw: &[u8]) -> Result<()> {
        self.send_raw("POST", "/me/sendMail", "text/plain", mailcraft_model::base64::encode(raw).into_bytes()).map(|_| ()).map_err(|e| match e {
            NetError::No(m) | NetError::Bad(m) => NetError::Smtp(m),
            e => e,
        })
    }

    /// The automatic replies.
    pub fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        let v = self.get("/me/mailboxSettings/automaticRepliesSetting")?;
        let status = v.get("status").and_then(Value::as_str).unwrap_or("disabled").to_ascii_lowercase();
        let day = |k: &str| {
            v.get(k).and_then(|d| d.get("dateTime")).and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| mailcraft_model::job::is_day(d))
        };
        let scheduled = status == "scheduled";
        let reply = |k: &str| crate::exchange::reply_text(v.get(k).and_then(Value::as_str).unwrap_or_default());
        let (text, external) = (reply("internalReplyMessage"), reply("externalReplyMessage"));
        let outside = match v.get("externalAudience").and_then(Value::as_str).unwrap_or("all").to_ascii_lowercase().as_str() {
            "none" => "none",
            "contactsonly" => "known",
            _ => "",
        };
        Ok(mailcraft_model::job::Vacation {
            enabled: status != "disabled",
            subject: String::new(),
            external: (!external.is_empty() && external != text).then_some(external),
            outside: outside.to_string(),
            text,
            from: day("scheduledStartDateTime").filter(|_| scheduled).map(str::to_string),
            until: day("scheduledEndDateTime").filter(|_| scheduled).map(str::to_string),
            ..Default::default()
        })
    }

    pub fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        let html = crate::exchange::reply_html;
        let dated = v.from.is_some() || v.until.is_some();
        let audience = match v.outside.as_str() {
            "none" => "none",
            "known" => "contactsOnly",
            _ => "all",
        };
        let mut s = json!({
            "status": if !v.enabled { "disabled" } else if dated { "scheduled" } else { "alwaysEnabled" },
            "externalAudience": audience,
            "internalReplyMessage": html(&v.text),
            "externalReplyMessage": html(v.external_text()),
        });
        if v.enabled && dated {
            // Whole days: from the start of the first to the end of the last (open ends far off).
            let from = v.from.clone().unwrap_or_else(|| "2000-01-01".into());
            let until = v.until.clone().unwrap_or_else(|| "2099-12-31".into());
            s["scheduledStartDateTime"] = json!({"dateTime": format!("{from}T00:00:00"), "timeZone": "UTC"});
            s["scheduledEndDateTime"] = json!({"dateTime": format!("{until}T23:59:59"), "timeZone": "UTC"});
        }
        let body = serde_json::to_vec(&json!({ "automaticRepliesSetting": s })).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", "/me/mailboxSettings", "application/json", body).map(|_| ())
    }

    /// Delete every message in a folder, a batch at a time.
    fn empty(&self, folder: &str) -> Result<()> {
        for _ in 0..1000 {
            let ids = self.all(&format!("/me/mailFolders/{}/messages?$top=100&$select=id", seg(folder)), 100)?;
            let reqs: Vec<(&str, String, Option<Value>)> = ids
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .map(|id| ("DELETE", format!("/me/messages/{}", seg(id)), None))
                .collect();
            if reqs.is_empty() {
                return Ok(());
            }
            self.batch_ok(&reqs)?;
        }
        Err(NetError::No("the folder didn't empty".into()))
    }

    /// Mark every message in a folder read, a page of the unread ones at a time (issue #197:
    /// Graph has nothing for a whole folder).
    fn mark_read(&self, folder: &str) -> Result<()> {
        for _ in 0..1000 {
            let ids = self.all(&format!("/me/mailFolders/{}/messages?$filter=isRead%20eq%20false&$top=100&$select=id", seg(folder)), 100)?;
            let reqs: Vec<(&str, String, Option<Value>)> = ids
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str))
                .map(|id| ("PATCH", format!("/me/messages/{}", seg(id)), Some(json!({"isRead": true}))))
                .collect();
            if reqs.is_empty() {
                return Ok(());
            }
            self.batch_ok(&reqs)?;
        }
        Err(NetError::No("the folder's mail didn't all get marked read".into()))
    }
}

/// `{"emailAddress": {"name", "address"}}` as a header value.
fn address(v: &Value) -> Option<String> {
    let e = v.get("emailAddress")?;
    let addr = e.get("address").and_then(Value::as_str).unwrap_or_default().trim();
    if addr.is_empty() || addr.contains(['\r', '\n', '<', '>']) {
        return None;
    }
    let name = e.get("name").and_then(Value::as_str).unwrap_or_default().replace(['"', '\\', '\r', '\n'], "");
    Some(if name.trim().is_empty() || name == addr { addr.to_string() } else { format!("\"{}\" <{addr}>", name.trim()) })
}

fn addresses(v: Option<&Value>) -> String {
    v.and_then(Value::as_array).map(|a| a.iter().take(500).filter_map(address).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// A header block for a message's details: its own internet headers when Graph has them, else
/// one made from its fields.
fn header_block(m: &Value) -> Vec<u8> {
    let mut out = String::new();
    let own = m.get("internetMessageHeaders").and_then(Value::as_array).filter(|h| !h.is_empty());
    match own {
        Some(headers) => {
            for h in headers.iter().take(500) {
                let (Some(name), Some(value)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str)) else { continue };
                if name.is_empty() || name.contains([':', '\r', '\n', ' ']) || value.contains(['\r', '\n']) {
                    continue;
                }
                out.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        None => {
            let clean = |s: &str| s.replace(['\r', '\n'], " ");
            for (name, value) in [
                ("From", m.get("from").and_then(address).or_else(|| m.get("sender").and_then(address)).unwrap_or_default()),
                ("To", addresses(m.get("toRecipients"))),
                ("Cc", addresses(m.get("ccRecipients"))),
                ("Reply-To", addresses(m.get("replyTo"))),
                ("Subject", clean(m.get("subject").and_then(Value::as_str).unwrap_or_default())),
                ("Message-ID", clean(m.get("internetMessageId").and_then(Value::as_str).unwrap_or_default())),
                ("Importance", importance_header(m)),
            ] {
                if !value.trim().is_empty() {
                    out.push_str(&format!("{name}: {value}\r\n"));
                }
            }
        }
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// The `Importance:` value for Graph's `importance` property (`normal` gives none).
fn importance_header(m: &Value) -> String {
    let level = m["importance"].as_str().unwrap_or_default();
    if level.eq_ignore_ascii_case("high") || level.eq_ignore_ascii_case("low") {
        let mut c = level.chars();
        c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
    } else {
        String::new()
    }
}

/// One `$batch` entry: numbered from 1, with the [`PREFER`] header (and a JSON body if any).
fn batch_entry(i: usize, (method, url, body): &(&str, String, Option<Value>)) -> Value {
    let mut headers = serde_json::Map::new();
    headers.insert("Prefer".into(), json!(PREFER));
    let mut e = json!({"id": (i + 1).to_string(), "method": method, "url": url});
    if let Some(b) = body {
        headers.insert("Content-Type".into(), json!("application/json"));
        e["body"] = b.clone();
    }
    e["headers"] = Value::Object(headers);
    e
}

/// A batch entry's `Retry-After`, whatever case the key has and whether it's a string or number.
fn entry_retry_after(r: &Value) -> Option<String> {
    let headers = r.get("headers")?.as_object()?;
    match headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("Retry-After"))?.1 {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `http::send`, resubmitted while the answer is 429, up to [`BUSY_ROUNDS`] attempts.
fn send_through_limits(r: &Request) -> Result<Response> {
    let mut round = 0;
    loop {
        round += 1;
        let resp = http::send(r)?;
        if resp.status != 429 || round >= BUSY_ROUNDS {
            return Ok(resp);
        }
        std::thread::sleep(std::time::Duration::from_secs(backoff_secs(resp.header("Retry-After"), round)));
    }
}

/// Seconds to pause before attempt `round + 1`: a numeric `Retry-After` hint if present,
/// otherwise 2 s per round so far; clamped to 1..=[`BUSY_WAIT_CAP`].
fn backoff_secs(hint: Option<&str>, round: u32) -> u64 {
    let fallback = 2 * u64::from(round.max(1));
    hint.and_then(|h| h.trim().parse().ok()).unwrap_or(fallback).clamp(1, BUSY_WAIT_CAP)
}

/// Refresh the folder list (and the folder ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    let boxes = c.mailboxes()?;
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    jmap::name_special(&mut found, &boxes);
    Ok(found.into_iter().map(|(f, _)| f).collect())
}

/// A message's list entry from its details.
fn message_summary(m: &Value, key: MessageKey) -> Summary {
    let received = m.get("receivedDateTime").and_then(Value::as_str).and_then(jmap::utc_date).unwrap_or(0);
    let mut s = mime::summary_from_header(&header_block(m), key, flags(m), 0, received);
    s.preview = m.get("bodyPreview").and_then(Value::as_str).map(mime::preview).unwrap_or_default();
    s.has_attachments = m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false);
    s
}

/// The messages of `folder` the server finds for a search (issue #4), the newest `cap` of them:
/// its words through `$search` (KQL), or, with none, its read and flag conditions through
/// `$filter`. Those not kept yet come into its list and stay there past the window while the
/// folder has them. Their keys, and how many came in. Nothing written once the account was
/// removed since it had been removed `seen` times: an error then.
pub fn find_on_server(
    c: &Client,
    st: &mut State,
    store: &Store,
    (account, seen): (&str, u64),
    folder: &str,
    query: &WordQuery,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let id = st.mailbox(folder)?.to_string();
    let fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let cap = cap.min(jmap::FOUND_MAX);
    let ask = if query.text.is_empty() {
        let mut f = Vec::new();
        if let Some(u) = query.unread {
            f.push(format!("isRead eq {}", !u));
        }
        if let Some(on) = query.flagged {
            f.push(format!("flag/flagStatus {} 'flagged'", if on { "eq" } else { "ne" }));
        }
        if let Some(a) = query.attachment {
            f.push(format!("hasAttachments eq {a}"));
        }
        // When it came, as the list has it (issue #179).
        if let Some(t) = query.after {
            f.push(format!("receivedDateTime ge {}", mailcraft_model::dates::utc_stamp(t)));
        }
        if let Some(t) = query.before {
            f.push(format!("receivedDateTime lt {}", mailcraft_model::dates::utc_stamp(t)));
        }
        format!("$filter={}", http::pct(&f.join(" and ")))
    } else {
        format!("$search={}", http::pct(&format!("\"{}\"", query.text.replace('"', "\\\""))))
    };
    let found = c.all(&format!("/me/mailFolders/{}/messages?{ask}&$top={}&$select={HEADER_FIELDS}", seg(&id), cap.clamp(1, 1000)), cap)?;
    let found: Vec<&Value> =
        found.iter().filter(|m| query.admits(&flags(m), m.get("hasAttachments").and_then(Value::as_bool).unwrap_or(false))).collect();
    let mut numbers = Vec::new();
    let mut have = crate::exchange::window::kept_numbers(&fs);
    let mut new = Vec::new();
    for m in found {
        let Some(mid) = m.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(mid) };
        numbers.push(key.uid);
        if have.insert(key.uid) {
            new.push(message_summary(m, key));
        }
    }
    crate::exchange::window::found(st, store, (account, seen), folder, fs, &numbers, new)
}

/// Download one message's source (and cache it, unless the account was removed since it had been
/// removed `seen` times).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey, seen: u64) -> Result<(Vec<u8>, Option<Summary>)> {
    crate::exchange::window::fetch_body(st, store, key, seen, |id| c.download(id))
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| crate::exchange::window::server_ids(st, keys);
    let patch_all = |keys: &[MessageKey], patch: Value| -> Result<()> {
        let reqs: Vec<(&str, String, Option<Value>)> =
            ids(keys).into_iter().map(|(id, _)| ("PATCH", format!("/me/messages/{}", seg(&id)), Some(patch.clone()))).collect();
        c.batch_ok(&reqs)
    };
    match op {
        Op::SetFlag { keys, .. } | Op::SetFollowUp { keys, .. } | Op::SetReminder { keys, .. } | Op::SetKeyword { keys, .. } => {
            match flag_patch(op) {
                Some(patch) => patch_all(keys, patch),
                // Answered and forwarded are Exchange's to set; categories stay on this computer.
                None => Ok(()),
            }
        }
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let reqs: Vec<(&str, String, Option<Value>)> = ids(keys)
                .into_iter()
                .filter(|(_, from)| from != to)
                .map(|(id, _)| ("POST", format!("/me/messages/{}/move", seg(&id)), Some(json!({"destinationId": dest}))))
                .collect();
            c.batch_ok(&reqs)
        }
        Op::Expunge { keys } => {
            let reqs: Vec<(&str, String, Option<Value>)> =
                ids(keys).into_iter().map(|(id, _)| ("DELETE", format!("/me/messages/{}", seg(&id)), None)).collect();
            c.batch_ok(&reqs)
        }
        Op::CreateFolder { path } => {
            let (parent, name) = st.split_path(path)?;
            let at = parent.map_or_else(|| "/me/mailFolders".to_string(), |p| format!("/me/mailFolders/{}/childFolders", seg(&p)));
            c.post(&at, &json!({"displayName": name})).map(|_| ())
        }
        Op::RenameFolder { from, to } => {
            let id = st.mailbox(from)?.to_string();
            let (old_parent, _) = st.split_path(from)?;
            let (parent, name) = st.split_path(to)?;
            let mut id = id;
            if parent != old_parent {
                let dest = parent.clone().unwrap_or_else(|| "msgfolderroot".into());
                let moved = c.post(&format!("/me/mailFolders/{}/move", seg(&id)), &json!({"destinationId": dest}))?;
                id = moved.get("id").and_then(Value::as_str).unwrap_or(&id).to_string();
            }
            let body = serde_json::to_vec(&json!({"displayName": name})).map_err(|e| NetError::Protocol(e.to_string()))?;
            c.send_raw("PATCH", &format!("/me/mailFolders/{}", seg(&id)), "application/json", body).map(|_| ())
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.send_raw("DELETE", &format!("/me/mailFolders/{}", seg(&id)), "application/json", Vec::new()).map(|_| ())
        }
        Op::EmptyFolder { path } => c.empty(st.mailbox(path)?),
        Op::MarkFolderRead { path } => c.mark_read(st.mailbox(path)?),
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.import(raw, Some(st.mailbox(folder)?))
}

/// Send a message. The MIME source names the recipients for Exchange, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    match crate::exchange::window::with_bcc(raw, recipients) {
        Some(with) => c.send_mime(&with),
        None => c.send_mime(raw),
    }
}

mod availability;
mod directory;
mod flags;
use flags::{flag_patch, flags, reminder_expand};
mod sync;
pub use sync::{sync_folder, sync_folder_as};
mod text_first;
pub use directory::{needs_consent, search_directory, search_directory_full};
pub use text_first::fetch_text;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_from_fields() {
        let m = json!({
            "from": {"emailAddress": {"name": "Zoe \"Z\" Park", "address": "zoe@studio.example"}},
            "toRecipients": [{"emailAddress": {"name": "", "address": "me@studio.example"}}, {"emailAddress": {"name": "bad", "address": "x\r\nBcc: evil"}}],
            "subject": "Hello\r\nX-Injected: 1",
            "internetMessageId": "<m1@x>",
        });
        let h = String::from_utf8(header_block(&m)).unwrap();
        assert!(h.starts_with("From: \"Zoe Z Park\" <zoe@studio.example>\r\nTo: me@studio.example\r\n"), "{h}");
        assert!(h.contains("Subject: Hello  X-Injected: 1\r\n"), "no header injection: {h}");
        assert!(!h.contains("evil"));
        let s = mime::summary_from_header(&header_block(&m), MessageKey::default(), Flags::default(), 0, 5);
        assert_eq!((s.from.email.as_str(), s.message_id.as_str()), ("zoe@studio.example", "m1@x"));
        let own = json!({"internetMessageHeaders": [{"name": "Subject", "value": "Own"}, {"name": "Bad Name", "value": "x"}]});
        assert_eq!(String::from_utf8(header_block(&own)).unwrap(), "Subject: Own\r\n\r\n");
        let f = flags(&json!({"isRead": true, "flag": {"flagStatus": "flagged"}, "isDraft": false}));
        assert!(f.seen && f.flagged && !f.draft && !f.follow_up_complete());
        let f = flags(&json!({"flag": {"flagStatus": "complete", "completedDateTime": {"dateTime": "2026-10-30T00:00:00", "timeZone": "UTC"}}}));
        assert!(!f.flagged && f.follow_up_complete(), "{f:?}");
        assert!(!flags(&json!({"flag": {"flagStatus": "notFlagged"}})).has(mailcraft_model::FOLLOW_UP_COMPLETE));
    }

    #[test]
    fn errors() {
        assert!(matches!(status_error(401, b""), Err(NetError::Auth(_))));
        assert!(matches!(status_error(503, b""), Err(NetError::Closed(_))));
        let e = status_error(403, br#"{"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}"#);
        assert!(matches!(e, Err(NetError::No(m)) if m == "HTTP 403 Access is denied."));
        assert!(Client::connect("http://graph.example.com/v1.0", "t").is_err(), "https only, except on this computer");
        // A batch entry's refusal read from its body as it came, and its wait in any case.
        let denied = json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}});
        assert!(matches!(status_error_of(403, Some(&denied)), Err(NetError::No(m)) if m == "HTTP 403 Access is denied."));
        assert!(status_error_of(200, Some(&Value::Null)).is_ok());
        for headers in [json!({"Retry-after": "7"}), json!({"RETRY-AFTER": 7}), json!({"retry-After": "7"})] {
            assert_eq!(entry_retry_after(&json!({ "headers": headers })).as_deref(), Some("7"));
        }
    }

    /// A picture the body shows comes in the text first as such (its Content-ID, inline), as in
    /// EWS's; a file, or a picture with no Content-ID, is a file.
    #[test]
    fn a_text_firsts_inline_picture_is_inline() {
        let v = json!({"subject": "Logo", "body": {"contentType": "html", "content": "<p><img src=\"cid:logo@x\"></p>"},
            "attachments": [
                {"name": "logo.png", "contentType": "image/png", "size": 120, "isInline": true, "contentId": "logo@x"},
                {"name": "notes.pdf", "contentType": "application/pdf", "size": 900, "isInline": false},
                {"name": "odd.png", "contentType": "image/png", "size": 5, "isInline": true}]});
        let p = mime::parse(&text_first::text_first(&v).unwrap()).unwrap();
        let files: Vec<(&str, bool, Option<&str>)> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline, a.content_id.as_deref())).collect();
        assert_eq!(files, [("logo.png", true, Some("logo@x")), ("notes.pdf", false, None), ("odd.png", false, None)]);
    }
}
