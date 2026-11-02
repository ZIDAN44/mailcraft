//! The search index kept up in the background ([`Job::Index`]): one "mail-index" thread runs a
//! pass at a time ([`search::Pass`]), an account's asks coming close together making one pass
//! once they stop for [`QUIET`]. A pass's messages are read and their text taken out on threads
//! of their own (all but two of the machine's, at most eight), each message under `catch_unwind`;
//! the pass thread adds them, writing a generation in short background transactions whenever it
//! holds enough, so a mark-read from the window waits for one of them at most. No pass runs for
//! an account a data file is being imported into, and [`Job::StopIndex`] stops one between two
//! messages, what it added kept. Summaries a pass finds to say otherwise than the index about a
//! message's files (pictures the text shows counted as files) are put right in the folder's list
//! and the window told ([`Event::AttachmentsKnown`]), so the paperclip, the Has files filter and
//! `has:attachment` agree.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};

use mailcraft_model::search::IndexDoc;
use mailcraft_store::search::{self, Pass};
use mailcraft_store::{Result as StoreResult, StoreError};

use super::*;

/// How long an account's asks must stop coming before its pass starts: a sync's run of
/// folders, or messages downloaded one by one, become one pass.
const QUIET: Duration = Duration::from_millis(1500);
/// How long before an index held elsewhere (by another writer), or an account being imported
/// into, is looked at again.
const RETRY: Duration = Duration::from_secs(5);
/// Messages read and not yet added, at most (each at most a source's text).
const IN_FLIGHT: usize = 64;
/// Bytes in flight across the readers: sources (by their summaries' sizes) being read and taken
/// apart, then the texts taken out of them until they're added. One of 30 MB on each of eight
/// threads at once held hundreds of megabytes, and 64 texts waiting more (the pass over the 41 GB
/// file peaked 545 MB above the window's own). A source is held uncompressed only, as its pieces
/// are read (`Store::read_body`), so it holds about what it counts for. A source bigger than this
/// is read alone; small ones each count as [`LEAST_COST`].
const READING_BYTES: u64 = 48 << 20;
/// What a small source counts for in [`READING_BYTES`] (its reading's own buffers).
const LEAST_COST: u64 = 64 << 10;
/// How often a reader waiting for room looks whether the pass has stopped.
const ROOM_WAIT: Duration = Duration::from_millis(50);
/// How often a pass says how it's going, besides at each generation it writes.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);

/// The threads that read a pass's messages: all but two of the machine's, from 1 to 8.
pub(super) fn readers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(2)).clamp(1, 8)
}

enum Ask {
    Index { account: String, folders: Vec<String> },
    Stop { account: String },
}

/// Events of which only the newest for an account matters (how its pass is going), held until
/// the window takes them: a window minimised through a long pass holds one, not hundreds. A
/// pass's end is kept apart from how the next is going, so one starting at once doesn't hide it.
#[derive(Default)]
pub(super) struct Latest(Mutex<BTreeMap<String, (Option<Event>, Option<Event>)>>);

impl Latest {
    pub(super) fn put(&self, account: &str, e: Event) {
        let mut g = lock(&self.0);
        let (ended, going) = g.entry(account.to_string()).or_default();
        if matches!(e, Event::Indexed { finished: true, .. }) {
            *ended = Some(e);
            *going = None;
        } else {
            *going = Some(e);
        }
    }

    pub(super) fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *lock(&self.0)).into_values().flat_map(|(ended, going)| ended.into_iter().chain(going)).collect()
    }

    pub(super) fn pop(&self) -> Option<Event> {
        let mut g = lock(&self.0);
        let mut first = g.first_entry()?;
        let (ended, going) = first.get_mut();
        let e = ended.take().or_else(|| going.take());
        if ended.is_none() && going.is_none() {
            first.remove();
        }
        e
    }
}

#[derive(Default)]
struct Shared {
    /// Accounts a data file is being imported into: their passes wait for its end. Held while a
    /// pass is chosen and said to be running, so an import or a stop then comes before or after.
    importing: Mutex<HashSet<String>>,
    /// The account whose pass is running, and what stops it.
    running: Mutex<Option<(String, Arc<AtomicBool>)>>,
    latest: Latest,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The pool's side of the indexer: its thread started at the first ask.
#[derive(Default)]
pub(super) struct Indexer {
    tx: Option<Sender<Ask>>,
    shared: Arc<Shared>,
}

/// An account being imported into: no pass runs for it until this is dropped.
pub(super) struct Importing {
    shared: Arc<Shared>,
    account: String,
}

impl Drop for Importing {
    fn drop(&mut self) {
        lock(&self.shared.importing).remove(&self.account);
    }
}

impl Indexer {
    /// Bring `account`'s index up to its mail in `folders` (none: all), once its asks go quiet.
    pub(super) fn ask(&mut self, store: &Store, sink: &Sink, account: String, folders: Vec<String>) {
        let mut ask = Ask::Index { account, folders };
        // A thread that has gone (it never should) is started again, once.
        for _ in 0..2 {
            let tx = match &self.tx {
                Some(tx) => tx.clone(),
                None => {
                    let (tx, rx) = mpsc::channel();
                    let (store, sink, shared) = (store.clone(), sink.clone(), self.shared.clone());
                    if let Err(e) = std::thread::Builder::new().name("mail-index".into()).spawn(move || run(&rx, &store, &sink, &shared)) {
                        log::warn!("the search index won't be kept up: {e}");
                        return;
                    }
                    self.tx = Some(tx.clone());
                    tx
                }
            };
            match tx.send(ask) {
                Ok(()) => return,
                Err(mpsc::SendError(a)) => {
                    ask = a;
                    self.tx = None;
                }
            }
        }
    }

    /// Stop `account`'s pass if one is running, and forget what was asked of it.
    pub(super) fn stop(&self, account: &str) {
        let _choosing = lock(&self.shared.importing);
        if let Some((a, stop)) = lock(&self.shared.running).as_ref()
            && a == account
        {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Ask::Stop { account: account.to_string() });
        }
    }

    /// A data file is being imported into `account`: its pass stops, and none runs for it until
    /// what this returns is dropped.
    pub(super) fn importing(&self, account: &str) -> Importing {
        let mut importing = lock(&self.shared.importing);
        importing.insert(account.to_string());
        if let Some((a, stop)) = lock(&self.shared.running).as_ref()
            && a == account
        {
            stop.store(true, Ordering::Relaxed);
        }
        drop(importing);
        Importing { shared: self.shared.clone(), account: account.to_string() }
    }

    /// How the passes are going, the newest for each account, since the last call.
    pub(super) fn latest(&self) -> Vec<Event> {
        self.shared.latest.take()
    }

    /// One of those, if any.
    pub(super) fn next_latest(&self) -> Option<Event> {
        self.shared.latest.pop()
    }

    /// Stop the running pass and let the thread end.
    pub(super) fn shutdown(&mut self) {
        if let Some((_, stop)) = lock(&self.shared.running).as_ref() {
            stop.store(true, Ordering::Relaxed);
        }
        self.tx = None;
    }
}

/// What's asked of an account: its folders (`None`: all) and when its pass may start.
pub(super) type Pending = BTreeMap<String, (Option<BTreeSet<String>>, Instant)>;

pub(super) fn want(pending: &mut Pending, account: String, folders: Vec<String>, at: Instant) {
    let e = pending.entry(account).or_insert_with(|| (Some(BTreeSet::new()), at));
    e.1 = e.1.max(at);
    match (&mut e.0, folders.is_empty()) {
        (_, true) => e.0 = None,
        (Some(set), false) => set.extend(folders),
        (None, false) => {}
    }
}

/// The indexer's thread: each account's pass once its asks have gone quiet, one at a time.
fn run(rx: &Receiver<Ask>, store: &Store, sink: &Sink, shared: &Shared) {
    let mut pending = Pending::new();
    // The accounts whose index another copy of MailCraft was writing at their last try: said
    // once, not at every retry.
    let mut elsewhere: HashSet<String> = HashSet::new();
    loop {
        let now = Instant::now();
        let choosing = lock(&shared.importing);
        let ready = pending.iter().find(|(a, (_, at))| *at <= now && !choosing.contains(*a)).map(|(a, _)| a.clone());
        let chosen = ready.and_then(|a| pending.remove_entry(&a));
        let stop = Arc::new(AtomicBool::new(false));
        if let Some((account, _)) = &chosen {
            *lock(&shared.running) = Some((account.clone(), stop.clone()));
        }
        let importing = choosing.clone();
        drop(choosing);
        let Some((account, (folders, _))) = chosen else {
            let wait = pending.iter().map(|(a, (_, at))| if importing.contains(a) { RETRY } else { at.saturating_duration_since(now) }).min();
            let ask = match wait {
                Some(w) => rx.recv_timeout(w),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match ask {
                Ok(Ask::Index { account, folders }) => want(&mut pending, account, folders, Instant::now() + QUIET),
                Ok(Ask::Stop { account }) => {
                    pending.remove(&account);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            continue;
        };
        let folders: Vec<String> = folders.map(|f| f.into_iter().collect()).unwrap_or_default();
        let mut report = |e: Event| match e {
            // Each of these counts: never one in place of another.
            e @ Event::AttachmentsKnown { .. } => sink.emit(e),
            e => {
                shared.latest.put(&account, e);
                sink.wake();
            }
        };
        let result = pass(store, &account, &folders, &stop, readers(), &mailcraft_mime::index_doc, &mut report);
        *lock(&shared.running) = None;
        if !matches!(result, Err(StoreError::Busy { .. })) {
            elsewhere.remove(&account);
        }
        match result {
            Ok(()) => {}
            // Held by another writer: tried again in a while. Another process's (the lease's)
            // is said once; searches meanwhile read the index as that writer leaves it.
            Err(StoreError::Busy { .. }) => {
                if !search::held_here(store, &account) && elsewhere.insert(account.clone()) {
                    log::info!("another copy of MailCraft is writing {account}'s search index; searching it as it stands");
                }
                want(&mut pending, account, folders, Instant::now() + RETRY);
            }
            Err(e) if search::is_damaged(&e) => {
                log::warn!("{account}: the search index is made again: {e}");
                if let Err(e) = search::set_aside(store, &account) {
                    log::warn!("{account}: {e}");
                }
            }
            // Stopped (the account removed, an import begun): what it couldn't write is no matter.
            Err(e) if stop.load(Ordering::Relaxed) => log::info!("{account}: an index pass stopped: {e}"),
            Err(e) => log::warn!("{account}: the search index wasn't brought up to the mail: {e}"),
        }
    }
}

/// Room for the sources being read at once ([`READING_BYTES`]).
struct Room {
    left: Mutex<u64>,
    freed: std::sync::Condvar,
}

impl Room {
    fn new() -> Room {
        Room { left: Mutex::new(READING_BYTES), freed: std::sync::Condvar::new() }
    }

    /// What a source of `size` bytes counts for.
    fn cost(size: u32) -> u64 {
        u64::from(size).clamp(LEAST_COST, READING_BYTES)
    }

    /// What the text taken out of a source holds until it's added (no more than the source
    /// counted for: what's left of it is given back once it's taken apart).
    fn kept(doc: &std::thread::Result<Option<IndexDoc>>, cost: u64) -> u64 {
        let bytes = match doc {
            Ok(Some(d)) => d.names.iter().fold(d.text.len(), |n, x| n.saturating_add(x.len())),
            _ => 0,
        };
        u64::try_from(bytes).unwrap_or(u64::MAX).min(cost)
    }

    /// Wait for room for `cost`; `false` when `done` says to stop meanwhile.
    fn take(&self, cost: u64, done: &dyn Fn() -> bool) -> bool {
        let mut left = lock(&self.left);
        while *left < cost {
            if done() {
                return false;
            }
            left = match self.freed.wait_timeout(left, ROOM_WAIT) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
        *left = left.saturating_sub(cost);
        true
    }

    fn give(&self, cost: u64) {
        let mut left = lock(&self.left);
        *left = left.saturating_add(cost).min(READING_BYTES);
        self.freed.notify_all();
    }
}

/// What a reader thread hands the pass: the message's place in the pass, what search keeps of it
/// (`Ok(None)`: its source has gone; `Err`: taking its text out panicked), and the room it holds
/// until it's added.
type Read = (usize, std::thread::Result<Option<IndexDoc>>, u64);

/// One pass over `account`'s index in `folders` (none: all): planned, its messages read on
/// `readers` threads and taken apart by `extract`, added, merged. `report` hears how it's going
/// ([`Event::Indexed`]), and its end unless it couldn't start, and of the summaries it put right
/// ([`Event::AttachmentsKnown`]). Stops between two messages when `stop` is set: what was added
/// is written, no folder marked.
pub(super) fn pass(
    store: &Store,
    account: &str,
    folders: &[String],
    stop: &AtomicBool,
    readers: usize,
    extract: &(dyn Fn(&[u8]) -> IndexDoc + Sync),
    report: &mut dyn FnMut(Event),
) -> StoreResult<()> {
    let mut p = Pass::start(store, account, folders)?;
    let indexed = |p: &Pass, finished| Event::Indexed {
        account: account.to_string(),
        work: p.work(),
        generation: p.generation(),
        failed: p.failed(),
        finished,
    };
    report(indexed(&p, false));
    settle_files(store, account, &mut p, report);
    let adds = p.adds().clone();
    let (next, quit, room) = (AtomicUsize::new(0), AtomicBool::new(false), Room::new());
    let (times, started) = (PassTimes::default(), Instant::now());
    let mut failed = None;
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::sync_channel::<Read>(IN_FLIGHT);
        for n in 0..readers.max(1) {
            let (tx, adds, next, quit, room, times) = (tx.clone(), &adds, &next, &quit, &room, &times);
            let read = move || {
                let done = || quit.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
                while !done() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(m) = adds.get(i) else { break };
                    let cost = Room::cost(m.size);
                    if !room.take(cost, &done) {
                        break;
                    }
                    let began = Instant::now();
                    let raw = catch_unwind(AssertUnwindSafe(|| store.read_body(&m.key)));
                    let read = began.elapsed();
                    let doc = raw.and_then(|raw| catch_unwind(AssertUnwindSafe(|| raw.map(|raw| extract(&raw)))));
                    times.add(read, began.elapsed().saturating_sub(read));
                    // The source is let go of: its text holds its room until it's added.
                    let kept = Room::kept(&doc, cost);
                    room.give(cost.saturating_sub(kept));
                    if tx.send((i, doc, kept)).is_err() {
                        room.give(kept);
                        break;
                    }
                }
            };
            if let Err(e) = std::thread::Builder::new().name(format!("mail-index-{n}")).spawn_scoped(s, read) {
                log::warn!("{account}: a thread to read mail for search: {e}");
            }
        }
        drop(tx);
        let mut told = Instant::now();
        let mut waited = Instant::now();
        for (i, doc, kept) in rx {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let began = Instant::now();
            times.waiting(began.saturating_duration_since(waited));
            let added = match doc {
                Ok(doc) => p.add(store, i, doc.as_ref()),
                Err(_) => {
                    log::warn!("{account}: a message couldn't be read for search");
                    p.add_unreadable(store, i)
                }
            };
            times.adding(began.elapsed());
            waited = Instant::now();
            room.give(kept);
            match added {
                Ok(flushed) if flushed.is_some() || told.elapsed() >= PROGRESS_EVERY => {
                    if flushed.is_some() {
                        settle_files(store, account, &mut p, report);
                    }
                    report(indexed(&p, false));
                    told = Instant::now();
                }
                Ok(_) => {}
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        // The readers stop at their next message (the channel they send on has gone too).
        quit.store(true, Ordering::Relaxed);
    });
    // What it found holds whether the pass ended well or not: the messages were read. Put right
    // before the folders are marked, so one that couldn't be isn't.
    settle_files(store, account, &mut p, report);
    let ended = match failed {
        Some(e) => Err(e),
        None => finish(store, &mut p, stop),
    };
    times.log(account, p.work().done, started.elapsed());
    report(indexed(&p, true));
    ended
}

/// Where a pass's time goes, all told: its readers' reading of sources and taking their text
/// out (each thread's summed), and the pass's own adding of what they read and waiting for it.
/// Logged at its end: which of them holds a pass back says what to make faster (the 41 GB
/// file's took 100 s on 1.6 cores of eight readers').
#[derive(Default)]
struct PassTimes {
    /// Microseconds of each.
    reading: AtomicU64,
    extracting: AtomicU64,
    adding: AtomicU64,
    waiting: AtomicU64,
}

impl PassTimes {
    fn count(of: &AtomicU64, d: Duration) {
        of.fetch_add(u64::try_from(d.as_micros()).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    fn add(&self, read: Duration, extract: Duration) {
        Self::count(&self.reading, read);
        Self::count(&self.extracting, extract);
    }

    fn adding(&self, d: Duration) {
        Self::count(&self.adding, d);
    }

    fn waiting(&self, d: Duration) {
        Self::count(&self.waiting, d);
    }

    fn log(&self, account: &str, messages: u64, all: Duration) {
        let s = |us: &AtomicU64| Duration::from_micros(us.load(Ordering::Relaxed)).as_secs_f64();
        log::info!(
            "{account}: an index pass read {messages} messages in {:.1} s: reading {:.1} s and taking text out {:.1} s on the readers, adding {:.1} s and waiting {:.1} s on the pass",
            all.as_secs_f64(),
            s(&self.reading),
            s(&self.extracting),
            s(&self.adding),
            s(&self.waiting),
        );
    }
}

/// Put right in their folders' lists the summaries the pass found to say otherwise than the
/// index about carrying files, and tell the window of each folder changed. A folder put right,
/// or that couldn't be, isn't marked: the next pass looks at it again ([`Pass::unmark`]).
fn settle_files(store: &Store, account: &str, p: &mut Pass, report: &mut dyn FnMut(Event)) {
    for (folder, found) in p.take_files() {
        match store.set_has_files(account, &folder, &found) {
            Ok(0) => {}
            Ok(_) => {
                p.unmark(&folder);
                report(Event::AttachmentsKnown { account: account.to_string(), folder, with: found.with, without: found.without });
            }
            Err(e) => {
                log::warn!("{account}: whether {} messages carry files wasn't kept: {e}", found.with.len().saturating_add(found.without.len()));
                p.unmark(&folder);
            }
        }
    }
}

/// The end of a pass: what's held written, the folders marked when it read all it planned, then
/// merged while merges are due.
fn finish(store: &Store, p: &mut Pass, stop: &AtomicBool) -> StoreResult<()> {
    let work = p.work();
    let complete = !stop.load(Ordering::Relaxed) && work.done >= work.total;
    p.finish(store, complete)?;
    let now = u64::try_from(now()).unwrap_or(0);
    while complete && !stop.load(Ordering::Relaxed) && p.merge_step(store, now)? {}
    Ok(())
}
