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

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mailcraft_mime::{self as mime, Draft, InlinePicture, OutAttachment};
use mailcraft_model::job::SkipReason;
use mailcraft_model::pim::{LOCAL, NOTE_YELLOW, Note, PIM_DOC, Pim, StoredCard, StoredEvent, StoredNote};
use mailcraft_model::{EmailAddress, Flags, Folder, FolderRole, MessageKey, utf7};
use mailcraft_store::{FolderState, Store};
use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::{TableContext, TableRowData};
use outlook_pst::messaging::attachment::{AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment};
use outlook_pst::messaging::folder::Folder as PstFolder;
use outlook_pst::messaging::message::{AnsiMessage, Message, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, EntryId, Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};

mod chunked;
mod embedded;
mod item;
mod open;
mod out;
mod props;
mod rebuild;
#[cfg(test)]
mod tests;

use chunked::Shared;
use embedded::{Holder, Part, item_with_files};
use item::{File, Item, attachment, item, message_class, message_code_page, unread_attachment};
use open::Recycle;
use out::Out;
#[doc(hidden)]
pub use props::Val;
pub(crate) use props::text_of;

/// Folders nested deeper than this are left out.
const MAX_DEPTH: usize = 32;
/// Attachments larger than this are left out of the imported message.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// A value (a body, an attachment's bytes, a table's rows, a store's, folder's or the name map's
/// properties) the file says is larger than this is an error before any of it is read: a damaged
/// size can't make the import, or an export that reads the file it adds to, run out of memory.
pub(crate) const MAX_VALUE: u64 = 256 * 1024 * 1024;
/// Original headers longer than this are not kept.
const MAX_HEADERS: usize = 256 * 1024;
/// Embedded messages inside embedded messages… stop here.
const MAX_NESTING: usize = 3;
/// The files of one item, its attached messages' included, come to at most this; files past it
/// are left out.
pub(crate) const MAX_ITEM_BYTES: usize = 128 * 1024 * 1024;
/// Message ids kept from an item's References (as many as threading reads).
const MAX_IDS: usize = 256;
/// Child blocks kept of an item's conversation index.
const MAX_THREAD_BLOCKS: usize = 50;
/// Categories kept on an item.
const MAX_KEYWORDS: usize = 64;
/// Longest Thread-Topic written, in characters.
const MAX_TOPIC: usize = 512;
/// Longest message id kept, in bytes: `Message-ID: <…>` stays within a header line's 998.
const MAX_ID: usize = 900;
/// Longest address kept (RFC 5321's path, less its brackets), and name, in characters.
const MAX_ADDRESS: usize = 254;
const MAX_NAME: usize = 256;

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub folders: Vec<Folder>,
    pub messages: usize,
    /// Contacts, calendar items and notes.
    pub contacts: usize,
    pub events: usize,
    pub notes: usize,
    /// Items that couldn't be read or written; and how many for each reason, with messages that
    /// came in with part of them lost (`CharsetUnsupported`: text, or an attached message's, in a
    /// code page MailCraft can't read, ISO-2022-KR, of which only the ASCII was kept), and the
    /// attachments left out of the messages that came in (a link to a file elsewhere, one too
    /// large, one that couldn't be read), which `skipped` doesn't count.
    pub skipped: usize,
    pub skips: BTreeMap<SkipReason, usize>,
    /// Messages whose only body was Rich Text that couldn't be read: imported without a body.
    /// Logged, and not in `skipped`, since the message itself came in.
    pub body_not_converted: usize,
    /// Asked to stop before the file was read to its end ([`ImportOptions::cancel`]): what was
    /// read is in, the rest isn't.
    pub cancelled: bool,
    /// How long it took, and where the time went.
    pub stats: ImportStats,
    /// Why the folder list or the contacts and calendar couldn't be saved at the end, when they
    /// couldn't: the messages above came in all the same.
    pub last_save: Option<String>,
}

/// How far an import has got, as it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Items read from the file: it only goes up (a batch that can't be written moves its
    /// messages from `imported` to `skipped`, not out of this).
    pub read: usize,
    /// Messages written so far, and items left out.
    pub imported: usize,
    pub skipped: usize,
}

/// Where an import's time went, and how much it held and wrote: numbers only, for measuring.
/// Times are summed over the threads that did the work; with one lane they all run on one, and
/// nothing waits for another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportStats {
    /// The whole import, from opening the file to the seal at its end.
    pub total_ms: u64,
    /// Opening the file and finding its named properties, then opening each folder (which loads
    /// its tables of subfolders and items).
    pub plan_ms: u64,
    /// Reading items (their properties, recipients and attachments) from the file.
    pub read_ms: u64,
    /// Rebuilding them as messages, with their summaries, and as contacts and events.
    pub build_ms: u64,
    /// Compressing the sources for the data file.
    pub pack_ms: u64,
    /// Writing the sources.
    pub write_ms: u64,
    /// Writing the folder lists, the folders, the contacts and events, and sealing the file.
    pub commit_ms: u64,
    /// The writer waiting for items to be read, and readers waiting for room in the budget.
    pub writer_idle_ms: u64,
    pub budget_wait_ms: u64,
    /// The most source bytes read and not yet written at once (the batch waiting for the disk
    /// with the one just added), counted as rebuilt, before compression: the size a budget
    /// charges, so later figures compare with this one.
    pub peak_budget_bytes: u64,
    /// The sources' bytes as rebuilt, and as kept in the cache.
    pub raw_bytes: u64,
    pub kept_bytes: u64,
    /// Items read at once.
    pub lanes: u64,
    /// Bytes read from the data file, in so many reads; and how many times the handle that reads
    /// items was opened again to let go of its caches.
    pub read_bytes: u64,
    pub reads: u64,
    pub reopens: u64,
    /// What the handle that reads folders and their tables (never reopened) was thought to hold
    /// in its caches at the end.
    pub walk_cache_bytes: u64,
}

/// A stage of an import, timed.
#[derive(Clone, Copy)]
enum Phase {
    Plan,
    Read,
    Build,
    Pack,
    Write,
    Commit,
}

/// An import's running counts, which any of its threads may add to.
#[derive(Default)]
struct Tally {
    /// Nanoseconds in each [`Phase`].
    nanos: [AtomicU64; 6],
    peak: AtomicU64,
    raw: AtomicU64,
    kept: AtomicU64,
    read_bytes: AtomicU64,
    reads: AtomicU64,
    reopens: AtomicU64,
    walk_cache: AtomicU64,
}

impl Tally {
    /// `f`, its time counted to `phase`.
    fn time<T>(&self, phase: Phase, f: impl FnOnce() -> T) -> T {
        let began = Instant::now();
        let r = f();
        self.since(phase, began);
        r
    }

    /// The time since `began`, counted to `phase`.
    fn since(&self, phase: Phase, began: Instant) {
        if let Some(n) = self.nanos.get(phase as usize) {
            n.fetch_add(nanos(began.elapsed()), Ordering::Relaxed);
        }
    }

    /// A source of `raw` bytes kept in `kept`, with `held` bytes (as rebuilt) now waiting to be written.
    fn source(&self, raw: usize, kept: usize, held: usize) {
        let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        self.raw.fetch_add(wide(raw), Ordering::Relaxed);
        self.kept.fetch_add(wide(kept), Ordering::Relaxed);
        self.peak.fetch_max(wide(held), Ordering::Relaxed);
    }

    /// What was read of the data file through `source`, with the item handle reopened `reopens`
    /// times and the folder handle's caches thought to hold `walk_cache` bytes.
    fn reading(&self, source: &Shared, reopens: u64, walk_cache: u64) {
        if let Ok(s) = source.try_borrow() {
            self.read_bytes.fetch_add(s.bytes_read, Ordering::Relaxed);
            self.reads.fetch_add(s.reads, Ordering::Relaxed);
        }
        self.reopens.fetch_add(reopens, Ordering::Relaxed);
        self.walk_cache.fetch_max(walk_cache, Ordering::Relaxed);
    }

    fn snapshot(&self, total: Duration, lanes: u64) -> ImportStats {
        let ms = |phase: Phase| self.nanos.get(phase as usize).map_or(0, |n| n.load(Ordering::Relaxed) / 1_000_000);
        ImportStats {
            total_ms: nanos(total) / 1_000_000,
            plan_ms: ms(Phase::Plan),
            read_ms: ms(Phase::Read),
            build_ms: ms(Phase::Build),
            pack_ms: ms(Phase::Pack),
            write_ms: ms(Phase::Write),
            commit_ms: ms(Phase::Commit),
            writer_idle_ms: 0,
            budget_wait_ms: 0,
            peak_budget_bytes: self.peak.load(Ordering::Relaxed),
            raw_bytes: self.raw.load(Ordering::Relaxed),
            kept_bytes: self.kept.load(Ordering::Relaxed),
            lanes,
            read_bytes: self.read_bytes.load(Ordering::Relaxed),
            reads: self.reads.load(Ordering::Relaxed),
            reopens: self.reopens.load(Ordering::Relaxed),
            walk_cache_bytes: self.walk_cache.load(Ordering::Relaxed),
        }
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// How an import runs; the default is how the app imports.
#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// Items read at once, each on a thread of its own (0: as many as suit the computer).
    pub lanes: usize,
    /// Bytes read but not yet written that the import may hold (0: its own limit).
    pub budget_bytes: usize,
    /// Set from another thread to stop: nothing more is read, and what was read is written and
    /// kept, as when a file ends early.
    pub cancel: Option<Arc<AtomicBool>>,
    /// The reader's chunk of the file, in KiB (0: its own), the items read between reopenings
    /// of the handle that reads them (0: only once its caches hold too much), and how much its
    /// caches may hold before that, in bytes (0: its own limit): for tests and measuring.
    #[doc(hidden)]
    pub chunk_kb: usize,
    #[doc(hidden)]
    pub recycle: usize,
    #[doc(hidden)]
    pub handle_cache_bytes: u64,
}

/// Import the data file at `path` into `account`'s cache (an account with no server), as the app
/// does. Calls `progress` with how far it has got, every so often.
pub fn import(path: &Path, store: &Store, account: &str, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    import_with(path, store, account, &ImportOptions::default(), progress)
}

/// [`import`], run as `opts` say. The reader is one lane for now, holding at most a batch of
/// sources: `lanes`, `budget_bytes`, `chunk_kb` and `recycle` are taken for when it's more.
pub fn import_with(path: &Path, store: &Store, account: &str, opts: &ImportOptions, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    let mut out = Out::new(store, account, opts, progress);
    // `!BDN` then the client magic: `SM` a data file, `SO` Outlook's offline copy of a server
    // account (MS-PST 2.2.2.6).
    let mut head = [0u8; 10];
    if std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head.starts_with(b"!BDN")
        && head.get(8..10) == Some(b"SO")
    {
        return Err("this is Outlook's offline copy of a server account (.ost), not a data file: add the account itself to see its mail".into());
    }
    let recycle = Recycle::of(opts);
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<()> {
        // Read only, and kept from writers while it's read. Locked by Outlook, this is what fails:
        // said so below, rather than tried as the older format.
        let source = open::source(path, opts.chunk_kb)?;
        let (walk, pages) = open::handle(&source);
        match UnicodePstFile::read_from(walk) {
            Ok(pst) => read_unicode(pst, &pages, &source, recycle, &mut out),
            Err(_) => {
                out.tally.reading(&source, 0, 0);
                drop(source);
                // The older format, read from the start through a handle of its own.
                let source = open::source(path, opts.chunk_kb)?;
                let (walk, pages) = open::handle(&source);
                read_ansi(AnsiPstFile::read_from(walk)?, &pages, &source, recycle, &mut out)
            }
        }
    }));
    let failed: Option<String> = match read {
        Ok(Ok(())) => None,
        // Windows' sharing and lock violations: Outlook (or another program) has it open.
        Ok(Err(e)) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            Some("Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into())
        }
        // Nothing found at all: say why; otherwise keep what was read.
        Ok(Err(e)) if out.folders.is_empty() => Some(format!("not an Outlook data file, or it is damaged ({e})")),
        Ok(Err(e)) => {
            log::warn!("{}: the rest of the data file could not be read: {e}", path.display());
            None
        }
        Err(_) if out.folders.is_empty() => Some("the data file is damaged".into()),
        Err(_) => {
            log::warn!("{}: the data file is damaged; stopped reading", path.display());
            None
        }
    };
    match failed {
        // Batches may have gone in already (a lock met part-way): on the disk and sealed all the
        // same, as at the end.
        Some(e) => {
            out.seal();
            Err(e)
        }
        None => Ok(out.finish()),
    }
}

/// A mail item rebuilt as a message from its properties and its recipients' alone, as the
/// import writes it, with its summary: for the fuzz target, which hands it hostile values. Its
/// categories are property 0x8000.
#[doc(hidden)]
pub fn rebuilt<'a>(props: &[(u16, Val<'a>)], recipients: &[Vec<(u16, Val<'a>)>]) -> Result<(Vec<u8>, mailcraft_model::Summary), String> {
    fn find<'a>(list: &[(u16, Val<'a>)], id: u16) -> Option<Val<'a>> {
        list.iter().find(|(i, _)| *i == id).map(|(_, v)| v.clone())
    }
    let rows: Vec<_> = recipients.iter().map(|r| move |id: u16| find(r, id)).collect();
    let it = item::item_of(&|id| find(props, id), &rows, Vec::new(), &crate::pst_pim::Named::with_keywords(0x8000));
    let (raw, glance) = rebuild::message(it, "fuzz@mailcraft.invalid")?;
    let key = MessageKey { account: "f1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let summary = rebuild::summary_of(&raw, glance, key, u32::try_from(raw.len()).unwrap_or(u32::MAX));
    Ok((raw, summary))
}

macro_rules! reader {
    ($name:ident, $file:ty, $store:ty, $message:ty, $attachment:ty) => {
        fn $name(pst: $file, walked: &chunked::Pages, source: &Shared, recycle: Recycle, out: &mut Out<'_>) -> io::Result<()> {
            let store_of = |pst: $file| {
                pst.set_max_value_size(MAX_VALUE);
                <$store>::read(Rc::new(pst))
            };
            // The handle that reads folders and their tables; items are read through another,
            // opened again (with what was read of the file still cached) as its caches grow.
            let store = store_of(pst)?;
            let any: Rc<dyn PstStore> = store.clone();
            let named = crate::pst_pim::Named::of(&*any);
            // From the start: the file opened, as either kind when the first wasn't it.
            out.tally.since(Phase::Plan, out.began);
            let items: RefCell<Option<(Rc<$store>, chunked::Pages, usize)>> = RefCell::new(None);
            let reopens = Cell::new(0u64);
            let read = |entry: &EntryId| -> io::Result<Item> {
                let store = {
                    let mut held = items.try_borrow_mut().map_err(io::Error::other)?;
                    if held.as_ref().is_none_or(|(_, pages, n)| recycle.due(*n, pages)) {
                        // The old handle, and its caches, go before the new one is made.
                        if held.take().is_some() {
                            reopens.set(reopens.get().saturating_add(1));
                        }
                        let (reader, pages) = open::handle(source);
                        *held = Some((store_of(<$file>::read_from(reader)?)?, pages, 0));
                    }
                    let Some((store, _, n)) = held.as_mut() else { return Err(io::Error::other("no handle to read with")) };
                    *n = n.saturating_add(1);
                    store.clone()
                };
                let m = <$message>::read(store, entry, None)?;
                Ok(item_with_files(&Held { m, named: &named }))
            };
            let done = walk(&*any, &read, out);
            out.tally.reading(source, reopens.get(), walked.estimate());
            done
        }

        impl Holder for Held<'_, $message> {
            fn attachments(&self, each: &mut dyn FnMut(&dyn Fn() -> Part<Self>) -> bool) {
                let cp = message_code_page(&*self.m);
                // Which hidden files are left out depends on what the item is.
                let class = message_class(&*self.m, cp);
                let Some(table) = self.m.attachment_table() else { return };
                for row in table.rows_matrix() {
                    let read = || {
                        let node = NodeId::from(u32::from(row.id()));
                        // The row itself is read only for an attachment the reader fails on.
                        match <$attachment>::read(self.m.clone(), node, None) {
                            Ok(a) => match attachment(&*a, cp, &class) {
                                Some(got) => Part::Got(got),
                                None => match a.embedded_message() {
                                    Some(m) => Part::Message(Held { m, named: self.named }),
                                    None => Part::Got(item::Got::Left(SkipReason::AttachedMessageUnreadable)),
                                },
                            },
                            Err(e) => Part::Got(unread_attachment(&e, &**table, row, cp, &class)),
                        }
                    };
                    if !each(&read) {
                        return;
                    }
                }
            }

            fn item(&self, files: Vec<File>) -> Item {
                item(&*self.m, files, self.named)
            }

            fn key(&self) -> Option<Vec<u8>> {
                search_key(&*self.m)
            }
        }
    };
}

/// A message of the file's own type (`M`), as the import reads it.
struct Held<'a, M> {
    m: Rc<M>,
    named: &'a crate::pst_pim::Named,
}

/// A message's search key (`PidTagSearchKey`), which a copy of it keeps: an attached message
/// with the key of one holding it holds itself.
fn search_key(m: &dyn Message) -> Option<Vec<u8>> {
    match m.properties().get(0x300B) {
        Some(PropertyValue::Binary(b)) if !b.buffer().is_empty() => Some(b.buffer().to_vec()),
        _ => None,
    }
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
    let top = out.tally.time(Phase::Plan, || store.properties().ipm_sub_tree_entry_id().and_then(|e| store.open_folder(&e)))?;
    visit(store, &*top, None, 0, read, out);
    Ok(())
}

fn visit(store: &dyn PstStore, folder: &dyn PstFolder, parent: Option<&str>, depth: usize, read: &ReadItem<'_>, out: &mut Out<'_>) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(table) = folder.hierarchy_table() else { return };
    for row in table.rows_matrix() {
        if out.stopped() {
            return;
        }
        // Opening a folder reads its tables, so it's timed with the plan.
        let opened =
            out.tally.time(Phase::Plan, || store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| store.open_folder(&e)));
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
        let outbox = out.tally.time(Phase::Plan, || {
            sub.contents_table().is_none_or(|t| t.rows_matrix().next().is_none()) && special_folder(store, 0x35E2) == Some(u32::from(row.id()))
        });
        if outbox {
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
            } else {
                // A kind MailCraft has no place for (a journal, RSS feeds…): counted when it
                // holds anything. The folders under it are read as any others: mail comes in
                // beside it, and ones of its kind are counted too.
                let items = out.tally.time(Phase::Plan, || sub.contents_table().map_or(0, |t| t.rows_matrix().count()));
                out.leave_out_folder(items);
                visit(store, &*sub, parent, depth + 1, read, out);
            }
            continue;
        }
        let name = sub.properties().get(0x3001).and_then(text_of).unwrap_or_default();
        let path = out.add_folder(parent, &name);
        if let Some(contents) = sub.contents_table() {
            for row in contents.rows_matrix() {
                if out.stopped() {
                    break;
                }
                let item = out.tally.time(Phase::Read, || store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e)));
                match item {
                    Ok(i) => out.add_message(&path, i),
                    Err(e) => {
                        log::info!("a message could not be read: {e}");
                        out.skip(SkipReason::MessageUnreadable, 1);
                    }
                }
                out.item_read();
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
        if out.stopped() {
            return;
        }
        match out.tally.time(Phase::Read, || store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e))) {
            Ok(i) => out.add_pim(i),
            Err(e) => {
                log::info!("an item could not be read: {e}");
                out.skip(SkipReason::ItemUnreadable, 1);
            }
        }
        out.item_read();
    }
}
