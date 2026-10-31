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

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::rc::Rc;

use mailcraft_mime::{self as mime, Draft, InlinePicture, OutAttachment};
use mailcraft_model::job::SkipReason;
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

mod item;
mod out;
mod props;
mod rebuild;
#[cfg(test)]
mod tests;

use item::{Item, attachment, item, message_code_page};
use out::Out;
#[doc(hidden)]
pub use props::Val;
pub(crate) use props::text_of;

/// Folders nested deeper than this are left out.
const MAX_DEPTH: usize = 32;
/// Attachments larger than this are left out of the imported message.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// A value (a body, an attachment's bytes, a table's rows) the file says is larger than this
/// is an error before any of it is read: a damaged size can't make the import run out of memory.
const MAX_VALUE: u64 = 256 * 1024 * 1024;
/// Original headers longer than this are not kept.
const MAX_HEADERS: usize = 256 * 1024;
/// Embedded messages inside embedded messages… stop here.
const MAX_NESTING: usize = 3;
/// Message ids kept from an item's References (as many as threading reads).
const MAX_IDS: usize = 256;
/// Child blocks kept of an item's conversation index.
const MAX_THREAD_BLOCKS: usize = 50;
/// Longest Thread-Topic written, in characters.
const MAX_TOPIC: usize = 512;

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub folders: Vec<Folder>,
    pub messages: usize,
    /// Contacts, calendar items and notes.
    pub contacts: usize,
    pub events: usize,
    pub notes: usize,
    /// Items that couldn't be read or written, and how many for each reason.
    pub skipped: usize,
    pub skips: BTreeMap<SkipReason, usize>,
    /// Messages with text (or an attached message with text) in a code page MailCraft can't read
    /// (ISO-2022-KR): imported with only its ASCII. Logged, and not in `skips`, since the message
    /// itself came in.
    pub charset_unsupported: usize,
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
        skips: BTreeMap::new(),
        charset_unsupported: 0,
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

/// A mail item rebuilt as a message from its properties and its recipients' alone, as the
/// import writes it, with its summary: for the fuzz target, which hands it hostile values.
#[doc(hidden)]
pub fn rebuilt<'a>(props: &[(u16, Val<'a>)], recipients: &[Vec<(u16, Val<'a>)>]) -> Result<(Vec<u8>, mailcraft_model::Summary), String> {
    fn find<'a>(list: &[(u16, Val<'a>)], id: u16) -> Option<Val<'a>> {
        list.iter().find(|(i, _)| *i == id).map(|(_, v)| v.clone())
    }
    let rows: Vec<_> = recipients.iter().map(|r| move |id: u16| find(r, id)).collect();
    let it = item::item_of(&|id| find(props, id), &rows, Vec::new(), &crate::pst_pim::Named::default());
    let raw = rebuild::to_raw(&it, "fuzz@mailcraft.invalid")?;
    let key = MessageKey { account: "f1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let summary = rebuild::summary_of(&raw, &it, key, u32::try_from(raw.len()).unwrap_or(u32::MAX));
    Ok((raw, summary))
}

macro_rules! reader {
    ($name:ident, $file:ty, $store:ty, $message:ty, $attachment:ty) => {
        fn $name(pst: $file, out: &mut Out<'_>) -> io::Result<()> {
            pst.set_max_value_size(MAX_VALUE);
            let store = <$store>::read(Rc::new(pst))?;
            let any: Rc<dyn PstStore> = store.clone();
            let named = crate::pst_pim::Named::of(&*any);
            let read = |entry: &EntryId| -> io::Result<Item> {
                let m = <$message>::read(store.clone(), entry, None)?;
                let cp = message_code_page(&*m);
                let mut files = Vec::new();
                if let Some(table) = m.attachment_table() {
                    for row in table.rows_matrix() {
                        let node = NodeId::from(u32::from(row.id()));
                        match <$attachment>::read(m.clone(), node, None) {
                            Ok(a) => files.extend(attachment(&*a, 0, cp, &named)),
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

/// Bytes from a data file as the import reads them as text: an HTML body (`html`) in its internet
/// code page, or an 8-bit string in its message's. For the fuzz target.
#[doc(hidden)]
pub fn text_from_bytes(bytes: &[u8], code_page: u32, html: bool) -> String {
    props::decode(bytes, code_page, html)
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
                        out.skip(SkipReason::MessageUnreadable, 1);
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
                out.skip(SkipReason::ItemNotUsable, 1);
            }
        }
    }
}
