//! Importing an Outlook data file (`.pst`) into an account kept on this computer.
//!
//! The file is read with Microsoft's MIT-licensed `outlook-pst` crate (MS-PST): the folder tree
//! under the top of the mailbox becomes folders, and each mail item is written out again as a
//! standard message (RFC 5322 / MIME) from its properties (MS-OXCMSG / MS-OXPROPS), keeping the
//! original headers when the item has them. Contacts, contact groups and calendar items go to the
//! account's address book and calendar (see `pst_pim`), notes and journal entries to its notes;
//! tasks show in the calendar on their due day (MailCraft has no task list), and those without
//! one are left out.
//!
//! A data file is hostile input like any other: the reader runs behind `catch_unwind`, items that
//! can't be read are skipped and counted, and sizes are capped.
//!
//! An import runs on several threads at once: a planner reads the folders and hands out their
//! items (`plan`), lanes read the items and make their messages, each through a handle on the
//! file of its own (`lanes`), within a budget of bytes held at once (`budget`), and the writer,
//! the thread that called, keeps them in the order the planner gave (`order`).

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
use outlook_pst::messaging::store::{AnsiStore, Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};

mod budget;
mod chunked;
mod embedded;
mod item;
mod lanes;
mod open;
mod order;
mod out;
mod plan;
mod props;
mod protected;
mod rebuild;
mod report;
#[cfg(test)]
mod tests;

use chunked::Shared;
use embedded::{Holder, Part, item_with_files};
use item::{File, Item, attachment, item, message_class, message_code_page, unread_attachment};
use open::Recycle;
use out::Out;
#[doc(hidden)]
pub use props::Val;
pub(crate) use props::{one_off_address, text_of};

/// Folders nested deeper than this are left out.
const MAX_DEPTH: usize = 32;
/// Attachments larger than this are left out of the imported message.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// The most an item is charged for before it's read, whatever the file says it weighs.
const MAX_CHARGE: u64 = 128 * 1024 * 1024;
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
    /// Items in the file (0 until it's known), and the bytes it gives the items read so far,
    /// and all of them.
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Told how an import goes, as it goes.
pub trait ImportSink {
    /// How far it has got, every so often.
    fn progress(&mut self, p: Progress);
    /// A folder is done: written, listed, counted and in the account's folder list. Each is
    /// said once, after the folder it's in.
    fn folder(&mut self, _done: &Folder) {}
}

impl<F: FnMut(Progress)> ImportSink for F {
    fn progress(&mut self, p: Progress) {
        self(p);
    }
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
    /// The writer waiting for something to apply.
    Idle,
    /// Lanes waiting for room in the budget.
    Wait,
}

/// An import's running counts, which any of its threads may add to.
#[derive(Default)]
struct Tally {
    /// Nanoseconds in each [`Phase`].
    nanos: [AtomicU64; 8],
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

    /// The writer waited since `began`.
    fn idle(&self, began: Instant) {
        self.since(Phase::Idle, began);
    }

    /// A lane waited for room since `began`.
    fn waited(&self, began: Instant) {
        self.since(Phase::Wait, began);
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
            writer_idle_ms: ms(Phase::Idle),
            budget_wait_ms: ms(Phase::Wait),
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
    /// A fault for a test to put in the import's threads.
    #[cfg(test)]
    pub(crate) fault: Option<lanes::tests::Fault>,
}

/// Import the data file at `path` into `account`'s cache (an account with no server), as the app
/// does. Calls `progress` with how far it has got, every so often.
pub fn import(path: &Path, store: &Store, account: &str, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    import_with(path, store, account, &ImportOptions::default(), progress)
}

/// [`import`], run as `opts` say.
pub fn import_with(path: &Path, store: &Store, account: &str, opts: &ImportOptions, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    import_into(path, store, account, opts, &mut |p| progress(p))
}

/// [`import_with`], telling `sink` how it goes and of each folder as it's done.
pub fn import_into(path: &Path, store: &Store, account: &str, opts: &ImportOptions, sink: &mut dyn ImportSink) -> Result<Imported, String> {
    let mut out = Out::new(store, account, opts, sink);
    // `!BDN` then the client magic: `SM` a data file, `SO` Outlook's offline copy of a server
    // account (MS-PST 2.2.2.6).
    let mut head = [0u8; 10];
    if std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head.starts_with(b"!BDN")
        && head.get(8..10) == Some(b"SO")
    {
        return Err("this is Outlook's offline copy of a server account (.ost), not a data file: add the account itself to see its mail".into());
    }
    // Kept from writers from here until the import is done, whichever handles are reading it
    // meanwhile (they all go before the writer is). A file this can't open, the planner can't
    // either, and says why.
    let _held = open::open_read(path).ok();
    let ctx = lanes::Ctx::new(path, opts, account, &out);
    let lanes = ctx.lanes();
    out.lanes = u64::try_from(lanes).unwrap_or(u64::MAX);
    let (job_tx, job_rx) = std::sync::mpsc::sync_channel(plan::JOBS);
    let jobs: lanes::Jobs = std::sync::Mutex::new(job_rx);
    let (tx, rx) = std::sync::mpsc::channel();
    let (planned, wrote) = std::thread::scope(|s| {
        let (ctx, jobs) = (&ctx, &jobs);
        // Whatever becomes of the writer (or of starting the threads), every other thread stops
        // when it does.
        struct StopAll<'c, 'a>(&'c lanes::Ctx<'a>);
        impl Drop for StopAll<'_, '_> {
            fn drop(&mut self) {
                self.0.stop();
            }
        }
        let stop_all = StopAll(ctx);
        let feed = plan::Feed { seq: 0, jobs: job_tx, results: tx.clone() };
        let planner = std::thread::Builder::new()
            .spawn_scoped(s, move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| planner(ctx, feed))))
            .map_err(|e| log::warn!("import: the planner could not start: {e}"))
            .ok();
        ctx.live.store(lanes, Ordering::Relaxed);
        let mut started = 0;
        if planner.is_some() {
            for _ in 0..lanes {
                let tx = tx.clone();
                let lane = std::thread::Builder::new().spawn_scoped(s, move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lanes::run(ctx, jobs, &tx)));
                    lanes::gone(ctx, jobs, 1);
                });
                if let Err(e) = lane {
                    log::warn!("import: a reader could not start: {e}");
                    break;
                }
                started += 1;
            }
        }
        // The lanes that didn't start are gone too: none at all, and the import stops.
        if started < lanes {
            lanes::gone(ctx, jobs, lanes - started);
        }
        drop(tx);
        let wrote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| order::write(&mut out, ctx, &rx)));
        drop(stop_all);
        drop(rx);
        (planner.map(|p| p.join().unwrap_or_else(Err)), wrote)
    });
    let Some(planned) = planned else {
        out.seal();
        return Err("the import could not start: the computer is short of resources; try again".into());
    };
    if wrote.is_err() {
        log::warn!("{}: the import stopped writing; what was written is kept", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    if ctx.orphaned.load(Ordering::Relaxed) {
        log::warn!("{}: no reader was left to read the data file; stopped reading", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    if ctx.damaged.load(Ordering::Relaxed) {
        log::warn!("{}: the data file is damaged; stopped reading", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    let failed: Option<String> = match planned {
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
            out.lose(SkipReason::StoppedEarly, 1);
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

/// The planner: opens the file (as the Unicode kind, else the older ANSI), plans it, and hands
/// out its work.
fn planner(ctx: &lanes::Ctx<'_>, mut feed: plan::Feed) -> io::Result<()> {
    let began = Instant::now();
    // Read only, and kept from writers while it's read. Locked by Outlook, this is what fails:
    // said so above, rather than tried as the older format.
    let source = open::source(ctx.path, ctx.opts.chunk_kb)?;
    let (walk, pages) = open::handle(&source);
    let store: Rc<dyn PstStore> = match UnicodePstFile::read_from(walk) {
        Ok(pst) => {
            ctx.format.store(1, Ordering::Release);
            pst.set_max_value_size(MAX_VALUE);
            UnicodeStore::read(Rc::new(pst))?
        }
        Err(_) => {
            ctx.tally.reading(&source, 0, 0);
            drop(source);
            // The older format, read from the start through a handle of its own.
            let source = open::source(ctx.path, ctx.opts.chunk_kb)?;
            let (walk, pages) = open::handle(&source);
            let pst = AnsiPstFile::read_from(walk)?;
            ctx.format.store(2, Ordering::Release);
            pst.set_max_value_size(MAX_VALUE);
            let store: Rc<dyn PstStore> = AnsiStore::read(Rc::new(pst))?;
            return run_plan(ctx, &*store, &mut feed, began, &source, &pages);
        }
    };
    run_plan(ctx, &*store, &mut feed, began, &source, &pages)
}

fn run_plan(
    ctx: &lanes::Ctx<'_>,
    store: &dyn PstStore,
    feed: &mut plan::Feed,
    began: Instant,
    source: &Shared,
    pages: &chunked::Pages,
) -> io::Result<()> {
    let planned = plan::plan(store, ctx);
    ctx.tally.since(Phase::Plan, began);
    let planned = planned?;
    plan::stream(store, &planned, feed, ctx);
    ctx.tally.reading(source, 0, pages.estimate());
    Ok(())
}

/// A mail item rebuilt as a message from its properties and its recipients' alone, as the
/// import writes it, with its summary: for the fuzz target, which hands it hostile values. Its
/// categories are property 0x8000; its one file, when it has one, the bytes of 0x3701 (an
/// attachment's data), of the type 0x370E says.
#[doc(hidden)]
pub fn rebuilt<'a>(props: &[(u16, Val<'a>)], recipients: &[Vec<(u16, Val<'a>)>]) -> Result<(Vec<u8>, mailcraft_model::Summary), String> {
    fn find<'a>(list: &[(u16, Val<'a>)], id: u16) -> Option<Val<'a>> {
        list.iter().find(|(i, _)| *i == id).map(|(_, v)| v.clone())
    }
    let rows: Vec<_> = recipients.iter().map(|r| move |id: u16| find(r, id)).collect();
    let files = match find(props, 0x3701) {
        Some(Val::Bin(data)) => {
            let mime = find(props, 0x370E).and_then(Val::into_text).unwrap_or_default();
            vec![File::Data { name: "smime.p7m".into(), mime, cid: String::new(), data: data.to_vec() }]
        }
        _ => Vec::new(),
    };
    let it = item::item_of(&|id| find(props, id), &rows, files, &crate::pst_pim::Named::with_keywords(0x8000));
    let (raw, glance) = rebuild::message(it, "fuzz@mailcraft.invalid")?;
    let key = MessageKey { account: "f1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let summary = rebuild::summary_of(&raw, glance, key, u32::try_from(raw.len()).unwrap_or(u32::MAX));
    Ok((raw, summary))
}

/// Bytes from a data file as the import reads them as text: an HTML body (`html`) in its internet
/// code page, or an 8-bit string in its message's. For the fuzz target.
#[doc(hidden)]
pub fn text_from_bytes(bytes: &[u8], code_page: u32, html: bool) -> String {
    props::decode(bytes, code_page, html)
}
