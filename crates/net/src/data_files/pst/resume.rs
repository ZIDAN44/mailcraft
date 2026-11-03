//! Where an import has got, noted in the account's data file as it goes (`import.json`, its
//! checkpoint), so an import paused, cancelled or cut off (MailCraft closed, the computer turned
//! off) goes on from there, not from the start.
//!
//! A checkpoint goes in with the batch of sources it names, in one durable, sealed commit (the
//! batches before it are on the disk with it, and a crash after it reopens at once), with the
//! contacts and calendar read so far: whatever a crash leaves, the checkpoint names nothing that
//! isn't there. It's written at least every [`CHECKPOINT_EVERY`] while the writer writes, before
//! finished folders go into the account's folder list, and as the import stops or ends. It notes
//! the folders done, the one part-way through (its next row, its next message number and its
//! list's tail), the rows the reader failed on (never read again), and the counts so far.
//!
//! Going on, sources written after the checkpoint (a crash before the next) are removed, and the
//! same rows are read again into the same numbers. The file is checked to be the one it was first:
//! the same size and time it last changed, or else the same record key and the same folders,
//! each with the same items and bytes (Outlook writes a file's header when it opens it, changing
//! its time and nothing else). Where it isn't, or isn't there, the import waits on the user. A
//! checkpoint is read from a file, so it's hostile: one that doesn't add up isn't gone on from.

use std::collections::HashSet;

use mailcraft_model::job::{Blocked, SkipCount};
use mailcraft_store::ListTail;
use serde::{Deserialize, Serialize};

use super::out::Open;
use super::*;

/// The account document the checkpoint is kept in (the engine reads its state and source too).
pub(crate) const CHECKPOINT_DOC: &str = "import.json";
const VERSION: u32 = 1;
/// A checkpoint is read only when it's at most this big.
const MAX_DOC: usize = 64 << 20;
/// Rows the reader failed on kept, at most (past it, the next row is enough).
const MAX_FAILED: usize = 4096;
/// A folder's path in a checkpoint is at most this long.
const MAX_PATH: usize = 4096;
/// A checkpoint is written at least this often while the writer writes.
pub(super) const CHECKPOINT_EVERY: Duration = Duration::from_secs(10);

/// How far the import is, as the checkpoint says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    /// Going, or paused, or cut off: it goes on by itself.
    #[default]
    Running,
    /// Stopped by the user: it goes on when asked.
    Cancelled,
    Done,
}

/// The data file read: where it was, how big, when it last changed, its record key, and the
/// fingerprint of what it holds ([`plan::Plan::fingerprint`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    pub path: String,
    pub size: u64,
    pub modified: i64,
    pub record_key: String,
    pub fingerprint: u64,
}

/// A folder of the file (its node) done, and the folder it became ("" for one that isn't a mail
/// folder: contacts, a calendar, one left out).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Placed {
    pub node: u32,
    pub path: String,
}

/// The folder part-way through: its next row to read, its next message number, and its list as
/// written so far (none once it's in the account's folder list: the list is read back instead).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Current {
    pub node: u32,
    pub path: String,
    pub next_row: u64,
    pub uid_next: u32,
    pub tail: Option<ListTail>,
}

/// A row the reader failed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Spot {
    pub node: u32,
    pub row: u64,
}

/// What the import has read of the replies to sent items (`out/sent_ids.rs`): each reply's
/// parent index (a hash) and the id it answers ("" when replies to it answer two), and the real
/// ids read (their hashes); `full` once past what's kept, when nothing is; `frozen` once the
/// sent items are read, when the ids they have and are given are kept past that (one each), in
/// `later`: apart from `seen`, so an import gone on with gives the sent items it has still to
/// read the ids the one that never stopped gave them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SentMap {
    pub replies: Vec<SentReply>,
    pub seen: Vec<u64>,
    pub later: Vec<u64>,
    pub full: bool,
    pub frozen: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SentReply {
    pub key: u64,
    pub id: String,
}

impl SentMap {
    /// Whether it's one an import could have kept: no more entries than its bytes allow, and
    /// ids no longer than one kept.
    pub(super) fn sound(&self) -> bool {
        let most = super::out::sent_ids::MAX_BYTES / 16;
        // Frozen, the Sent Items folder's rows add an id each at most (to `seen` itself in a
        // checkpoint written before `later` was kept apart).
        let (seen, later) = if self.frozen { (most.saturating_add(super::plan::MAX_ROWS), super::plan::MAX_ROWS) } else { (most, 0) };
        self.replies.len() <= most && self.seen.len() <= seen && self.later.len() <= later && self.replies.iter().all(|r| r.id.len() <= MAX_ID)
    }

    /// A checkpoint's without one (written before sent items were given ids): the replies read
    /// aren't known, so none is given.
    fn absent() -> SentMap {
        SentMap { full: true, ..SentMap::default() }
    }
}

/// What the import has counted so far.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Counts {
    pub read: u64,
    pub imported: u64,
    pub skipped: u64,
    pub body_not_converted: u64,
    pub bytes_done: u64,
    pub skips: Vec<SkipCount>,
}

/// An import's checkpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Checkpoint {
    pub version: u32,
    pub source: Source,
    pub state: CheckState,
    /// The mail folders made so far, the one part-way through too, in the order they were made.
    pub folders: Vec<Folder>,
    pub done: Vec<Placed>,
    pub current: Option<Current>,
    /// The Sent Items folder, made in its place but read after every other (not read yet, or,
    /// going on, read in part), and the replies read for it.
    pub deferred: Option<Current>,
    #[serde(default = "SentMap::absent")]
    pub sent: SentMap,
    pub failed: Vec<Spot>,
    pub counts: Counts,
    /// The contacts, calendar and notes read so far are in the account's (written with this).
    pub pim: bool,
    /// Time spent on the import in all its runs, and how many there have been.
    pub elapsed_ms: u64,
    pub runs: u32,
}

impl Checkpoint {
    /// A checkpoint read from `bytes`, when it's one that adds up.
    pub fn parse(bytes: &[u8]) -> Option<Checkpoint> {
        if bytes.len() > MAX_DOC {
            return None;
        }
        let cp: Checkpoint = serde_json::from_slice(bytes).ok()?;
        cp.sound().then_some(cp)
    }

    /// Whether it adds up: its version, its counts within what an import ever holds, folders
    /// named once and named where they're used, a part-way folder that is one of them, and a
    /// list tail that could be gone on from.
    fn sound(&self) -> bool {
        let path_ok = |p: &str| !p.is_empty() && p.len() <= MAX_PATH;
        let mut paths = HashSet::new();
        let folders_ok = self.folders.len() <= plan::MAX_FOLDERS && self.folders.iter().all(|f| path_ok(&f.path) && paths.insert(f.path.as_str()));
        let mut nodes = HashSet::new();
        let done_ok = self.done.len() <= plan::MAX_FOLDERS
            && self.done.iter().all(|d| nodes.insert(d.node) && (d.path.is_empty() || paths.contains(d.path.as_str())));
        let current_ok = self.current.as_ref().is_none_or(|c| {
            !nodes.contains(&c.node)
                && c.uid_next >= 1
                && (c.path.is_empty() || paths.contains(c.path.as_str()))
                && c.tail.as_ref().is_none_or(|t| !c.path.is_empty() && t.is_sound())
        });
        let deferred_ok = self.deferred.as_ref().is_none_or(|d| {
            !nodes.contains(&d.node)
                && self.current.as_ref().is_none_or(|c| c.node != d.node)
                && d.uid_next >= 1
                && paths.contains(d.path.as_str())
                && d.tail.as_ref().is_none_or(ListTail::is_sound)
        });
        self.version == VERSION
            && folders_ok
            && done_ok
            && current_ok
            && deferred_ok
            && self.sent.sound()
            && self.failed.len() <= MAX_FAILED
            && self.counts.skips.len() <= 64
    }

    /// What an import done came to, from its checkpoint alone (the data file isn't read again).
    pub(super) fn imported(&self, store: &Store, account: &str) -> Imported {
        let n = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
        let pim = if self.pim { store.load_account_doc::<Pim>(account, PIM_DOC).unwrap_or_default() } else { Pim::default() };
        Imported {
            folders: store.load_folders(account).unwrap_or_default(),
            messages: n(self.counts.imported),
            contacts: pim.cards.len(),
            events: calendar_items(&pim),
            notes: pim.notes.len(),
            tasks: pim.tasks.len(),
            skipped: n(self.counts.skipped),
            skips: self.counts.skips.iter().map(|s| (s.reason, n(s.count))).collect(),
            body_not_converted: n(self.counts.body_not_converted),
            runs: self.runs,
            earlier_ms: self.elapsed_ms,
            ..Imported::default()
        }
    }
}

/// What was found of an account's checkpoint.
pub(super) enum Found {
    /// None: the import starts from the beginning.
    Nothing,
    /// One that can't be gone on from.
    Unsound,
    Some(Box<Checkpoint>),
}

/// `account`'s checkpoint.
pub(super) fn load(store: &Store, account: &str) -> Found {
    match store.load_account_doc_bytes(account, CHECKPOINT_DOC) {
        Ok(None) => Found::Nothing,
        Ok(Some(bytes)) => Checkpoint::parse(&bytes).map_or(Found::Unsound, |cp| Found::Some(Box::new(cp))),
        Err(e) => {
            log::warn!("import: the checkpoint couldn't be read: {e}");
            Found::Unsound
        }
    }
}

/// Keep what a stopped import brought in as it is: it won't go on. Paused (or stopped for want of
/// room, or waiting on a file that changed or went), the folder it was part-way through and the
/// Sent Items folder made to be read last were kept out of the account's folder list; each is
/// finished now with the messages its checkpoint names (sources written past those go), and
/// every folder the import made is in the list. The checkpoint then says the import is done.
/// Returns the folders put in the list (none when there was nothing to finish). Thrown away
/// (`stop`: its account removed), nothing more is written.
pub fn keep_as_is(store: &Store, account: &str, stop: &Stop) -> Result<Vec<Folder>, String> {
    let thrown_away = || stop.how() == Some(StopHow::Discard);
    let mut cp = match load(store, account) {
        Found::Some(cp) if cp.state != CheckState::Done => *cp,
        // None, done already, or one that can't be read: what's listed stays as it is.
        _ => return Ok(Vec::new()),
    };
    let listed: HashSet<String> = store.load_folders(account).map_err(|e| e.to_string())?.into_iter().map(|f| f.path).collect();
    for c in [cp.current.take(), cp.deferred.take()].into_iter().flatten() {
        if thrown_away() {
            return Err("its account was removed".into());
        }
        // Listed already (cancelled: finished as it stopped), or contacts and calendar items.
        let Some(tail) = c.tail.filter(|_| !c.path.is_empty() && !listed.contains(&c.path)) else { continue };
        let list = store.resume_list_writer(account, &c.path, tail).map_err(|e| e.to_string())?;
        store.drop_bodies_from(account, &c.path, 1, c.uid_next).map_err(|e| e.to_string())?;
        let head = FolderState { validity: 1, uid_next: c.uid_next, ..Default::default() };
        let (unread, total) = list.finish(store, &head).map_err(|e| e.to_string())?;
        if let Some(f) = cp.folders.iter_mut().find(|f| f.path == c.path) {
            (f.unread, f.total) = (unread, total);
        }
    }
    let missing: Vec<Folder> = cp.folders.iter().filter(|f| !listed.contains(&f.path)).cloned().collect();
    if thrown_away() {
        return Err("its account was removed".into());
    }
    store.upsert_folders(account, &missing).map_err(|e| e.to_string())?;
    cp.state = CheckState::Done;
    store.save_account_doc(account, CHECKPOINT_DOC, &cp).map_err(|e| e.to_string())?;
    Ok(missing)
}

/// A checkpoint read for the fuzz target: none when it isn't one that would be gone on from, or
/// whether it reads back the same once written again.
#[doc(hidden)]
pub fn checkpoint_from_bytes(bytes: &[u8]) -> Option<bool> {
    let cp = Checkpoint::parse(bytes)?;
    let again = serde_json::to_vec(&cp).ok().and_then(|b| Checkpoint::parse(&b));
    Some(again.as_ref() == Some(&cp))
}

/// What the planner leaves out going on: the folders done, the rows of the one part-way through
/// (and of the Sent Items folder put off, when it was) read already, and the rows the reader
/// failed on; and the file it's to be.
pub(super) struct Skip {
    done: HashSet<u32>,
    current: Option<(u32, u64)>,
    deferred: Option<(u32, u64)>,
    failed: HashSet<Spot>,
    source: Source,
}

impl Skip {
    fn of(cp: &Checkpoint) -> Skip {
        Skip {
            done: cp.done.iter().map(|d| d.node).collect(),
            current: cp.current.as_ref().map(|c| (c.node, c.next_row)),
            deferred: cp.deferred.as_ref().map(|c| (c.node, c.next_row)),
            failed: cp.failed.iter().copied().collect(),
            source: cp.source.clone(),
        }
    }

    pub(super) fn done(&self, node: u32) -> bool {
        self.done.contains(&node)
    }

    /// The first row of the folder at `node` still to read.
    pub(super) fn start(&self, node: u32) -> u64 {
        match (self.current, self.deferred) {
            (Some((n, row)), _) | (_, Some((n, row))) if n == node => row,
            _ => 0,
        }
    }

    pub(super) fn failed(&self, node: u32, row: u64) -> bool {
        self.failed.contains(&Spot { node, row })
    }

    /// Whether the file at `path`, planned as `record_key` and `fingerprint`, is the one the
    /// import began on: unchanged on the disk, or changed only where its items aren't.
    pub(super) fn same_file(&self, path: &Path, record_key: &str, fingerprint: u64) -> bool {
        let (size, modified) = facts(path);
        (size, modified) == (self.source.size, self.source.modified)
            || (record_key == self.source.record_key && fingerprint == self.source.fingerprint)
    }
}

/// The size of the file at `path` and when it last changed (seconds), as the file system says.
pub(super) fn facts(path: &Path) -> (u64, i64) {
    std::fs::metadata(path).map_or((0, 0), |m| {
        let modified = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        (m.len(), modified)
    })
}

/// Why a file can't be gone on with, from the error opening it gave.
pub(super) fn blocked_by(e: &io::Error) -> Blocked {
    match (e.kind(), e.raw_os_error()) {
        (io::ErrorKind::NotFound, _) => Blocked::SourceMissing,
        (_, Some(32 | 33)) => Blocked::Locked,
        _ => Blocked::SourceChanged,
    }
}

/// An import's own note of where it is, kept by the writer for its checkpoints.
#[derive(Default)]
pub(super) struct Note {
    /// The file read.
    pub(super) source: Source,
    /// The folder of the file being read now: its node, the mail folder it is (none for contacts
    /// or a calendar), and its next row.
    at: Option<At>,
    done: Vec<Placed>,
    failed: Vec<Spot>,
    /// Earlier runs: how many, and the time they took.
    pub(super) runs: u32,
    pub(super) earlier_ms: u64,
    /// The items read and their bytes as this run began (going on from a checkpoint).
    pub(super) start_read: usize,
    pub(super) start_bytes: u64,
    /// When the last checkpoint was written (none: not yet), how often one is ([`CHECKPOINT_EVERY`],
    /// but in a test), and whether one may be: not until the file is known (what it holds, and,
    /// going on, that it's the same one).
    last: Option<Instant>,
    every: Duration,
    ready: bool,
    /// Contacts or calendar items read since the last checkpoint.
    pim_dirty: bool,
    /// An Inbox made up at the end (the file had none): not the file's, so not noted.
    pub(super) invented: Option<String>,
    /// How the checkpoints say the import ends, once it's ending.
    ending: Option<(CheckState, Option<Current>)>,
    /// Going on from a checkpoint: the folders done and where each went, and the one part-way
    /// through, until it's opened again; the folders to put back in the account's list.
    placed: HashMap<u32, String>,
    resumed: Option<Current>,
    reconcile: Vec<Folder>,
    /// The Sent Items folder, made in its place to be read last (`order::Step::Folder`); going
    /// on, as the checkpoint left it, until it's made again.
    deferred: Option<At>,
    resumed_deferred: Option<Current>,
}

impl Note {
    /// Nothing noted yet, with a checkpoint at least every `every`.
    pub(super) fn every(every: Duration) -> Note {
        Note { every, ..Note::default() }
    }
}

#[derive(Clone, Debug)]
struct At {
    node: u32,
    path: Option<String>,
    next_row: u64,
}

impl Out<'_> {
    /// Go on from `cp`: its folders, counts, contacts and calendar, and where it got; returns what
    /// the planner leaves out. Nothing is written yet: [`Out::source_known`] puts back what a
    /// crash kept out, once the file is known to be the same.
    pub(super) fn resume_from(&mut self, cp: Checkpoint) -> Skip {
        let skip = Skip::of(&cp);
        let n = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
        self.read = n(cp.counts.read);
        self.imported = n(cp.counts.imported);
        self.skipped = n(cp.counts.skipped);
        self.body_not_converted = n(cp.counts.body_not_converted);
        self.skips = cp.counts.skips.iter().map(|s| (s.reason, n(s.count))).collect();
        self.bytes_done = cp.counts.bytes_done;
        if cp.pim {
            match self.store.load_account_doc::<Pim>(self.account, PIM_DOC) {
                Ok(p) => self.pim = p,
                Err(e) => log::warn!("import: the contacts and calendar read so far couldn't be read: {e}"),
            }
        }
        let note = &mut self.note;
        note.source = cp.source;
        note.runs = cp.runs;
        note.earlier_ms = cp.elapsed_ms;
        (note.start_read, note.start_bytes) = (self.read, self.bytes_done);
        note.failed = cp.failed;
        note.placed = cp.done.iter().filter(|d| !d.path.is_empty()).map(|d| (d.node, d.path.clone())).collect();
        let done_paths: HashSet<&str> = note.placed.values().map(String::as_str).collect();
        note.reconcile = cp.folders.iter().filter(|f| done_paths.contains(f.path.as_str())).cloned().collect();
        note.done = cp.done;
        note.resumed = cp.current;
        note.resumed_deferred = cp.deferred;
        self.sent = super::out::sent_ids::SentIds::from_map(&cp.sent);
        self.folders = cp.folders;
        self.room_kept();
        skip
    }

    /// The file at `path` is known (planned as `record_key` and `fingerprint`), and, going on,
    /// it's the one the checkpoint was of: checkpoints may be written from here, and the folders
    /// the import had done go back in the account's folder list where a crash kept them out.
    pub(super) fn source_known(&mut self, path: &Path, record_key: String, fingerprint: u64) {
        let (size, modified) = facts(path);
        self.note.source = Source { path: path.to_string_lossy().into_owned(), size, modified, record_key, fingerprint };
        self.note.ready = true;
        let reconcile = std::mem::take(&mut self.note.reconcile);
        if !reconcile.is_empty() {
            let listed: HashSet<String> = self.store.load_folders(self.account).unwrap_or_default().into_iter().map(|f| f.path).collect();
            let missing: Vec<Folder> = reconcile.into_iter().filter(|f| !listed.contains(&f.path)).collect();
            if let Err(e) = self.store.upsert_folders(self.account, &missing) {
                log::warn!("import: {e}");
            }
        }
    }

    /// The mail folder for the file's folder at `node` (`name`, under `parent`): the one it went
    /// to when it's done already, the one part-way through opened again, or a new one. One
    /// `deferred` (the Sent Items folder in its place) is made, or opened again, but not read
    /// until it begins again after every other.
    pub(super) fn begin_folder(&mut self, node: u32, parent: Option<&str>, name: &str, deferred: bool) -> Option<String> {
        if !deferred && let Some(at) = self.note.deferred.take_if(|d| d.node == node) {
            let path = at.path.clone();
            self.note.at = Some(at);
            return path;
        }
        if let Some(path) = self.note.placed.get(&node) {
            return Some(path.clone());
        }
        let resumed = match self.note.resumed.take_if(|c| c.node == node) {
            Some(c) => Some(c),
            None => self.note.resumed_deferred.take_if(|c| c.node == node),
        };
        let reading = self.note.at.take();
        let path = match resumed {
            Some(current) => self.reopen(current),
            None => {
                let path = self.add_folder(parent, name);
                self.note.at = Some(At { node, path: Some(path.clone()), next_row: 0 });
                Some(path)
            }
        };
        if deferred {
            // Made, not read: the folder being read (none, between folders) stays the one.
            self.note.deferred = self.note.at.take();
            self.note.at = reading;
        }
        path
    }

    /// Going on, where the file's folder at `node` went already: a folder done, or the one
    /// part-way through (or put off to be read last) as the import stopped.
    pub(super) fn known_path(&self, node: u32) -> Option<&str> {
        if let Some(path) = self.note.placed.get(&node) {
            return Some(path);
        }
        [self.note.resumed.as_ref(), self.note.resumed_deferred.as_ref()]
            .into_iter()
            .flatten()
            .find(|c| c.node == node && !c.path.is_empty())
            .map(|c| c.path.as_str())
    }

    /// The file's contacts, calendar, tasks, notes or journal folder at `node` begins.
    pub(super) fn begin_pim_folder(&mut self, node: u32) {
        let next_row = match self.note.resumed.take_if(|c| c.node == node) {
            Some(c) => c.next_row,
            None => 0,
        };
        self.note.at = Some(At { node, path: None, next_row });
    }

    /// The folder part-way through opened again: its list from its tail when it was never
    /// shown (paused or cut off, whether or not a cancel came before it was reached again), or
    /// from what's listed when it was (stopped by the user: its numbers aren't given again).
    /// Sources written past it go. `None` when its list can't be read: its items then go
    /// nowhere, and the rest is left as it was. Its messages written before a crash may not be
    /// in the search index: the next pass looks at it.
    fn reopen(&mut self, c: Current) -> Option<String> {
        let path = c.path.clone();
        let (uid_next, list) = match c.tail {
            Some(tail) => (c.uid_next, self.store.resume_list_writer(self.account, &path, tail)),
            _ => match self.store.load_folder(self.account, &path) {
                Ok(st) => {
                    let uid_next = st.free_uid(self.account, &path).unwrap_or(c.uid_next).max(c.uid_next);
                    let mut messages: Vec<mailcraft_model::Summary> = st.messages.into_iter().map(std::sync::Arc::unwrap_or_clone).collect();
                    messages.sort_by_key(|m| m.key.uid);
                    let mut list = self.store.list_writer(self.account, &path);
                    (uid_next, list.push(self.store, messages).map(|()| list))
                }
                Err(e) => (c.uid_next, Err(e)),
            },
        };
        let list = match list {
            Ok(list) => list,
            Err(e) => {
                log::warn!("import: the folder part-way through couldn't be opened again: {e}");
                return None;
            }
        };
        if let Err(e) = self.store.drop_bodies_from(self.account, &path, 1, uid_next) {
            log::warn!("import: {e}");
        }
        self.index_missed(&path);
        self.states.insert(path.clone(), Open { uid_next, waiting: Vec::new(), list });
        self.note.at = Some(At { node: c.node, path: Some(path.clone()), next_row: c.next_row });
        Some(path)
    }

    /// The item at `row` of the folder at `node` is dealt with (`failed`: the reader failed on it).
    pub(super) fn row_done(&mut self, node: u32, row: u64, failed: bool) {
        if let Some(at) = self.note.at.as_mut().filter(|a| a.node == node) {
            at.next_row = at.next_row.max(row.saturating_add(1));
        }
        if failed && self.note.failed.len() < MAX_FAILED {
            self.note.failed.push(Spot { node, row });
        }
    }

    /// Contacts or calendar items were read since the last checkpoint.
    pub(super) fn pim_read(&mut self) {
        self.note.pim_dirty = true;
    }

    /// The file's folder at `node` that isn't a mail folder is done (one left out, or contacts
    /// and calendar items read).
    pub(super) fn other_folder_done(&mut self, node: u32) {
        if self.note.at.as_ref().is_some_and(|a| a.node == node) {
            self.note.at = None;
        }
        self.note.done.push(Placed { node, path: String::new() });
    }

    /// The mail folder at `path` is written whole: done, when it's the file's folder being read.
    pub(super) fn mail_folder_done(&mut self, path: &str) {
        if let Some(at) = self.note.at.take_if(|a| a.path.as_deref() == Some(path)) {
            self.note.done.push(Placed { node: at.node, path: path.to_string() });
        }
    }

    /// The import is ending as `state` says: the folder part-way through as it is now (before its
    /// list is finished, when cancelled), then nothing is.
    pub(super) fn ending(&mut self, state: CheckState) {
        let current = if state == CheckState::Done { None } else { self.current(state == CheckState::Running) };
        if state == CheckState::Cancelled {
            self.note.at = None;
        }
        self.note.ending = Some((state, current));
    }

    /// Whether the file is known yet (its plan's first step applied).
    pub(super) fn note_ready(&self) -> bool {
        self.note.ready
    }

    /// Whether a checkpoint is due with the next batch.
    pub(super) fn checkpoint_due(&self) -> bool {
        self.note.ready && self.note.last.is_none_or(|t| t.elapsed() >= self.note.every)
    }

    /// The mail folder part-way through, as the checkpoint says it.
    pub(super) fn part_way_path(&self) -> Option<String> {
        self.current(false).map(|c| c.path).filter(|p| !p.is_empty())
    }

    /// The Sent Items folder made and not read yet, as the checkpoint says it.
    pub(super) fn deferred_path(&self) -> Option<&str> {
        match &self.note.deferred {
            Some(d) => d.path.as_deref(),
            None => self.note.resumed_deferred.as_ref().map(|c| c.path.as_str()),
        }
    }

    /// The Sent Items folder made and not read yet (or, going on, read in part), for a
    /// checkpoint, as the folder part-way through is.
    fn deferred(&self) -> Option<Current> {
        match self.note.deferred.as_ref() {
            Some(d) => Some(self.current_of(d, true)),
            None => self.note.resumed_deferred.clone(),
        }
    }

    /// The folder part-way through as it stands (with its list's tail when `tail`).
    fn current(&self, tail: bool) -> Option<Current> {
        // Going on, before the folder part-way through is reached again: as it was.
        let Some(at) = self.note.at.as_ref() else { return self.note.resumed.clone() };
        Some(self.current_of(at, tail))
    }

    /// The folder `at` as it stands (with its list's tail when `tail`, and it isn't listed yet).
    fn current_of(&self, at: &At, tail: bool) -> Current {
        let open = at.path.as_deref().and_then(|p| self.states.get(p));
        Current {
            node: at.node,
            path: at.path.clone().unwrap_or_default(),
            next_row: at.next_row,
            uid_next: open.map_or(1, |o| o.uid_next),
            tail: open.filter(|_| tail).map(|o| o.list.tail(&o.waiting)),
        }
    }

    /// The checkpoint as things stand (every source handed to the store is written), and the
    /// contacts and calendar to go with it when they changed: the documents to write.
    pub(super) fn checkpoint_docs(&self) -> Vec<(&'static str, Vec<u8>)> {
        let (state, current) = match &self.note.ending {
            Some((state, current)) => (*state, current.clone()),
            None => (CheckState::Running, self.current(true)),
        };
        let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        let pim = self.pim.has_items();
        let invented = self.note.invented.as_deref();
        let cp = Checkpoint {
            version: VERSION,
            source: self.note.source.clone(),
            state,
            folders: self.folders.iter().filter(|f| Some(f.path.as_str()) != invented).cloned().collect(),
            done: self.note.done.clone(),
            current,
            deferred: self.deferred(),
            sent: self.sent.to_map(),
            failed: self.note.failed.clone(),
            counts: Counts {
                read: wide(self.read),
                imported: wide(self.imported),
                skipped: wide(self.skipped),
                body_not_converted: wide(self.body_not_converted),
                bytes_done: self.bytes_done,
                skips: self.skips.iter().map(|(r, n)| SkipCount { reason: *r, count: wide(*n) }).collect(),
            },
            pim,
            elapsed_ms: self.note.earlier_ms.saturating_add(u64::try_from(self.began.elapsed().as_millis()).unwrap_or(u64::MAX)),
            runs: self.note.runs.saturating_add(1),
        };
        let mut docs = Vec::new();
        if self.note.pim_dirty {
            match serde_json::to_vec(&self.pim) {
                Ok(b) => docs.push((PIM_DOC, b)),
                Err(e) => log::warn!("import: {e}"),
            }
        }
        match serde_json::to_vec(&cp) {
            Ok(b) => docs.push((CHECKPOINT_DOC, b)),
            Err(e) => log::warn!("import: {e}"),
        }
        docs
    }

    /// A checkpoint was written (`ok`), or tried: the next is a while off either way, and the
    /// contacts and calendar go with it again until they're written.
    pub(super) fn checkpointed(&mut self, ok: bool) {
        self.note.last = Some(Instant::now());
        if ok {
            self.note.pim_dirty = false;
            self.room_kept();
        }
    }

    /// Write a checkpoint now, on its own (every source handed to the store is written).
    pub(super) fn checkpoint(&mut self) {
        // Not before the file is known, nor once writing stopped for want of room: the last one
        // written is where the import goes on from.
        if !self.note.ready || self.writes_stopped() {
            return;
        }
        // Its list's tail names messages waiting for their sources: it goes with them.
        // A batch refused for want of room stops writing: no checkpoint goes past it.
        if self.write_pending(true) || self.writes_stopped() {
            return;
        }
        let began = Instant::now();
        let docs = self.checkpoint_docs();
        let reopened = self.reopened();
        let wrote = self.tally.time(Phase::Commit, || self.store.write_batch(self.account, Vec::new(), docs, reopened));
        self.tally.wrote(began, true);
        if let Err(e) = &wrote {
            log::warn!("import: the checkpoint couldn't be written: {e}");
            // Failed at the disk, or refused once the data file was opened again: nothing more is
            // written, and the import goes on from the last one written.
            self.writes_stopped();
        }
        self.checkpointed(wrote.is_ok());
    }
}
