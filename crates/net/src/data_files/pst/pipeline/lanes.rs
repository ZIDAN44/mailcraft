//! The lanes: threads that each read items through a handle of their own on the data file, and
//! make each mail item's message, summary and compressed source, and its words for search, ready
//! for the writer to keep.
//!
//! Every job a lane takes is answered once, whatever happens to it: read, left out, or lost to a
//! panic in the reader (the lane then opens its handle again and goes on). A run of
//! [`MAX_PANICS`] panics stops the import: the file is taken as damaged from there on.

use std::cell::OnceCell;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Mutex, PoisonError};

use super::budget::{Budget, held};
use super::order::{Done, Step};
use super::out::sent_ids::{self, Given, SentIds};
use super::rebuild::{message, summary_of};
use super::*;

/// Panics in a row, in any lane, that stop the import.
pub(crate) const MAX_PANICS: usize = 32;

/// An item to read.
pub(crate) struct Job {
    pub(crate) seq: u64,
    pub(crate) node: u32,
    /// Its size as the file gives it (0: not given), and what it's charged.
    pub(crate) size: u64,
    pub(crate) charge: u64,
    /// The plan's folder it's in, and whether that's contacts or a calendar; and that folder's
    /// node and the item's row in it (from 0), where a checkpoint notes it.
    pub(crate) folder: usize,
    pub(crate) pim: bool,
    /// In Deleted Items or a folder under it (`PlanFolder::trash`).
    pub(crate) trash: bool,
    pub(crate) folder_node: u32,
    pub(crate) row: u64,
    /// In the Sent Items folder, read last: the replies read before it, which give its items
    /// with no id of their own theirs (`out/sent_ids.rs`).
    pub(crate) sent: Option<Arc<SentIds>>,
}

/// What became of an item.
pub(crate) enum Outcome {
    /// A message, ready to keep.
    Built(Box<Built>),
    /// A contact, calendar item, task or note.
    Pim(Box<Item>),
    /// Something in a mail folder that isn't mail (an appointment, a contact): not brought in.
    NotMail,
    Unreadable,
    /// The reader failed on it (a panic): never read again when the import goes on.
    Failed,
    NotRebuilt,
    /// Not read: the import is stopping.
    Stopped,
}

/// A message made from an item: its summary (its key is the writer's to give), its source ready
/// to keep, its words for search (when the import indexes), and what the report counts of it.
pub(crate) struct Built {
    pub(crate) summary: mailcraft_model::Summary,
    pub(crate) prepared: mailcraft_store::Prepared,
    pub(crate) doc: Option<mailcraft_store::search::Ready>,
    pub(crate) raw_len: usize,
    pub(crate) charset_unsupported: bool,
    pub(crate) body_not_converted: bool,
    pub(crate) without_address: usize,
    pub(crate) left_out: BTreeMap<SkipReason, usize>,
    /// The reply it is (its parent index's hash, and the id it answers), and the id it was
    /// given as a sent item, for the writer to note.
    pub(crate) reply: Option<(u64, String)>,
    pub(crate) given: Option<Given>,
    /// The local day it's flagged to be followed up by, and the day it was done.
    pub(crate) due: Option<i64>,
    pub(crate) done: Option<i64>,
}

/// What an import's threads share.
pub(crate) struct Ctx<'a> {
    pub(crate) path: &'a Path,
    pub(crate) opts: &'a ImportOptions,
    pub(crate) account: &'a str,
    /// Names this file in the message ids made for items that have none.
    pub(crate) filetag: String,
    pub(crate) packs: bool,
    /// Whether the messages' words are made for the account's search index.
    pub(crate) index: bool,
    pub(crate) budget: Arc<Budget>,
    pub(crate) tally: Arc<Tally>,
    /// Going on from a checkpoint: what the planner leaves out, and whether the file turned out
    /// not to be the one the checkpoint was of.
    pub(crate) resume: Option<super::resume::Skip>,
    pub(crate) changed: AtomicBool,
    /// Set to stop every thread: the writer is done (or failed), or the file is damaged.
    pub(crate) halt: AtomicBool,
    pub(crate) damaged: AtomicBool,
    panics: AtomicUsize,
    /// Lanes still reading, and whether the last went with work left to read.
    pub(crate) live: AtomicUsize,
    pub(crate) orphaned: AtomicBool,
    /// Links' files read, which stop once a drive is too slow to answer.
    pub(crate) links: item::LinkReads,
    /// Items the lanes have taken, for a test's faults.
    #[cfg(test)]
    pub(crate) items_taken: std::sync::atomic::AtomicU64,
}

impl<'a> Ctx<'a> {
    /// For a file imported into `account`, as `out` writes it.
    pub(crate) fn new(path: &'a Path, opts: &'a ImportOptions, account: &'a str, out: &Out<'_>) -> Self {
        Ctx {
            account,
            filetag: filetag(path),
            packs: out.store.packs(account),
            index: out.indexing(),
            budget: out.budget.clone(),
            tally: out.tally.clone(),
            ..Ctx::looking(path, opts)
        }
    }

    /// For a file only looked at ([`super::inspect`]): planned, and nothing written.
    pub(crate) fn looking(path: &'a Path, opts: &'a ImportOptions) -> Self {
        Ctx {
            path,
            opts,
            account: "",
            filetag: String::new(),
            packs: false,
            index: false,
            budget: Arc::new(Budget::new(0)),
            tally: Arc::default(),
            resume: None,
            changed: AtomicBool::new(false),
            halt: AtomicBool::new(false),
            damaged: AtomicBool::new(false),
            panics: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            orphaned: AtomicBool::new(false),
            links: item::LinkReads::default(),
            #[cfg(test)]
            items_taken: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Whether to stop: told to, or asked to by the user.
    pub(crate) fn stopped(&self) -> bool {
        self.halt.load(Ordering::Relaxed) || self.opts.stop.raised()
    }

    /// Stop every thread, waking the lanes waiting for room.
    pub(crate) fn stop(&self) {
        self.halt.store(true, Ordering::Relaxed);
        self.budget.wake();
    }

    /// How many lanes to read with.
    pub(crate) fn lanes(&self) -> usize {
        let n = if self.opts.lanes > 0 { self.opts.lanes } else { std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1) };
        n.clamp(1, MAX_LANES)
    }
}

/// Where the planner lets a test put a fault: nothing outside tests (a test's are in `tests`).
#[cfg(not(test))]
impl Ctx<'_> {
    /// As the planner looks at the folder that is `index` into the file.
    pub(crate) fn fault_looking(&self, _index: usize) -> io::Result<()> {
        Ok(())
    }

    /// The size an item of `size` is handed out with, after `handed` others (`sent`: one of the
    /// Sent Items folder, read last).
    pub(crate) fn fault_handing(&self, _handed: u64, _sent: bool, size: u64) -> u64 {
        size
    }
}

/// The lanes read at most this many items at once.
const MAX_LANES: usize = 12;

/// A tag for the data file at `path`: from the start of its header (which holds the file's own
/// unique number, changed with every write) and its size. The same file gives the same ids
/// imported again; another file, others.
fn filetag(path: &Path) -> String {
    let mut head = [0u8; 512];
    let mut got = 0;
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    if let Ok(mut f) = open::open_read(path) {
        while got < head.len() {
            match io::Read::read(&mut f, head.get_mut(got..).unwrap_or_default()) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
    }
    let mut h = mailcraft_model::hash::Fnv::new();
    h.add(head.get(..got).unwrap_or_default());
    h.add(&size.to_le_bytes());
    format!("{:08x}", h.value() >> 32)
}

/// The jobs, shared by the lanes: each takes the next in turn.
pub(crate) type Jobs = Mutex<Receiver<Job>>;

/// The next job, or none once the planner is done.
fn next_job(jobs: &Jobs) -> Option<Job> {
    jobs.lock().unwrap_or_else(PoisonError::into_inner).recv().ok()
}

/// A lane: reads jobs until there are none.
pub(crate) fn run(ctx: &Ctx<'_>, jobs: &Jobs, results: &Sender<Done>) {
    let Some(first) = next_job(jobs) else { return };
    lane_of_file(ctx, first, jobs, results);
}

/// `n` lanes are gone. The last (every one could fail to open the file, or not start): nothing
/// would take the planner's jobs, so the import stops, and if the planner had more to hand out
/// (it hasn't said it's done), the file wasn't read to its end.
pub(crate) fn gone(ctx: &Ctx<'_>, jobs: &Jobs, n: usize) {
    if ctx.live.fetch_sub(n, Ordering::AcqRel) == n {
        let done = matches!(jobs.lock().unwrap_or_else(PoisonError::into_inner).try_recv(), Err(TryRecvError::Disconnected));
        if !done && !ctx.stopped() {
            ctx.orphaned.store(true, Ordering::Relaxed);
        }
        ctx.stop();
    }
}

/// Answers a job when dropped, unless it was answered: a lane that unwinds with a job in hand
/// still answers it, so the writer, which applies items in order, never waits for one.
struct Answer<'a> {
    job: Option<Asked>,
    results: &'a Sender<Done>,
}

/// What the writer is told of a job, whatever came of it: its number, the plan's folder it's
/// in, its size as the file gives it, whether the folder is contacts or a calendar, and the
/// folder's node and the item's row in it.
struct Asked {
    seq: u64,
    folder: usize,
    size: u64,
    pim: bool,
    node: u32,
    row: u64,
}

impl<'a> Answer<'a> {
    fn new(job: &Job, results: &'a Sender<Done>) -> Self {
        let asked = Asked { seq: job.seq, folder: job.folder, size: job.size, pim: job.pim, node: job.folder_node, row: job.row };
        Answer { job: Some(asked), results }
    }

    fn send(&mut self, outcome: Outcome, charge: u64) {
        if let Some(Asked { seq, folder, size, pim, node, row }) = self.job.take() {
            let _ = self.results.send(Done { seq, step: Step::Item { folder, size, pim, node, row, outcome }, charge });
        }
    }
}

impl Drop for Answer<'_> {
    fn drop(&mut self) {
        self.send(Outcome::Unreadable, 0);
    }
}

/// Reads an item of the file by its node.
pub(crate) type ReadItem<'a> = dyn Fn(u32) -> io::Result<Item> + 'a;

/// The lane's loop over its jobs, from `first`: `read` reads an item, `reset` lets go of the
/// handle it reads through (it's opened again for the next).
pub(crate) fn work(ctx: &Ctx<'_>, first: Job, jobs: &Jobs, results: &Sender<Done>, read: &ReadItem<'_>, reset: &dyn Fn()) {
    let mut next = Some(first);
    while let Some(job) = next.take().or_else(|| next_job(jobs)) {
        let mut answer = Answer::new(&job, results);
        if ctx.stopped() || !ctx.budget.acquire(job.seq, job.charge, &|| ctx.stopped(), &ctx.tally) {
            answer.send(Outcome::Stopped, 0);
            continue;
        }
        let mut charge = job.charge;
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| one(ctx, &job, read, &mut charge))) {
            // Asked to stop meanwhile: the reads failed from then on (`chunked::stopping`), so
            // whatever came of the item, it's let go of, not counted, and read again going on.
            _ if ctx.opts.stop.raised() => Outcome::Stopped,
            Ok(o) => {
                ctx.panics.store(0, Ordering::Relaxed);
                o
            }
            Err(_) => {
                log::info!("an item could not be read: the reader failed on it");
                reset();
                if ctx.panics.fetch_add(1, Ordering::Relaxed).saturating_add(1) >= MAX_PANICS {
                    ctx.damaged.store(true, Ordering::Relaxed);
                    ctx.stop();
                }
                Outcome::Failed
            }
        };
        // What the writer holds from here is the source as kept with its words, or the contact or
        // calendar item as read (until it's added): the rest goes back now.
        let kept = match &outcome {
            Outcome::Built(b) => {
                let words = b.doc.as_ref().map_or(0, mailcraft_store::search::Ready::bytes);
                u64::try_from(b.prepared.kept_len().saturating_add(words)).unwrap_or(u64::MAX).min(charge)
            }
            Outcome::Pim(it) => weight(it).min(charge),
            _ => 0,
        };
        ctx.budget.release(charge.saturating_sub(kept));
        answer.send(outcome, kept);
    }
}

/// Read and make one item.
fn one(ctx: &Ctx<'_>, job: &Job, read: &ReadItem<'_>, charge: &mut u64) -> Outcome {
    #[cfg(test)]
    tests::fault(ctx, job);
    let mut it = match ctx.tally.time(Phase::Read, || read(job.node)) {
        // Read whole as the import was asked to stop: it isn't made.
        Ok(_) if ctx.opts.stop.raised() => return Outcome::Stopped,
        Ok(it) => it,
        Err(e) => {
            log::info!("an item could not be read: {e}");
            return Outcome::Unreadable;
        }
    };
    #[cfg(test)]
    tests::touch(ctx, job, &mut it);
    // A contact, calendar item, task, note or journal entry in a mail folder goes where one in a
    // folder of its own does; one in Deleted Items was thrown away, and is only counted.
    let pim = job.pim || (!job.trash && is_pim(&it.class));
    if !pim && !is_mail(&it.class) {
        return Outcome::NotMail;
    }
    // An item bigger than the file said waits for room for what it is (unless it's the one the
    // writer needs next).
    let need = held(weight(&it));
    if need > *charge {
        if !ctx.budget.acquire_more(job.seq, *charge, need - *charge, &|| ctx.stopped(), &ctx.tally) {
            return Outcome::Stopped;
        }
        *charge = need;
    }
    if pim {
        return Outcome::Pim(Box::new(it));
    }
    let fallback = format!("pst.{}.{}.{:x}@mailcraft.invalid", ctx.account, ctx.filetag, job.node);
    // A reply read before the sent items is noted; a sent item may be given the id one answered.
    let (reply, given) = match job.sent.as_deref() {
        None => (sent_ids::reply_of(&it), None),
        Some(ids) => (None, sent_ids::give(&mut it, ids)),
    };
    let mut built = build(it, &fallback, ctx.packs, ctx.index, &ctx.tally);
    if let Outcome::Built(b) = &mut built {
        b.reply = reply;
        b.given = given.map(|id| Given { id, fallback });
    }
    built
}

/// Whether an item of `class` comes in as mail: meeting requests and receipts are mail too, and so
/// are a post (an RSS feed's among them), a document, a recall (and its report), a sharing
/// invitation and a task request; appointments, contacts and tasks in a mail folder aren't
/// ([`is_pim`]).
pub(crate) fn is_mail(class: &str) -> bool {
    super::item::Class::of(class).is_mail()
}

/// Whether an item of `class` is a contact, contact group, calendar item, task, note or journal
/// entry, which goes to the address book, calendar, tasks or notes wherever it's found.
pub(crate) fn is_pim(class: &str) -> bool {
    super::item::Class::of(class).is_pim()
}

/// An item's bytes as read: its text and its files'.
fn weight(it: &Item) -> u64 {
    let files: usize = it
        .files
        .iter()
        .map(|f| match f {
            item::File::Data { data, .. } => data.len(),
            item::File::Message(m) => usize::try_from(weight(m)).unwrap_or(usize::MAX),
        })
        .fold(0, usize::saturating_add);
    let text = it.body.len().saturating_add(it.html.as_ref().map_or(0, String::len)).saturating_add(it.headers.len());
    let photo = it.photo.as_ref().map_or(0, |(_, data)| data.len());
    u64::try_from(files.saturating_add(text).saturating_add(photo)).unwrap_or(u64::MAX)
}

/// An item made into its message, summary and source ready to keep (`packs`: compressed, as
/// the data file keeps it), and its words for search (`index`), from the message as rebuilt.
pub(crate) fn build(mut it: Item, fallback: &str, packs: bool, index: bool, tally: &Tally) -> Outcome {
    // What the report counts, read before the item goes into its message.
    let (charset_unsupported, body_not_converted, without_address) = (it.charset_unsupported, it.body_not_converted, it.recipients_without_address);
    let (due, done) = (it.due, it.done);
    let left_out = std::mem::take(&mut it.left_out);
    let (raw, glance) = match tally.time(Phase::Build, || message(it, fallback)) {
        Ok(r) => r,
        Err(e) => {
            log::info!("a message could not be rebuilt: {e}");
            return Outcome::NotRebuilt;
        }
    };
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    // Its key is the writer's to give, with its place in its folder.
    let key = MessageKey { account: String::new(), folder: String::new(), validity: 1, uid: 0 };
    let summary = tally.time(Phase::Build, || summary_of(&raw, glance, key, size));
    let doc = index.then(|| tally.time(Phase::Index, || mailcraft_store::search::Ready::of(&mime::index_doc(&raw))));
    let raw_len = raw.len();
    let prepared = tally.time(Phase::Pack, || Store::prepare(packs, raw));
    Outcome::Built(Box::new(Built {
        summary,
        prepared,
        doc,
        raw_len,
        charset_unsupported,
        body_not_converted,
        without_address,
        left_out,
        reply: None,
        given: None,
        due,
        done,
    }))
}

fn lane_of_file(ctx: &Ctx<'_>, first: Job, jobs: &Jobs, results: &Sender<Done>) {
    // Read only, through a cache of its own.
    let source = match open::source(ctx.path, ctx.opts) {
        Ok(s) => s,
        Err(e) => {
            log::info!("a reader could not open the data file: {e}");
            // This lane's job goes back as unread; the other lanes read the rest.
            Answer::new(&first, results).send(Outcome::Unreadable, 0);
            return;
        }
    };
    #[cfg(test)]
    if let Some(tests::Fault::SlowReads(ms)) = ctx.opts.fault
        && let Ok(mut s) = source.try_borrow_mut()
    {
        s.slow = Some((Duration::from_millis(ms), tests::slow_reads_of(ctx.path)));
    }
    let store_of = |pst: PstFile| {
        pst.set_max_value_size(MAX_VALUE);
        PstStore::read(Rc::new(pst))
    };
    let named: OnceCell<crate::pst_pim::Named> = OnceCell::new();
    let held: RefCell<Option<Rc<PstStore>>> = RefCell::new(None);
    let read = |node: u32| -> io::Result<Item> {
        let store = {
            let mut held = held.try_borrow_mut().map_err(io::Error::other)?;
            if held.is_none() {
                *held = Some(store_of(PstFile::read_from(open::handle(&source))?)?);
            }
            let Some(store) = held.as_ref() else { return Err(io::Error::other("no handle to read with")) };
            store.clone()
        };
        let named = named.get_or_init(|| {
            let any: Rc<PstStore> = store.clone();
            crate::pst_pim::Named::of(&any)
        });
        let m = Message::read(store, node)?;
        Ok(item_with_files(&Held { m, named }, &|path, cap| ctx.links.read(path, cap)))
    };
    let reset = || {
        if let Ok(mut h) = held.try_borrow_mut() {
            *h = None;
        }
    };
    work(ctx, first, jobs, results, &read, &reset);
    ctx.tally.reading(&source);
}

/// A message, as the import reads it.
struct Held<'a> {
    m: Rc<Message>,
    named: &'a crate::pst_pim::Named,
}

/// A message's search key (`PidTagSearchKey`), which a copy of it keeps: an attached message
/// with the key of one holding it holds itself.
fn search_key(m: &Message) -> Option<Vec<u8>> {
    match m.properties().get(0x300B) {
        Some(PropertyValue::Binary(b)) if !b.buffer().is_empty() => Some(b.buffer().to_vec()),
        _ => None,
    }
}

impl Holder for Held<'_> {
    fn attachments(&self, top: bool, each: &mut dyn FnMut(&dyn Fn() -> Part<Self>) -> bool) {
        let cp = message_code_page(&self.m);
        // Which hidden files are left out depends on what the item is.
        let class = message_class(&self.m, cp);
        let Some(table) = self.m.attachment_table() else { return };
        for row in table.rows_matrix() {
            let read = || {
                let node = row.id();
                // The row itself is read only for an attachment the reader fails on.
                match Attachment::read(self.m.clone(), node) {
                    Ok(a) => match attachment(&a, cp, &class, top) {
                        Some(got) => Part::Got(got),
                        None => match a.embedded_message() {
                            Some(m) => Part::Message(Held { m, named: self.named }),
                            None => Part::Got(item::Got::Left(SkipReason::AttachedMessageUnreadable)),
                        },
                    },
                    Err(e) => Part::Got(unread_attachment(&e, table, row, cp, &class, top)),
                }
            };
            if !each(&read) {
                return;
            }
        }
    }

    fn item(&self, files: Vec<File>) -> Item {
        item(&self.m, files, self.named)
    }

    fn key(&self) -> Option<Vec<u8>> {
        search_key(&self.m)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fault a test puts in an import's threads.
    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Fault {
        /// The reader panics on the item that is this many into the file (from 0).
        PanicAt(u64),
        /// …on every item.
        PanicAlways,
        /// …on the item at this row of its folder (from 0), however many times it's read.
        PanicRow(u64),
        /// The planner panics once it has handed out this many items.
        PlannerAfter(u64),
        /// …or as it plans the folder that is this many into the file (from 0).
        PlanAt(usize),
        /// The writer panics on applying this many items.
        WriterAfter(u64),
        /// The import is cut off once the writer has applied this many items, as if the
        /// process ended there: nothing more is written, not even what an end writes.
        CutOffAfter(u64),
        /// The file says every item is a byte.
        Understate,
        /// Each item takes this many milliseconds longer to read (an import slow enough to be
        /// stopped part-way).
        Slow(u64),
        /// Each read the lanes make from the data file takes this many milliseconds (a slow
        /// drive: an item takes many of them).
        SlowReads(u64),
        /// Items at this row of their folder (from 0) or past it wait, before they're read, until
        /// the import is asked to stop: their folder stays part-way for as long as a test looks.
        HoldRow(u64),
        /// …until the import is thrown away: a pause meanwhile (the import's own, its data file
        /// let go of under it, among them) doesn't let them go, so the import can't end before.
        HoldRowUntilDiscard(u64),
    }

    /// The planner's faults ([`Fault::PlanAt`], [`Fault::PlannerAfter`], [`Fault::Understate`]).
    impl Ctx<'_> {
        /// Looking at the folder that is `index` into the file fails at [`Fault::PlanAt`].
        pub(crate) fn fault_looking(&self, index: usize) -> io::Result<()> {
            match self.opts.fault {
                Some(Fault::PlanAt(n)) if index == n => Err(io::Error::other("a test's plan fault")),
                _ => Ok(()),
            }
        }

        /// Handing out an item after `handed` others: the walk fails at [`Fault::PlannerAfter`]
        /// (the Sent Items folder read after it, `sent`, doesn't), and the file says every item
        /// is a byte at [`Fault::Understate`].
        pub(crate) fn fault_handing(&self, handed: u64, sent: bool, size: u64) -> u64 {
            match self.opts.fault {
                Some(Fault::PlannerAfter(n)) if handed == n && !sent => panic!("a test's planner fault"),
                Some(Fault::Understate) => 1,
                _ => size,
            }
        }
    }

    /// What a test does to each item of a data file read (whether it's one of the Sent Items
    /// folder's) before it's made: what the files its writer makes can't hold.
    pub(crate) type Touch = fn(bool, &mut Item);
    static TOUCH: std::sync::Mutex<Vec<(std::path::PathBuf, Touch)>> = std::sync::Mutex::new(Vec::new());

    /// Items read from the data file at `path` are touched by `touch` from here.
    pub(crate) fn touch_items(path: &Path, touch: Touch) {
        if let Ok(mut t) = TOUCH.lock() {
            t.push((path.to_path_buf(), touch));
        }
    }

    /// The reads the lanes have begun of a data file with [`Fault::SlowReads`], counted as each
    /// begins, by its path.
    static SLOW_READS: std::sync::Mutex<Vec<(std::path::PathBuf, Arc<AtomicU64>)>> = std::sync::Mutex::new(Vec::new());

    /// The count of the slow reads begun of the data file at `path`.
    pub(super) fn slow_reads_of(path: &Path) -> Arc<AtomicU64> {
        let Ok(mut r) = SLOW_READS.lock() else { return Arc::default() };
        if let Some((_, n)) = r.iter().find(|(p, _)| p == path) {
            return n.clone();
        }
        let n = Arc::new(AtomicU64::new(0));
        r.push((path.to_path_buf(), n.clone()));
        n
    }

    /// The slow reads the lanes have begun of the data file at `path`, in every import of it.
    pub(crate) fn slow_reads(path: &Path) -> u64 {
        SLOW_READS.lock().ok().and_then(|r| r.iter().find(|(p, _)| p == path).map(|(_, n)| n.load(Ordering::Relaxed))).unwrap_or(0)
    }

    pub(super) fn touch(ctx: &Ctx<'_>, job: &Job, it: &mut Item) {
        let found = TOUCH.lock().ok().and_then(|t| t.iter().find(|(p, _)| p == ctx.path).map(|(_, f)| *f));
        if let Some(touch) = found {
            touch(job.sent.is_some(), it);
        }
    }

    /// A panic the test asked for, at the item `job` is (the items are counted as the lanes take
    /// them, so which one is the nth depends on the lanes: one panics all the same).
    pub(super) fn fault(ctx: &Ctx<'_>, job: &Job) {
        let n = ctx.items_taken.fetch_add(1, Ordering::Relaxed);
        match ctx.opts.fault {
            Some(Fault::PanicAlways) => panic!("a test's reader fault"),
            Some(Fault::PanicAt(at)) if n == at => panic!("a test's reader fault"),
            Some(Fault::PanicRow(row)) if job.row == row => panic!("a test's reader fault"),
            Some(Fault::Slow(ms)) => std::thread::sleep(Duration::from_millis(ms)),
            Some(Fault::HoldRow(row)) if job.row >= row => {
                // A guard against hanging: a test that never stops it fails on its own.
                let end = std::time::Instant::now() + Duration::from_secs(120);
                while !ctx.opts.stop.raised() && std::time::Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Some(Fault::HoldRowUntilDiscard(row)) if job.row >= row => {
                let end = std::time::Instant::now() + Duration::from_secs(120);
                while ctx.opts.stop.how() != Some(StopHow::Discard) && std::time::Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            _ => {}
        }
    }
}
