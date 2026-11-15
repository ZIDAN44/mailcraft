//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, FailureKind, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, sort_folders};
use mailcraft_store::{OutboxItem, Store};

use crate::imap::{Client, uid_set};
use crate::jmap;
use crate::stream::Stream;
use crate::sync::{SyncOptions, sync_folder_as};
use crate::{NetError, Result, smtp};

/// Sends events and wakes the UI.
#[derive(Clone)]
struct Sink {
    tx: Sender<Event>,
    wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

impl Sink {
    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
        self.wake();
    }

    fn wake(&self) {
        if let Ok(w) = self.wake.lock()
            && let Some(f) = w.as_ref()
        {
            f();
        }
    }
}

/// An account's thread, as the pool holds it. The thread itself isn't held: stopped, it ends on its
/// own once it notices (a slow network call may hold it a while), and nothing waits for it.
struct Handle {
    tx: Sender<Job>,
    /// The account's second connection, for work the user sees happen (sending, automatic
    /// replies): it never waits behind a sync. `None` where it can't be used (POP, JMAP sends).
    quick: Option<Sender<Job>>,
    /// Set by the account's thread when the keychain had no password: the pool forgets the
    /// thread, as if it had never started ([`Pool::poll`]).
    no_password: Arc<AtomicBool>,
    /// The main connection is at work: a message opened meanwhile goes to the second one.
    busy: Arc<AtomicBool>,
    /// The main connection's way to the second one, emptied when the account stops: otherwise
    /// each would keep the other's queue open, and neither thread would end.
    quick_link: QuickLink,
    /// What the pool did as it stopped the worker: that it did, and the drafts it kept then.
    stop: Arc<changes::Stopped>,
    /// How many waits to connect again the main connection began ([`Worker::waits`]): read by
    /// the tests only, counted in every build.
    #[cfg_attr(not(test), allow(dead_code))]
    waits: Arc<AtomicUsize>,
}

impl Handle {
    /// The pool's hold on an account's main connection `w` (its only one, for calendars alone),
    /// which reads `tx`.
    fn of(w: &Worker, tx: Sender<Job>) -> Handle {
        let quick_link = w.quick.clone();
        let quick = quick_link.lock().ok().and_then(|q| q.clone());
        Handle { tx, quick, no_password: w.no_password.clone(), busy: w.busy.clone(), quick_link, stop: w.stop.clone(), waits: w.waits.clone() }
    }
}

/// The second connection's queue, as the main one holds it ([`Handle::quick_link`]).
type QuickLink = Arc<Mutex<Option<Sender<Job>>>>;

/// What an account's connections share with the pool, and with each other, handed to each as
/// it's made ([`Worker::with`]): set one by one after, a part forgotten ran against a stand-in of
/// its own rather than the pool's.
#[derive(Clone)]
struct Shared {
    store: Store,
    sink: Sink,
    opts: SyncOptions,
    /// Where passwords live (none for a worker of the tests' own).
    creds: Option<Arc<dyn Credentials>>,
    /// The server search still wanted ([`Pool::search`]).
    search: Arc<AtomicU64>,
    order: changes::Order,
    filing: Arc<changes::Filing>,
    sieve_port: u16,
}

/// What an account signs in with, shared by its connections: the password or refresh token, and
/// the current access token (refreshed by one, used by both).
struct Auth {
    secret: String,
    access: Option<crate::oauth::Tokens>,
}

impl Auth {
    fn shared(secret: impl Into<String>) -> Arc<Mutex<Auth>> {
        Arc::new(Mutex::new(Auth { secret: secret.into(), access: None }))
    }

    /// An account's sign-in, held: taken back from a thread that panicked holding it (each part
    /// of it is only ever set whole), never left unset for want of the lock.
    fn held(auth: &Mutex<Auth>) -> std::sync::MutexGuard<'_, Auth> {
        auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Queue a job, unless it's a folder to read that is waiting already (a few messages sent in a
/// row each ask for Sent Items: once is enough), or a calendar sync that is (a second one would
/// run before the first one's results are merged, and send its new items again: "changed on the
/// server meanwhile").
fn push_job(queue: &mut VecDeque<Job>, j: Job) {
    if matches!(j, Job::SyncFolder { .. } | Job::SyncDav { .. }) && queue.contains(&j) {
        return;
    }
    queue.push_back(j);
}

/// Whether a job goes on the account's second connection: sending (not over JMAP, whose filing
/// needs the main connection's mailbox ids) and automatic replies.
fn quick_job(j: &Job, acc: &AccountConfig) -> bool {
    match j {
        Job::Send { .. } => acc.pop.is_none() && acc.jmap.is_none(),
        Job::GetVacation { .. } | Job::SetVacation { .. } => acc.pop.is_none(),
        _ => false,
    }
}

/// Jobs a full sync handles between two folders rather than at its end.
fn between_folders(j: &Job) -> bool {
    matches!(j, Job::FetchBody { .. } | Job::Apply { .. } | Job::SearchDirectory { .. })
}

/// Runs jobs for every account.
pub struct Pool {
    store: Store,
    creds: Arc<dyn Credentials>,
    sink: Sink,
    rx: Receiver<Event>,
    workers: HashMap<String, Handle>,
    configs: HashMap<String, AccountConfig>,
    pub opts: SyncOptions,
    /// Where ManageSieve listens on IMAP hosts (automatic replies); tests change it.
    pub sieve_port: u16,
    /// Picture downloads, on their own thread so they never wait behind a sync.
    fetcher: Option<Sender<String>>,
    /// Working offline: no account connects until it's turned off.
    offline: bool,
    /// The server search still wanted ([`Job::SearchServer`]'s `search`; 0: none), shared with
    /// every worker so a search running stops when it's no longer wanted.
    search: Arc<AtomicU64>,
    /// The search index kept up in the background.
    indexer: indexing::Indexer,
    /// Data files being imported, by account, and the last one looked at (`imports.rs`).
    imports: imports::Imports,
    /// Changes to mail, made in the cache on a thread of their own (`edits.rs`).
    edits: edits::Edits,
    /// Data files being compacted, and the jobs waiting for one to give its file back
    /// (`compact.rs`).
    compacts: compact::Compacts,
    /// The order drafts were handed out to be saved and kept as MailCraft closed, this run
    /// (`changes.rs`), shared with every worker.
    order: changes::Order,
    /// The drafts being filed on the server, shared with every worker (`changes/filing.rs`).
    filing: Arc<changes::Filing>,
}

use changes::{DraftOrder, keep_draft};
#[cfg(test)]
use changes::{KEPT_DRAFTS, Order, PENDING_DRAFTS, PendingDraft, kept_raws, sweep_kept};
pub use mailcraft_model::MICROSOFT_NO_PASSWORDS;

/// What a mailbox Microsoft won't open over IMAP needs: IMAP switched on (Outlook.com's own
/// setting; an organisation's administrator for work accounts), the right account picked in the
/// browser, or Microsoft Graph, which doesn't use IMAP.
fn microsoft_imap_closed(email: &str, signed_in_as: Option<&str>) -> String {
    let who = signed_in_as.map(|o| format!(" (the browser signed in as {o})")).unwrap_or_default();
    let fix = if mailcraft_model::microsoft_consumer(email) {
        "Turn on IMAP on Outlook.com (Settings › Mail › Forwarding and IMAP), or connect the account through Microsoft Graph instead"
    } else {
        "Your organisation may have turned IMAP off: connect the account through Microsoft Graph instead, or ask its administrator"
    };
    format!("{}{who}. {fix}.", mailcraft_model::MICROSOFT_IMAP_CLOSED)
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX)).unwrap_or(0)
}

/// `m` locked, even when a thread panicked holding it: what it guards is still read.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Pool {
    pub fn new(store: Store, creds: Arc<dyn Credentials>) -> Pool {
        let (tx, rx) = mpsc::channel();
        let filing: Arc<changes::Filing> = Arc::default();
        // A test's folders go once the pool and its accounts' threads (each holds `filing`)
        // have ended: none writes in them after.
        #[cfg(test)]
        {
            let threads = Arc::downgrade(&filing);
            crate::data_files::scratch::wait_for(move || threads.strong_count() > 0);
        }
        Pool {
            store,
            creds,
            sink: Sink { tx, wake: Arc::new(Mutex::new(None)) },
            rx,
            workers: HashMap::new(),
            configs: HashMap::new(),
            opts: SyncOptions::default(),
            sieve_port: crate::sieve::PORT,
            fetcher: None,
            offline: false,
            search: Arc::new(AtomicU64::new(0)),
            indexer: indexing::Indexer::default(),
            imports: imports::Imports::default(),
            edits: edits::Edits::default(),
            compacts: compact::Compacts::default(),
            order: changes::Order::default(),
            filing,
        }
    }

    /// Called (from worker threads) after every event, e.g. to request a UI repaint.
    pub fn set_waker(&self, f: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut w) = self.sink.wake.lock() {
            *w = Some(f);
        }
    }

    /// Events reported since the last call (of how a pass of the search index is going, the
    /// newest for each account). Jobs held while a compaction had a data file to itself go on.
    pub fn poll(&mut self) -> Vec<Event> {
        self.release_compacted();
        let mut events: Vec<Event> = self.rx.try_iter().collect();
        self.forget_unstarted();
        events.extend(self.indexer.latest());
        events
    }

    /// Block until an event arrives or `timeout` passes (for the CLI).
    pub fn wait(&self, timeout: Duration) -> Option<Event> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(e) = self.indexer.next_latest() {
                return Some(e);
            }
            // The indexer's news isn't sent on the channel: looked at between slices.
            let left = end.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(e) => return Some(e),
                Err(RecvTimeoutError::Timeout) if !left.is_zero() => {}
                Err(_) => return None,
            }
        }
    }

    /// The accounts whose keychain had no password ([`Handle::no_password`]): their threads
    /// have ended, and each is as an account never started (offline here, its jobs answered by
    /// the pool) till it's given one.
    fn forget_unstarted(&mut self) {
        let unstarted: Vec<String> = self.workers.iter().filter(|(_, h)| h.no_password.load(Ordering::Relaxed)).map(|(id, _)| id.clone()).collect();
        for id in unstarted {
            self.stop(&id);
            self.configs.remove(&id);
        }
    }

    /// What an account's connections share with the pool.
    fn shared(&self) -> Shared {
        Shared {
            store: self.store.clone(),
            sink: self.sink.clone(),
            opts: self.opts,
            creds: Some(self.creds.clone()),
            search: self.search.clone(),
            order: self.order.clone(),
            filing: self.filing.clone(),
            sieve_port: self.sieve_port,
        }
    }

    fn start(&mut self, account: AccountConfig, password: Option<Secret>) {
        self.stop(&account.id);
        let (tx, rx) = mpsc::channel();
        // The password is kept in the keychain, or read from it, by the account's own thread
        // (`credentials.rs`): the keychain may ask the user first (macOS, after an update), and
        // the window keeps drawing meanwhile.
        let mut w = Worker::main_connection(account.clone(), self.shared(), password.map(|p| p.0));
        // No mail server (issue #61): its calendars and contacts are all its thread does.
        let calendars_only = account.is_calendar_only();
        if !calendars_only {
            let q = w.second_connection(tx.clone(), self.shared());
            w.pair(q);
        }
        let handle = Handle::of(&w, tx);
        let name = format!("mail-{}", account.id);
        let spawned = std::thread::Builder::new().name(name).spawn(move || if calendars_only { w.run_calendars(rx) } else { w.run(rx) });
        match spawned {
            Ok(_) => {
                self.workers.insert(account.id.clone(), handle);
            }
            Err(e) => self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::error(e.to_string()) }),
        }
        self.configs.insert(account.id.clone(), account);
    }

    fn stop(&mut self, account: &str) {
        if let Some(mut h) = self.workers.remove(account) {
            // Its drafts still to save kept here first, before a new worker reads what's kept.
            self.keep_saves_on_their_way(account, &h.stop);
            drop(h.tx);
            drop(h.quick.take());
            if let Ok(mut q) = h.quick_link.lock() {
                q.take();
            }
            // The thread notices the closed channel within half a second (IDLE slices), and ends
            // on its own: the UI never waits for a slow network call to finish. The second
            // connection's thread ends as its queue closes (it was started by this one).
        }
    }

    /// How many waits to connect again `account`'s main connection has begun.
    #[cfg(test)]
    fn waits_begun(&self, account: &str) -> usize {
        self.workers.get(account).map_or(0, |h| h.waits.load(Ordering::Relaxed))
    }

    /// Stop every account's threads.
    fn stop_all(&mut self) {
        let ids: Vec<String> = self.workers.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }

    /// Shut down as [`Pool::shutdown`] does, then wait for every account's threads to end (each
    /// holds the pool's `filing`): nothing writes in the test's folder after, when it's removed.
    #[cfg(test)]
    pub(crate) fn shutdown_and_wait(mut self) {
        self.shutdown();
        crate::waiting::eventually("the pool's account threads to end", || Arc::strong_count(&self.filing) == 1);
    }
}

mod changes;
mod compact;
mod connect;
mod credentials;
mod directory;
mod edits;
mod imports;
mod indexing;
mod opening;
mod pgp;
mod remote;
mod routing;
mod send;
mod smime;

/// The keychain entry of an account's calendar and contacts server's own password.
pub(crate) const DAV_SECRET: &str = "dav";
/// Where an Exchange Online account's directory sign-in (its refresh token) is kept (issue #9).
pub(crate) const DIRECTORY_SECRET: &str = "directory";
mod sync;

pub use credentials::{Credentials, MemoryCredentials};
#[cfg(test)]
pub(crate) use remote::apply_remote;
pub use remote::apply_remote_with;
use remote::*;

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

const FULL_SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(120);
/// When Sent Items is read again after Graph or Exchange took a message (its copy is filed a
/// moment later), and once more in case that was too soon.
const SENT_SOON: Duration = Duration::from_secs(3);
const SENT_LATER: Duration = Duration::from_secs(15);
/// How often the wait for those looks whether its connection has ended.
const SENT_LATER_SLICE: Duration = Duration::from_millis(250);
/// An account's second connection lets go after this long with nothing to do.
const QUICK_IDLE: Duration = Duration::from_secs(5 * 60);
/// The first wait before connecting again after a connection is lost; it grows with each failure,
/// and comes back to this once signed in again (the second connection) or synced (the main one).
const FIRST_BACKOFF: Duration = Duration::from_secs(5);

struct Worker {
    acc: AccountConfig,
    auth: Arc<Mutex<Auth>>,
    /// Set on an account's second connection: the main one's queue, for what this one leaves to
    /// it (a message that can't go yet, Sent Items to look at).
    main: Option<Sender<Job>>,
    store: Store,
    sink: Sink,
    opts: SyncOptions,
    conn: Option<Conn>,
    /// JMAP and Graph accounts: message numbers and mailbox ids.
    jstate: jmap::State,
    folders: Vec<Folder>,
    queue: VecDeque<Job>,
    /// Messages whose text went first (issue #24), the whole of each still to come: downloaded
    /// when nothing else waits, so the next one opened shows its text without waiting on them.
    rest: VecDeque<mailcraft_model::MessageKey>,
    /// The account's second connection, which downloads the whole of those (the main one stays
    /// free to open the next message).
    quick: QuickLink,
    /// Messages it couldn't get, shared with it: those it gives back are downloaded on the main
    /// connection (one opened again meanwhile is handed to it again, not downloaded twice).
    given_back: Arc<std::sync::Mutex<std::collections::HashSet<mailcraft_model::MessageKey>>>,
    /// What the pool did as it stopped this worker: it says nothing more of the account, and
    /// leaves the drafts the pool kept then (`changes/drafts.rs`).
    stop: Arc<changes::Stopped>,
    /// Held while this worker lives: a thread it starts to ask something of the main connection
    /// later (Sent Items read again) ends with it, rather than keep a stopped account's main
    /// thread waiting for its jobs.
    alive: Arc<()>,
    /// The order drafts were handed out to be saved and kept as MailCraft closed (the pool's).
    order: changes::Order,
    /// The drafts being filed on the server now, by this worker or another (the pool's).
    filing: Arc<changes::Filing>,
    /// The drafts kept on this computer, as this worker files them (`changes/drafts.rs`).
    kept: changes::KeptDrafts,
    /// The folders whose counts a sync changed and that aren't said yet ([`Worker::say_counts`]).
    counts_unsaid: Vec<String>,
    /// Folders gone from the server whose cache was kept as changes to the folders waited to be
    /// sent ([`Worker::relist_with`]): it goes once they're sent.
    gone_meanwhile: Vec<String>,
    /// How many times its account had been removed as it started ([`Store::while_kept`]).
    removals: u64,
    /// Messages whose text the main connection gave first, shared with the second one: of those
    /// it brings only the whole (others it's handed were opened while the main one was busy).
    whole_only: Arc<std::sync::Mutex<std::collections::HashSet<mailcraft_model::MessageKey>>>,
    /// Set while the main connection works rather than waits ([`Worker::working`]).
    busy: Arc<AtomicBool>,
    /// How many waits to connect again after a connection failed it began, shared with the
    /// pool: counted, not timed (the tests ask).
    waits: Arc<AtomicUsize>,
    backoff: Duration,
    next_attempt: Instant,
    last_full: Option<Instant>,
    /// When the Inbox was last looked at, for an account that looks at set times (issue #58).
    looked: Option<Instant>,
    /// The token for searching the organisation's directory (Exchange Online), while it lasts.
    directory_token: Option<crate::oauth::Tokens>,
    /// Not connecting again until asked (Send / Receive, a new start): the sign-in was refused,
    /// or the server said no to the account itself.
    auth_failed: bool,
    /// An OAuth refusal was answered with a fresh token once; another means signing in again.
    token_retried: bool,
    /// The connections made again at once with a renewed token, lately: only so many are.
    renewals: connect::Renewals,
    closed: bool,
    /// Where a rotated OAuth refresh token is saved.
    creds: Option<Arc<dyn Credentials>>,
    /// The ManageSieve port (4190; tests use their own).
    sieve_port: u16,
    /// The JMAP server can't push: poll instead.
    no_push: bool,
    /// The last push wait ended in an Inbox sync (so the next one needn't check first).
    pushed: bool,
    /// Folders to look at again soon: Graph and Exchange file a sent message's copy a moment
    /// after taking it, so Sent Items is read again then rather than at the next poll.
    soon: Vec<(Instant, String)>,
    /// Folders asked to keep more than the setting (Show more on the server, issue #4): how
    /// many of each (`windows.json` with the account).
    windows: HashMap<String, usize>,
    /// The server search still wanted (the pool's).
    search: Arc<AtomicU64>,
    /// The calendar server's Digest challenge, answered from the next sync's first request.
    dav_digest: Option<crate::dav::digest::Challenge>,
    /// Why the calendars and contacts couldn't be read, as last said: said once, not at every
    /// sync.
    dav_said: Option<String>,
    /// Done on this thread before it first connects ([`Worker::settle_password`]): the password
    /// just given, to keep in the keychain, or `None` to read the one kept there.
    keychain: Option<Option<String>>,
    /// The second connection, started once the password is known.
    quick_start: Option<Box<dyn FnOnce() + Send>>,
    /// Set when the keychain had no password ([`Handle::no_password`]).
    no_password: Arc<AtomicBool>,
    /// Outbox messages sent, or refused for good, that the Outbox couldn't be told of (its files
    /// couldn't be changed): not sent again while this runs.
    outbox_done: std::collections::HashSet<u64>,
}

/// Where an account keeps its folders' own windows.
const WINDOWS: &str = "windows.json";
/// The most messages of a folder a search of the server brings in.
const FOUND_MAX: usize = 500;

impl Worker {
    /// A connection of the tests' own: nothing shared with a pool, no passwords kept.
    #[cfg(test)]
    fn new(acc: AccountConfig, auth: Arc<Mutex<Auth>>, store: Store, sink: Sink, opts: SyncOptions) -> Worker {
        let shared = Shared {
            store,
            sink,
            opts,
            creds: None,
            search: Arc::default(),
            order: changes::Order::default(),
            filing: Arc::default(),
            sieve_port: crate::sieve::PORT,
        };
        Worker::with(acc, auth, shared)
    }

    /// An account's main connection, its only one when it has calendars alone. `password`: one
    /// just given, kept in the keychain on its own thread before it first connects; `None` to read
    /// the one kept there.
    fn main_connection(acc: AccountConfig, shared: Shared, password: Option<String>) -> Worker {
        let mut w = Worker::with(acc, Auth::shared(String::new()), shared);
        w.keychain = Some(password);
        w
    }

    /// The account's second connection, for work the user sees happen (sending, automatic
    /// replies): quiet, the account's status being the main one's (`self`), and connecting only
    /// when there's something to do. It signs in as the main one does, leaves to it (`main`, its
    /// queue) what it can't do, and shares with it the messages given back and those whose text
    /// went first.
    fn second_connection(&self, main: Sender<Job>, shared: Shared) -> Worker {
        let mut q = Worker::with(self.acc.clone(), self.auth.clone(), shared);
        q.main = Some(main);
        q.given_back = self.given_back.clone();
        q.whole_only = self.whole_only.clone();
        q
    }

    /// The main connection's second one, `q`: started once the password is known (what's for it
    /// waits in its queue till then).
    fn pair(&mut self, q: Worker) {
        let (qtx, qrx) = mpsc::channel();
        let name = format!("mail-{}-quick", self.acc.id);
        self.quick_start = Some(Box::new(move || {
            if let Err(e) = std::thread::Builder::new().name(name).spawn(move || q.run_quick(qrx)) {
                log::warn!("the account's second connection didn't start: {e}");
            }
        }));
        self.quick = Arc::new(Mutex::new(Some(qtx)));
    }

    /// A connection of `acc`'s, signing in with `auth`, with what it shares with the pool.
    fn with(acc: AccountConfig, auth: Arc<Mutex<Auth>>, shared: Shared) -> Worker {
        let Shared { store, sink, opts, creds, search, order, filing, sieve_port } = shared;
        let folders = store.load_folders(&acc.id).unwrap_or_default();
        let jstate = if acc.is_api() { jmap::State::load(&store, &acc.id) } else { jmap::State::default() };
        // Online only (issue #4): nothing downloaded in full ahead of reading; a message comes
        // when it's opened, and is held in memory only.
        let opts = if acc.keeps_online() { SyncOptions { prefetch: 0, ..opts } } else { opts };
        let windows = store.load_account_doc(&acc.id, WINDOWS).unwrap_or_default();
        let removals = store.removals(&acc.id);
        Worker {
            acc,
            auth,
            main: None,
            store,
            sink,
            opts,
            conn: None,
            jstate,
            folders,
            windows,
            queue: VecDeque::new(),
            rest: VecDeque::new(),
            quick: Arc::default(),
            given_back: Arc::default(),
            stop: Arc::default(),
            alive: Arc::default(),
            order,
            filing,
            kept: changes::KeptDrafts::default(),
            counts_unsaid: Vec::new(),
            gone_meanwhile: Vec::new(),
            removals,
            whole_only: Arc::default(),
            busy: Arc::default(),
            waits: Arc::default(),
            backoff: FIRST_BACKOFF,
            next_attempt: Instant::now(),
            last_full: None,
            looked: None,
            directory_token: None,
            auth_failed: false,
            token_retried: false,
            renewals: connect::Renewals::default(),
            closed: false,
            creds,
            sieve_port,
            no_push: false,
            pushed: false,
            soon: Vec::new(),
            search,
            dav_digest: None,
            dav_said: None,
            keychain: None,
            quick_start: None,
            no_password: Arc::default(),
            outbox_done: std::collections::HashSet::new(),
        }
    }

    fn id(&self) -> String {
        self.acc.id.clone()
    }

    fn status(&self, status: SyncStatus) {
        // The second connection's comings and goings aren't the account's, nor a stopped one's.
        if self.main.is_none() && !self.stopped() {
            match &status {
                SyncStatus::Error { error, .. } => log::warn!("{}: {error}", self.acc.id),
                SyncStatus::NeedsPassword { error } => log::warn!("{}: needs signing in again: {error}", self.acc.id),
                SyncStatus::Online { .. } | SyncStatus::Syncing { .. } => {}
                s => log::info!("{}: {s:?}", self.acc.id),
            }
            self.sink.emit(Event::Status { account: self.id(), status });
        }
    }

    fn notice(&self, message: impl Into<String>) {
        self.sink.emit(Event::Notice { account: self.id(), message: message.into() });
    }

    fn drain(&mut self, rx: &Receiver<Job>) {
        loop {
            match rx.try_recv() {
                Ok(j) => push_job(&mut self.queue, j),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    return;
                }
            }
        }
    }

    fn run(mut self, rx: Receiver<Job>) {
        #[cfg(test)]
        let _ending = self.main.is_none().then(|| connect::ended::Ending(self.store.root().to_path_buf(), self.acc.id.clone()));
        self.status(SyncStatus::Connecting);
        if !self.settle_password() {
            return;
        }
        while !self.closed {
            self.working(true);
            self.drain(&rx);
            if self.closed {
                break;
            }
            self.fresh_token();
            // Connected by the last job itself: the drafts kept here go first.
            if self.file_next_kept() {
                continue;
            }
            if let Some(job) = self.queue.pop_front() {
                self.handle(job);
                continue;
            }
            if let Some(key) = self.rest.pop_front() {
                self.fetch_whole(key);
                continue;
            }
            if self.conn.is_none() {
                if self.auth_failed {
                    self.working(false);
                    match rx.recv() {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(_) => break,
                    }
                    continue;
                }
                let wait = self.next_attempt.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    self.waits.fetch_add(1, Ordering::Relaxed);
                    self.working(false);
                    match rx.recv_timeout(wait) {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    continue;
                }
                self.connect_and_sync();
                continue;
            }
            if self.last_full.is_none_or(|t| t.elapsed() >= self.full_sync_every()) {
                if self.last_full.is_some() && self.check_every().is_some() && self.dropped_while_quiet(Instant::now()) {
                    continue;
                }
                self.sync_all(&rx);
                continue;
            }
            self.wait_for_mail(&rx);
        }
        self.keep_unsaved_drafts(&rx);
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
    }
}

#[cfg(test)]
mod tests;
