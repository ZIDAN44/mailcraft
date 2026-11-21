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
use mailcraft_model::{Flag, Flags, Folder, FolderRole, MessageKey, Op, Summary, base64};
use mailcraft_store::Store;

use crate::http::{self, Conn, Request, Response};
use crate::jmap::{self, Mailbox, State, VALIDITY};
use crate::sync::{SyncOptions, SyncReport, WordQuery};
pub use crate::xml::{El, esc, parse};
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

impl El {
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

fn envelope(body: &str) -> String {
    envelope_as("Exchange2010_SP2", body)
}

/// The request under a server version of its own: operations Exchange 2013 brought (reminders)
/// are refused under the version the rest asks for.
fn envelope_as(version: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
<soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\" \
xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
<soap:Header><t:RequestServerVersion Version=\"{version}\"/></soap:Header>\
<soap:Body>{body}</soap:Body></soap:Envelope>"
    )
}

/// Messages kept of one folder at most, when all of them are asked for.
const MAX_KEPT: usize = 100_000;

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
    /// Kerberos in `Negotiate` (gap 5), for a server that offers nothing else.
    Kerberos,
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
    /// Its other addresses, and the name it signs in by when that's one: a meeting may name its
    /// owner by any of them (issue #144).
    also: Vec<String>,
    /// The public folder mailbox requests are routed to (`X-AnchorMailbox` and
    /// `X-PublicFolderMailbox`), while reading a public folder's content.
    route: RefCell<Option<String>>,
    /// Public folder mailboxes found by Autodiscover: GUID → address.
    pf_mailboxes: RefCell<HashMap<String, String>>,
    /// The account's public folder hierarchy mailbox (Autodiscover's PublicFolderInformation),
    /// once asked: `None` inside when there is none.
    pf_hierarchy: RefCell<Option<Option<String>>>,
    /// The public folders listed: the public folder mailbox holding each one's content (its
    /// GUID, from the folder's PR_REPLICA_LIST), where requests about it are routed.
    pf_folders: RefCell<HashMap<String, String>>,
    /// The mailbox requests are routed to (`X-AnchorMailbox`) while working in a shared mailbox.
    anchor: RefCell<Option<String>>,
    /// Shared mailboxes added to the account by hand (their addresses).
    shared_added: RefCell<Vec<String>>,
    /// The shared mailboxes whose folders were listed: requests about their calendars are
    /// routed to them.
    shared_open: RefCell<Vec<String>>,
    /// The account's Autodiscover answer, once asked (`None` inside when there was none).
    discovered: RefCell<Option<Option<String>>>,
    /// The folders of the shared mailboxes listed: the mailbox each is in (its address) and the
    /// role it has there (`Other` for its top folder and the folders that aren't special).
    shared_roles: RefCell<HashMap<String, (String, FolderRole)>>,
    /// The public folders the last folder listing walked (the root's id and the tree), and the
    /// shared mailboxes it found: the calendars and contacts read next take them ([`Listed`]).
    pf_listed: RefCell<Option<Listed<(String, Vec<TreeFolder>)>>>,
    shared_listed: RefCell<Option<Listed<Vec<SharedMailbox>>>>,
    /// The User-Agent every request names, instead of MailCraft's (issue #9: a server that lets
    /// in only the clients it knows).
    user_agent: Option<String>,
    /// Those of [`NEWER`] the server refused (an older Exchange's schema): not asked again.
    unknown_ids: RefCell<Vec<&'static str>>,
    /// The server refused a message's text first (issue #191): not asked again here, each
    /// message coming whole.
    text_first_refused: RefCell<bool>,
    /// The names the directory gives addresses asked of it ([`Client::directory_name_of`]),
    /// `None` for one it has none for: asked once on a connection.
    names: RefCell<HashMap<String, Option<String>>>,
}

/// Another mailbox the account opens (Outlook's shared mailboxes): its address and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedMailbox {
    pub address: String,
    pub name: String,
}

/// Most shared mailboxes an account opens.
const MAX_SHARED: usize = 20;
/// Most other addresses of the account's a session knows (as many as an account has, and the
/// name it signs in by).
const MAX_ADDRESSES: usize = mailcraft_model::addr::identity::MAX_IDENTITIES + 1;

/// A mailbox's special folders: their distinguished ids and roles. Its Archive folder is asked
/// for apart, as one of [`NEWER`].
const SPECIAL: [(&str, FolderRole); 5] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
];

/// Distinguished folders that Exchange 2013 and later brought: the Archive folder, and two kept
/// out of the folder list as Outlook keeps them. Exchange 2010's schema doesn't know them, and
/// refused a request naming any of them as a whole, the first sync with it (#149); so they're
/// asked for apart ([`Client::newer_ids`]).
const NEWER: [&str; 3] = ["archive", "conversationhistory", "syncissues"];

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

/// What a folder listing found that is costly to ask again (the public folder tree: a request
/// for each folder holding others; the shared mailboxes: one for each added by hand), kept for
/// the calendars and contacts read after it: taken by the first, while it's fresh. Read later,
/// or a second time, it's asked again, so a folder made on the server since still comes in.
struct Listed<T> {
    at: std::time::Instant,
    found: T,
}

/// How long what a listing found serves the calendars and contacts read after it: a sync reads
/// them right after its folders, a first sync of a big mailbox perhaps hours later.
const LISTED_FOR: std::time::Duration = std::time::Duration::from_secs(10 * 60);

impl<T> Listed<T> {
    fn now(found: T) -> Option<Listed<T>> {
        Some(Listed { at: std::time::Instant::now(), found })
    }

    /// What `kept` holds, taken out, while it's fresh.
    fn take(kept: &RefCell<Option<Listed<T>>>) -> Option<T> {
        kept.borrow_mut().take().filter(|l| l.at.elapsed() < LISTED_FOR).map(|l| l.found)
    }
}

/// A folder found walking the public folders.
struct TreeFolder {
    mailbox: Mailbox,
    class: String,
    children: bool,
    replica: Option<String>,
}

/// `PR_REPLICA_LIST`'s property tag, asked for with public folders.
const REPLICA_TAG: u32 = 0x6698;

/// `PR_REPLICA_LIST` as a property asked for (its tag in decimal, as it has always been sent).
fn replica_list() -> String {
    format!("<t:ExtendedFieldURI PropertyTag=\"{REPLICA_TAG}\" PropertyType=\"Binary\"/>")
}

/// Whether a property tag as an answer writes it (`0x6698`, or `26264` in decimal) is
/// `PR_REPLICA_LIST`'s.
fn is_replica_tag(tag: &str) -> bool {
    tag_number(tag) == Some(REPLICA_TAG)
}

/// A property tag as an answer writes it: `0x6698`, or `26264` in decimal.
fn tag_number(tag: &str) -> Option<u32> {
    let tag = tag.trim();
    match tag.get(..2).filter(|p| p.eq_ignore_ascii_case("0x")) {
        Some(_) => tag.get(2..).and_then(|h| u32::from_str_radix(h, 16).ok()),
        None => tag.parse::<u32>().ok(),
    }
}

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

/// How a refusal for something that isn't on the server begins (EWS's and Graph's).
pub(crate) const GONE: &str = "not on the server any more";

/// Whether a refusal says the thing isn't on the server any more ([`GONE`]): a delete or an
/// answer then has nothing left to do. Any other (access denied, the server busy) didn't happen.
pub(crate) fn gone(e: &NetError) -> bool {
    matches!(e, NetError::No(m) if m.starts_with(GONE))
}

/// The code of a refusal for want of rights, kept in what it says ([`check_message`]).
pub(crate) const DENIED: &str = "ErrorAccessDenied";

/// Whether a refusal was for want of rights ([`DENIED`]): a colleague's calendar shared for its
/// subject and place, not its items, is refused so (issue #140).
pub(crate) fn denied(e: &NetError) -> bool {
    matches!(e, NetError::No(m) if m.contains(DENIED))
}

/// How a refusal begins for an id the server can't read as one (EWS's and Graph's): malformed,
/// or the other transport's. It names nothing there, and never will.
pub(crate) const NO_SUCH_ID: &str = "not an id of this server's";
/// How both servers' codes for such an id begin (`ErrorInvalidIdMalformed`, `ErrorInvalidIdEmpty`
/// and the like).
pub(crate) const INVALID_ID: &str = "ErrorInvalidId";

/// Whether a refusal says the id can't be one of the server's ([`NO_SUCH_ID`]): asking again
/// would be refused the same, where any other refusal but [`gone`] (rights, the server busy)
/// may be taken later.
pub(crate) fn no_such_id(e: &NetError) -> bool {
    matches!(e, NetError::No(m) if m.starts_with(NO_SUCH_ID))
}

/// What a `…ResponseMessage` said.
pub(crate) fn check_message(m: &El) -> Result<()> {
    match m.attr("ResponseClass") {
        Some("Error") => {
            let code = m.text_of("ResponseCode").unwrap_or("");
            let text = m.text_of("MessageText").unwrap_or(code);
            Err(match code {
                "ErrorItemNotFound" | "ErrorFolderNotFound" => NetError::No(format!("{GONE} ({text})")),
                _ if code.starts_with(INVALID_ID) => NetError::No(format!("{NO_SUCH_ID} ({text})")),
                // Refused for want of rights: said as the server says it, its code kept ([`denied`]).
                DENIED if !text.contains(DENIED) => NetError::No(format!("{text} ({DENIED})")),
                // Any other (the server busy) is refused as it says.
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

/// The folder id each answer of a `GetFolder` gives, in the order they were asked for: `None`
/// for one the mailbox hasn't.
fn folder_ids(doc: &El) -> Vec<Option<String>> {
    messages(doc)
        .iter()
        .map(|m| if m.attr("ResponseClass") == Some("Error") { None } else { m.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) })
        .collect()
}

impl Client {
    /// A session for `url` (checked with the first request).
    pub fn new(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        let u = http::Url::parse(url)?;
        if !u.https && !http::is_loopback(&u.host) {
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
            also: Vec::new(),
            route: RefCell::new(None),
            pf_mailboxes: RefCell::new(HashMap::new()),
            pf_hierarchy: RefCell::new(None),
            pf_folders: RefCell::new(HashMap::new()),
            anchor: RefCell::new(None),
            shared_added: RefCell::new(Vec::new()),
            shared_open: RefCell::new(Vec::new()),
            discovered: RefCell::new(None),
            shared_roles: RefCell::new(HashMap::new()),
            pf_listed: RefCell::new(None),
            shared_listed: RefCell::new(None),
            user_agent: None,
            unknown_ids: RefCell::new(Vec::new()),
            text_first_refused: RefCell::new(false),
            names: RefCell::new(HashMap::new()),
        })
    }

    /// The account's own address.
    pub(crate) fn email(&self) -> &str {
        &self.email
    }

    /// Every address of the account's: its own, then those [`Client::with_addresses`] gave.
    pub(crate) fn own_addresses(&self) -> Vec<&str> {
        std::iter::once(self.email.as_str()).chain(self.also.iter().map(String::as_str)).collect()
    }

    /// The same session knowing the account's other `addresses` (what isn't one is left out).
    #[must_use]
    pub fn with_addresses<'a>(mut self, addresses: impl IntoIterator<Item = &'a str>) -> Client {
        self.also = addresses.into_iter().map(str::trim).filter(|a| a.contains('@')).take(MAX_ADDRESSES).map(str::to_string).collect();
        self
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

/// Whether a folder of this class holds mail (one without a class does).
fn is_mail_class(class: &str) -> bool {
    class.is_empty() || class == "IPF.Note" || class.starts_with("IPF.Note.")
}

/// A count a listed folder gives (`TotalCount`, `UnreadCount`, `ChildFolderCount`): 0 when it
/// doesn't.
fn count_of(f: &El, name: &str) -> u32 {
    f.text_of(name).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0)
}

/// The read, flag and draft state, asked for with every item, the day a flag is due
/// (`PidLidTaskDueDate`) and its reminder (`PidLidReminderSet`, `PidLidReminderSignalTime`,
/// `PidLidReminderTime`): [`follow_up`].
const FLAG_FIELDS: &str = "<t:FieldURI FieldURI=\"message:IsRead\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/>\
<t:ExtendedFieldURI PropertySetId=\"00062003-0000-0000-C000-000000000046\" PropertyId=\"33029\" PropertyType=\"SystemTime\"/>\
<t:ExtendedFieldURI PropertySetId=\"00062008-0000-0000-C000-000000000046\" PropertyId=\"34051\" PropertyType=\"Boolean\"/>\
<t:ExtendedFieldURI PropertySetId=\"00062008-0000-0000-C000-000000000046\" PropertyId=\"34144\" PropertyType=\"SystemTime\"/>\
<t:ExtendedFieldURI PropertySetId=\"00062008-0000-0000-C000-000000000046\" PropertyId=\"34050\" PropertyType=\"SystemTime\"/>";
const HEADER_FIELDS: &str = "<t:FieldURI FieldURI=\"item:Subject\"/><t:FieldURI FieldURI=\"message:From\"/>\
<t:FieldURI FieldURI=\"message:ToRecipients\"/><t:FieldURI FieldURI=\"message:CcRecipients\"/>\
<t:FieldURI FieldURI=\"message:ReplyTo\"/><t:FieldURI FieldURI=\"item:DateTimeReceived\"/>\
<t:FieldURI FieldURI=\"item:DateTimeSent\"/><t:FieldURI FieldURI=\"message:InternetMessageId\"/>\
<t:FieldURI FieldURI=\"item:InReplyTo\"/><t:FieldURI FieldURI=\"message:References\"/>\
<t:FieldURI FieldURI=\"item:HasAttachments\"/><t:FieldURI FieldURI=\"item:Size\"/>";

/// An extended property's value.
fn extended(it: &El, tag: &str) -> Option<i64> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case(tag)))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse().ok())
}

fn flags(it: &El) -> Flags {
    let status = extended(it, "0x1090");
    Flags {
        seen: it.text_of("IsRead").is_some_and(|v| v == "true"),
        flagged: status == Some(2),
        draft: extended(it, "0x0E07").is_some_and(|f| f & 8 != 0),
        // Marked complete in Outlook: done, not flagged.
        keywords: if status == Some(1) { vec![mailcraft_model::FOLLOW_UP_COMPLETE.to_string()] } else { Vec::new() },
        follow_up_due: follow_up::due_day(it),
        reminder: follow_up::reminder(it),
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
    // What it answers: a reply joins its conversation by these. They weren't asked for, and an
    // Exchange account's replies threaded by nothing, its own from Sent Items not at all (#148).
    for (field, name) in [("InReplyTo", "In-Reply-To"), ("References", "References")] {
        if let Some(v) = it.text_of(field).filter(|v| !v.trim().is_empty()) {
            let v: String = clean(v).chars().take(16 * 1024).collect();
            h.push_str(&format!("{name}: {v}\r\n"));
        }
    }
    h.push_str("\r\n");
    h.into_bytes()
}

/// A subject as an RFC 2047 word when it isn't ASCII.
fn mime_word(s: &str) -> String {
    if s.is_ascii() { s.to_string() } else { format!("=?UTF-8?B?{}?=", base64::encode(s.as_bytes())) }
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
    let ids = c.in_folder(&id, || c.find(&id, query, cap.min(jmap::FOUND_MAX)))?;
    let numbers: Vec<u32> = ids.iter().map(|i| st.number(i)).collect();
    let mut have = crate::exchange::window::kept_numbers(&fs);
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| !have.contains(*n)).map(|(i, _)| i.clone()).collect();
    let mut new = Vec::new();
    for it in c.in_folder(&id, || c.headers(&missing))? {
        let Some(i) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(i) };
        if have.insert(key.uid) {
            new.push(item_summary(&it, key));
        }
    }
    crate::exchange::window::found(st, store, (account, seen), folder, fs, &numbers, new)
}

/// A `Restriction` for a search's read, flag and attachment conditions, and when the mail came
/// (issue #179).
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
    let received = |op: &str, t: i64| {
        let at = mailcraft_model::dates::utc_stamp(t);
        format!(
            "<t:{op}><t:FieldURI FieldURI=\"item:DateTimeReceived\"/><t:FieldURIOrConstant><t:Constant Value=\"{at}\"/></t:FieldURIOrConstant></t:{op}>"
        )
    };
    parts.extend(q.after.map(|t| received("IsGreaterThanOrEqualTo", t)));
    parts.extend(q.before.map(|t| received("IsLessThan", t)));
    match parts.len() {
        0 => String::new(),
        1 => format!("<m:Restriction>{}</m:Restriction>", parts.concat()),
        _ => format!("<m:Restriction><t:And>{}</t:And></m:Restriction>", parts.concat()),
    }
}

/// Download one message's source (and cache it, unless the account was removed since it had been
/// removed `seen` times).
pub fn fetch_body(c: &Client, st: &State, store: &Store, key: &MessageKey, seen: u64) -> Result<(Vec<u8>, Option<Summary>)> {
    crate::exchange::window::fetch_body(st, store, key, seen, |id| in_path(c, st, &key.folder, || c.download(id)))
}

/// A message's text first, its files listed but not downloaded (issue #24), when it's big and
/// has files and isn't here already: `None` otherwise (then [`fetch_body`] brings it whole).
pub fn fetch_text(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Option<Vec<u8>>> {
    if store.has_body(key) || key.validity != VALIDITY {
        return Ok(None);
    }
    let fs = store.load_folder(&key.account, &key.folder)?;
    let big = fs.find(key.uid).is_some_and(|m| m.size >= text_first::TEXT_FIRST_OVER && m.has_attachments);
    match st.id(key.uid).filter(|_| big) {
        Some(id) => in_path(c, st, &key.folder, || c.download_text(id)),
        None => Ok(None),
    }
}

/// Run `f` where the folder at `path` is kept on the server ([`Client::in_folder`]).
fn in_path<T>(c: &Client, st: &State, path: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match st.mailbox(path) {
        Ok(id) => c.in_folder(id, f),
        Err(_) => f(),
    }
}

pub(crate) fn item_ids(ids: &[String]) -> String {
    ids.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect()
}

fn folder_ref(id: Option<&str>) -> String {
    match id {
        Some(id) => format!("<t:FolderId Id=\"{}\"/>", esc(id)),
        None => "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".into(),
    }
}

/// Apply one change on the server, sent where the folder it's about is kept.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    // The messages' ids by the folder they're in (`skip`'s left out), a batch at a time.
    let each_folder = |keys: &[MessageKey], skip: Option<&str>, send: &dyn Fn(&[String]) -> Result<()>| -> Result<()> {
        let mut folders: Vec<(String, Vec<String>)> = Vec::new();
        for (id, folder) in crate::exchange::window::server_ids(st, keys).into_iter().filter(|(_, f)| Some(f.as_str()) != skip) {
            match folders.iter_mut().find(|(f, _)| *f == folder) {
                Some((_, ids)) => ids.push(id),
                None => folders.push((folder, vec![id])),
            }
        }
        folders.iter().try_for_each(|(folder, ids)| in_path(c, st, folder, || ids.chunks(BATCH).try_for_each(send)))
    };
    let update = |keys: &[MessageKey], change: &str| -> Result<()> {
        each_folder(keys, None, &|chunk| {
            let changes: String = chunk
                .iter()
                .map(|id| format!("<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{change}</t:Updates></t:ItemChange>", esc(id)))
                .collect();
            c.call_all(&format!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
            ))
        })
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
        // A follow-up's days and reminder, or marked complete (gap 9: they stayed on this computer).
        Op::SetFollowUp { keys, .. } | Op::SetReminder { keys, .. } | Op::SetKeyword { keys, .. } if follow_up::updates(op).is_some() => {
            update(keys, &follow_up::updates(op).unwrap_or_default())
        }
        // Answered and forwarded are Exchange's to set; categories stay on this computer.
        Op::SetFlag { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            each_folder(keys, Some(to), &|chunk| {
                c.call_all(&format!(
                    "<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:MoveItem>",
                    folder_ref(Some(&dest)),
                    item_ids(chunk)
                ))
            })
        }
        Op::Expunge { keys } => each_folder(keys, None, &|chunk| {
            c.call_all(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>", item_ids(chunk)))
        }),
        Op::CreateFolder { path } => {
            let (parent, name) = st.split_path(path)?;
            let create = || {
                c.call_one(&format!(
                    "<m:CreateFolder><m:ParentFolderId>{}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass>\
                     <t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
                    folder_ref(parent.as_deref()),
                    esc(&name)
                ))
            };
            match parent.as_deref() {
                Some(p) => c.in_folder(p, create),
                None => create(),
            }
            .map(drop)
        }
        Op::RenameFolder { from, to } => in_path(c, st, from, || {
            let mut id = st.mailbox(from)?.to_string();
            let (old_parent, _) = st.split_path(from)?;
            let (parent, name) = st.split_path(to)?;
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
        }),
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.in_folder(&id, || {
                c.call_one(&format!(
                    "<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
                    esc(&id)
                ))
            })
            .map(drop)
        }
        Op::EmptyFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.in_folder(&id, || {
                c.call_one(&format!(
                    "<m:EmptyFolder DeleteType=\"HardDelete\" DeleteSubFolders=\"false\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:EmptyFolder>",
                    esc(&id)
                ))
            })
            .map(drop)
        }
        Op::MarkFolderRead { path } => {
            let id = st.mailbox(path)?.to_string();
            c.in_folder(&id, || c.mark_read(&id))
        }
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.create(raw, st.mailbox(folder)?, true)
}

/// Send a message. Exchange reads the recipients from the MIME source, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    match crate::exchange::window::with_bcc(raw, recipients) {
        Some(with) => c.send_mime(&with),
        None => c.send_mime(raw),
    }
}

mod availability;
pub use availability::{WINDOW_MAX as AVAILABILITY_WINDOW_MAX, answers_in as availability_answers};
mod directory;
mod discover;
mod follow_up;
pub use directory::DIRECTORY_MAX;
#[cfg(test)]
pub(crate) use directory::directory_name;
mod mailbox;
mod photo;
pub use photo::MAX_PHOTO;
mod quota;
mod requests;
mod rules;
#[cfg(test)]
use discover::autodiscover_answer;
pub use discover::{autodiscover, autodiscover_as};
use discover::{autodiscover_request, autodiscover_urls};
pub(crate) use requests::into_messages;
#[cfg(test)]
use requests::scheme_token;
pub use rules::{CHANGED_ELSEWHERE, RuleFolder, read_rules_answer, rule_written};
mod shared;
mod sync;
pub use sync::{sync_folder, sync_folder_as};
mod text_first;
mod watch;
pub use watch::{CHANGED_MAX as WATCH_CHANGED_MAX, Events as WatchEvents, Watch, events_in as watch_events};

#[cfg(test)]
mod tests;
