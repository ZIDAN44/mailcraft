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
use crate::sync::{SyncOptions, SyncReport, WordQuery, fill_from_source};
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
    /// The folders in an answer, whatever their kind: Exchange gives each in an element of its
    /// own (`Folder`, `ContactsFolder`, `CalendarFolder`, `TasksFolder`, `SearchFolder`).
    pub fn folders(&self) -> Vec<&El> {
        fn walk<'a>(el: &'a El, out: &mut Vec<&'a El>) {
            for k in &el.kids {
                if FOLDER_KINDS.contains(&k.name.as_str()) {
                    out.push(k);
                } else {
                    walk(k, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }
}

/// The elements a folder comes in (EWS's array of folders).
const FOLDER_KINDS: [&str; 5] = ["Folder", "ContactsFolder", "CalendarFolder", "TasksFolder", "SearchFolder"];

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_string()
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
                    attrs: e.attributes().flatten().map(|a| (local(a.key.as_ref()), unescape(&a.value))).collect(),
                    ..Default::default()
                };
                stack.push(el);
                if empty {
                    close(&mut stack);
                }
            }
            X::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::GeneralRef(g) => {
                let name = g.to_string();
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

/// Messages kept of one folder at most, when all of them are asked for.
const MAX_KEPT: usize = 100_000;

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
    /// The public folder mailbox requests are routed to (`X-AnchorMailbox` and
    /// `X-PublicFolderMailbox`), while reading a public folder's content.
    route: RefCell<Option<String>>,
    /// Public folder mailboxes found by Autodiscover: GUID → address.
    pf_mailboxes: RefCell<HashMap<String, String>>,
    /// The account's public folder hierarchy mailbox (Autodiscover's PublicFolderInformation),
    /// once asked: `None` inside when there is none.
    pf_hierarchy: RefCell<Option<Option<String>>>,
    /// The public folder mailboxes holding the public folders' content (their GUIDs, from the
    /// folders' PR_REPLICA_LIST): a request one of their folders or items isn't found by is
    /// tried there.
    pf_replicas: RefCell<Vec<String>>,
    /// The mailbox requests are routed to (`X-AnchorMailbox`) while working in a shared mailbox.
    anchor: RefCell<Option<String>>,
    /// Shared mailboxes added to the account by hand (their addresses).
    shared_added: RefCell<Vec<String>>,
    /// The shared mailboxes whose folders were listed: a request not found in this one is
    /// tried routed to them.
    shared_open: RefCell<Vec<String>>,
    /// The account's Autodiscover answer, once asked (`None` inside when there was none).
    discovered: RefCell<Option<Option<String>>>,
    /// The folders of the shared mailboxes listed: the mailbox each is in (its address) and the
    /// role it has there (`Other` for its top folder and the folders that aren't special).
    shared_roles: RefCell<HashMap<String, (String, FolderRole)>>,
    /// The User-Agent every request names, instead of MailCraft's (issue #9: a server that lets
    /// in only the clients it knows).
    user_agent: Option<String>,
}

/// Another mailbox the account opens (Outlook's shared mailboxes): its address and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedMailbox {
    pub address: String,
    pub name: String,
}

/// Most shared mailboxes an account opens.
const MAX_SHARED: usize = 20;

/// A mailbox's special folders: their distinguished ids and roles.
const SPECIAL: [(&str, FolderRole); 6] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
    ("archive", FolderRole::Archive),
];

/// A public folder in the tree.
pub(crate) struct PublicFolder {
    pub id: String,
    /// Its path under Public Folders.
    pub path: String,
    pub class: String,
    pub total: u32,
    /// The GUID of the public folder mailbox holding its content (`PR_REPLICA_LIST`).
    pub replica: Option<String>,
}

/// A folder found walking the public folders.
struct TreeFolder {
    mailbox: Mailbox,
    class: String,
    children: bool,
    replica: Option<String>,
}

/// `PR_REPLICA_LIST`, asked for with public folders.
const REPLICA_LIST: &str = "<t:ExtendedFieldURI PropertyTag=\"26264\" PropertyType=\"Binary\"/>";

/// The mailbox GUID in a `PR_REPLICA_LIST` value (base64 of the GUID as ASCII, NUL-ended).
fn replica_guid(value: &str) -> Option<String> {
    let bytes = unb64(value.trim())?;
    let guid = String::from_utf8(bytes.get(..36)?.to_vec()).ok()?;
    let ok = guid.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() });
    ok.then(|| guid.to_ascii_lowercase())
}

/// The address in an Autodiscover answer about a mailbox.
fn autodiscover_smtp(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("AutoDiscoverSMTPAddress").map(|a| a.text.trim().to_string()).filter(|a| a.contains('@') && a.len() < 320)
}

/// The public folder hierarchy mailbox in an Autodiscover answer about the account (Exchange
/// 2013 and later: `PublicFolderInformation`'s `SmtpAddress`).
fn autodiscover_public(xml: &str) -> Option<String> {
    let doc = parse(xml);
    doc.find("PublicFolderInformation")
        .and_then(|p| p.find("SmtpAddress"))
        .map(|a| a.text.trim().to_string())
        .filter(|a| a.contains('@') && a.len() < 320)
}

/// A mailbox Autodiscover maps to the account (Outlook opens these by itself).
struct Alternative {
    name: String,
    smtp: Option<String>,
    legacy_dn: Option<String>,
}

/// The mailboxes an Autodiscover answer maps to the account: `AlternativeMailbox` entries for
/// mailboxes it may open (`Delegate`, `TeamMailbox`), not its archive.
fn alternative_mailboxes(xml: &str) -> Vec<Alternative> {
    let doc = parse(xml);
    let mut alts = Vec::new();
    doc.all("AlternativeMailbox", &mut alts);
    alts.iter()
        .filter(|a| a.text_of("Type").is_some_and(|t| matches!(t.trim(), "Delegate" | "TeamMailbox")))
        .map(|a| {
            let text = |k: &str| a.text_of(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v.len() < 500);
            Alternative {
                name: text("DisplayName").unwrap_or_default(),
                smtp: text("SmtpAddress").filter(|v| v.contains('@')),
                legacy_dn: text("LegacyDN"),
            }
        })
        .take(MAX_SHARED)
        .collect()
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
            route: RefCell::new(None),
            pf_mailboxes: RefCell::new(HashMap::new()),
            pf_hierarchy: RefCell::new(None),
            pf_replicas: RefCell::new(Vec::new()),
            anchor: RefCell::new(None),
            shared_added: RefCell::new(Vec::new()),
            shared_open: RefCell::new(Vec::new()),
            discovered: RefCell::new(None),
            shared_roles: RefCell::new(HashMap::new()),
            user_agent: None,
        })
    }

    /// The same session naming itself `agent` to the server (`None`, or nothing usable: as
    /// MailCraft).
    #[must_use]
    pub fn with_agent(mut self, agent: Option<&str>) -> Client {
        self.user_agent = agent.and_then(mailcraft_model::user_agent);
        self
    }
}

/// Drop folders whose parent was dropped, repeating until every remaining parent is listed.
fn keep_rooted(out: &mut Vec<Mailbox>) {
    loop {
        let ids: std::collections::HashSet<String> = out.iter().map(|m| m.id.clone()).collect();
        let before = out.len();
        out.retain(|m| m.parent.as_ref().is_none_or(|p| ids.contains(p)));
        if out.len() == before {
            break;
        }
    }
}

/// The `PidTagAttributeHidden` property ([MS-OXPROPS]), asked for with each folder.
const HIDDEN: &str = "<t:ExtendedFieldURI PropertyTag=\"0x10f4\" PropertyType=\"Boolean\"/>";

/// Whether a listed folder carries that property set to true.
fn is_hidden(f: &El) -> bool {
    let tagged = |p: &&El| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).map(str::to_ascii_lowercase).as_deref() == Some("0x10f4");
    f.children("ExtendedProperty").filter(tagged).filter_map(|p| p.text_of("Value")).any(|v| matches!(v.trim(), "true" | "True" | "1"))
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
    let boxes = c.mailboxes()?;
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    jmap::name_special(&mut found, &boxes);
    // A shared mailbox's folders say whose they are, and what each is there.
    let shared = c.shared_roles.borrow();
    for (f, id) in &mut found {
        if let Some((address, role)) = shared.get(id.as_str()) {
            f.mailbox = Some(address.clone());
            f.mailbox_role = *role;
        }
    }
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
    let mut wanted: HashMap<u32, Flags> = newest.iter().map(|(id, f)| (st.number(id), f.clone())).collect();
    // Found by a search of the server past the window: kept while still in the folder.
    let extra = st.found_outside(folder, &fs.messages, &wanted);
    for (found, f) in c.still_in(&id, &extra)? {
        wanted.insert(st.number(&found), f);
    }
    st.keep_found(folder, |n| wanted.contains_key(&n));
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
        fs.messages.push(item_summary(&it, key));
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

/// An item's list entry from its details.
fn item_summary(it: &El, key: MessageKey) -> Summary {
    let received = it.text_of("DateTimeReceived").and_then(jmap::utc_date).unwrap_or(0);
    let size = it.text_of("Size").and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
    let mut s = mime::summary_from_header(&header_block(it), key, flags(it), size, received);
    s.has_attachments = it.text_of("HasAttachments").is_some_and(|v| v == "true");
    s
}

/// The messages of `folder` the server finds for a search (issue #4), the newest `cap` of them:
/// its words as a `QueryString` (AQS), or, with none, its read and flag conditions as a
/// `Restriction`. Those not kept yet come into its list and stay there past the window while the
/// folder has them. Their keys, and how many came in.
pub fn find_on_server(
    c: &Client,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    query: &WordQuery,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let id = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let ids = c.find(&id, query, cap.min(jmap::FOUND_MAX))?;
    let numbers: Vec<u32> = ids.iter().map(|i| st.number(i)).collect();
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| fs.find(**n).is_none()).map(|(i, _)| i.clone()).collect();
    let mut new = 0;
    for it in c.headers(&missing)? {
        let Some(i) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(i) };
        if fs.find(key.uid).is_none() {
            fs.messages.push(item_summary(&it, key));
            new += 1;
        }
    }
    st.add_found(folder, &numbers);
    if new > 0 {
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        store.save_folder(account, folder, &fs)?;
    }
    Ok((numbers.iter().filter_map(|n| fs.find(*n).map(|m| m.key.clone())).collect(), new))
}

/// A `Restriction` for a search's read, flag and attachment conditions.
fn restriction(q: &WordQuery) -> String {
    let equal = |field: &str, value: &str| {
        format!("<t:IsEqualTo>{field}<t:FieldURIOrConstant><t:Constant Value=\"{value}\"/></t:FieldURIOrConstant></t:IsEqualTo>")
    };
    let mut parts = Vec::new();
    if let Some(u) = q.unread {
        parts.push(equal("<t:FieldURI FieldURI=\"message:IsRead\"/>", if u { "false" } else { "true" }));
    }
    if let Some(on) = q.flagged {
        let e = equal("<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>", "2");
        parts.push(if on { e } else { format!("<t:Not>{e}</t:Not>") });
    }
    if let Some(a) = q.attachment {
        parts.push(equal("<t:FieldURI FieldURI=\"item:HasAttachments\"/>", if a { "true" } else { "false" }));
    }
    match parts.len() {
        0 => String::new(),
        1 => format!("<m:Restriction>{}</m:Restriction>", parts.concat()),
        _ => format!("<m:Restriction><t:And>{}</t:And></m:Restriction>", parts.concat()),
    }
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
    let mut failures = crate::sync::AheadFailures::new();
    for (n, id) in &wanted {
        let raw = match c.download(id) {
            Ok(raw) => {
                failures.ok();
                raw
            }
            Err(e) => {
                failures.failed(&format!("{account}/{folder}: message {n}"), e)?;
                continue;
            }
        };
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

mod directory;
mod discover;
pub use directory::DIRECTORY_MAX;
mod mailbox;
mod requests;
#[cfg(test)]
use discover::autodiscover_answer;
use discover::autodiscover_urls;
pub use discover::{autodiscover, autodiscover_as};
mod shared;

#[cfg(test)]
mod tests;
