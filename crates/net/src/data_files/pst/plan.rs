//! What a data file holds, read before any item is: its folders in the order they come in, what
//! each is (mail, contacts or calendar, or a kind left out), and how many items and bytes each
//! holds (the totals the progress goes by). Then the same folders handed out as work: each
//! mail folder's start and end to the writer, and one job per item to the lanes, all numbered in
//! the order the writer applies them.
//!
//! The file's tables are hostile: a folder listed under itself (or under one below it) is read
//! once, nesting stops at [`MAX_DEPTH`], and the folders, the items of each and the items in all
//! are capped. A folder the reader fails on (a panic) is left out, and the plan goes on.

use std::collections::HashSet;
use std::sync::mpsc::{RecvTimeoutError, Sender, SyncSender, TrySendError};

use super::lanes::{Ctx, Job};
use super::order::{Done, Step};
use super::out::sent_ids::SentIds;
use super::*;

/// Jobs waiting for a lane, at most.
pub(super) const JOBS: usize = 1024;
/// Folders read, at most; past this the rest of the file isn't.
pub(super) const MAX_FOLDERS: usize = 100_000;
/// Items handed out, at most.
const MAX_ITEMS: u64 = 10_000_000;
/// Items read of one folder, at most.
const MAX_ROWS: usize = 1_000_000;
/// PidTagMessageSize: an item's size as the file gives it.
const MESSAGE_SIZE: u16 = 0x0E08;

/// What a folder is brought in as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Mail,
    /// Contacts, a calendar, tasks, notes or a journal: their items go to the address book and calendar.
    Pim,
    /// A kind MailCraft has no place for (RSS feeds): its items are counted.
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
pub(super) fn plan(store: &dyn PstStore, ctx: &Ctx<'_>) -> io::Result<Plan> {
    let top = store.properties().ipm_sub_tree_entry_id().and_then(|e| store.open_folder(&e))?;
    let mut plan = Plan { record_key: record_key(store), ..Plan::default() };
    let mut guard = Guard::default();
    let outbox = special_folder(store, 0x35E2);
    visit(store, &*top, None, 0, outbox, &mut guard, &mut plan, ctx);
    plan.sent = special_folder(store, 0x35E4);
    // A test's file has no Sent Items folder of its own: one is named.
    #[cfg(test)]
    if let Some(name) = ctx.opts.sent_folder {
        plan.sent = plan.folders.iter().find(|f| f.name == name).map(|f| f.node);
    }
    Ok(plan)
}

/// What a folder is and holds, read under `catch_unwind`: the reader may panic on a damaged
/// table.
struct Looked {
    sub: Rc<dyn PstFolder>,
    rows: u64,
    bytes: u64,
    kind: Kind,
    name: String,
}

#[allow(clippy::too_many_arguments)]
fn visit(
    store: &dyn PstStore,
    folder: &dyn PstFolder,
    parent: Option<usize>,
    depth: usize,
    outbox: Option<u32>,
    guard: &mut Guard,
    plan: &mut Plan,
    ctx: &Ctx<'_>,
) {
    let listed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        folder.hierarchy_table().map(|t| t.rows_matrix().take(MAX_FOLDERS).map(|row| u32::from(row.id())).collect::<Vec<_>>())
    }));
    let nodes = match listed {
        Ok(nodes) => nodes.unwrap_or_default(),
        Err(_) => {
            log::info!("a folder's subfolders could not be read: the reader failed on them");
            plan.lose(SkipReason::FolderUnreadable);
            return;
        }
    };
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
        let looked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| look(store, node, plan.folders.len(), ctx)));
        let Looked { sub, rows, bytes, kind, name } = match looked {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => {
                log::info!("a folder could not be read: {e}");
                plan.lose(SkipReason::FolderUnreadable);
                continue;
            }
            Err(_) => {
                log::info!("a folder could not be read: the reader failed on it");
                plan.lose(SkipReason::FolderUnreadable);
                continue;
            }
        };
        // Outlook's own Outbox, when it has nothing waiting (MailCraft has an Outbox of its own).
        if rows == 0 && outbox == Some(node) {
            continue;
        }
        let index = plan.folders.len();
        plan.folders.push(PlanFolder { node, parent, name, kind, rows, bytes });
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
        visit(store, &*sub, under, depth + 1, outbox, guard, plan, ctx);
    }
}

/// The folder at `node` (the plan's `index`th) opened, with what it is and holds.
fn look(store: &dyn PstStore, node: u32, index: usize, ctx: &Ctx<'_>) -> io::Result<Looked> {
    #[cfg(test)]
    if let Some(super::lanes::tests::Fault::PlanAt(n)) = ctx.opts.fault
        && index == n
    {
        panic!("a test's plan fault");
    }
    let _ = (index, ctx);
    let sub = store.properties().make_entry_id(NodeId::from(node)).and_then(|e| store.open_folder(&e))?;
    let (rows, bytes) = sub.contents_table().map_or((0, 0), |t| sizes(&**t).fold((0u64, 0u64), |(n, b), (_, s)| (n + 1, b.saturating_add(s))));
    let kind = classify(&sub.properties().get(0x3613).and_then(text_of).unwrap_or_default());
    let name = sub.properties().get(0x3001).and_then(text_of).unwrap_or_default();
    Ok(Looked { sub, rows, bytes, kind, name })
}

/// What a folder of class `class` comes in as: no class is mail, in old files.
fn classify(class: &str) -> Kind {
    let mail = (class.is_empty() || class.starts_with("IPF.Note") || class.starts_with("IPF.Imap"))
        // Outlook's RSS Feeds folder is its own bookkeeping, not mail.
        && !class.starts_with("IPF.Note.OutlookHomepage");
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
fn sizes<'a>(table: &'a dyn TableContext) -> impl Iterator<Item = (u32, u64)> + 'a {
    let context = table.context();
    let column = context.columns().iter().position(|c| c.prop_id() == MESSAGE_SIZE);
    table.rows_matrix().take(MAX_ROWS).map(move |row| {
        let size = column
            .and_then(|i| {
                let values = row.columns(context).ok()?;
                let value = values.into_iter().nth(i).flatten()?;
                let prop = context.columns().get(i)?.prop_type();
                match table.read_column(&value, prop).ok()? {
                    PropertyValue::Integer32(n) => u64::try_from(n).ok(),
                    _ => None,
                }
            })
            .unwrap_or(0);
        (u32::from(row.id()), size)
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
    fn job(&mut self, node: u32, size: u64, at: (usize, u32, u64), pim: bool, sent: Option<&Arc<SentIds>>, stopped: &dyn Fn() -> bool) -> bool {
        let seq = self.seq;
        self.seq = self.seq.saturating_add(1);
        let charge = budget::charge_for(i64::try_from(size).ok().filter(|s| *s > 0));
        let (folder, folder_node, row) = at;
        let mut job = Job { seq, node, size, charge, folder, pim, folder_node, row, sent: sent.cloned() };
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
/// after every other, with the replies read before it (`out/sent_ids.rs`): even when the
/// reader failed on a folder before it (a panic), which is raised again once it's read.
pub(super) fn stream(store: &dyn PstStore, plan: &Plan, feed: &mut Feed, ctx: &Ctx<'_>) {
    let skip = ctx.resume.as_ref();
    let folders = plan.folders.iter().filter(|f| f.kind == Kind::Mail).count();
    let totals = Step::Totals { items: plan.items, bytes: plan.bytes, folders, record_key: plan.record_key.clone(), fingerprint: plan.fingerprint() };
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
fn sent_folder(store: &dyn PstStore, index: usize, f: &PlanFolder, feed: &mut Feed, ctx: &Ctx<'_>) {
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
fn folder(store: &dyn PstStore, index: usize, f: &PlanFolder, feed: &mut Feed, ctx: &Ctx<'_>, sent: Option<&Arc<SentIds>>) -> bool {
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
    let opened = store.properties().make_entry_id(NodeId::from(f.node)).and_then(|e| store.open_folder(&e));
    ctx.tally.since(Phase::Plan, began);
    match opened {
        Ok(sub) => {
            if let Some(table) = sub.contents_table() {
                for (row, (node, size)) in sizes(&**table).enumerate() {
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
                    if !feed.job(node, size, (index, f.node, row), f.kind == Kind::Pim, sent, &stopped) {
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
fn record_key(store: &dyn PstStore) -> String {
    match store.properties().get(0x0FF9) {
        Some(PropertyValue::Binary(b)) if b.buffer().len() <= 64 => b.buffer().iter().map(|x| format!("{x:02x}")).collect(),
        _ => String::new(),
    }
}

/// The node of one of the store's special folders (`0x35E2`: the Outbox, `0x35E4`: Sent Items),
/// from its entry id (flags, the store's 16-byte key, then the node id).
fn special_folder(store: &dyn PstStore, prop: u16) -> Option<u32> {
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
        assert_eq!(classify("IPF.Note.OutlookHomepage"), Kind::LeftOut);
        assert_eq!(classify("IPF.Contact"), Kind::Pim);
        assert_eq!(classify("IPF.Appointment"), Kind::Pim);
        assert_eq!(classify("IPF.Journal"), Kind::Pim);
        assert_eq!(classify("IPF.RssFeed"), Kind::LeftOut);
    }
}
