//! Importing an Outlook data file (`.pst`) into an account kept on this computer.
//!
//! The file is read with Microsoft's MIT-licensed `outlook-pst` crate (MS-PST): the folder tree
//! under the top of the mailbox becomes folders, and each mail item is written out again as a
//! standard message (RFC 5322 / MIME) from its properties (MS-OXCMSG / MS-OXPROPS), keeping the
//! original headers when the item has them. Contacts and calendar items go to the account's address
//! book and calendar (see `pst_pim`), notes to its notes; tasks show in the calendar on their due
//! day (MailCraft has no task list), and those without one are left out.
//!
//! A data file is hostile input like any other: the reader runs behind `catch_unwind`, items that
//! can't be read are skipped and counted, and sizes are capped.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::rc::Rc;

use mailcraft_mime::{self as mime, Draft, InlinePicture, OutAttachment};
use mailcraft_model::pim::{LOCAL, NOTE_YELLOW, Note, PIM_DOC, Pim, StoredCard, StoredEvent, StoredNote};
use mailcraft_model::{EmailAddress, Flags, Folder, FolderRole, MessageKey, utf7};
use mailcraft_store::{FolderState, Store};
use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::TableContext;
use outlook_pst::messaging::attachment::{AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment};
use outlook_pst::messaging::folder::Folder as PstFolder;
use outlook_pst::messaging::message::{AnsiMessage, Message, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, EntryId, Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};

/// Folders nested deeper than this are left out.
const MAX_DEPTH: usize = 32;
/// Attachments larger than this are left out of the imported message.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// Original headers longer than this are not kept.
const MAX_HEADERS: usize = 256 * 1024;
/// Embedded messages inside embedded messages… stop here.
const MAX_NESTING: usize = 3;

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub folders: Vec<Folder>,
    pub messages: usize,
    /// Contacts, calendar items and notes.
    pub contacts: usize,
    pub events: usize,
    pub notes: usize,
    /// Items that couldn't be read or written.
    pub skipped: usize,
}

/// Import the data file at `path` into `account`'s cache (an account with no server). Calls
/// `progress` with the number of messages imported so far, every so often.
pub fn import(path: &Path, store: &Store, account: &str, progress: &mut dyn FnMut(usize)) -> Result<Imported, String> {
    let mut out = Out {
        store,
        account,
        folders: Vec::new(),
        states: HashMap::new(),
        imported: 0,
        skipped: 0,
        progress,
        pim: Pim::default(),
        pending: Vec::new(),
        pending_bytes: 0,
    };
    // `!BDN` then the client magic: `SM` a data file, `SO` Outlook's offline copy of a server
    // account (MS-PST 2.2.2.6).
    let mut head = [0u8; 10];
    if std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head.starts_with(b"!BDN")
        && head.get(8..10) == Some(b"SO")
    {
        return Err("this is Outlook's offline copy of a server account (.ost), not a data file: add the account itself to see its mail".into());
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<()> {
        match UnicodePstFile::open(path) {
            Ok(pst) => read_unicode(pst, &mut out),
            // Locked by Outlook: say so, rather than trying it as the older format.
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Err(e),
            Err(_) => read_ansi(AnsiPstFile::open(path)?, &mut out),
        }
    }));
    match read {
        Ok(Ok(())) => {}
        // Windows' sharing and lock violations: Outlook (or another program) has it open.
        Ok(Err(e)) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            return Err(
                "Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into()
            );
        }
        // Nothing found at all: say why; otherwise keep what was read.
        Ok(Err(e)) if out.folders.is_empty() => return Err(format!("not an Outlook data file, or it is damaged ({e})")),
        Ok(Err(e)) => log::warn!("{}: the rest of the data file could not be read: {e}", path.display()),
        Err(_) if out.folders.is_empty() => return Err("the data file is damaged".into()),
        Err(_) => log::warn!("{}: the data file is damaged; stopped reading", path.display()),
    }
    out.finish()
}

macro_rules! reader {
    ($name:ident, $file:ty, $store:ty, $message:ty, $attachment:ty) => {
        fn $name(pst: $file, out: &mut Out<'_>) -> io::Result<()> {
            let store = <$store>::read(Rc::new(pst))?;
            let any: Rc<dyn PstStore> = store.clone();
            let named = crate::pst_pim::Named::of(&*any);
            let read = |entry: &EntryId| -> io::Result<Item> {
                let m = <$message>::read(store.clone(), entry, None)?;
                let mut files = Vec::new();
                if let Some(table) = m.attachment_table() {
                    for row in table.rows_matrix() {
                        let node = NodeId::from(u32::from(row.id()));
                        match <$attachment>::read(m.clone(), node, None) {
                            Ok(a) => files.extend(attachment(&*a, 0, &named)),
                            Err(e) => log::info!("an attachment could not be read: {e}"),
                        }
                    }
                }
                Ok(item(&*m, files, &named))
            };
            walk(&*any, &read, out)
        }
    };
}

reader!(read_unicode, UnicodePstFile, UnicodeStore, UnicodeMessage, UnicodeAttachment);
reader!(read_ansi, AnsiPstFile, AnsiStore, AnsiMessage, AnsiAttachment);

type ReadItem<'a> = dyn Fn(&EntryId) -> io::Result<Item> + 'a;

fn walk(store: &dyn PstStore, read: &ReadItem<'_>, out: &mut Out<'_>) -> io::Result<()> {
    let top = store.open_folder(&store.properties().ipm_sub_tree_entry_id()?)?;
    visit(store, &*top, None, 0, read, out);
    Ok(())
}

fn visit(store: &dyn PstStore, folder: &dyn PstFolder, parent: Option<&str>, depth: usize, read: &ReadItem<'_>, out: &mut Out<'_>) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(table) = folder.hierarchy_table() else { return };
    for row in table.rows_matrix() {
        let opened = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| store.open_folder(&e));
        let sub = match opened {
            Ok(f) => f,
            Err(e) => {
                log::info!("a folder could not be read: {e}");
                continue;
            }
        };
        // Mail folders (no class means mail, in old files), contacts and calendars.
        let class = sub.properties().get(0x3613).and_then(text_of).unwrap_or_default();
        let mail = (class.is_empty() || class.starts_with("IPF.Note") || class.starts_with("IPF.Imap"))
            // Outlook's RSS Feeds folder is its own bookkeeping, not mail.
            && !class.starts_with("IPF.Note.OutlookHomepage");
        // Outlook's own Outbox, when it has nothing waiting (MailCraft has an Outbox of its own).
        let empty = sub.contents_table().is_none_or(|t| t.rows_matrix().next().is_none());
        if empty && special_folder(store, 0x35E2) == Some(u32::from(row.id())) {
            continue;
        }
        if !mail {
            if class.starts_with("IPF.Contact")
                || class.starts_with("IPF.Appointment")
                || class.starts_with("IPF.Task")
                || class.starts_with("IPF.StickyNote")
            {
                read_items(store, &*sub, read, out);
                visit(store, &*sub, parent, depth + 1, read, out);
            }
            continue;
        }
        let name = sub.properties().get(0x3001).and_then(text_of).unwrap_or_default();
        let path = out.add_folder(parent, &name);
        if let Some(contents) = sub.contents_table() {
            for row in contents.rows_matrix() {
                let item = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e));
                match item {
                    Ok(i) => out.add_message(&path, i),
                    Err(e) => {
                        log::info!("a message could not be read: {e}");
                        out.skipped += 1;
                    }
                }
            }
        }
        out.save_folder(&path);
        visit(store, &*sub, Some(&path), depth + 1, read, out);
    }
}

/// The node of one of the store's special folders (`0x35E2`: the Outbox), from its entry id
/// (flags, the store's 16-byte key, then the node id).
fn special_folder(store: &dyn PstStore, prop: u16) -> Option<u32> {
    let (_, v) = store.properties().iter().find(|(id, _)| **id == prop)?;
    let PropertyValue::Binary(b) = v else { return None };
    let id = b.buffer().get(20..24)?;
    Some(u32::from_le_bytes([*id.first()?, *id.get(1)?, *id.get(2)?, *id.get(3)?]))
}

/// A contacts or calendar folder's items.
fn read_items(store: &dyn PstStore, folder: &dyn PstFolder, read: &ReadItem<'_>, out: &mut Out<'_>) {
    let Some(contents) = folder.contents_table() else { return };
    for row in contents.rows_matrix() {
        match store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e)) {
            Ok(i) => out.add_pim(i),
            Err(e) => {
                log::info!("an item could not be read: {e}");
                out.skipped += 1;
            }
        }
    }
}

/// A mail item, as read.
#[derive(Default)]
struct Item {
    class: String,
    subject: String,
    from: EmailAddress,
    to: Vec<EmailAddress>,
    cc: Vec<EmailAddress>,
    bcc: Vec<EmailAddress>,
    date: i64,
    body: String,
    html: Option<String>,
    message_id: String,
    in_reply_to: String,
    references: Vec<String>,
    headers: String,
    flags: Flags,
    files: Vec<File>,
    /// A contact's or appointment's own properties.
    extra: Option<crate::pst_pim::Extra>,
}

enum File {
    Data { name: String, mime: String, cid: String, data: Vec<u8> },
    Message(Box<Item>),
}

fn item(m: &dyn Message, files: Vec<File>, named: &crate::pst_pim::Named) -> Item {
    let p = |id: u16| m.properties().get(id);
    let s = |id: u16| p(id).and_then(text_of).unwrap_or_default();
    let mut it = Item { class: s(0x001A), files, ..Default::default() };
    let class = it.class.to_ascii_lowercase();
    if class.starts_with("ipm.contact")
        || class.starts_with("ipm.appointment")
        || class.starts_with("ipm.task")
        || class.starts_with("ipm.stickynote")
    {
        it.extra = Some(crate::pst_pim::extract(&p, named));
    }
    // A subject may start with a prefix marker (0x01, length).
    it.subject = {
        let raw = s(0x0037);
        if raw.starts_with('\u{1}') { raw.chars().skip(2).collect() } else { raw }
    };
    let email = |ids: &[u16]| ids.iter().map(|id| s(*id)).find(|e| e.contains('@')).unwrap_or_default();
    let name = |ids: &[u16]| ids.iter().map(|id| s(*id)).find(|n| !n.is_empty()).unwrap_or_default();
    it.from = EmailAddress::new(name(&[0x0C1A, 0x0042]), email(&[0x5D01, 0x0C1F, 0x5D02, 0x0065]));
    it.date = [0x0E06, 0x0039, 0x3007].iter().find_map(|id| p(*id).and_then(time_of)).unwrap_or(0);
    it.body = s(0x1000);
    it.html = p(0x1013).and_then(|v| match v {
        PropertyValue::Binary(b) => {
            let page = match p(0x3FDE) {
                Some(PropertyValue::Integer32(n)) => *n,
                _ => 0,
            };
            Some(decode(b.buffer(), page))
        }
        other => text_of(other),
    });
    it.message_id = s(0x1035);
    it.in_reply_to = s(0x1042);
    it.references = s(0x1039).split_whitespace().map(str::to_string).collect();
    it.headers = s(0x007D);
    let flags = match p(0x0E07) {
        Some(PropertyValue::Integer32(n)) => *n,
        _ => 0,
    };
    let follow_up = matches!(p(0x1090), Some(PropertyValue::Integer32(2)));
    let replied = matches!(p(0x1081), Some(PropertyValue::Integer32(n)) if *n == 0x0066 || *n == 0x0067);
    // MSGFLAG_READ, MSGFLAG_UNSENT.
    it.flags = Flags { seen: flags & 0x1 != 0, draft: flags & 0x8 != 0, flagged: follow_up, answered: replied, ..Default::default() };
    if let Some(table) = m.recipient_table() {
        for row in rows(&**table) {
            let get = |id: u16| row.get(&id).and_then(text_of).unwrap_or_default();
            let address = [get(0x39FE), get(0x3003)].into_iter().find(|e| e.contains('@'));
            let Some(address) = address else { continue };
            let a = EmailAddress::new(get(0x3001), address);
            match row.get(&0x0C15) {
                Some(PropertyValue::Integer32(2)) => it.cc.push(a),
                Some(PropertyValue::Integer32(3)) => it.bcc.push(a),
                _ => it.to.push(a),
            }
        }
    }
    it
}

/// An attachment's file, or the message it holds.
fn attachment(a: &dyn Attachment, nesting: usize, named: &crate::pst_pim::Named) -> Option<File> {
    let p = |id: u16| a.properties().get(id);
    let s = |id: u16| p(id).and_then(text_of).unwrap_or_default();
    // Hidden attachments (e.g. a contact picture) aren't part of the message.
    if matches!(p(0x7FFE), Some(PropertyValue::Boolean(true))) && s(0x3712).is_empty() {
        return None;
    }
    match a.data()? {
        AttachmentData::Binary(b) => {
            if b.buffer().len() > MAX_ATTACHMENT {
                log::info!("an attachment of {} bytes was left out", b.buffer().len());
                return None;
            }
            let name = [s(0x3707), s(0x3704), s(0x3001)].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "attachment".into());
            Some(File::Data { name, mime: s(0x370E), cid: s(0x3712), data: b.buffer().to_vec() })
        }
        AttachmentData::Message(m) if nesting < MAX_NESTING => {
            // The embedded message's own attachments need its concrete type: left out.
            Some(File::Message(Box::new(item(&**m, Vec::new(), named))))
        }
        AttachmentData::Message(_) => None,
    }
}

/// A table's rows as property maps.
fn rows(table: &dyn TableContext) -> Vec<HashMap<u16, PropertyValue>> {
    let context = table.context();
    let mut out = Vec::new();
    for row in table.rows_matrix() {
        let Ok(values) = row.columns(context) else { continue };
        let mut map = HashMap::new();
        for (column, value) in context.columns().iter().zip(values) {
            if let Some(v) = value
                && let Ok(pv) = table.read_column(&v, column.prop_type())
            {
                map.insert(column.prop_id(), pv);
            }
        }
        out.push(map);
    }
    out
}

pub(crate) fn text_of(v: &PropertyValue) -> Option<String> {
    match v {
        PropertyValue::Unicode(u) => Some(String::from_utf16_lossy(u.buffer()).trim_end_matches('\0').to_string()),
        PropertyValue::String8(s) => Some(decode(s.buffer(), 0).trim_end_matches('\0').to_string()),
        _ => None,
    }
}

/// A FILETIME (100 ns since 1601) as Unix seconds.
pub(crate) fn time_of(v: &PropertyValue) -> Option<i64> {
    match v {
        PropertyValue::Time(t) if *t > 0 => Some(t / 10_000_000 - 11_644_473_600),
        _ => None,
    }
}

/// Bytes in a Windows code page: UTF-8 when they are (or say so), else Windows-1252.
fn decode(bytes: &[u8], code_page: i32) -> String {
    if code_page == 65001 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('\u{FFFD}'),
            _ => char::from(b),
        })
        .collect()
}

/// The item as a standard message.
fn to_raw(it: &Item, fallback_id: &str) -> Result<Vec<u8>, String> {
    let html = it.html.clone().filter(|h| !h.trim().is_empty());
    let body = if it.body.trim().is_empty() { html.as_deref().map(mime::html_to_text).unwrap_or_default() } else { it.body.clone() };
    let mut d = Draft {
        to: it.to.clone(),
        cc: it.cc.clone(),
        subject: it.subject.clone(),
        body,
        in_reply_to: it.in_reply_to.trim().trim_matches(|c| c == '<' || c == '>').to_string(),
        references: it.references.iter().map(|r| r.trim_matches(|c| c == '<' || c == '>').to_string()).collect(),
        html: html.clone(),
        ..Default::default()
    };
    for f in &it.files {
        match f {
            File::Data { name, mime: m, cid, data } => {
                let cid = cid.trim().trim_matches(|c| c == '<' || c == '>');
                if !cid.is_empty() && html.as_deref().is_some_and(|h| h.contains(&format!("cid:{cid}"))) {
                    d.inline.push(InlinePicture { cid: cid.to_string(), name: name.clone(), mime: m.clone(), data: data.clone() });
                } else {
                    d.attachments.push(OutAttachment { name: name.clone(), mime: m.clone(), data: data.clone() });
                }
            }
            File::Message(inner) => {
                let raw = to_raw(inner, &format!("{fallback_id}.1"))?;
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                d.attachments.push(OutAttachment { name: format!("{subject}.eml"), mime: "message/rfc822".into(), data: raw });
            }
        }
    }
    let id = it.message_id.trim().trim_matches(|c| c == '<' || c == '>');
    let id = if id.is_empty() { fallback_id } else { id };
    let built = mime::build(&d, &it.from, it.date, id)?;
    Ok(with_headers(built, &it.headers, &it.bcc))
}

/// Keep the item's original headers (when it came in by mail) above the rebuilt body; a sent
/// item's Bcc goes back in.
fn with_headers(built: Vec<u8>, original: &str, bcc: &[EmailAddress]) -> Vec<u8> {
    let Some(split) = built.windows(4).position(|w| w == b"\r\n\r\n") else { return built };
    let (head, body) = built.split_at(split);
    let head = String::from_utf8_lossy(head);
    let mime_header = |line: &str| {
        let l = line.to_ascii_lowercase();
        l.starts_with("content-") || l.starts_with("mime-version:")
    };
    let mut out = String::new();
    let original = original.trim();
    if !original.is_empty() && original.len() <= MAX_HEADERS && original.contains(':') {
        // The original's own headers except its body's description, then the rebuilt body's.
        let mut keep = false;
        for line in original.lines() {
            if !line.starts_with([' ', '\t']) {
                keep = !mime_header(line);
            }
            if keep {
                out.push_str(line.trim_end_matches('\r'));
                out.push_str("\r\n");
            }
        }
        let mut keep = false;
        for line in head.split("\r\n") {
            if !line.starts_with([' ', '\t']) {
                keep = mime_header(line);
            }
            if keep {
                out.push_str(line);
                out.push_str("\r\n");
            }
        }
    } else {
        if !bcc.is_empty() {
            out.push_str(&format!("Bcc: {}\r\n", mailcraft_model::addr::format_list(bcc)));
        }
        out.push_str(&head);
        out.push_str("\r\n");
    }
    let mut raw = out.into_bytes();
    // `body` starts with the blank line ending the header.
    raw.extend_from_slice(body.get(2..).unwrap_or_default());
    raw
}

/// Message sources written to the cache together (one transaction): at most this many…
const BATCH_MESSAGES: usize = 2000;
/// …or this many bytes.
const BATCH_BYTES: usize = 16 << 20;

/// A message's summary from its rebuilt source's headers, with its preview and attachments
/// from the item itself (parsing the whole source again, attachments and all, only to find them
/// cost more than reading it).
fn summary_of(raw: &[u8], it: &Item, key: MessageKey, size: u32) -> mailcraft_model::Summary {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let mut s = mime::summary_from_header(raw.get(..end).unwrap_or(raw), key, it.flags.clone(), size, it.date);
    s.has_attachments = !it.files.is_empty();
    let text = if it.body.trim().is_empty() {
        it.html
            .as_deref()
            .map(|h| mime::html_to_text(h.get(..h.char_indices().nth(64 * 1024).map_or(h.len(), |(i, _)| i)).unwrap_or(h)))
            .unwrap_or_default()
    } else {
        it.body.replace("\r\n", "\n")
    };
    s.preview = mime::preview(&text);
    s
}

/// A folder being read.
struct Open {
    uid_next: u32,
    /// Messages whose sources may not be written yet (they go to the list once they are).
    waiting: Vec<mailcraft_model::Summary>,
    list: mailcraft_store::ListWriter,
}

/// What has been written so far.
struct Out<'a> {
    store: &'a Store,
    account: &'a str,
    folders: Vec<Folder>,
    /// The folders being read: their next number, and their list going out a page at a time
    /// (a folder of hundreds of thousands isn't held whole; written ones keep only their counts).
    states: HashMap<String, Open>,
    imported: usize,
    skipped: usize,
    progress: &'a mut dyn FnMut(usize),
    /// Contacts and calendar items read so far.
    pim: Pim,
    /// Sources read but not yet written, and their size.
    pending: Vec<(MessageKey, Vec<u8>)>,
    pending_bytes: usize,
}

impl Out<'_> {
    /// A folder for `name` under `parent`, with a role when it's a usual top-level one.
    fn add_folder(&mut self, parent: Option<&str>, name: &str) -> String {
        let name = name.trim().replace('/', "∕");
        let name = if name.is_empty() { "Folder".to_string() } else { name };
        let mut role = match parent {
            None => FolderRole::from_name(&name, None),
            Some(_) => FolderRole::Other,
        };
        if role != FolderRole::Other && self.folders.iter().any(|f| f.role == role) {
            role = FolderRole::Other;
        }
        let base = match (role, parent) {
            (FolderRole::Inbox, _) => "INBOX".to_string(),
            (_, Some(p)) => format!("{p}/{}", utf7::encode(&name)),
            (_, None) => utf7::encode(&name),
        };
        let mut path = base.clone();
        let mut n = 2;
        while self.folders.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
            path = format!("{base} ({n})");
            n += 1;
        }
        self.folders.push(Folder {
            account: self.account.to_string(),
            path: path.clone(),
            delimiter: Some('/'),
            role,
            selectable: true,
            unread: 0,
            total: 0,
        });
        self.states.insert(path.clone(), Open { uid_next: 1, waiting: Vec::new(), list: self.store.list_writer(self.account, &path) });
        path
    }

    /// A contact, calendar item or note (anything else in their folders is left out).
    fn add_pim(&mut self, it: Item) {
        let Some(x) = &it.extra else { return };
        let n = self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len();
        let uid = format!("pst-{}-{n}@mailcraft.invalid", self.account);
        if it.class.to_ascii_lowercase().starts_with("ipm.stickynote") {
            // A note's text is its body (its subject is the first line of it).
            let text = if it.body.trim().is_empty() { it.subject.clone() } else { it.body.replace("\r\n", "\n") };
            if text.trim().is_empty() {
                self.skipped += 1;
                return;
            }
            let note =
                Note { uid, text: text.trim_end().to_string(), color: x.note_color.unwrap_or(NOTE_YELLOW), modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if it.class.to_ascii_lowercase().starts_with("ipm.task") {
            // MailCraft has no task list: a task with a due date shows in the calendar that day.
            let Some(due) = x.due else {
                self.skipped += 1;
                return;
            };
            let day = crate::pst_pim::Extra { start: Some(due), end: Some(due), all_day: true, ..Default::default() };
            let summary = if x.done { format!("✓ {}", it.subject) } else { it.subject.clone() };
            if let Some(event) = crate::pst_pim::event_of(&day, &summary, &it.body, &EmailAddress::default(), &[], uid) {
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            }
        } else if it.class.to_ascii_lowercase().starts_with("ipm.contact") {
            let card = crate::pst_pim::card_of(x, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            match crate::pst_pim::event_of(x, &it.subject, &it.body, &it.from, &people, uid) {
                Some(event) => self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() }),
                None => self.skipped += 1,
            }
        }
    }

    fn add_message(&mut self, path: &str, it: Item) {
        // Mail only: meeting requests and receipts are mail too; appointments, contacts and
        // tasks in a mail folder aren't.
        let class = it.class.to_ascii_lowercase();
        if !(class.is_empty() || class.starts_with("ipm.note") || class.starts_with("ipm.schedule") || class.starts_with("report.")) {
            return;
        }
        let Some(st) = self.states.get_mut(path) else { return };
        let uid = st.uid_next;
        let fallback = format!("pst.{}.{}.{uid}@mailcraft.invalid", self.account, self.imported);
        let raw = match to_raw(&it, &fallback) {
            Ok(r) => r,
            Err(e) => {
                log::info!("a message could not be rebuilt: {e}");
                self.skipped += 1;
                return;
            }
        };
        let key = MessageKey { account: self.account.to_string(), folder: path.to_string(), validity: 1, uid };
        st.uid_next = uid.saturating_add(1);
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        st.waiting.push(summary_of(&raw, &it, key.clone(), size));
        let page_full = st.waiting.len() >= mailcraft_store::PAGE_MESSAGES;
        self.imported += 1;
        self.pending_bytes += raw.len();
        self.pending.push((key, raw));
        if page_full || self.pending.len() >= BATCH_MESSAGES || self.pending_bytes >= BATCH_BYTES {
            self.flush();
        }
        // A page's worth whose sources are written: into the folder's list.
        if page_full {
            self.list_out(path);
        }
        if self.imported.is_multiple_of(200) {
            (self.progress)(self.imported);
        }
    }

    /// Write the sources read so far, together. A batch that can't be written leaves its
    /// messages out of their folders' lists (counted as skipped), as one failed write did.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        if let Err(e) = self.store.write_bodies(&batch) {
            log::warn!("import: {e}");
            for (key, _) in &batch {
                if let Some(st) = self.states.get_mut(&key.folder) {
                    st.waiting.retain(|m| m.key.uid != key.uid);
                }
            }
            self.imported = self.imported.saturating_sub(batch.len());
            self.skipped += batch.len();
        }
    }

    /// Hand a folder's messages whose sources are written to its list (full pages go out).
    fn list_out(&mut self, path: &str) {
        let Some(st) = self.states.get_mut(path) else { return };
        let ready = std::mem::take(&mut st.waiting);
        if let Err(e) = st.list.push(self.store, ready) {
            log::warn!("import: {e}");
        }
    }

    /// Write a finished folder's list and counts, then let go of it (only the counts are needed
    /// from here).
    fn save_folder(&mut self, path: &str) {
        self.flush();
        self.list_out(path);
        let Some(st) = self.states.remove(path) else { return };
        let head = FolderState { validity: 1, uid_next: st.uid_next, ..Default::default() };
        let (unread, total) = match st.list.finish(self.store, &head) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("import: {e}");
                (0, 0)
            }
        };
        if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
            f.unread = unread;
            f.total = total;
        }
    }

    fn finish(mut self) -> Result<Imported, String> {
        self.flush();
        // An Inbox to open, even when the file had none.
        if !self.folders.iter().any(|f| f.role == FolderRole::Inbox) {
            let path = self.add_folder(None, "Inbox");
            self.save_folder(&path);
        }
        // Folders left part-way (the file ended early) still get their list.
        let unsaved: Vec<String> = self.states.keys().cloned().collect();
        for p in unsaved {
            self.save_folder(&p);
        }
        self.store.save_folders(self.account, &self.folders).map_err(|e| e.to_string())?;
        let (contacts, events, notes) = (self.pim.cards.len(), self.pim.events.len(), self.pim.notes.len());
        if contacts + events + notes > 0 {
            self.pim.ensure_defaults();
            self.store.save_account_doc(self.account, PIM_DOC, &self.pim).map_err(|e| e.to_string())?;
        }
        Ok(Imported { folders: self.folders, messages: self.imported, contacts, events, notes, skipped: self.skipped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outlook's notes come in as notes: their text, colour and time; an empty one doesn't.
    #[test]
    fn notes_come_in_as_notes() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut progress = |_| {};
        let mut out = Out {
            store: &store,
            account: "d1",
            folders: Vec::new(),
            states: HashMap::new(),
            imported: 0,
            skipped: 0,
            progress: &mut progress,
            pim: Pim::default(),
            pending: Vec::new(),
            pending_bytes: 0,
        };
        let note = |body: &str, color: Option<u8>| Item {
            class: "IPM.StickyNote".into(),
            subject: body.lines().next().unwrap_or_default().into(),
            body: body.into(),
            date: 1_700_000_000,
            extra: Some(crate::pst_pim::Extra { note_color: color, modified: Some(1_700_000_500), ..Default::default() }),
            ..Default::default()
        };
        out.add_pim(note("Poster\r\nAsk about 170 gsm", Some(1)));
        out.add_pim(note("Plain", None));
        out.add_pim(note("  ", Some(2)));
        let done = out.finish().unwrap();
        assert_eq!((done.notes, done.skipped), (2, 1));
        let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
        let n = &pim.notes[0].note;
        assert_eq!((n.text.as_str(), n.color, n.modified), ("Poster\nAsk about 170 gsm", 1, 1_700_000_500));
        assert_eq!(pim.notes[1].note.color, NOTE_YELLOW, "no colour: Outlook's yellow");
        assert!(pim.notes.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_1252_and_utf8() {
        assert_eq!(decode(b"caf\xe9 \x80 \x93q\x94", 1252), "café € “q”");
        assert_eq!(decode("café".as_bytes(), 0), "café");
        assert_eq!(decode(b"\xff\xfe", 65001), "\u{FFFD}\u{FFFD}");
    }

    #[test]
    fn filetime() {
        // 2024-01-01T00:00:00Z
        assert_eq!(time_of(&PropertyValue::Time(133_485_408_000_000_000)), Some(1_704_067_200));
        assert_eq!(time_of(&PropertyValue::Time(0)), None);
    }

    #[test]
    fn original_headers_are_kept_over_the_rebuilt_body() {
        let it = Item {
            subject: "Hello".into(),
            from: EmailAddress::new("Ana", "ana@example.com"),
            to: vec![EmailAddress::new("", "me@example.com")],
            date: 1_704_067_200,
            body: "Hi there".into(),
            message_id: "<orig@example.com>".into(),
            headers: "Received: from x\r\n by y\r\nFrom: Ana <ana@example.com>\r\nTo: me@example.com\r\nSubject: Hello\r\nList-Id: <l.example.com>\r\nContent-Type: multipart/mixed;\r\n boundary=zzz\r\nMIME-Version: 1.0\r\n"
                .into(),
            ..Default::default()
        };
        let raw = to_raw(&it, "fallback@x").unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("List-Id: <l.example.com>"));
        assert!(text.contains("Received: from x\r\n by y"));
        assert!(!text.contains("boundary=zzz"), "the original body's description goes");
        assert_eq!(text.matches("MIME-Version").count(), 1);
        let p = mime::parse(&raw).unwrap();
        assert_eq!(p.subject, "Hello");
        assert!(p.text.contains("Hi there"));
    }

    #[test]
    fn a_sent_item_keeps_its_bcc_and_gets_an_id() {
        let it = Item {
            subject: "Plans".into(),
            from: EmailAddress::new("Me", "me@example.com"),
            bcc: vec![EmailAddress::new("", "secret@example.com")],
            date: 1_704_067_200,
            body: "Hi".into(),
            ..Default::default()
        };
        let raw = to_raw(&it, "fallback@x").unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.starts_with("Bcc: secret@example.com\r\n"));
        assert!(text.contains("<fallback@x>"));
    }

    #[test]
    fn an_offline_copy_is_explained() {
        let dir = std::env::temp_dir().join(format!("mailcraft-ost-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("me@example.com.ost");
        std::fs::write(&file, b"!BDN    SO$ rest").unwrap();
        let store = Store::open(dir.join("cache")).unwrap();
        let e = import(&file, &store, "a1", &mut |_| {}).unwrap_err();
        assert!(e.contains(".ost") && e.contains("add the account"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn junk_is_refused_and_nothing_is_written() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-junk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("junk.pst");
        std::fs::write(&file, b"!BDN this is not really a data file").unwrap();
        let store = Store::open(dir.join("cache")).unwrap();
        assert!(import(&file, &store, "a1", &mut |_| {}).is_err());
        assert!(import(&dir.join("missing.pst"), &store, "a1", &mut |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_a_data_file() {
        use pst_rw::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Archive 2024.pst");
        let recipients = [
            UnicodePstRecipient { name: "Ana Ruiz", email: "ana@example.com", recipient_type: UnicodePstRecipientType::To },
            UnicodePstRecipient { name: "Ben", email: "ben@example.com", recipient_type: UnicodePstRecipientType::Cc },
        ];
        let message = UnicodePstMessage {
            subject: "Quarterly plan — café",
            sender_name: "Riley Moreno",
            sender_email: "riley@example.com",
            recipients: &recipients,
            body: "Here is the plan.",
            html_body: Some("<p>Here is <b>the plan</b>.</p><img src=\"cid:chart@example.com\">"),
            message_id: "<plan@example.com>",
            // 2024-01-01T00:00:00Z
            delivery_time: 133_485_408_000_000_000,
        };
        let files = [
            UnicodePstAttachment {
                filename: "chart.png",
                mime_type: "image/png",
                content_id: Some("<chart@example.com>"),
                data: b"\x89PNG\r\n\x1a\nfake",
            },
            UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember the milk" },
        ];
        drop(Writer::create_with_attachments(&file, &message, &files).unwrap());
        let nested = UnicodePstMessage { subject: "Old thread", message_id: "<old@example.com>", html_body: None, ..message };
        drop(Writer::append_in_folder(&file, &["Projects", "2024"], &nested).unwrap());

        let store = Store::open(dir.join("cache")).unwrap();
        let mut ticks = 0;
        let done = import(&file, &store, "a1", &mut |_| ticks += 1).unwrap();
        assert_eq!(done.messages, 2, "{done:?}");
        assert_eq!(done.skipped, 0);
        let inbox = done.folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap();
        assert_eq!(inbox.path, "INBOX");
        let all = store.load_folders("a1").unwrap();
        assert_eq!(all, done.folders);
        let nested_folder = all.iter().find(|f| f.path == "Projects/2024").unwrap();
        assert_eq!(nested_folder.total, 1);
        assert!(all.iter().any(|f| f.path == "Projects"));

        // The message where the writer put it, with its addresses, body, picture and file.
        let (path, st) = all
            .iter()
            .filter(|f| f.path != "Projects/2024")
            .find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty()).map(|s| (f.path.clone(), s)))
            .unwrap();
        let s = &st.messages[0];
        assert_eq!(s.subject, "Quarterly plan — café");
        assert_eq!(s.date, 1_704_067_200);
        let key = MessageKey { account: "a1".into(), folder: path, validity: 1, uid: s.key.uid };
        let raw = store.read_body(&key).unwrap();
        let p = mime::parse(&raw).unwrap();
        assert_eq!(p.from.email, "riley@example.com");
        assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@example.com"]);
        assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
        assert_eq!(p.message_id, "plan@example.com");
        assert!(p.had_html && p.text.contains("the plan"), "{}", p.text);
        let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"notes.txt"), "{names:?}");
        assert!(!names.contains(&"chart.png"), "the picture goes inline: {names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod real_files {
    /// Import a real data file named by `MAILCRAFT_PST` (counts only).
    #[test]
    #[ignore]
    fn imports_the_file_in_mailcraft_pst() {
        let Ok(path) = std::env::var("MAILCRAFT_PST") else { return };
        let dir = std::env::temp_dir().join(format!("mailcraft-real-pst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = mailcraft_store::Store::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let r = super::import(std::path::Path::new(&path), &store, "r1", &mut |_| {});
        match r {
            Ok(done) => {
                eprintln!(
                    "messages {} contacts {} events {} skipped {} in {:?}",
                    done.messages,
                    done.contacts,
                    done.events,
                    done.skipped,
                    t.elapsed()
                );
                let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
                for f in &done.folders {
                    eprintln!("  {:30} {:?} total {} unread {}", f.path, f.role, f.total, f.unread);
                    for m in store.load_folder("r1", &f.path).unwrap().messages {
                        dated += usize::from(m.date > 0);
                        if let Some(p) = store.read_body(&m.key).and_then(|raw| mailcraft_mime::parse(&raw)) {
                            parsed += 1;
                            html += usize::from(p.had_html);
                            files += p.attachments.len();
                        }
                    }
                }
                eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
            }
            Err(e) => eprintln!("FAILED: {e}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
