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
use std::sync::mpsc::{Sender, SyncSender, TrySendError};

use super::lanes::{Ctx, Job};
use super::order::{Done, Step};
use super::*;

/// Jobs waiting for a lane, at most.
pub(super) const JOBS: usize = 1024;
/// Folders read, at most; past this the rest of the file isn't.
const MAX_FOLDERS: usize = 100_000;
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
    let mut plan = Plan::default();
    let mut guard = Guard::default();
    let outbox = special_folder(store, 0x35E2);
    visit(store, &*top, None, 0, outbox, &mut guard, &mut plan, ctx);
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
        plan.folders.push(PlanFolder { node, parent, name, kind, rows });
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
}

impl Feed {
    /// Send `step` to the writer; false once nothing is there to take it.
    fn step(&mut self, step: Step) -> bool {
        let seq = self.seq;
        self.seq = self.seq.saturating_add(1);
        self.results.send(Done { seq, step, charge: 0 }).is_ok()
    }

    /// Hand a job to the lanes, waiting while they have [`JOBS`] waiting already; false once
    /// they're gone, or the import is stopping (`stopped`).
    fn job(&mut self, node: u32, size: u64, folder: usize, pim: bool, stopped: &dyn Fn() -> bool) -> bool {
        let seq = self.seq;
        self.seq = self.seq.saturating_add(1);
        let charge = budget::charge_for(i64::try_from(size).ok().filter(|s| *s > 0));
        let mut job = Job { seq, node, size, charge, folder, pim };
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

/// Hand out the work of `plan`, read from `store`, until it's done or the import stops.
pub(super) fn stream(store: &dyn PstStore, plan: &Plan, feed: &mut Feed, ctx: &Ctx<'_>) {
    let stopped = || ctx.stopped();
    if !feed.step(Step::Totals { items: plan.items, bytes: plan.bytes }) {
        return;
    }
    for (reason, n) in &plan.lost {
        if !feed.step(Step::Lost { reason: *reason, n: *n, items: false }) {
            return;
        }
    }
    let mut handed = 0u64;
    for (index, f) in plan.folders.iter().enumerate() {
        if stopped() {
            return;
        }
        match f.kind {
            Kind::LeftOut => {
                if !feed.step(Step::LeftOut { items: count(f.rows) }) {
                    return;
                }
                continue;
            }
            Kind::Mail => {
                if !feed.step(Step::Folder { index, parent: f.parent, name: f.name.clone() }) {
                    return;
                }
            }
            Kind::Pim => {}
        }
        let began = Instant::now();
        let opened = store.properties().make_entry_id(NodeId::from(f.node)).and_then(|e| store.open_folder(&e));
        ctx.tally.since(Phase::Plan, began);
        match opened {
            Ok(sub) => {
                if let Some(table) = sub.contents_table() {
                    for (node, size) in sizes(&**table) {
                        if stopped() {
                            return;
                        }
                        if handed >= MAX_ITEMS {
                            feed.step(Step::Lost { reason: SkipReason::StoppedEarly, n: 1, items: false });
                            return;
                        }
                        #[cfg(test)]
                        let size = match ctx.opts.fault {
                            Some(super::lanes::tests::Fault::PlannerAfter(n)) if handed == n => panic!("a test's planner fault"),
                            // The file says every item is tiny.
                            Some(super::lanes::tests::Fault::Understate) => 1,
                            _ => size,
                        };
                        handed += 1;
                        if !feed.job(node, size, index, f.kind == Kind::Pim, &stopped) {
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                log::info!("a folder could not be read: {e}");
                if !feed.step(Step::Lost { reason: SkipReason::FolderUnreadable, n: 1, items: false }) {
                    return;
                }
            }
        }
        if f.kind == Kind::Mail && !feed.step(Step::FolderEnd { index }) {
            return;
        }
    }
}

fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// The node of one of the store's special folders (`0x35E2`: the Outbox), from its entry id
/// (flags, the store's 16-byte key, then the node id).
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
