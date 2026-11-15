//! Importing an Outlook data file (`.pst`) into an account kept on this computer.
//!
//! The file is read with MailCraft's own `.pst` crate (`mailcraft-pst`; MS-PST), which the export
//! writes with too: the folder tree under the top of the mailbox becomes folders, and each mail
//! item is written out again as a standard message (RFC 5322 / MIME) from its properties (MS-OXCMSG
//! / MS-OXPROPS), keeping the original headers when the item has them. Contacts, contact groups and
//! calendar items go to the account's address book and calendar (see `pst_pim`), notes and journal
//! entries to its notes, and tasks, with or without a due day, to its tasks.
//!
//! A data file is hostile input like any other: items that can't be read are skipped and counted,
//! and sizes are capped.
//!
//! An import runs on several threads at once: a planner reads the folders and hands out their
//! items (`plan`), lanes read the items and make their messages, each through a handle on the
//! file of its own (`lanes`), within a budget of bytes held at once (`budget`), and the writer,
//! the thread that called, keeps them in the order the planner gave (`order`).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mailcraft_mime::{self as mime, Draft, InlinePicture, OutAttachment};
use mailcraft_model::job::{PlannedFolder, SkipReason, StopHow};
use mailcraft_model::pim::{LOCAL, NOTE_YELLOW, Note, PIM_DOC, Pim, StoredCard, StoredEvent, StoredNote, StoredTask};
use mailcraft_model::{EmailAddress, Flags, Folder, FolderRole, MessageKey, utf7};
use mailcraft_pst::{Attachment, AttachmentData, Message, PropertyValue, PstFile, PstFolder, PstStore, Row, Table};
use mailcraft_store::{FolderState, Store};

mod budget;
mod chunked;
mod embedded;
mod fuzzing;
mod item;
mod lanes;
mod open;
mod order;
mod out;
mod plan;
pub(in crate::data_files) use plan::{Guard, MAX_ROWS};
mod props;
mod rebuild;
mod resume;
mod run;
mod stats;
#[cfg(test)]
mod tests;

pub use super::space::{LOW_SPACE, Space};
use chunked::Shared;
use embedded::{Holder, Part, item_with_files};
#[doc(hidden)]
pub use fuzzing::{rebuilt, repeating, repeating_task, text_from_bytes};
pub(crate) use item::given_name;
use item::{File, Item, attachment, item, message_class, message_code_page, unread_attachment};
#[cfg(test)]
pub(crate) use lanes::tests::Fault;
pub(crate) use open::locked_by_another;
use out::Out;
#[doc(hidden)]
pub use props::Val;
pub(crate) use props::{decode, one_off_address, one_off_entry_id, text_of, without_nuls};
use rebuild::{protected, report};
#[doc(hidden)]
pub use resume::checkpoint_from_bytes;
pub use resume::keep_as_is;
pub use run::import_into;
use run::opened;
pub use stats::ImportStats;
use stats::{Phase, Tally};

/// A message from outside a data file (an Outlook `.msg`, issue #38), as the import reads an
/// item: its properties, its recipients' and its files, each attached message one of these.
pub(crate) struct Loose<'a> {
    pub props: Vec<(u16, Val<'a>)>,
    pub recipients: Vec<Vec<(u16, Val<'a>)>>,
    pub files: Vec<LooseFile<'a>>,
}

/// One of a [`Loose`] message's files.
pub(crate) enum LooseFile<'a> {
    Data { name: String, mime: String, cid: String, data: Vec<u8> },
    Message(Box<Loose<'a>>),
}

/// The loose message as a standard one, as the import writes an item (`named`: the file's
/// named properties).
pub(crate) fn loose_message(m: Loose<'_>, named: &crate::pst_pim::Named) -> Result<Vec<u8>, String> {
    let it = item::loose_item(m, named, 0);
    rebuild::to_raw(it, "msg@mailcraft.invalid")
}

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
/// Attached messages inside attached messages… stop here: a guard against a damaged or hostile
/// file, which no real one comes near (the whole item is bounded besides, by `embedded`'s
/// `MAX_NODES` and [`MAX_ITEM_BYTES`]). Each level is an `.eml` inside the one holding it, sent
/// as it is (7bit or 8bit), which mail-parser reads without recursing; opened, each is read on
/// its own.
const MAX_NESTING: usize = 32;
/// The files of one item, its attached messages' included, come to at most this; files past it
/// are left out.
pub(crate) const MAX_ITEM_BYTES: usize = 128 * 1024 * 1024;
/// Categories kept on an item.
const MAX_KEYWORDS: usize = 64;
/// A meeting's guests' answers kept (one per recipient row).
const MAX_ANSWERS: usize = 10_000;
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
    /// Contacts, calendar items, notes and tasks.
    pub contacts: usize,
    pub events: usize,
    pub notes: usize,
    pub tasks: usize,
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
    /// Asked to stop before the file was read to its end ([`ImportOptions::stop`]), and how: what
    /// was read is in, the rest isn't (thrown away, nothing more was written); and the folder it
    /// was part-way through, when it was.
    pub stopped: Option<StopHow>,
    pub stopped_in: Option<String>,
    /// How long it took, and where the time went.
    pub stats: ImportStats,
    /// Why the folder list or the contacts and calendar couldn't be saved at the end, when they
    /// couldn't: the messages above came in all the same.
    pub last_save: Option<String>,
    /// The runs the import has taken, this one included (it went on from where it stopped), and
    /// the time the earlier ones took.
    pub runs: u32,
    pub earlier_ms: u64,
    /// It couldn't go on from where it stopped, and why: nothing was read or written.
    pub blocked: Option<mailcraft_model::job::Blocked>,
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
    /// Mail folders in the file (0 until it's known).
    pub folders: usize,
    /// Where this run began (an import going on from where it stopped): the items read and their
    /// bytes by then, and the time the earlier runs took.
    pub start_read: usize,
    pub start_bytes: u64,
    pub earlier_ms: u64,
}

/// Told how an import goes, as it goes.
pub trait ImportSink {
    /// How far it has got, every so often.
    fn progress(&mut self, p: Progress);
    /// A folder is done: written, listed, counted and in the account's folder list. Each is
    /// said once, after the folder it's in.
    fn folder(&mut self, _done: &Folder) {}
    /// The mail folders the file holds, as planned, before any comes in (each run says them).
    fn planned(&mut self, _folders: &[PlannedFolder]) {}
}

impl<F: FnMut(Progress)> ImportSink for F {
    fn progress(&mut self, p: Progress) {
        self(p);
    }
}

/// How an import runs; the default is how the app imports.
#[derive(Clone, Debug, Default)]
pub struct ImportOptions {
    /// Items read at once, each on a thread of its own (0: as many as suit the computer).
    pub lanes: usize,
    /// Bytes read but not yet written that the import may hold (0: its own limit).
    pub budget_bytes: usize,
    /// Raised from another thread to stop: nothing more is read. Cancelled, what was read is
    /// written and kept, as when a file ends early; paused, the same, but a folder part-way
    /// through isn't listed (the import goes on with it later); thrown away, nothing more is
    /// written at all.
    pub stop: Stop,
    /// The file as planned when it was looked at ([`inspect`]), when it hasn't changed since:
    /// its folders aren't walked again.
    pub planned: Option<Arc<Planned>>,
    /// Go on from where the import into this account stopped, as its checkpoint says (none:
    /// from the start). One done already is told as it ended, without reading the file.
    pub resume: bool,
    /// Where the room left on the drive is asked (none: the computer's own drives): the import
    /// stops itself, as if paused, when there's less than [`LOW_SPACE`] where it writes.
    #[doc(hidden)]
    pub space: Option<Arc<dyn Space>>,
    /// Leave the account's search index alone: the mail is indexed afterwards, by reading every
    /// message again. For a file read into a store of its own before it's added to an account
    /// (`worker/imports/rereading.rs`), and for measuring what indexing as it imports costs.
    #[doc(hidden)]
    pub no_index: bool,
    /// The reader's chunk of the file, in KiB, the data file's cache while the import writes to
    /// it, and the most a batch of sources holds, in bytes, as a test or a measurement sets them
    /// (0: the import's own), read through `ImportOptions::tuning`.
    #[cfg(test)]
    pub(crate) chunk_kb: usize,
    #[cfg(test)]
    pub(crate) cache_bytes: usize,
    #[cfg(test)]
    pub(crate) batch_bytes: usize,
    /// A fault for a test to put in the import's threads.
    #[cfg(test)]
    pub(crate) fault: Option<lanes::tests::Fault>,
    /// How often a test's import writes its checkpoint (none: as the app's does).
    #[cfg(test)]
    pub(crate) checkpoint_every: Option<Duration>,
    /// The folder a test's import takes for the file's Sent Items, by its name.
    #[cfg(test)]
    pub(crate) sent_folder: Option<&'static str>,
}

/// How an import reads and writes, as a test or a measurement sets it (0: the import's own,
/// which the app always has).
#[derive(Clone, Copy, Debug, Default)]
struct Tuning {
    /// The reader's chunk of the file, in KiB.
    chunk_kb: usize,
    /// The data file's cache while the import writes to it, and the most a batch of sources
    /// holds, in bytes.
    cache_bytes: usize,
    batch_bytes: usize,
}

impl ImportOptions {
    /// How the import reads and writes: as a test sets it, else its own.
    fn tuning(&self) -> Tuning {
        #[cfg(test)]
        return Tuning { chunk_kb: self.chunk_kb, cache_bytes: self.cache_bytes, batch_bytes: self.batch_bytes };
        #[cfg(not(test))]
        Tuning::default()
    }
}

/// How far an import is asked to stop. It's raised from any thread and never lowered: paused,
/// then cancelled, then thrown away, so a pause asked for after a discard (MailCraft closing
/// while an account is removed) leaves it thrown away.
#[derive(Clone, Debug, Default)]
pub struct Stop(Arc<AtomicU8>);

impl Stop {
    /// Ask to stop as `how` says, unless already asked to stop further.
    pub fn raise(&self, how: StopHow) {
        let level = match how {
            StopHow::Pause => 1,
            StopHow::Cancel => 2,
            StopHow::Discard => 3,
        };
        self.0.fetch_max(level, Ordering::AcqRel);
    }

    /// How it's asked to stop; none: it isn't.
    pub fn how(&self) -> Option<StopHow> {
        match self.0.load(Ordering::Acquire) {
            0 => None,
            1 => Some(StopHow::Pause),
            2 => Some(StopHow::Cancel),
            _ => Some(StopHow::Discard),
        }
    }

    pub fn raised(&self) -> bool {
        self.0.load(Ordering::Relaxed) != 0
    }
}

/// Said when Windows refuses the file to be read (a sharing or lock violation): Outlook, or
/// another program, has it open.
const LOCKED: &str = "Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again";

/// [`import_with`] as the app runs it: for tests.
#[cfg(test)]
pub(crate) fn import(path: &Path, store: &Store, account: &str, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    import_with(path, store, account, &ImportOptions::default(), progress)
}

/// Import the data file at `path` into `account`'s cache (an account with no server), run as
/// `opts` say. Calls `progress` with how far it has got, every so often.
pub fn import_with(path: &Path, store: &Store, account: &str, opts: &ImportOptions, progress: &mut dyn FnMut(Progress)) -> Result<Imported, String> {
    import_into(path, store, account, opts, &mut |p| progress(p))
}

/// A data file planned when it was looked at, before it's imported ([`inspect`]): handed to its
/// import ([`ImportOptions::planned`]), which then doesn't walk the file's folders a second time.
/// Only for the file as it was then: the caller checks it hasn't changed since.
pub struct Planned {
    plan: plan::Plan,
    /// Messages in its mail folders, those folders, and the items in its contacts, calendar,
    /// tasks, notes and journal.
    pub messages: u64,
    pub folders: u32,
    pub pim_items: u64,
    /// The items' own sizes, summed (what the progress goes by).
    pub bytes: u64,
    /// The file's own record key, in hex ("" when it has none).
    pub record_key: String,
}

impl std::fmt::Debug for Planned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Planned")
            .field("messages", &self.messages)
            .field("folders", &self.folders)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// Plan the data file at `path` before it's imported: what it holds, as the import would bring
/// it in. Read only; nothing is written anywhere.
pub fn inspect(path: &Path) -> Result<Planned, String> {
    let opts = ImportOptions::default();
    let ctx = lanes::Ctx::looking(path, &opts);
    let plan = match opened(&ctx).and_then(|(store, _)| plan::plan(&store, &ctx)) {
        Ok(plan) => plan,
        Err(e) if open::locked_by_another(&e) => return Err(LOCKED.into()),
        Err(e) => return Err(format!("not an Outlook data file, or it is damaged ({e})")),
    };
    let rows = |kind: plan::Kind| plan.folders.iter().filter(|f| f.kind == kind).fold(0u64, |n, f| n.saturating_add(f.rows));
    let (messages, pim_items) = (rows(plan::Kind::Mail), rows(plan::Kind::Pim));
    let folders = u32::try_from(plan.folders.iter().filter(|f| f.kind == plan::Kind::Mail).count()).unwrap_or(u32::MAX);
    Ok(Planned { messages, folders, pim_items, bytes: plan.bytes, record_key: plan.record_key.clone(), plan })
}

/// The calendar items among the events read: a changed occurrence of a repeating one is an
/// event of its own, not another item.
fn calendar_items(pim: &Pim) -> usize {
    pim.events.iter().filter(|e| e.event.recurrence_id.is_none()).count()
}
