//! What a data file holds, read before any item is: its folders in the order they come in, what
//! each is (mail, contacts or calendar, or a kind left out), and how many items and bytes each
//! holds (the totals the progress goes by). Then the same folders handed out as work: each
//! mail folder's start and end to the writer, and one job per item to the lanes, all numbered in
//! the order the writer applies them.
//!
//! The file's tables are hostile: a folder listed under itself (or under one below it) is read
//! once, nesting stops at [`MAX_DEPTH`], and the folders, the items of each and the items in all
//! are capped. A folder the reader fails on is left out, and the plan goes on.

use std::collections::HashSet;
use std::sync::mpsc::{RecvTimeoutError, Sender, SyncSender, TrySendError};

use super::lanes::{Ctx, Job};
use super::order::{Done, Outlined, Step};
use super::out::sent_ids::SentIds;
use super::*;

/// Jobs waiting for a lane, at most.
pub(super) const JOBS: usize = 1024;
/// Folders read, at most; past this the rest of the file isn't.
pub(super) const MAX_FOLDERS: usize = 100_000;
/// Items handed out, at most.
const MAX_ITEMS: u64 = 10_000_000;
/// Items read of one folder, at most.
pub(super) const MAX_ROWS: usize = 1_000_000;
/// PidTagMessageSize: an item's size as the file gives it.
const MESSAGE_SIZE: u16 = 0x0E08;

/// What a folder is brought in as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Mail,
    /// Contacts, a calendar, tasks, notes or a journal: their items go to the address book and calendar.
    Pim,
    /// A kind MailCraft has no place for (a configuration folder, a shortcuts or files list): its
    /// items are counted.
    LeftOut,
}

pub(super) struct PlanFolder {
    pub(super) node: u32,
    /// The mail folder it goes under (by its place in [`Plan::folders`]); none at the top.
    pub(super) parent: Option<usize>,
    pub(super) name: String,
    pub(super) kind: Kind,
    pub(super) rows: u64,
    pub(super) bytes: u64,
    /// The file's Deleted Items folder, or one under it: a contact, calendar item, task or note
    /// there was thrown away, and stays out of the address book, calendar, tasks and notes.
    pub(super) trash: bool,
}

#[derive(Default)]
pub(super) struct Plan {
    pub(super) folders: Vec<PlanFolder>,
    /// Items in the folders brought in, and the bytes the file gives them.
    pub(super) items: u64,
    pub(super) bytes: u64,
    /// Folders the plan left out, by reason.
    pub(super) lost: BTreeMap<SkipReason, usize>,
    /// Past [`MAX_FOLDERS`] or [`MAX_ITEMS`]: nothing more is planned.
    full: bool,
    /// The file's own record key (PidTagRecordKey of its store), in hex.
    pub(super) record_key: String,
    /// The Sent Items folder's node, read last (`stream`).
    pub(super) sent: Option<u32>,
}

/// The folders a plan has taken, against the file going round in circles or on for ever.
#[derive(Default)]
pub(super) struct Guard {
    seen: HashSet<u32>,
}

/// Why a folder isn't read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refused {
    /// Seen already: a folder listed twice, or under itself.
    Again,
    TooDeep,
    /// Past [`MAX_FOLDERS`]: nothing more is read.
    TooMany,
}

impl Guard {
    /// Whether the folder at `node`, `depth` below the top, is read.
    pub(super) fn admit(&mut self, node: u32, depth: usize) -> Result<(), Refused> {
        if depth >= MAX_DEPTH {
            return Err(Refused::TooDeep);
        }
        if self.seen.len() >= MAX_FOLDERS {
            return Err(Refused::TooMany);
        }
        if !self.seen.insert(node) { Err(Refused::Again) } else { Ok(()) }
    }
}

impl Plan {
    /// What the file holds, in one number: each folder brought in, by its node, kind, items and
    /// their bytes, and the file's record key. A file changed where its items aren't (Outlook
    /// writing its header) has the same; one with an item more or less, another.
    pub(super) fn fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut add = |bytes: &[u8]| {
            for b in bytes {
                h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
            }
        };
        add(self.record_key.as_bytes());
        for f in self.folders.iter().filter(|f| f.kind != Kind::LeftOut) {
            add(&f.node.to_le_bytes());
            add(&[u8::from(f.kind == Kind::Mail)]);
            add(&f.rows.to_le_bytes());
            add(&f.bytes.to_le_bytes());
        }
        h
    }

    fn lose(&mut self, reason: SkipReason) {
        let n = self.lost.entry(reason).or_default();
        *n = n.saturating_add(1);
    }

    /// Nothing more is planned, said once.
    fn fill(&mut self) {
        if !self.full {
            self.full = true;
            self.lose(SkipReason::StoppedEarly);
        }
    }
}

/// The plan of the file `store` reads (stopping early, with what was planned, when the import
/// stops).
pub(super) fn plan(store: &PstStore, ctx: &Ctx<'_>) -> io::Result<Plan> {
    let top = store.ipm_subtree().and_then(|n| store.open_folder(n))?;
    let mut plan = Plan { record_key: record_key(store), ..Plan::default() };
    let mut guard = Guard::default();
    let special = Special { outbox: special_folder(store, 0x35E2), wastebasket: special_folder(store, 0x35E3) };
    visit(store, &top, (None, false), 0, &special, &mut guard, &mut plan, ctx);
    plan.sent = special_folder(store, 0x35E4);
    // A test's file has no Sent Items folder of its own: one is named.
    #[cfg(test)]
    if let Some(name) = ctx.opts.sent_folder {
        plan.sent = plan.folders.iter().find(|f| f.name == name).map(|f| f.node);
    }
    Ok(plan)
}

/// What a folder is and holds.
struct Looked {
    sub: Rc<PstFolder>,
    rows: u64,
    bytes: u64,
    kind: Kind,
    name: String,
}

/// The store's special folders the plan treats as such: the Outbox and Deleted Items.
struct Special {
    outbox: Option<u32>,
    wastebasket: Option<u32>,
}

/// Plan the folders under `folder`: they go under `parent` (`.0`), and are thrown away (`.1`)
/// when it's Deleted Items or under it.
#[allow(clippy::too_many_arguments)]
fn visit(
    store: &PstStore,
    folder: &PstFolder,
    (parent, trash): (Option<usize>, bool),
    depth: usize,
    special: &Special,
    guard: &mut Guard,
    plan: &mut Plan,
    ctx: &Ctx<'_>,
) {
    let nodes = folder.hierarchy_table().map(|t| t.rows_matrix().take(MAX_FOLDERS).map(|row| row.id()).collect::<Vec<_>>()).unwrap_or_default();
    for node in nodes {
        if ctx.stopped() || plan.full {
            return;
        }
        match guard.admit(node, depth) {
            Ok(()) => {}
            Err(Refused::Again) => {
                log::info!("a folder is listed twice in the data file; read once");
                plan.lose(SkipReason::FolderUnreadable);
                continue;
            }
            Err(Refused::TooDeep) => {
                plan.lose(SkipReason::FolderTooDeep);
                continue;
            }
            Err(Refused::TooMany) => {
                plan.fill();
                return;
            }
        }
        let Looked { sub, rows, bytes, kind, name } = match look(store, node, plan.folders.len(), ctx) {
            Ok(l) => l,
            Err(e) => {
                log::info!("a folder could not be read: {e}");
                plan.lose(SkipReason::FolderUnreadable);
                continue;
            }
        };
        // Outlook's own Outbox, when it has nothing waiting (MailCraft has an Outbox of its own).
        if rows == 0 && special.outbox == Some(node) {
            continue;
        }
        let index = plan.folders.len();
        let trash = trash || special.wastebasket == Some(node);
        plan.folders.push(PlanFolder { node, parent, name, kind, rows, bytes, trash });
        if kind != Kind::LeftOut {
            plan.items = plan.items.saturating_add(rows);
            plan.bytes = plan.bytes.saturating_add(bytes);
            if plan.items >= MAX_ITEMS {
                plan.fill();
                return;
            }
        }
        // The folders under a mail folder go in it; those under any other, beside it (mail
        // comes in, and ones of its kind are read too).
        let under = if kind == Kind::Mail { Some(index) } else { parent };
        visit(store, &sub, (under, trash), depth + 1, special, guard, plan, ctx);
    }
}

/// The folder at `node` (the plan's `index`th) opened, with what it is and holds.
fn look(store: &PstStore, node: u32, index: usize, ctx: &Ctx<'_>) -> io::Result<Looked> {
    #[cfg(test)]
    if let Some(super::lanes::tests::Fault::PlanAt(n)) = ctx.opts.fault
        && index == n
    {
        return Err(io::Error::other("a test's plan fault"));
    }
    let _ = (index, ctx);
    let sub = store.open_folder(node)?;
    let (rows, bytes) = sub.contents_table().map_or((0, 0), |t| sizes(t).fold((0u64, 0u64), |(n, b), (_, s)| (n + 1, b.saturating_add(s))));
    let kind = classify(&sub.properties().get(0x3613).and_then(text_of).unwrap_or_default());
    let name = sub.properties().get(0x3001).and_then(text_of).unwrap_or_default();
    Ok(Looked { sub, rows, bytes, kind, name })
}

/// What a folder of class `class` comes in as: no class is mail, in old files.
/// Outlook's RSS Feeds folder (`IPF.Note.OutlookHomepage`) and its feeds' folders are mail
/// folders of posts (`IPM.Post.Rss`): they come in as such, the feeds under RSS Feeds.
fn classify(class: &str) -> Kind {
    let mail = class.is_empty() || ["IPF.Note", "IPF.Imap", "IPF.Rss"].iter().any(|c| class.starts_with(c));
    if mail {
        Kind::Mail
    } else if ["IPF.Contact", "IPF.Appointment", "IPF.Task", "IPF.StickyNote", "IPF.Journal"].iter().any(|k| class.starts_with(k)) {
        Kind::Pim
    } else {
        Kind::LeftOut
    }
}

/// A contents table's items, the first [`MAX_ROWS`]: each one's node and the size the file gives
/// it (0 when it gives none, or one that isn't a size).
fn sizes<'a>(table: &'a Table) -> impl Iterator<Item = (u32, u64)> + 'a {
    let column = table.columns().iter().position(|c| c.prop_id() == MESSAGE_SIZE);
    table.rows_matrix().take(MAX_ROWS).map(move |row| {
        let size = column.and_then(|i| table.value(row, i)).and_then(|v| match v {
            PropertyValue::Integer32(n) => u64::try_from(n).ok(),
            _ => None,
        });
        (row.id(), size.unwrap_or(0))
    })
}

/// Hands the plan out: steps to the writer, jobs to the lanes, numbered as one sequence.
pub(super) struct Feed {
    pub(super) seq: u64,
    pub(super) jobs: SyncSender<Job>,
    pub(super) results: Sender<Done>,
    /// Items handed out so far, and whether that came to [`MAX_ITEMS`] (said once).
    pub(super) handed: u64,
    pub(super) capped: bool,
}

impl Feed {
    /// Send `step` to the writer; false once nothing is there to take it.
    fn step(&mut self, step: Step) -> bool {
        let seq = self.seq;
        self.seq = self.seq.saturating_add(1);
        self.results.send(Done { seq, step, charge: 0 }).is_ok()
    }

    /// Hand a job to the lanes (`sent`: one of the Sent Items folder, with the replies read
    /// before it), waiting while they have [`JOBS`] waiting already; false once they're gone, or
    /// the import is stopping (`stopped`).
    fn job(&mut self, node: u32, size: u64, at: (usize, &PlanFolder, u64), sent: Option<&Arc<SentIds>>, stopped: &dyn Fn() -> bool) -> bool {
        let seq = self.seq;
        self.seq = self.seq.saturating_add(1);
        let charge = budget::charge_for(i64::try_from(size).ok().filter(|s| *s > 0));
        let (folder, f, row) = at;
        let (pim, trash, folder_node) = (f.kind == Kind::Pim, f.trash, f.node);
        let mut job = Job { seq, node, size, charge, folder, pim, trash, folder_node, row, sent: sent.cloned() };
        loop {
            match self.jobs.try_send(job) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(j)) if !stopped() => {
                    job = j;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Full(_)) => return false,
            }
        }
    }
}

/// Hand out the work of `plan`, read from `store`, until it's done or the import stops. The
/// Sent Items folder is made in its place, so the folders come out as they would, but read
/// after every other, with the replies read before it (`out/sent_ids.rs`): even when a folder
/// before it panicked, which is raised again once it's read.
pub(super) fn stream(store: &PstStore, plan: &Plan, feed: &mut Feed, ctx: &Ctx<'_>) {
    let skip = ctx.resume.as_ref();
    let folders = plan.folders.iter().filter(|f| f.kind == Kind::Mail).count();
    let outline = plan
        .folders
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == Kind::Mail)
        .map(|(index, f)| Outlined { index, parent: f.parent, node: f.node, name: f.name.clone(), items: f.rows })
        .collect();
    let totals =
        Step::Totals { items: plan.items, bytes: plan.bytes, folders, record_key: plan.record_key.clone(), fingerprint: plan.fingerprint(), outline };
    if !feed.step(totals) {
        return;
    }
    // Going on, what the plan lost was counted the first time.
    for (reason, n) in plan.lost.iter().filter(|_| skip.is_none()) {
        if !feed.step(Step::Lost { reason: *reason, n: *n, items: false }) {
            return;
        }
    }
    let mut deferred = None;
    let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for (index, f) in plan.folders.iter().enumerate() {
            if ctx.stopped() {
                return;
            }
            let done = skip.is_some_and(|s| s.done(f.node));
            if !done && deferred.is_none() && f.kind == Kind::Mail && plan.sent == Some(f.node) {
                deferred = Some((index, f));
                if !feed.step(begins(index, f, true)) {
                    return;
                }
                continue;
            }
            if !folder(store, index, f, feed, ctx, None) {
                return;
            }
        }
    }));
    let read = match deferred {
        Some((index, f)) if !ctx.stopped() => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sent_folder(store, index, f, feed, ctx))),
        _ => Ok(()),
    };
    if let Err(panic) = walked.and(read) {
        std::panic::resume_unwind(panic);
    }
}

/// The step that begins the plan's folder `f` (its `index`th).
fn begins(index: usize, f: &PlanFolder, deferred: bool) -> Step {
    Step::Folder { index, node: f.node, parent: f.parent, name: f.name.clone(), pim: f.kind == Kind::Pim, deferred }
}

/// The Sent Items folder, read last: once the writer has applied everything before it, with
/// the replies it read then.
fn sent_folder(store: &PstStore, index: usize, f: &PlanFolder, feed: &mut Feed, ctx: &Ctx<'_>) {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if !feed.step(Step::Freeze { reply: tx }) {
        return;
    }
    let ids = loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(ids) => break ids,
            Err(RecvTimeoutError::Timeout) if !ctx.stopped() => {}
            Err(_) => return,
        }
    };
    folder(store, index, f, feed, ctx, Some(&ids));
}

/// Hand out the plan's folder `f` (its `index`th): its start, a job per item, its end (`sent`:
/// the Sent Items folder's, with the replies read before it). False once the import stops, or
/// nothing more is handed out.
fn folder(store: &PstStore, index: usize, f: &PlanFolder, feed: &mut Feed, ctx: &Ctx<'_>, sent: Option<&Arc<SentIds>>) -> bool {
    let stopped = || ctx.stopped();
    let skip = ctx.resume.as_ref();
    // Done already: a mail folder says only where it went.
    if skip.is_some_and(|s| s.done(f.node)) {
        return f.kind != Kind::Mail || feed.step(begins(index, f, false));
    }
    match f.kind {
        Kind::LeftOut => return feed.step(Step::LeftOut { node: f.node, items: count(f.rows) }),
        Kind::Mail | Kind::Pim => {
            if !feed.step(begins(index, f, false)) {
                return false;
            }
        }
    }
    let start = skip.map_or(0, |s| s.start(f.node));
    let began = Instant::now();
    let opened = store.open_folder(f.node);
    ctx.tally.since(Phase::Plan, began);
    match opened {
        Ok(sub) => {
            if let Some(table) = sub.contents_table() {
                for (row, (node, size)) in sizes(table).enumerate() {
                    if stopped() {
                        return false;
                    }
                    // Read already, or one the reader failed on.
                    let row = u64::try_from(row).unwrap_or(u64::MAX);
                    if row < start || skip.is_some_and(|s| s.failed(f.node, row)) {
                        continue;
                    }
                    if feed.handed >= MAX_ITEMS {
                        if !feed.capped {
                            feed.capped = true;
                            feed.step(Step::Lost { reason: SkipReason::StoppedEarly, n: 1, items: false });
                        }
                        return false;
                    }
                    #[cfg(test)]
                    let size = match ctx.opts.fault {
                        // The walk fails; the Sent Items folder read after it doesn't.
                        Some(super::lanes::tests::Fault::PlannerAfter(n)) if feed.handed == n && sent.is_none() => panic!("a test's planner fault"),
                        // The file says every item is tiny.
                        Some(super::lanes::tests::Fault::Understate) => 1,
                        _ => size,
                    };
                    feed.handed += 1;
                    if !feed.job(node, size, (index, f, row), sent, &stopped) {
                        return false;
                    }
                }
            }
        }
        Err(e) => {
            log::info!("a folder could not be read: {e}");
            if !feed.step(Step::Lost { reason: SkipReason::FolderUnreadable, n: 1, items: false }) {
                return false;
            }
        }
    }
    feed.step(Step::FolderEnd { index, node: f.node, pim: f.kind == Kind::Pim })
}

fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// The store's record key (`0x0FF9`), in hex: "" when it has none, or one too long to be one.
fn record_key(store: &PstStore) -> String {
    match store.properties().get(0x0FF9) {
        Some(PropertyValue::Binary(b)) if b.buffer().len() <= 64 => b.buffer().iter().map(|x| format!("{x:02x}")).collect(),
        _ => String::new(),
    }
}

/// The node of one of the store's special folders (`0x35E2`: the Outbox, `0x35E3`: Deleted
/// Items, `0x35E4`: Sent Items),
/// from its entry id (flags, the store's 16-byte key, then the node id).
fn special_folder(store: &PstStore, prop: u16) -> Option<u32> {
    let (_, v) = store.properties().iter().find(|(id, _)| **id == prop)?;
    let PropertyValue::Binary(b) = v else { return None };
    let id = b.buffer().get(20..24)?;
    Some(u32::from_le_bytes([*id.first()?, *id.get(1)?, *id.get(2)?, *id.get(3)?]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder listed under itself, or twice, is read once; nesting and the number of folders
    /// stop where they're capped.
    #[test]
    fn a_folder_seen_again_is_read_once() {
        let mut g = Guard::default();
        assert_eq!(g.admit(0x8022, 0), Ok(()));
        assert_eq!(g.admit(0x8042, 1), Ok(()));
        assert_eq!(g.admit(0x8022, 2), Err(Refused::Again), "under itself");
        assert_eq!(g.admit(0x8042, 1), Err(Refused::Again), "listed twice");
        assert_eq!(g.admit(0x8062, MAX_DEPTH), Err(Refused::TooDeep));
        let mut g = Guard::default();
        for n in 0..MAX_FOLDERS {
            assert_eq!(g.admit(u32::try_from(n).unwrap(), 1), Ok(()));
        }
        assert_eq!(g.admit(u32::MAX, 1), Err(Refused::TooMany));
    }

    #[test]
    fn folders_are_classed_by_what_they_hold() {
        assert_eq!(classify(""), Kind::Mail);
        assert_eq!(classify("IPF.Note"), Kind::Mail);
        assert_eq!(classify("IPF.Imap"), Kind::Mail);
        assert_eq!(classify("IPF.Note.OutlookHomepage"), Kind::Mail, "RSS Feeds");
        assert_eq!(classify("IPF.Configuration"), Kind::LeftOut);
        assert_eq!(classify("IPF.Contact"), Kind::Pim);
        assert_eq!(classify("IPF.Appointment"), Kind::Pim);
        assert_eq!(classify("IPF.Journal"), Kind::Pim);
        assert_eq!(classify("IPF.RssFeed"), Kind::Mail);
    }
}
