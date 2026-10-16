//! Exchange Web Services: the transport for Exchange servers run by an organisation itself
//! (on-premises Exchange 2010 SP2 and later) that don't offer IMAP, behind the same
//! [`Job`](mailcraft_model::Job)/[`Event`](mailcraft_model::Event) interface as the others.
//!
//! Every request is a SOAP call over HTTPS to the server's `/EWS/Exchange.asmx`, signed in with
//! NTLMv2 ([`crate::ntlm`], on a connection kept open, as NTLM signs in the connection) or Basic
//! authentication, whichever the server offers. Mail: folders (`FindFolder`), the newest
//! messages of a folder (`FindItem`), their details (`GetItem`), MIME sources, changes
//! (`UpdateItem`, `MoveItem`, `DeleteItem`, folder operations), drafts and sending
//! (`CreateItem` from MIME) and automatic replies (`Get`/`SetUserOofSettings`). Item ids are
//! numbered the JMAP transport's way ([`jmap::State`]); an item that moves gets a new id in
//! Exchange, so it's a new message in its new folder.
//!
//! Answers are hostile input: sizes, depth and counts are capped, nothing here panics.

use std::cell::RefCell;
use std::collections::HashMap;

use mailcraft_mime as mime;
use mailcraft_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, sort_newest_first, utf7};
use mailcraft_store::{FolderState, Store};
use quick_xml::Reader;
use quick_xml::events::Event as X;

use crate::http::{self, Conn, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, fill_from_source};
use crate::{NetError, Result};

/// The EWS address of an account whose server Autodiscover finds.
pub const AUTODISCOVER: &str = mailcraft_model::EWS_AUTODISCOVER;
/// Largest answer accepted (message sources come base64-encoded inside).
const MAX_ANSWER: usize = 96 << 20;
/// Items asked about in one request.
const BATCH: usize = 50;
const MAX_FOLDERS: usize = 5000;
/// Public folders: how deep the tree is walked, and how many are listed.
const PUBLIC_DEPTH: usize = 6;
const MAX_PUBLIC: usize = 500;
/// The folder Exchange's public folders are listed under.
pub const PUBLIC_FOLDERS: &str = "Public Folders";
/// Elements read from one answer at most, and how deep.
const MAX_ELEMENTS: usize = 500_000;
const MAX_DEPTH: usize = 64;

/// An element of an answer: its local name, attributes, text and children.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub kids: Vec<El>,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// The first element called `name` below this one (depth first).
    pub fn find(&self, name: &str) -> Option<&El> {
        self.kids.iter().find_map(|k| if k.name == name { Some(k) } else { k.find(name) })
    }
    /// Every element called `name` below this one.
    pub fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for k in &self.kids {
            if k.name == name {
                out.push(k);
            } else {
                k.all(name, out);
            }
        }
    }
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }
}

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    s.rsplit(':').next().unwrap_or("").to_string()
}

/// An attribute's value with its entities replaced.
fn unescape(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = after.get(..end).unwrap_or("");
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n => n
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read XML into elements (namespaces dropped: the local names are enough here).
pub fn parse(xml: &str) -> El {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<El> = vec![El { name: "#doc".into(), ..Default::default() }];
    let mut count = 0usize;
    let close = |stack: &mut Vec<El>| {
        if stack.len() > 1
            && let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.kids.push(done);
        }
    };
    while let Ok(ev) = r.read_event() {
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(e) | X::Empty(e) if stack.len() < MAX_DEPTH && count < MAX_ELEMENTS => {
                count += 1;
                let el = El {
                    name: local(e.name().as_ref()),
                    attrs: e.attributes().flatten().map(|a| (local(a.key.as_ref()), unescape(&String::from_utf8_lossy(&a.value)))).collect(),
                    ..Default::default()
                };
                stack.push(el);
                if empty {
                    close(&mut stack);
                }
            }
            X::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().unwrap_or_default());
                }
            }
            X::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().unwrap_or_default());
                }
            }
            X::GeneralRef(g) => {
                let name = g.decode().unwrap_or_default().to_string();
                let c = match name.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    n => n
                        .strip_prefix("#x")
                        .or_else(|| n.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                if let (Some(c), Some(top)) = (c, stack.last_mut()) {
                    top.text.push(c);
                }
            }
            X::End(_) => close(&mut stack),
            X::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

/// Text for XML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\" \
xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
<soap:Header><t:RequestServerVersion Version=\"Exchange2010_SP2\"/></soap:Header>\
<soap:Body>{body}</soap:Body></soap:Envelope>"
    )
}

/// Base64 without line breaks.
fn b64(data: &[u8]) -> String {
    crate::imap::base64(data)
}

/// Base64 text to bytes (whitespace skipped); `None` when it isn't base64.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap_or(0));
        }
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    /// Not known yet: the first answer says.
    Unknown,
    Basic,
    /// NTLM, under the name the server offered (`NTLM` or `Negotiate`).
    Ntlm(&'static str),
}

/// An EWS session: the address, who signs in, and the connection (kept open for NTLM).
pub struct Client {
    url: String,
    user: String,
    password: String,
    conn: RefCell<Option<Conn>>,
    scheme: RefCell<Scheme>,
    /// The connection is signed in (NTLM).
    signed_in: RefCell<bool>,
    /// The account's own address (automatic replies are asked for by it).
    email: String,
}

/// What a `…ResponseMessage` said.
fn check_message(m: &El) -> Result<()> {
    match m.attr("ResponseClass") {
        Some("Error") => {
            let code = m.text_of("ResponseCode").unwrap_or("");
            let text = m.text_of("MessageText").unwrap_or(code);
            Err(match code {
                "ErrorItemNotFound" | "ErrorFolderNotFound" => NetError::No(format!("not on the server any more ({text})")),
                "ErrorAccessDenied" => NetError::No(text.to_string()),
                _ => NetError::No(text.to_string()),
            })
        }
        _ => Ok(()),
    }
}

/// The `…ResponseMessage` elements of an answer.
pub(crate) fn messages(doc: &El) -> Vec<&El> {
    let mut out = Vec::new();
    if let Some(body) = doc.find("Body") {
        let mut all = Vec::new();
        body.all("ResponseMessages", &mut all);
        for rm in all {
            out.extend(rm.kids.iter());
        }
    }
    out
}

impl Client {
    /// A session for `url` (checked with the first request).
    pub fn new(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        let u = http::Url::parse(url)?;
        let local = matches!(u.host.as_str(), "127.0.0.1" | "localhost" | "::1");
        if !u.https && !local {
            return Err(NetError::Protocol("Exchange is reached over https only".into()));
        }
        Ok(Client {
            url: url.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            conn: RefCell::new(None),
            scheme: RefCell::new(Scheme::Unknown),
            signed_in: RefCell::new(false),
            email: email.to_string(),
        })
    }

    /// Connect and check the password (asks for the Inbox).
    pub fn connect(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        let c = Client::new(url, user, password, email)?;
        c.call("<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>")?;
        Ok(c)
    }

    fn send_on_conn(&self, req: &Request) -> Result<Response> {
        let mut slot = self.conn.borrow_mut();
        if slot.as_ref().is_none_or(|c| !c.usable()) {
            *slot = Some(Conn::open(&self.url)?);
            *self.signed_in.borrow_mut() = false;
        }
        let Some(conn) = slot.as_mut() else { return Err(NetError::Closed("no connection".into())) };
        let r = conn.send(req);
        if r.is_err() {
            *slot = None;
        }
        r
    }

    /// The `WWW-Authenticate` schemes of a 401.
    fn offered(resp: &Response) -> Vec<String> {
        resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")).map(|(_, v)| v.trim().to_string()).collect()
    }

    /// Send a request signed in the way the server wants.
    pub(crate) fn post(&self, body: Vec<u8>) -> Result<Response> {
        let base = Request::post(&self.url, "text/xml; charset=utf-8", body, MAX_ANSWER).header("Accept", "text/xml");
        for attempt in 0..3 {
            let scheme = *self.scheme.borrow();
            let resp = match scheme {
                Scheme::Basic => {
                    let token = b64(format!("{}:{}", self.user, self.password).as_bytes());
                    self.send_on_conn(&base.clone().header("Authorization", &format!("Basic {token}")))?
                }
                Scheme::Ntlm(name) if !*self.signed_in.borrow() => {
                    let first = self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", b64(&crate::ntlm::negotiate()))))?;
                    if first.status != 401 {
                        first
                    } else {
                        let token = Client::offered(&first)
                            .iter()
                            .find_map(|v| v.strip_prefix(name).map(|t| t.trim().to_string()))
                            .filter(|t| !t.is_empty())
                            .ok_or_else(|| NetError::Auth("the server didn't answer the sign-in".into()))?;
                        let challenge = crate::ntlm::challenge(&unb64(&token).ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))?)?;
                        let mut client_challenge = [0u8; 8];
                        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut client_challenge)
                            .map_err(|_| NetError::Protocol("no randomness".into()))?;
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        let filetime = (now + 11_644_473_600) * 10_000_000;
                        let answer = crate::ntlm::authenticate(&challenge, &self.user, &self.password, filetime, client_challenge);
                        let resp = self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", b64(&answer))))?;
                        if resp.status != 401 {
                            *self.signed_in.borrow_mut() = true;
                        }
                        resp
                    }
                }
                Scheme::Ntlm(_) | Scheme::Unknown => self.send_on_conn(&base)?,
            };
            if resp.status != 401 {
                return Ok(resp);
            }
            // Not signed in: learn how, once.
            let offered = Client::offered(&resp);
            let starts = |p: &str| offered.iter().any(|v| v.len() >= p.len() && v.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
            let next = if starts("NTLM") {
                Scheme::Ntlm("NTLM")
            } else if starts("Negotiate") {
                Scheme::Ntlm("Negotiate")
            } else if starts("Basic") {
                Scheme::Basic
            } else {
                return Err(NetError::Auth("the server offers no sign-in MailCraft knows".into()));
            };
            if scheme == next || attempt == 2 {
                return Err(NetError::Auth("the server refused the user name or password".into()));
            }
            *self.scheme.borrow_mut() = next;
            *self.signed_in.borrow_mut() = false;
        }
        Err(NetError::Auth("the server refused the user name or password".into()))
    }

    /// Run one operation; the answer's elements (a SOAP fault is an error).
    pub fn call(&self, body: &str) -> Result<El> {
        let resp = self.post(envelope(body).into_bytes())?;
        let text = String::from_utf8_lossy(&resp.body);
        let doc = parse(&text);
        if let Some(fault) = doc.find("Fault") {
            let why = fault.find("faultstring").map(|f| f.text.clone()).unwrap_or_else(|| "the server refused the request".into());
            return Err(NetError::Protocol(why));
        }
        if !resp.ok() {
            return Err(NetError::Protocol(format!("Exchange answered HTTP {}", resp.status)));
        }
        Ok(doc)
    }

    /// Run an operation on one thing: its single response message, checked.
    pub(crate) fn call_one(&self, body: &str) -> Result<El> {
        let doc = self.call(body)?;
        let m = messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        check_message(&m)?;
        Ok(m)
    }

    /// Run an operation on many things: every response message checked.
    pub(crate) fn call_all(&self, body: &str) -> Result<()> {
        let doc = self.call(body)?;
        for m in messages(&doc) {
            check_message(m)?;
        }
        Ok(())
    }

    /// The account's mail folders, with their roles and counts.
    pub fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let roles = [
            ("inbox", FolderRole::Inbox),
            ("drafts", FolderRole::Drafts),
            ("sentitems", FolderRole::Sent),
            ("deleteditems", FolderRole::Trash),
            ("junkemail", FolderRole::Junk),
            ("archive", FolderRole::Archive),
        ];
        let hidden = ["msgfolderroot", "outbox", "conversationhistory", "syncissues"];
        let ids: String = roles.iter().map(|(n, _)| *n).chain(hidden).map(|n| format!("<t:DistinguishedFolderId Id=\"{n}\"/>")).collect();
        let doc = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{ids}</m:FolderIds></m:GetFolder>"
        ))?;
        // One answer per asked-for folder, in order; missing ones are errors.
        let found: Vec<Option<String>> = messages(&doc)
            .iter()
            .map(|m| if m.attr("ResponseClass") == Some("Error") { None } else { m.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) })
            .collect();
        let role_of: HashMap<String, FolderRole> = roles.iter().zip(&found).filter_map(|((_, r), id)| Some((id.clone()?, *r))).collect();
        let root = found.get(roles.len()).cloned().flatten();
        let skip: Vec<String> = found.iter().skip(roles.len() + 1).flatten().cloned().collect();

        let mut out: Vec<Mailbox> = Vec::new();
        let mut offset = 0;
        loop {
            let doc = self.call(&format!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:ParentFolderId\"/>\
                 <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/>\
                 <t:FieldURI FieldURI=\"folder:FolderClass\"/></t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>"
            ))?;
            let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { break };
            check_message(&m)?;
            let Some(rf) = m.child("RootFolder") else { break };
            let mut folders = Vec::new();
            rf.all("Folder", &mut folders);
            let n = folders.len();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                let class = f.text_of("FolderClass").unwrap_or("");
                if !(class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")) || skip.contains(&id) || out.len() >= MAX_FOLDERS {
                    continue;
                }
                let parent = f.child("ParentFolderId").and_then(|p| p.attr("Id")).map(str::to_string).filter(|p| Some(p) != root.as_ref());
                let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                out.push(Mailbox {
                    name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                    parent,
                    role: role_of.get(&id).copied().unwrap_or(FolderRole::Other),
                    total: count("TotalCount"),
                    unread: count("UnreadCount"),
                    id,
                });
            }
            let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || out.len() >= MAX_FOLDERS {
                break;
            }
        }
        // Folders under a hidden one go too.
        let ids: Vec<String> = out.iter().map(|m| m.id.clone()).collect();
        out.retain(|m| m.parent.as_ref().is_none_or(|p| ids.contains(p)));
        // Exchange's public folders, under a folder of their own (a server without them, or
        // one that won't show them, just has none).
        match self.public_folders() {
            Ok(pf) => out.extend(pf),
            Err(e) => log::info!("no public folders: {e}"),
        }
        Ok(out)
    }

    /// Exchange's public folders (the organization's shared folders): their mail folders, under
    /// a "Public Folders" folder. Public folders only list one level at a time, so the tree is
    /// walked, a few levels deep and up to a few hundred folders.
    fn public_folders(&self) -> Result<Vec<Mailbox>> {
        let root = self.call_one(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"publicfoldersroot\"/></m:FolderIds></m:GetFolder>",
        )?;
        let Some(root_id) = root.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::from([(root_id.clone(), 0)]);
        while let Some((parent, depth)) = queue.pop_front() {
            if depth >= PUBLIC_DEPTH || out.len() >= MAX_PUBLIC {
                break;
            }
            let m = self.call_one(&format!(
                "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:TotalCount\"/>\
                 <t:FieldURI FieldURI=\"folder:UnreadCount\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/>\
                 <t:FieldURI FieldURI=\"folder:ChildFolderCount\"/></t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"200\" Offset=\"0\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindFolder>",
                esc(&parent)
            ))?;
            let mut folders = Vec::new();
            if let Some(rf) = m.child("RootFolder") {
                rf.all("Folder", &mut folders);
            }
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                let class = f.text_of("FolderClass").unwrap_or("");
                let mail = class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.");
                // Calendars and contacts kept as public folders aren't mail; a folder holding
                // others is kept for them.
                if !mail && count("ChildFolderCount") == 0 {
                    continue;
                }
                out.push(Mailbox {
                    name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                    parent: Some(parent.clone()),
                    role: FolderRole::Other,
                    total: count("TotalCount"),
                    unread: count("UnreadCount"),
                    id: id.clone(),
                });
                if count("ChildFolderCount") > 0 {
                    queue.push_back((id, depth + 1));
                }
                if out.len() >= MAX_PUBLIC {
                    break;
                }
            }
        }
        if out.is_empty() {
            return Ok(out);
        }
        out.insert(0, Mailbox { id: root_id, name: PUBLIC_FOLDERS.into(), parent: None, role: FolderRole::Other, total: 0, unread: 0 });
        Ok(out)
    }

    /// A folder's (total, unread).
    fn counts(&self, folder: &str) -> Result<(u32, u32)> {
        let m = self.call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"folder:TotalCount\"/><t:FieldURI FieldURI=\"folder:UnreadCount\"/></t:AdditionalProperties></m:FolderShape>\
             <m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:GetFolder>",
            esc(folder)
        ))?;
        let f = m.find("Folder");
        let count = |k: &str| f.and_then(|f| f.text_of(k)).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
        Ok((count("TotalCount"), count("UnreadCount")))
    }

    /// The newest `limit` messages of a folder with their flags.
    fn newest(&self, folder: &str, limit: usize) -> Result<Vec<(String, Flags)>> {
        let m = self.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"0\" BasePoint=\"Beginning\"/>\
             <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
            limit.clamp(1, 1000),
            esc(folder)
        ))?;
        let Some(items) = m.find("Items") else { return Ok(Vec::new()) };
        Ok(items.kids.iter().filter_map(|it| Some((it.child("ItemId")?.attr("Id")?.to_string(), flags(it)))).collect())
    }

    /// Messages' details, for the list.
    fn headers(&self, ids: &[String]) -> Result<Vec<El>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
            let doc = self.call(&format!(
                "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{HEADER_FIELDS}{FLAG_FIELDS}</t:AdditionalProperties></m:ItemShape>\
                 <m:ItemIds>{list}</m:ItemIds></m:GetItem>"
            ))?;
            for m in messages(&doc) {
                if m.attr("ResponseClass") == Some("Error") {
                    continue;
                }
                if let Some(items) = m.child("Items") {
                    out.extend(items.kids.iter().cloned());
                }
            }
        }
        Ok(out)
    }

    /// A message's MIME source.
    pub fn download(&self, id: &str) -> Result<Vec<u8>> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let mime = m.find("MimeContent").ok_or_else(|| NetError::Protocol("the message has no source".into()))?;
        unb64(&mime.text).ok_or_else(|| NetError::Protocol("a damaged message source".into()))
    }

    /// Store a MIME message in a folder (as a draft when it's one); its id.
    pub fn create(&self, raw: &[u8], folder: &str, draft: bool) -> Result<String> {
        let flags = if draft {
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/><t:Value>8</t:Value></t:ExtendedProperty>"
        } else {
            ""
        };
        let m = self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent>{flags}</t:Message></m:Items></m:CreateItem>",
            esc(folder),
            b64(raw)
        ))?;
        Ok(m.find("ItemId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string())
    }

    /// Send a MIME message, keeping a copy in Sent Items.
    pub fn send_mime(&self, raw: &[u8]) -> Result<()> {
        self.call_one(&format!(
            "<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId>\
             <m:Items><t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent></t:Message></m:Items></m:CreateItem>",
            b64(raw)
        ))
        .map(drop)
    }

    /// The automatic replies.
    pub fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        let doc = self.call(&format!(
            "<m:GetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox></m:GetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        if let Some(rm) = doc.find("ResponseMessage") {
            check_message(rm)?;
        }
        let s = doc.find("OofSettings").ok_or_else(|| NetError::Protocol("no automatic reply settings".into()))?;
        let state = s.text_of("OofState").unwrap_or("Disabled");
        let day = |name: &str, end: bool| -> Option<String> {
            let t = s.find(name)?.text.clone();
            let secs = jmap::utc_date(&t)?;
            // The end is the moment after the last day.
            let secs = if end { secs - 1 } else { secs };
            let (y, m, d) = mailcraft_model::dates::civil_from_days(secs.div_euclid(86_400));
            Some(format!("{y:04}-{m:02}-{d:02}"))
        };
        let scheduled = state == "Scheduled";
        let message = s.find("InternalReply").and_then(|r| r.child("Message")).map(|m| m.text.clone()).unwrap_or_default();
        let text = if message.contains('<') { mime::html_to_text(&message) } else { message };
        Ok(mailcraft_model::job::Vacation {
            enabled: state != "Disabled",
            subject: String::new(),
            text: text.trim().to_string(),
            from: if scheduled { day("StartTime", false) } else { None },
            until: if scheduled { day("EndTime", true) } else { None },
        })
    }

    pub fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        let at = |day: &Option<String>, end: bool| -> Option<String> {
            let d = day.as_deref().filter(|d| mailcraft_model::job::is_day(d))?;
            Some(if end { format!("{d}T23:59:59Z") } else { format!("{d}T00:00:00Z") })
        };
        let (start, end) = (at(&v.from, false), at(&v.until, true));
        let state = match (v.enabled, start.is_some() || end.is_some()) {
            (false, _) => "Disabled",
            (true, true) => "Scheduled",
            (true, false) => "Enabled",
        };
        let duration = format!(
            "<t:Duration><t:StartTime>{}</t:StartTime><t:EndTime>{}</t:EndTime></t:Duration>",
            start.unwrap_or_else(|| "2000-01-01T00:00:00Z".into()),
            end.unwrap_or_else(|| "2100-01-01T00:00:00Z".into())
        );
        let text = esc(&v.text).replace('\n', "&lt;br&gt;");
        let doc = self.call(&format!(
            "<m:SetUserOofSettingsRequest><t:Mailbox><t:Address>{}</t:Address></t:Mailbox><t:UserOofSettings>\
             <t:OofState>{state}</t:OofState><t:ExternalAudience>All</t:ExternalAudience>{duration}\
             <t:InternalReply><t:Message>{text}</t:Message></t:InternalReply><t:ExternalReply><t:Message>{text}</t:Message></t:ExternalReply>\
             </t:UserOofSettings></m:SetUserOofSettingsRequest>",
            esc(&self.email)
        ))?;
        match doc.find("ResponseMessage") {
            Some(rm) => check_message(rm),
            None => Ok(()),
        }
    }
}

/// Where Autodiscover is asked (POX, MS-OXDSCLI): the domain's `autodiscover.` host, then the
/// domain itself.
fn autodiscover_urls(email: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    if domain.is_empty() || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Vec::new();
    }
    vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")]
}

/// The EWS address in an Autodiscover answer (the outside one first), or the address to ask
/// about instead (a redirect).
fn autodiscover_answer(xml: &str) -> (Option<String>, Option<String>) {
    let doc = parse(xml);
    let Some(account) = doc.find("Account") else { return (None, None) };
    if account.text_of("Action") == Some("redirectAddr") {
        return (None, account.text_of("RedirectAddr").map(|r| r.trim().to_string()));
    }
    let protocols: Vec<&El> = account.children("Protocol").collect();
    let url = ["EXPR", "EXCH", "WEB"]
        .iter()
        .find_map(|kind| protocols.iter().find(|p| p.text_of("Type") == Some(kind)).and_then(|p| p.text_of("EwsUrl")))
        .map(|u| u.trim().to_string())
        .filter(|u| u.starts_with("https://") || (cfg!(test) && u.starts_with("http://127.0.0.1")));
    (url, None)
}

/// Find a mailbox's EWS address with Autodiscover, signed in as the account. `base`: where to
/// ask instead of the usual addresses (tests).
pub fn autodiscover(email: &str, user: &str, password: &str, base: Option<&str>) -> Result<String> {
    let mut address = email.to_string();
    for _ in 0..3 {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
             <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
            esc(&address)
        );
        let urls = base.map_or_else(|| autodiscover_urls(&address), |b| vec![b.to_string()]);
        let mut redirect = None;
        let mut last = NetError::No("Autodiscover found no Exchange server for this address".into());
        for url in urls {
            let c = match Client::new(&url, user, password, email) {
                Ok(c) => c,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => match autodiscover_answer(&String::from_utf8_lossy(&resp.body)) {
                    (Some(url), _) => return Ok(url),
                    (None, Some(r)) => {
                        redirect = Some(r);
                        break;
                    }
                    _ => {}
                },
                Ok(resp) => last = NetError::Protocol(format!("Autodiscover answered HTTP {}", resp.status)),
                Err(NetError::Auth(m)) => return Err(NetError::Auth(m)),
                Err(e) => last = e,
            }
        }
        match redirect {
            Some(r) if r != address && r.contains('@') => address = r,
            _ => return Err(last),
        }
    }
    Err(NetError::No("Autodiscover sent MailCraft round in circles".into()))
}

/// The read, flag and draft state, asked for with every item.
const FLAG_FIELDS: &str = "<t:FieldURI FieldURI=\"message:IsRead\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/>";
const HEADER_FIELDS: &str = "<t:FieldURI FieldURI=\"item:Subject\"/><t:FieldURI FieldURI=\"message:From\"/>\
<t:FieldURI FieldURI=\"message:ToRecipients\"/><t:FieldURI FieldURI=\"message:CcRecipients\"/>\
<t:FieldURI FieldURI=\"message:ReplyTo\"/><t:FieldURI FieldURI=\"item:DateTimeReceived\"/>\
<t:FieldURI FieldURI=\"item:DateTimeSent\"/><t:FieldURI FieldURI=\"message:InternetMessageId\"/>\
<t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/>";

/// An extended property's value.
fn extended(it: &El, tag: &str) -> Option<i64> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse().ok())
}

fn flags(it: &El) -> Flags {
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: extended(it, "0x1090") == Some(2),
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        ..Default::default()
    }
}

/// A mailbox element as an address (an Exchange-internal address keeps the name only).
fn address(m: &El) -> Option<String> {
    let name = m.text_of("Name").unwrap_or("").replace('"', "");
    let email = m.text_of("EmailAddress").filter(|_| m.text_of("RoutingType").is_none_or(|r| r.eq_ignore_ascii_case("SMTP"))).unwrap_or("");
    if email.contains('@') {
        Some(if name.is_empty() { email.to_string() } else { format!("\"{name}\" <{email}>") })
    } else if name.is_empty() {
        None
    } else {
        Some(format!("\"{name}\" <>"))
    }
}

fn addresses(it: &El, name: &str) -> String {
    it.child(name).map(|l| l.children("Mailbox").filter_map(address).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// A header block for the list, from an item's details (its date is the received one).
fn header_block(it: &El) -> Vec<u8> {
    let mut h = String::new();
    let clean = |s: &str| s.replace(['\r', '\n'], " ");
    if let Some(f) = it.child("From").and_then(|f| f.child("Mailbox")).and_then(address) {
        h.push_str(&format!("From: {}\r\n", clean(&f)));
    }
    for (field, name) in [("ToRecipients", "To"), ("CcRecipients", "Cc"), ("ReplyTo", "Reply-To")] {
        let list = addresses(it, field);
        if !list.is_empty() {
            h.push_str(&format!("{name}: {}\r\n", clean(&list)));
        }
    }
    if let Some(s) = it.text_of("Subject") {
        h.push_str(&format!("Subject: {}\r\n", mime_word(&clean(s))));
    }
    if let Some(id) = it.text_of("InternetMessageId") {
        h.push_str(&format!("Message-ID: {}\r\n", clean(id)));
    }
    h.push_str("\r\n");
    h.into_bytes()
}

/// A subject as an RFC 2047 word when it isn't ASCII.
fn mime_word(s: &str) -> String {
    if s.is_ascii() { s.to_string() } else { format!("=?UTF-8?B?{}?=", b64(s.as_bytes())) }
}

/// Refresh the folder list (and the folder ids behind it).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    let found = jmap::folders(account, &c.mailboxes()?);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    Ok(found.into_iter().map(|(f, _)| f).collect())
}

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    let newest = c.newest(&id, opts.window)?;
    let (total, unread) = c.counts(&id)?;
    let mut fs = store.load_folder(account, folder)?;
    let mut report = SyncReport::default();
    if fs.validity != VALIDITY && (fs.validity != 0 || !fs.messages.is_empty()) {
        store.clear_folder(account, folder)?;
        fs = FolderState::default();
        report.reset = true;
    }
    fs.validity = VALIDITY;
    let wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    let (kept, gone): (Vec<Summary>, Vec<Summary>) = std::mem::take(&mut fs.messages).into_iter().partition(|m| wanted.contains_key(&m.key.uid));
    for m in &gone {
        store.remove_body(&m.key);
    }
    report.removed = gone.len();
    fs.messages = kept;
    for m in &mut fs.messages {
        if let Some(f) = wanted.get(&m.key.uid) {
            let mut next = m.flags.clone();
            next.seen = f.seen;
            next.flagged = f.flagged;
            next.draft = f.draft;
            if next != m.flags {
                m.flags = next;
                report.changed += 1;
            }
        }
    }
    let missing: Vec<String> = newest.iter().filter(|(id, _)| fs.find(st.number(id)).is_none()).map(|(id, _)| id.clone()).collect();
    for it in c.headers(&missing)? {
        let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_some() {
            continue;
        }
        let received = it.text_of("DateTimeReceived").and_then(jmap::utc_date).unwrap_or(0);
        let size = it.text_of("Size").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
        let mut s = mime::summary_from_header(&header_block(&it), key, flags(&it), size, received);
        s.has_attachments = it.text_of("HasAttachments").is_some_and(|v| v == "true");
        fs.messages.push(s);
        report.new += 1;
    }
    sort_newest_first(&mut fs.messages);
    fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
    store.save_folder(account, folder, &fs)?;
    prefetch(c, st, store, account, folder, &mut fs, opts)?;
    report.total = total;
    report.unread = unread;
    Ok(report)
}

fn prefetch(c: &Client, st: &State, store: &Store, account: &str, folder: &str, fs: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let wanted: Vec<(u32, String)> = fs
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| u64::from(m.size) <= u64::from(opts.prefetch_max_bytes) && !store.has_body(&m.key))
        .filter_map(|m| Some((m.key.uid, st.id(m.key.uid)?.to_string())))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    for (n, id) in &wanted {
        let raw = c.download(id)?;
        let Some(m) = fs.find_mut(*n) else { continue };
        store.write_body(&m.key, &raw)?;
        fill_from_source(m, &raw);
    }
    store.save_folder(account, folder, fs)?;
    Ok(())
}

/// Download one message's source (and cache it).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    let raw = c.download(id)?;
    store.write_body(key, &raw)?;
    let mut fs = store.load_folder(&key.account, &key.folder)?;
    if let Some(m) = fs.find_mut(key.uid) {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &fs)?;
    }
    Ok(raw)
}

fn item_ids(ids: &[String]) -> String {
    ids.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect()
}

fn folder_ref(id: Option<&str>) -> String {
    match id {
        Some(id) => format!("<t:FolderId Id=\"{}\"/>", esc(id)),
        None => "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".into(),
    }
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
    };
    let update = |keys: &[MessageKey], change: &str| -> Result<()> {
        for chunk in ids(keys).chunks(BATCH) {
            let changes: String = chunk
                .iter()
                .map(|(id, _)| format!("<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{change}</t:Updates></t:ItemChange>", esc(id)))
                .collect();
            c.call_all(&format!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
            ))?;
        }
        Ok(())
    };
    match op {
        Op::SetFlag { keys, flag: Flag::Seen, on } => update(
            keys,
            &format!("<t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/><t:Message><t:IsRead>{on}</t:IsRead></t:Message></t:SetItemField>"),
        ),
        Op::SetFlag { keys, flag: Flag::Flagged, on: true } => update(
            keys,
            "<t:SetItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Message><t:ExtendedProperty>\
             <t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
        ),
        Op::SetFlag { keys, flag: Flag::Flagged, on: false } => {
            update(keys, "<t:DeleteItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/></t:DeleteItemField>")
        }
        // Answered and forwarded are Exchange's to set; categories stay on this computer.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let moving: Vec<String> = ids(keys).into_iter().filter(|(_, from)| from != to).map(|(id, _)| id).collect();
            for chunk in moving.chunks(BATCH) {
                c.call_all(&format!(
                    "<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:MoveItem>",
                    folder_ref(Some(&dest)),
                    item_ids(chunk)
                ))?;
            }
            Ok(())
        }
        Op::Expunge { keys } => {
            let gone: Vec<String> = ids(keys).into_iter().map(|(id, _)| id).collect();
            for chunk in gone.chunks(BATCH) {
                c.call_all(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>", item_ids(chunk)))?;
            }
            Ok(())
        }
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            c.call_one(&format!(
                "<m:CreateFolder><m:ParentFolderId>{}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass>\
                 <t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
                folder_ref(parent.as_deref()),
                esc(&name)
            ))
            .map(drop)
        }
        Op::RenameFolder { from, to } => {
            let mut id = st.mailbox(from)?.to_string();
            let (old_parent, _) = split_path(st, from)?;
            let (parent, name) = split_path(st, to)?;
            if parent != old_parent {
                let m = c.call_one(&format!(
                    "<m:MoveFolder><m:ToFolderId>{}</m:ToFolderId><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:MoveFolder>",
                    folder_ref(parent.as_deref()),
                    esc(&id)
                ))?;
                if let Some(new) = m.find("FolderId").and_then(|f| f.attr("Id")) {
                    id = new.to_string();
                }
            }
            c.call_one(&format!(
                "<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{}\"/><t:Updates><t:SetFolderField>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></t:SetFolderField>\
                 </t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>",
                esc(&id),
                esc(&name)
            ))
            .map(drop)
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
                esc(&id)
            ))
            .map(drop)
        }
        Op::EmptyFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:EmptyFolder DeleteType=\"HardDelete\" DeleteSubFolders=\"false\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:EmptyFolder>",
                esc(&id)
            ))
            .map(drop)
        }
    }
}

/// The parent folder id (`None` at the top) and the leaf name of a folder path.
fn split_path(st: &State, path: &str) -> Result<(Option<String>, String)> {
    match path.rsplit_once('/') {
        Some((parent, leaf)) => Ok((Some(st.mailbox(parent)?.to_string()), utf7::decode(leaf))),
        None => Ok((None, utf7::decode(path))),
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.create(raw, st.mailbox(folder)?, true)
}

/// Send a message. Exchange reads the recipients from the MIME source, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    let shown: Vec<String> = mime::parse(raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect()).unwrap_or_default();
    let bcc: Vec<&str> = recipients.iter().map(String::as_str).filter(|r| !shown.contains(&r.to_ascii_lowercase())).collect();
    if bcc.is_empty() {
        return c.send_mime(raw);
    }
    let mut with = format!("Bcc: {}\r\n", bcc.join(", ")).into_bytes();
    with.extend_from_slice(raw);
    c.send_mime(&with)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_carefully() {
        let doc = parse(
            "<s:Envelope xmlns:s=\"x\"><s:Body><m:GetItemResponse><m:ResponseMessages>\
             <m:GetItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Items>\
             <t:Message><t:ItemId Id=\"A&amp;1\" ChangeKey=\"c\"/><t:Subject>Caf&#233; &lt;plan&gt;</t:Subject><t:IsRead>true</t:IsRead>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>\
             <t:From><t:Mailbox><t:Name>Zoe \"Z\" Park</t:Name><t:EmailAddress>zoe@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:From>\
             <t:ToRecipients><t:Mailbox><t:Name>Me</t:Name><t:EmailAddress>/o=Org/cn=me</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:ToRecipients>\
             <t:DateTimeSent>2026-10-15T09:00:00Z</t:DateTimeSent></t:Message></m:Items></m:GetItemResponseMessage>\
             <m:GetItemResponseMessage ResponseClass=\"Error\"><m:MessageText>The specified object was not found.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:GetItemResponseMessage>\
             </m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>",
        );
        let ms = messages(&doc);
        assert_eq!(ms.len(), 2);
        assert!(check_message(ms[0]).is_ok());
        assert!(matches!(check_message(ms[1]), Err(NetError::No(m)) if m.contains("not found")));
        let it = ms[0].find("Message").unwrap();
        assert_eq!(it.child("ItemId").unwrap().attr("Id"), Some("A&1"));
        assert_eq!(it.text_of("Subject"), Some("Café <plan>"));
        let f = flags(it);
        assert!(f.seen && f.flagged && !f.draft);
        let h = String::from_utf8(header_block(it)).unwrap();
        assert!(h.contains("From: \"Zoe Z Park\" <zoe@contoso.example>\r\n"), "{h}");
        assert!(h.contains("To: \"Me\" <>\r\n"), "an Exchange-internal address keeps the name: {h}");
        assert!(h.contains("Subject: =?UTF-8?B?"), "{h}");
        // Hostile: deep, broken, junk.
        let deep = "<a>".repeat(10_000);
        let _ = parse(&deep);
        let _ = parse("<a><b></a></b>&bogus;&#xFFFFFFFF;");
        assert_eq!(unb64("aGk=\r\n"), Some(b"hi".to_vec()));
        assert_eq!(unb64("**"), None);
        assert_eq!(esc("a<b&\"c\u{1}"), "a&lt;b&amp;&quot;c");
    }

    #[test]
    fn autodiscover_answers() {
        let ok = "<Autodiscover><Response><Account><AccountType>email</AccountType><Action>settings</Action>\
                  <Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.local/EWS/Exchange.asmx</EwsUrl></Protocol>\
                  <Protocol><Type>EXPR</Type><EwsUrl>https://mail.contoso.example/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>";
        assert_eq!(autodiscover_answer(ok).0.as_deref(), Some("https://mail.contoso.example/EWS/Exchange.asmx"), "the outside address first");
        let redirect = "<Autodiscover><Response><Account><Action>redirectAddr</Action><RedirectAddr>riley@corp.example</RedirectAddr></Account></Response></Autodiscover>";
        assert_eq!(autodiscover_answer(redirect).1.as_deref(), Some("riley@corp.example"));
        assert_eq!(autodiscover_answer("<x/>"), (None, None));
        let plain = "<Autodiscover><Response><Account><Protocol><Type>EXPR</Type><EwsUrl>http://mail.contoso.example/EWS</EwsUrl></Protocol></Account></Response></Autodiscover>";
        assert_eq!(autodiscover_answer(plain).0, None, "never plain http");
        assert_eq!(autodiscover_urls("riley@Contoso.example")[0], "https://autodiscover.contoso.example/autodiscover/autodiscover.xml");
        assert!(autodiscover_urls("riley@bad/host").is_empty());
    }

    #[test]
    fn only_https_unless_local() {
        assert!(Client::new("http://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_err());
        assert!(Client::new("https://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_ok());
        assert!(Client::new("http://127.0.0.1:9/EWS/Exchange.asmx", "u", "p", "e").is_ok());
    }
}
