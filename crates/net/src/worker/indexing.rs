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
use std::sync::atomic::{AtomicBool, AtomicUsize};

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
        match result {
            Ok(()) => {}
            // Held by another writer: tried again in a while.
            Err(StoreError::Busy { .. }) => want(&mut pending, account, folders, Instant::now() + RETRY),
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

/// What a reader thread hands the pass: the message's place in the pass, and what search keeps
/// of it (`Ok(None)`: its source has gone; `Err`: taking its text out panicked).
type Read = (usize, std::thread::Result<Option<IndexDoc>>);

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
    let (next, quit) = (AtomicUsize::new(0), AtomicBool::new(false));
    let mut failed = None;
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::sync_channel::<Read>(IN_FLIGHT);
        for n in 0..readers.max(1) {
            let (tx, adds, next, quit) = (tx.clone(), &adds, &next, &quit);
            let read = move || {
                while !quit.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(m) = adds.get(i) else { break };
                    let doc = catch_unwind(AssertUnwindSafe(|| store.read_body(&m.key).map(|raw| extract(&raw))));
                    if tx.send((i, doc)).is_err() {
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
        for (i, doc) in rx {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let added = match doc {
                Ok(doc) => p.add(store, i, doc.as_ref()),
                Err(_) => {
                    log::warn!("{account}: a message couldn't be read for search");
                    p.add_unreadable(store, i)
                }
            };
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
    let ended = match failed {
        Some(e) => Err(e),
        None => finish(store, &mut p, stop),
    };
    // What it found holds whether the pass ended well or not: the messages were read.
    settle_files(store, account, &mut p, report);
    report(indexed(&p, true));
    ended
}

/// Put right in their folders' lists the summaries the pass found to say otherwise than the
/// index about carrying files, and tell the window of each folder changed.
fn settle_files(store: &Store, account: &str, p: &mut Pass, report: &mut dyn FnMut(Event)) {
    for (folder, found) in p.take_files() {
        match store.set_has_files(account, &folder, &found) {
            Ok(0) => {}
            Ok(_) => report(Event::AttachmentsKnown { account: account.to_string(), folder, with: found.with, without: found.without }),
            Err(e) => log::warn!("{account}: whether {} messages carry files wasn't kept: {e}", found.with.len().saturating_add(found.without.len())),
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
