//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, sort_folders};
use mailcraft_store::{OutboxItem, Store};

use crate::imap::{Client, uid_set};
use crate::jmap;
use crate::stream::Stream;
use crate::sync::{SyncOptions, fetch_body, sync_folder_as};
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

struct Handle {
    tx: Sender<Job>,
    thread: Option<JoinHandle<()>>,
    /// The account's second connection, for work the user sees happen (sending, automatic
    /// replies): it never waits behind a sync. `None` where it can't be used (POP, JMAP sends).
    quick: Option<Sender<Job>>,
    quick_thread: Option<JoinHandle<()>>,
    /// The main connection is at work: a message opened meanwhile goes to the second one.
    busy: Arc<AtomicBool>,
    /// The main connection's way to the second one, emptied when the account stops: otherwise
    /// each would keep the other's queue open, and neither thread would end.
    quick_link: QuickLink,
    /// What the pool did as it stopped the worker: that it did, and the drafts it kept then.
    stop: Arc<changes::Stopped>,
}

/// The second connection's queue, as the main one holds it ([`Handle::quick_link`]).
type QuickLink = Arc<Mutex<Option<Sender<Job>>>>;

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
use changes::{Order, PENDING_DRAFTS, PendingDraft};
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

impl Pool {
    pub fn new(store: Store, creds: Arc<dyn Credentials>) -> Pool {
        let (tx, rx) = mpsc::channel();
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
            filing: Arc::default(),
        }
    }

    /// Called (from worker threads) after every event, e.g. to request a UI repaint.
    pub fn set_waker(&self, f: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut w) = self.sink.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn submit(&mut self, job: Job) {
        self.release_compacted();
        let Some(job) = self.held_for_compaction(job) else { return };
        if let Job::SearchServer { search, .. } = &job {
            self.search.store(*search, Ordering::Relaxed);
        }
        match job {
            Job::FetchUrl { url } => self.fetch_url(url),
            Job::FetchCalendar { account, calendar, url, etag, modified } => self.fetch_calendar(account, calendar, url, etag, modified),
            Job::StopSearch => self.search.store(0, Ordering::Relaxed),
            Job::Index { account, folders } => self.indexer.ask(&self.store, &self.sink, account, folders),
            Job::StopIndex { account } => self.indexer.stop(&account),
            Job::CheckUpdates => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-updates".into()).spawn(move || {
                    let (releases, error) = match crate::updates::releases(&crate::updates::Feed::current()) {
                        Ok(r) => (r, None),
                        Err(e) => (Vec::new(), Some(e.to_string())),
                    };
                    sink.emit(Event::Releases { releases, error });
                });
                if let Err(e) = spawned {
                    self.sink.emit(Event::Releases { releases: Vec::new(), error: Some(e.to_string()) });
                }
            }
            Job::SignIn { account } => self.sign_in(account),
            Job::AllowDirectory { account } => self.allow_directory(account),
            Job::Discover { email } => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-discover".into()).spawn(move || {
                    let found = crate::autoconfig::discover(&email);
                    let (config, source) = match found {
                        Some(f) => (Some(Box::new(f.config)), f.source),
                        None => (None, String::new()),
                    };
                    sink.emit(Event::Discovered { email, config, source });
                });
                if let Err(e) = spawned {
                    log::warn!("autoconfiguration: {e}");
                }
            }
            Job::ImportPst { account, path, resume } => self.import_pst(account, path, resume),
            Job::RereadDataFile { account, path } => self.reread_data_file(account, path),
            Job::InspectDataFile { path } => self.inspect_data_file(path),
            Job::ReadMessageFile { path } => self.read_message_file(path),
            // Into an account on this computer: with the other changes to its mail. An account
            // with a server that isn't connected can't take them now.
            Job::FileMessages { account, folder, paths } if !self.workers.contains_key(&account) => {
                if self.configs.contains_key(&account) {
                    let failed = paths.iter().map(|p| (imports::filing::file_name(p), "the account is offline".to_string())).collect();
                    self.sink.emit(Event::MessagesFiled { account, folder, added: 0, failed });
                } else {
                    self.edits.file(&self.store, &self.sink, account, folder, paths);
                }
            }
            Job::StopImport { account, how } => self.stop_import(&account, how),
            Job::KeepImport { account } => self.keep_import(account),
            Job::DataFileHealth { account } => self.data_file_health(account),
            Job::CompactDataFile { account, mode } => self.compact_data_file(account, mode),
            Job::CancelCompact { account } => self.cancel_compact(&account),
            // The desktop keeps keys in the system's keychain: nothing to keep unlocked.
            Job::KeepUnlocked { .. } => {}
            j @ (Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::ForgetOlderIdentity { .. }
            | Job::UseOlderIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }) => self.smime(j),
            Job::Pgp { account, work } => self.pgp(account, work),
            Job::SealDraft { account, raw, folder, replaces, keep } => {
                let raw = self.seal(&account, raw.0);
                let account = account.id;
                self.submit(if keep { Job::KeepDraft { account, raw, folder, replaces } } else { Job::SaveDraft { account, raw, folder, replaces } });
            }
            Job::ExportPst { account, path, name, follow_up } => self.export_pst(account, path, name, follow_up),
            Job::SetSync { messages, offline } => {
                let opts =
                    SyncOptions { window: if messages == 0 { usize::MAX } else { messages as usize }, prefetch: offline as usize, ..self.opts };
                if opts.window != self.opts.window || opts.prefetch != self.opts.prefetch {
                    self.opts = opts;
                    // Connected accounts start again with it (a rare change, from Options).
                    let running: Vec<AccountConfig> = self.workers.keys().filter_map(|id| self.configs.get(id).cloned()).collect();
                    for c in running {
                        self.start(c, None);
                    }
                }
            }
            Job::WorkOffline { on } => {
                self.offline = on;
                if on {
                    let ids: Vec<String> = self.workers.keys().cloned().collect();
                    for id in ids {
                        self.stop(&id);
                        self.sink.emit(Event::Status { account: id, status: SyncStatus::Offline });
                    }
                } else {
                    let configs: Vec<AccountConfig> = self.configs.values().cloned().collect();
                    for c in configs {
                        self.start(c, None);
                    }
                }
            }
            Job::Start { account, password } if self.offline => {
                // Working offline: remembered (and its password kept) for when that's turned off.
                if let Some(p) = &password
                    && let Err(e) = self.creds.set(&account, &p.0)
                {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the password could not be saved in the keychain: {e}"),
                    });
                }
                self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Offline });
                self.configs.insert(account.id.clone(), account);
            }
            Job::Start { account, password } => self.start(account, password),
            // An empty one: the calendar server signs in as the mail server again.
            Job::DavPassword { account, password } if password.0.is_empty() => self.creds.delete_secret(&account, DAV_SECRET),
            Job::DavPassword { account, password } => {
                if let Err(e) = self.creds.set_secret(&account, DAV_SECRET, &password.0) {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the calendar server's password could not be saved in the keychain: {e}"),
                    });
                }
            }
            Job::SmtpPassword { account, password } => {
                if let Err(e) = self.creds.set_smtp(&account, &password.0) {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the outgoing server's password could not be saved in the keychain: {e}"),
                    });
                }
            }
            Job::Remove { account } => {
                self.forget_saves(&account);
                self.stop(&account);
                self.indexer.stop(&account);
                // An import into it is thrown away, its data file with it.
                self.stop_import(&account, mailcraft_model::job::StopHow::Discard);
                if let Some(c) = self.configs.remove(&account) {
                    self.creds.delete(&c);
                    self.creds.delete_smtp(&c);
                    // Its certificates' keys and its calendar server's password go with it.
                    smime::forget_all(self.creds.as_ref(), &c);
                    pgp::forget_all(self.creds.as_ref(), &c);
                    self.creds.delete_secret(&c, DAV_SECRET);
                    self.creds.delete_secret(&c, DIRECTORY_SECRET);
                }
            }
            Job::FetchBody { key } if !self.workers.contains_key(&key.account) => {
                // Offline or not started: the cache may still have it.
                match self.store.read_body(&key) {
                    Some(raw) => self.sink.emit(Event::Body { key, raw }),
                    None => self.sink.emit(Event::BodyFailed { key, error: "not downloaded yet, and the account is offline".into() }),
                }
            }
            Job::Send { account, id, raw, from, recipients, sent_folder } if !self.workers.contains_key(&account) => {
                // Not connected: straight into the Outbox.
                let item = OutboxItem { id, from, recipients, sent_folder, queued: now(), attempts: 0, error: Some("offline".into()), not_before: 0 };
                match self.store.outbox_add(&account, &item, &raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account, id, error: "offline".into() }),
                    Err(e) => self.sink.emit(Event::SendFailed { account, id, error: e.to_string() }),
                }
            }
            // Offline: changes wait in the cache for the next sync.
            Job::SyncDav { account } if !self.workers.contains_key(&account) => {}
            Job::SearchServer { account, query, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::ServerSearched { account, query, keys: Vec::new(), error: Some("the account is offline".into()), done: true });
            }
            Job::SearchDirectory { account, query } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::Directory { account, query, people: Vec::new(), error: Some("the account is offline".into()), consent: false });
            }
            // Working offline: nothing connects, by choice.
            Job::SyncAll { account } | Job::SyncFolder { account, .. } | Job::FlushOutbox { account }
                if self.offline && !self.workers.contains_key(&account) => {}
            // Closing with a message written: on disk before the app goes (the worker may be busy
            // or about to stop); filed on the server at the next connection, as offline drafts.
            Job::KeepDraft { account, raw, folder, replaces } => {
                if let Err(e) = keep_draft(&self.store, &account, &folder, replaces, raw, true, &self.order) {
                    log::warn!("{account}: the draft couldn't be kept: {e}");
                }
            }
            Job::SaveDraft { account, raw, folder, replaces } if !self.workers.contains_key(&account) => {
                let message_id = mailcraft_model::thread::written_message_id(&raw);
                // Handed out now, as one handed to a worker is: newer than a save of the draft a
                // stopped worker is still filing, which, filed, leaves it kept (found in review:
                // taken for older, it was taken off this computer, and never got to the server).
                DraftOrder::handed(&self.order, &raw);
                let error = keep_draft(&self.store, &account, &folder, replaces, raw, false, &self.order).err().map(|e| e.to_string());
                let kept = error.is_none();
                self.sink.emit(Event::DraftSaved { account: account.clone(), message_id, error });
                // Said after the save's end, so it's the last word on it.
                if kept {
                    self.sink.emit(Event::Notice {
                        account,
                        message: "the draft is kept on this computer; it goes to Drafts when you're back online".into(),
                    });
                }
            }
            Job::GetVacation { account } | Job::SetVacation { account, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::Vacation { account, vacation: None, error: Some("the account is offline".into()) });
            }
            // Persisted on the pool's thread for changes, not the account's: it may be minutes
            // into a full sync, and waiting meant the cache could be overwritten by a folder
            // refresh and an exit dropped the job. Nor the caller's, the window's (`edits.rs`).
            // The account's thread is woken after, to upload the queue at its next opportunity.
            // POP accounts keep the old route since their worker does extra local bookkeeping.
            Job::Apply { account, op } if self.records_edits(&account) => {
                let wake = self.workers.get(&account).map(|h| h.tx.clone());
                self.edits.send(&self.store, &self.sink, account, op, wake);
            }
            other => {
                let id = other.account().to_string();
                let quick = self.configs.get(&id).is_some_and(|c| quick_job(&other, c));
                self.save_on_its_way(&other);
                match self.workers.get(&id) {
                    Some(h) => {
                        // A message opened while the main connection syncs a folder, or the
                        // calendars, doesn't wait for that to end (issue #24).
                        let opened = matches!(other, Job::FetchBody { .. })
                            && h.busy.load(Ordering::Relaxed)
                            && self.configs.get(&id).is_some_and(|c| c.pop.is_none());
                        let quick = quick || opened;
                        // Sending and automatic replies go on the second connection; if it has
                        // stopped, the main one does it (after its sync).
                        let other = match h.quick.as_ref().filter(|_| quick) {
                            Some(q) => match q.send(other) {
                                Ok(()) => return,
                                Err(mpsc::SendError(j)) => j,
                            },
                            None => other,
                        };
                        if h.tx.send(other).is_err() {
                            self.sink.emit(Event::Notice {
                                account: id,
                                message: "the account's connection stopped; restart it from the account menu".into(),
                            });
                        }
                    }
                    None => self.sink.emit(Event::Notice { account: id, message: "the account is not connected".into() }),
                }
            }
        }
    }

    /// Whether this side writes an account's edits to the cache and ops.json: always when no
    /// thread runs for it; for a running one unless it is POP (whose thread does it, with its
    /// own bookkeeping).
    fn records_edits(&self, account: &str) -> bool {
        match self.workers.get(account) {
            None => true,
            Some(_) => matches!(self.configs.get(account), Some(c) if c.pop.is_none()),
        }
    }

    /// Events reported since the last call (of how a pass of the search index is going, the
    /// newest for each account). Jobs held while a compaction had a data file to itself go on.
    pub fn poll(&mut self) -> Vec<Event> {
        self.release_compacted();
        let mut events: Vec<Event> = self.rx.try_iter().collect();
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

    fn start(&mut self, account: AccountConfig, password: Option<Secret>) {
        self.stop(&account.id);
        if let Some(p) = &password
            && let Err(e) = self.creds.set(&account, &p.0)
        {
            self.sink.emit(Event::Notice { account: account.id.clone(), message: format!("the password could not be saved in the keychain: {e}") });
        }
        let pw = password.map(|p| p.0).or_else(|| self.creds.get(&account));
        let Some(pw) = pw else {
            self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
            return;
        };
        let (tx, rx) = mpsc::channel();
        let auth = Auth::shared(pw);
        let mut w = Worker::new(account.clone(), auth.clone(), self.store.clone(), self.sink.clone(), self.opts);
        w.creds = Some(self.creds.clone());
        w.sieve_port = self.sieve_port;
        w.search = self.search.clone();
        // The second connection: quiet (the account's status is the main one's), connecting
        // only when there's something to do.
        let (qtx, qrx) = mpsc::channel();
        let mut q = Worker::new(account.clone(), auth, self.store.clone(), self.sink.clone(), self.opts);
        q.creds = Some(self.creds.clone());
        q.sieve_port = self.sieve_port;
        q.main = Some(tx.clone());
        q.given_back = w.given_back.clone();
        q.whole_only = w.whole_only.clone();
        let busy = w.busy.clone();
        let quick_thread = std::thread::Builder::new().name(format!("mail-{}-quick", account.id)).spawn(move || q.run_quick(qrx)).ok();
        let quick = quick_thread.is_some().then_some(qtx);
        let quick_link: QuickLink = Arc::new(Mutex::new(quick.clone()));
        w.quick = quick_link.clone();
        let stop: Arc<changes::Stopped> = Arc::default();
        w.stop = stop.clone();
        w.order = self.order.clone();
        w.filing = self.filing.clone();
        let name = format!("mail-{}", account.id);
        match std::thread::Builder::new().name(name).spawn(move || w.run(rx)) {
            Ok(thread) => {
                self.workers.insert(account.id.clone(), Handle { tx, thread: Some(thread), quick, quick_thread, busy, quick_link, stop });
            }
            Err(e) => self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Error { error: e.to_string() } }),
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
            // The thread notices the closed channel within half a second (IDLE slices); don't
            // block the UI waiting for a slow network call to finish.
            for t in [h.thread.take(), h.quick_thread.take()].into_iter().flatten() {
                std::thread::spawn(move || {
                    let _ = t.join();
                });
            }
        }
    }

    /// Stop every account's threads.
    fn stop_all(&mut self) {
        let ids: Vec<String> = self.workers.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
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
mod send;
mod smime;

/// The keychain entry of an account's calendar and contacts server's own password.
pub(crate) const DAV_SECRET: &str = "dav";
/// Where an Exchange Online account's directory sign-in (its refresh token) is kept (issue #9).
pub(crate) const DIRECTORY_SECRET: &str = "directory";
mod sync;
mod threads;

pub use credentials::{Credentials, MemoryCredentials};
use remote::*;
pub use remote::{apply_remote, apply_remote_with};

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

const FULL_SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(120);
/// When Sent Items is read again after Graph or Exchange took a message (its copy is filed a
/// moment later), and once more in case that was too soon.
const SENT_SOON: Duration = Duration::from_secs(3);
const SENT_LATER: Duration = Duration::from_secs(15);
/// An account's second connection lets go after this long with nothing to do.
const QUICK_IDLE: Duration = Duration::from_secs(5 * 60);

/// The connection to an account's server.
enum Conn {
    Imap(Client<Stream>),
    Jmap(jmap::Client),
    Graph(crate::graph::Client),
    Ews(Box<crate::ews::Client>),
    /// POP3: signed in once to check the password; every download is a session of its own.
    Pop,
}

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
    backoff: Duration,
    next_attempt: Instant,
    last_full: Option<Instant>,
    /// The token for searching the organisation's directory (Exchange Online), while it lasts.
    directory_token: Option<crate::oauth::Tokens>,
    /// Not connecting again until asked (Send / Receive, a new start): the sign-in was refused,
    /// or the server said no to the account itself.
    auth_failed: bool,
    /// An OAuth refusal was answered with a fresh token once; another means signing in again.
    token_retried: bool,
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
}

/// Where an account keeps its folders' own windows.
const WINDOWS: &str = "windows.json";
/// The most messages of a folder a search of the server brings in.
const FOUND_MAX: usize = 500;

impl Worker {
    fn new(acc: AccountConfig, auth: Arc<Mutex<Auth>>, store: Store, sink: Sink, opts: SyncOptions) -> Worker {
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
            order: changes::Order::default(),
            filing: Arc::default(),
            kept: changes::KeptDrafts::default(),
            counts_unsaid: Vec::new(),
            gone_meanwhile: Vec::new(),
            removals,
            whole_only: Arc::default(),
            busy: Arc::default(),
            backoff: Duration::from_secs(5),
            next_attempt: Instant::now(),
            last_full: None,
            directory_token: None,
            auth_failed: false,
            token_retried: false,
            closed: false,
            creds: None,
            sieve_port: crate::sieve::PORT,
            no_push: false,
            pushed: false,
            soon: Vec::new(),
            search: Arc::new(AtomicU64::new(0)),
        }
    }

    fn id(&self) -> String {
        self.acc.id.clone()
    }

    fn status(&self, status: SyncStatus) {
        // The second connection's comings and goings aren't the account's, nor a stopped one's.
        if self.main.is_none() && !self.stopped() {
            match &status {
                SyncStatus::Error { error } => log::warn!("{}: {error}", self.acc.id),
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
        self.status(SyncStatus::Connecting);
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
                    #[cfg(test)]
                    connect::waits::began(&self.store, &self.acc.id);
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
            if self.last_full.is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY) {
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
