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

pub mod sse;
pub mod url;

use std::collections::{BTreeMap, HashMap, HashSet};

use mailcraft_mime as mime;
use mailcraft_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, SharedSummary, Summary, sort_newest_first, utf7};
use mailcraft_store::{FolderState, Store, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use url::{Url, pct};

mod client;
mod mailboxes;
mod search;
mod state;
mod sync;
pub use client::Client;
pub use mailboxes::{Mailbox, folders, list_folders, name_special};
pub use search::*;
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
/// Most messages asked for in one `Email/get` (less when the server says so).
const GET_CHUNK: usize = 200;

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

impl Error {
    /// Whether a change that met it waits in its queue for the next connection (the network,
    /// the sign-in), rather than being given up as refused.
    pub fn stops_the_queue(&self) -> bool {
        matches!(self, Error::Io(_) | Error::Closed(_) | Error::Tls(_) | Error::Auth(_))
    }
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

/// An HTTP response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
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
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    let num = |a: usize, b: usize| s.get(a..b).filter(|x| digits(x)).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut t = mailcraft_model::dates::days_from_civil(y, u32::try_from(mo).ok()?, u32::try_from(d).ok()?) * 86_400 + h * 3600 + mi * 60 + se;
    // Fractions are skipped; an offset other than Z is applied.
    let tail = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(off) = tail.strip_prefix('+').map(|o| (1, o)).or_else(|| tail.strip_prefix('-').map(|o| (-1, o))) {
        let (sign, o) = off;
        let oh = o.get(0..2).filter(|x| digits(x)).and_then(|x| x.parse::<i64>().ok())?;
        let om = o.get(3..5).filter(|x| digits(x)).and_then(|x| x.parse::<i64>().ok())?;
        t -= sign * (oh * 3600 + om * 60);
    }
    Some(t)
}

/// The messages of `folder` the server finds for `filter` ([`search_filter`]), the newest `cap`
/// of them: those not kept yet come into its list, and stay there past the window while the
/// server has them there (issue #4). Their keys, and how many came in. A folder never synced is
/// left to its sync. Nothing written once the account was removed since it had been removed
/// `seen` times ([`Store::while_kept`]): an error then.
pub async fn find_on_server<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    (account, seen): (&str, u64),
    folder: &str,
    filter: &Value,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let mailbox = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let ids = c.search(&mailbox, filter, cap.min(FOUND_MAX)).await?;
    let numbers: Vec<u32> = ids.iter().map(|id| st.number(id)).collect();
    // The numbers kept, in a set (looked up in the list, each found was a pass over the folder).
    let mut kept: HashSet<u32> = fs.messages.iter().map(|m| m.key.uid).collect();
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| !kept.contains(*n)).map(|(id, _)| id.clone()).collect();
    let mut new = 0;
    for e in c.headers(&missing).await? {
        let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if kept.insert(key.uid) {
            fs.messages.push(email_summary(&e, key).into());
            new += 1;
        }
    }
    st.add_found(folder, &numbers);
    if new > 0 {
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        sync::if_kept(store, account, seen, || store.save_folder(account, folder, &fs))?;
    }
    // (The first kept under a number, as a search of the list found it.)
    let keys: HashMap<u32, &MessageKey> = fs.messages.iter().rev().map(|m| (m.key.uid, &m.key)).collect();
    Ok((numbers.iter().filter_map(|n| keys.get(n).map(|k| (*k).clone())).collect(), new))
}

/// Download one message's source (and cache it), with its summary as its folder now has it, filled
/// from it ([`keep_source`]); one cached already comes as it is, with no summary. Not kept once
/// the account was removed since it had been removed `seen` times: an error then.
pub async fn fetch_body<H: Http>(c: &Client<H>, st: &State, store: &Store, key: &MessageKey, seen: u64) -> Result<(Vec<u8>, Option<Summary>)> {
    if let Some(raw) = store.read_body(key) {
        return Ok((raw, None));
    }
    let gone = || Error::No("the message is no longer on the server".into());
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(gone)?.to_string();
    let blob = c.blobs(std::slice::from_ref(&id)).await?.remove(&id).ok_or_else(gone)?;
    let raw = c.download(&blob).await?;
    let filled = sync::if_kept(store, &key.account, seen, || keep_source(store, key, &raw))?;
    Ok((raw, filled))
}

/// Apply one change on the server.
pub async fn apply_remote<H: Http>(c: &Client<H>, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
    };
    let patch_all = |keys: &[MessageKey], field: &str, on: bool| -> Map<String, Value> {
        let value = if on { Value::Bool(true) } else { Value::Null };
        ids(keys).into_iter().map(|(id, _)| (id, json!({ field: value.clone() }))).collect()
    };
    match op {
        Op::SetFlag { keys, flag, on } => match flag_keyword(*flag) {
            Some(k) => c.set_emails(patch_all(keys, &format!("keywords/{k}"), *on), Vec::new()).await,
            None => Ok(()),
        },
        Op::SetKeyword { keys, keyword, on } if mailcraft_model::is_keyword(keyword) => {
            c.set_emails(patch_all(keys, &format!("keywords/{}", keyword.to_ascii_lowercase()), *on), Vec::new()).await
        }
        // JMAP keeps no follow-up days (MailCraft keeps them).
        Op::SetKeyword { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } => Ok(()),
        Op::Move { keys, to } => {
            let to_box = st.mailbox(to)?.to_string();
            let mut update = Map::new();
            for (id, from) in ids(keys) {
                if from == *to {
                    continue;
                }
                let mut p = Map::new();
                if let Ok(b) = st.mailbox(&from) {
                    p.insert(format!("mailboxIds/{b}"), Value::Null);
                }
                p.insert(format!("mailboxIds/{to_box}"), Value::Bool(true));
                update.insert(id, Value::Object(p));
            }
            c.set_emails(update, Vec::new()).await
        }
        Op::Expunge { keys } => c.set_emails(Map::new(), ids(keys).into_iter().map(|(id, _)| id).collect()).await,
        Op::CreateFolder { path } => {
            let (parent, name) = st.split_path(path)?;
            c.set_mailboxes(json!({"create": {"f": {"name": name, "parentId": parent}}})).await.map(|_| ())
        }
        Op::RenameFolder { from, to } => {
            let id = st.mailbox(from)?.to_string();
            let (parent, name) = st.split_path(to)?;
            c.set_mailboxes(json!({"update": {id: {"name": name, "parentId": parent}}})).await.map(|_| ())
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.set_mailboxes(json!({"destroy": [id], "onDestroyRemoveEmails": true})).await.map(|_| ())
        }
        Op::EmptyFolder { path } => c.empty(st.mailbox(path)?).await,
        Op::MarkFolderRead { path } => c.mark_read(st.mailbox(path)?).await,
    }
}

/// Store a message in a folder (a draft, or the copy of a sent message).
pub async fn append<H: Http>(c: &Client<H>, st: &State, folder: &str, raw: &[u8], keywords: &[&str]) -> Result<String> {
    c.import(raw, st.mailbox(folder)?, keywords).await
}

/// Send a message: stored in the Sent folder, then submitted for the server to deliver (so the
/// copy in Sent needs no second upload).
pub async fn send<H: Http>(c: &Client<H>, st: &State, sent: Option<&str>, raw: &[u8], from: &str, recipients: &[String]) -> Result<()> {
    let folder = sent.filter(|f| st.mailboxes.contains_key(*f)).ok_or_else(|| Error::Send("the account has no Sent folder to send from".into()))?;
    let id = append(c, st, folder, raw, &["$seen"]).await?;
    match c.submit(&id, from, recipients).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Not sent: don't leave a copy that looks sent.
            let _ = c.set_emails(Map::new(), vec![id]).await;
            Err(e)
        }
    }
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
