//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, sort_folders};
use mailcraft_store::{OutboxItem, Store};

use crate::imap::{Client, uid_set};
use crate::stream::Stream;
use crate::sync::{SyncOptions, fetch_body, sync_folder};
use crate::{NetError, Result, smtp};

/// Where passwords live. The desktop app backs this with the system keychain.
pub trait Credentials: Send + Sync {
    fn get(&self, account: &AccountConfig) -> Option<String>;
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete(&self, account: &AccountConfig);
}

/// Passwords held in memory only (tests, the CLI, and when no keychain is available).
#[derive(Default)]
pub struct MemoryCredentials(Mutex<HashMap<String, String>>);

impl Credentials for MemoryCredentials {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&account.id).cloned())
    }
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(account.id.clone(), password.to_string());
        Ok(())
    }
    fn delete(&self, account: &AccountConfig) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&account.id);
        }
    }
}

/// Sends events and wakes the UI.
#[derive(Clone)]
struct Sink {
    tx: Sender<Event>,
    wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

impl Sink {
    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
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
    /// Picture downloads, on their own thread so they never wait behind a sync.
    fetcher: Option<Sender<String>>,
}

fn now() -> i64 {
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
            fetcher: None,
        }
    }

    fn fetch_url(&mut self, url: String) {
        if self.fetcher.is_none() {
            let (tx, rx) = mpsc::channel::<String>();
            let sink = self.sink.clone();
            let spawned = std::thread::Builder::new().name("mail-pictures".into()).spawn(move || {
                for url in rx {
                    let r = crate::http::get(&url, MAX_PICTURE);
                    let e = match r {
                        Ok(resp) if resp.ok() => Event::Fetched { url, data: Some(resp.body), error: None },
                        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
                        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
                    };
                    sink.emit(e);
                }
            });
            match spawned {
                Ok(_) => self.fetcher = Some(tx),
                Err(e) => return self.sink.emit(Event::Fetched { url, data: None, error: Some(e.to_string()) }),
            }
        }
        if let Some(tx) = &self.fetcher {
            let _ = tx.send(url);
        }
    }

    /// Called (from worker threads) after every event, e.g. to request a UI repaint.
    pub fn set_waker(&self, f: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut w) = self.sink.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn submit(&mut self, job: Job) {
        match job {
            Job::FetchUrl { url } => self.fetch_url(url),
            Job::Start { account, password } => self.start(account, password),
            Job::Remove { account } => {
                self.stop(&account);
                if let Some(c) = self.configs.remove(&account) {
                    self.creds.delete(&c);
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
                let item = OutboxItem { id, from, recipients, sent_folder, queued: now(), attempts: 0, error: Some("offline".into()) };
                match self.store.outbox_add(&account, &item, &raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account, id, error: "offline".into() }),
                    Err(e) => self.sink.emit(Event::SendFailed { account, id, error: e.to_string() }),
                }
            }
            Job::Apply { account, op } if !self.workers.contains_key(&account) => {
                // Offline: change the cache now and keep the change for the next connection.
                match self.store.apply_op(&account, &op) {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            let folders = self.store.load_folders(&account).unwrap_or_default();
                            self.sink.emit(Event::Folders { account: account.clone(), folders });
                        }
                        let mut ops = self.store.load_ops(&account).unwrap_or_default();
                        ops.push(op);
                        if let Err(e) = self.store.save_ops(&account, &ops) {
                            self.sink.emit(Event::Notice { account: account.clone(), message: e.to_string() });
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: account.clone(), folder });
                        }
                    }
                    Err(e) => self.sink.emit(Event::Notice { account, message: e.to_string() }),
                }
            }
            other => {
                let id = other.account().to_string();
                match self.workers.get(&id) {
                    Some(h) => {
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

    /// Events reported since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    /// Block until an event arrives or `timeout` passes (for the CLI).
    pub fn wait(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
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
        let w = Worker::new(account.clone(), pw, self.store.clone(), self.sink.clone(), self.opts);
        let name = format!("mail-{}", account.id);
        match std::thread::Builder::new().name(name).spawn(move || w.run(rx)) {
            Ok(thread) => {
                self.workers.insert(account.id.clone(), Handle { tx, thread: Some(thread) });
            }
            Err(e) => self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Error { error: e.to_string() } }),
        }
        self.configs.insert(account.id.clone(), account);
    }

    fn stop(&mut self, account: &str) {
        if let Some(mut h) = self.workers.remove(account) {
            drop(h.tx);
            // The thread notices the closed channel within half a second (IDLE slices); don't
            // block the UI waiting for a slow network call to finish.
            if let Some(t) = h.thread.take() {
                std::thread::spawn(move || {
                    let _ = t.join();
                });
            }
        }
    }

    pub fn shutdown(&mut self) {
        let ids: Vec<String> = self.workers.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

const FULL_SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(120);

struct Worker {
    acc: AccountConfig,
    pw: String,
    store: Store,
    sink: Sink,
    opts: SyncOptions,
    client: Option<Client<Stream>>,
    folders: Vec<Folder>,
    ops: Vec<Op>,
    queue: VecDeque<Job>,
    backoff: Duration,
    next_attempt: Instant,
    last_full: Option<Instant>,
    auth_failed: bool,
    closed: bool,
}

impl Worker {
    fn new(acc: AccountConfig, pw: String, store: Store, sink: Sink, opts: SyncOptions) -> Worker {
        let ops = store.load_ops(&acc.id).unwrap_or_default();
        let folders = store.load_folders(&acc.id).unwrap_or_default();
        Worker {
            acc,
            pw,
            store,
            sink,
            opts,
            client: None,
            folders,
            ops,
            queue: VecDeque::new(),
            backoff: Duration::from_secs(5),
            next_attempt: Instant::now(),
            last_full: None,
            auth_failed: false,
            closed: false,
        }
    }

    fn id(&self) -> String {
        self.acc.id.clone()
    }

    fn status(&self, status: SyncStatus) {
        self.sink.emit(Event::Status { account: self.id(), status });
    }

    fn notice(&self, message: impl Into<String>) {
        self.sink.emit(Event::Notice { account: self.id(), message: message.into() });
    }

    fn drain(&mut self, rx: &Receiver<Job>) {
        loop {
            match rx.try_recv() {
                Ok(j) => self.queue.push_back(j),
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
            self.drain(&rx);
            if self.closed {
                break;
            }
            if let Some(job) = self.queue.pop_front() {
                self.handle(job);
                continue;
            }
            if self.client.is_none() {
                if self.auth_failed {
                    match rx.recv() {
                        Ok(j) => self.queue.push_back(j),
                        Err(_) => break,
                    }
                    continue;
                }
                let wait = self.next_attempt.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    match rx.recv_timeout(wait) {
                        Ok(j) => self.queue.push_back(j),
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
        if let Some(c) = self.client.as_mut() {
            c.logout();
        }
    }

    /// Wait for new mail: IDLE on the Inbox when the server supports it, else poll.
    fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        let inbox = self.inbox();
        let idle = self.client.as_ref().is_some_and(|c| c.has("IDLE"));
        if !idle || inbox.is_none() {
            match rx.recv_timeout(POLL_EVERY) {
                Ok(j) => self.queue.push_back(j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(f) = inbox {
                        self.sync_one(&f);
                    }
                }
            }
            return;
        }
        let Some(inbox) = inbox else { return };
        let result = (|| -> Result<bool> {
            let c = self.client.as_mut().ok_or_else(|| NetError::Closed("not connected".into()))?;
            if c.selected() != Some(inbox.as_str()) {
                c.select(&inbox, false)?;
            }
            let queue = &mut self.queue;
            let closed = &mut self.closed;
            c.idle(IDLE_FOR, || {
                loop {
                    match rx.try_recv() {
                        Ok(j) => queue.push_back(j),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            *closed = true;
                            break;
                        }
                    }
                }
                !queue.is_empty() || *closed
            })
        })();
        match result {
            Ok(true) => self.sync_one(&inbox),
            Ok(false) => {}
            Err(e) => self.failed(e),
        }
    }

    fn inbox(&self) -> Option<String> {
        self.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone())
    }

    /// Note a failure; drop the connection when it is unusable.
    fn failed(&mut self, e: NetError) {
        log::warn!("{}: {e}", self.acc.id);
        match e {
            NetError::Auth(m) => {
                self.client = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error: m });
            }
            e if e.is_fatal() => {
                self.client = None;
                self.next_attempt = Instant::now() + self.backoff;
                self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
                self.status(SyncStatus::Error { error: e.to_string() });
            }
            e => self.notice(e.to_string()),
        }
    }

    fn connect(&mut self) -> Result<()> {
        if self.client.is_some() {
            return Ok(());
        }
        self.status(SyncStatus::Connecting);
        let mut c = Client::connect(&self.acc.imap)?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        c.login(&user, &self.pw)?;
        self.client = Some(c);
        self.backoff = Duration::from_secs(5);
        Ok(())
    }

    fn connect_and_sync(&mut self) {
        if let Err(e) = self.connect() {
            self.failed(e);
            return;
        }
        self.replay();
        self.flush_outbox();
        self.last_full = None;
    }

    fn client(&mut self) -> Result<&mut Client<Stream>> {
        self.client.as_mut().ok_or_else(|| NetError::Closed("not connected".into()))
    }

    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    fn relist(&mut self) -> bool {
        let id = self.id();
        let listed = (|| -> Result<Vec<Folder>> { self.client()?.list(&id) })();
        match listed {
            Ok(mut list) => {
                for f in &mut list {
                    if let Some(old) = self.folders.iter().find(|o| o.path == f.path) {
                        f.unread = old.unread;
                        f.total = old.total;
                    }
                }
                sort_folders(&mut list);
                // Folders gone from the server lose their cache.
                for old in &self.folders {
                    if !list.iter().any(|f| f.path == old.path) {
                        let _ = self.store.clear_folder(&id, &old.path);
                    }
                }
                self.folders = list;
                self.publish_folders();
                true
            }
            Err(e) => {
                self.failed(e);
                false
            }
        }
    }

    fn sync_all(&mut self, rx: &Receiver<Job>) {
        if !self.relist() {
            return;
        }
        let paths: Vec<String> = self.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect();
        for path in paths {
            self.drain(rx);
            // Opened messages jump the queue.
            let urgent: Vec<Job> = self.queue.iter().filter(|j| matches!(j, Job::FetchBody { .. })).cloned().collect();
            self.queue.retain(|j| !matches!(j, Job::FetchBody { .. }));
            for j in urgent {
                self.handle(j);
            }
            if self.closed || self.client.is_none() {
                return;
            }
            self.sync_one(&path);
        }
        if self.client.is_some() {
            self.last_full = Some(Instant::now());
            // Anything left from a failed attempt gets another try every full sync.
            self.flush_outbox();
            self.status(SyncStatus::Online { at: now() });
        }
    }

    fn publish_folders(&self) {
        let _ = self.store.save_folders(&self.acc.id, &self.folders);
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    fn sync_one(&mut self, path: &str) {
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        let (store, opts, id) = (self.store.clone(), self.opts, self.id());
        let result = self.client().and_then(|c| sync_folder(c, &store, &id, path, &opts));
        match result {
            Ok(r) => {
                let first = self.folders.iter().find(|f| f.path == path).is_some_and(|f| f.total == 0 && r.total > 0);
                if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                    let counts_changed = f.unread != r.unread || f.total != r.total;
                    f.unread = r.unread;
                    f.total = r.total;
                    if counts_changed {
                        self.publish_folders();
                    }
                }
                if r.touched() || first {
                    self.sink.emit(Event::FolderChanged { account: id, folder: path.to_string() });
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            Err(e) => self.failed(e),
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::SyncAll { .. } => {
                if self.client.is_none() {
                    self.auth_failed = false;
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                self.last_full = None;
            }
            Job::SyncFolder { folder, .. } => {
                if self.client.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.client.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::FetchBody { key } => self.fetch(key),
            Job::Apply { op, .. } => {
                match self.store.apply_op(&self.acc.id, &op) {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                            self.publish_folders();
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: self.id(), folder });
                        }
                    }
                    Err(e) => self.notice(e.to_string()),
                }
                self.ops.push(op);
                let _ = self.store.save_ops(&self.acc.id, &self.ops);
                if self.client.is_some() {
                    self.replay();
                }
            }
            Job::Send { id, raw, from, recipients, sent_folder, .. } => self.send(id, &raw, &from, &recipients, sent_folder),
            Job::SaveDraft { raw, folder, replaces, .. } => {
                let r = (|| -> Result<()> {
                    if self.client.is_none() {
                        self.connect()?;
                    }
                    let c = self.client()?;
                    c.append(&folder, "\\Draft \\Seen", &raw)?;
                    if let Some(old) = replaces {
                        expunge_keys(c, &[old])?;
                    }
                    Ok(())
                })();
                match r {
                    Ok(()) => self.sync_one(&folder),
                    Err(e) => self.failed(e),
                }
            }
            Job::FlushOutbox { .. } => self.flush_outbox(),
            Job::FetchUrl { .. } => {}
            Job::Start { .. } | Job::Remove { .. } => {}
        }
    }

    fn fetch(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let r = (|| -> Result<Vec<u8>> {
            if self.client.is_none() {
                self.connect()?;
            }
            fetch_body(self.client()?, &store, &key)
        })();
        match r {
            Ok(raw) => {
                self.sink.emit(Event::Body { key: key.clone(), raw });
                self.sink.emit(Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
            }
            Err(e) => {
                self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }

    fn send(&mut self, id: u64, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<String>) {
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        if let Err(e) = smtp::send(&self.acc.smtp, &user, &self.pw, from, recipients, raw) {
            if e.is_retryable() {
                // Offline or asked to wait: keep it in the Outbox and send it on reconnect.
                let item = OutboxItem {
                    id,
                    from: from.to_string(),
                    recipients: recipients.to_vec(),
                    sent_folder,
                    queued: now(),
                    attempts: 1,
                    error: Some(e.to_string()),
                };
                match self.store.outbox_add(&self.acc.id, &item, raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account: self.id(), id, error: e.to_string() }),
                    Err(se) => {
                        self.sink.emit(Event::SendFailed { account: self.id(), id, error: format!("{e} (and the Outbox couldn't keep it: {se})") })
                    }
                }
            } else {
                self.sink.emit(Event::SendFailed { account: self.id(), id, error: e.to_string() });
            }
            return;
        }
        self.after_sent(id, raw, sent_folder);
    }

    /// Send what waits in the Outbox, oldest first. Stops at the first failure that may clear
    /// up later (still offline); a message refused for good stays, marked with the error.
    fn flush_outbox(&mut self) {
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        for mut item in self.store.outbox(&self.acc.id) {
            let Some(raw) = self.store.outbox_raw(&self.acc.id, item.id) else { continue };
            if item.error.as_deref().is_some_and(|e| e.starts_with("refused:")) {
                // Refused for good: waits for the user to edit or delete it.
                continue;
            }
            match smtp::send(&self.acc.smtp, &user, &self.pw, &item.from, &item.recipients, &raw) {
                Ok(()) => {
                    self.store.outbox_remove(&self.acc.id, item.id);
                    let folder = item.sent_folder.take();
                    self.after_sent(item.id, &raw, folder);
                }
                Err(e) => {
                    let retry = e.is_retryable();
                    item.attempts = item.attempts.saturating_add(1);
                    item.error = Some(if retry { e.to_string() } else { format!("refused: {e}") });
                    let _ = self.store.outbox_update(&self.acc.id, &item);
                    if retry {
                        self.sink.emit(Event::Queued { account: self.id(), id: item.id, error: e.to_string() });
                        return;
                    }
                    self.sink.emit(Event::SendFailed { account: self.id(), id: item.id, error: e.to_string() });
                }
            }
        }
    }

    /// A message went out: tell the engine and file a copy in Sent.
    fn after_sent(&mut self, id: u64, raw: &[u8], sent_folder: Option<String>) {
        self.sink.emit(Event::Sent { account: self.id(), id });
        // Gmail files sent mail itself; elsewhere keep a copy in Sent.
        let files_itself = self.acc.smtp.host.ends_with("gmail.com") || self.acc.smtp.host.ends_with("googlemail.com");
        if let Some(folder) = sent_folder.filter(|_| !files_itself) {
            let r = (|| -> Result<()> {
                if self.client.is_none() {
                    self.connect()?;
                }
                self.client()?.append(&folder, "\\Seen", raw).map(|_| ())
            })();
            match r {
                Ok(()) => self.sync_one(&folder),
                Err(e) => self.notice(format!("sent, but no copy was saved in Sent Items: {e}")),
            }
        }
    }

    /// Replay offline changes on the server, oldest first.
    fn replay(&mut self) {
        let mut destinations: Vec<String> = Vec::new();
        let mut relist = false;
        while let Some(op) = self.ops.first().cloned() {
            let r = (|| -> Result<()> { apply_remote(self.client()?, &op) })();
            match r {
                Ok(()) => {
                    match &op {
                        Op::Move { to, .. } => destinations.push(to.clone()),
                        Op::RenameFolder { to, .. } => {
                            relist = true;
                            destinations.push(to.clone());
                        }
                        Op::CreateFolder { .. } | Op::DeleteFolder { .. } => relist = true,
                        Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } => {}
                    }
                    self.ops.remove(0);
                }
                Err(e) if e.is_fatal() => {
                    let _ = self.store.save_ops(&self.acc.id, &self.ops);
                    return self.failed(e);
                }
                Err(e) => {
                    // The server refused: drop the change and resync so the cache matches the
                    // server again (forcing a full flag check).
                    self.ops.remove(0);
                    let mut folders: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
                    folders.dedup();
                    for f in &folders {
                        if let Ok(mut st) = self.store.load_folder(&self.acc.id, f) {
                            st.highest_modseq = 0;
                            st.uid_next = 0;
                            let _ = self.store.save_folder(&self.acc.id, f, &st);
                        }
                    }
                    relist |= op.is_folder_op();
                    self.sink.emit(Event::OpFailed { account: self.id(), op, error: e.to_string() });
                    destinations.extend(folders);
                }
            }
        }
        let _ = self.store.save_ops(&self.acc.id, &self.ops);
        if relist {
            self.relist();
        }
        destinations.sort();
        destinations.dedup();
        for d in destinations {
            if self.client.is_some() {
                self.sync_one(&d);
            }
        }
    }
}

/// Group keys by folder, select each folder, and skip keys whose UIDVALIDITY is stale.
fn by_folder<S: std::io::Read + std::io::Write>(
    c: &mut Client<S>,
    keys: &[MessageKey],
    mut f: impl FnMut(&mut Client<S>, &str, &str) -> Result<()>,
) -> Result<()> {
    let mut folders: Vec<&str> = keys.iter().map(|k| k.folder.as_str()).collect();
    folders.sort_unstable();
    folders.dedup();
    for folder in folders {
        let info = c.select(folder, false)?;
        let uids: Vec<u32> = keys.iter().filter(|k| k.folder == folder && k.validity == info.validity).map(|k| k.uid).collect();
        if uids.is_empty() {
            continue;
        }
        for chunk in crate::imap::uid_chunks(&uids, 500) {
            f(c, folder, &uid_set(&chunk))?;
        }
    }
    Ok(())
}

fn expunge_keys<S: std::io::Read + std::io::Write>(c: &mut Client<S>, keys: &[MessageKey]) -> Result<()> {
    by_folder(c, keys, |c, _, set| {
        c.uid_store(set, true, Flag::Deleted.imap())?;
        c.expunge(set)
    })
}

/// Apply one change on the server.
pub fn apply_remote<S: std::io::Read + std::io::Write>(c: &mut Client<S>, op: &Op) -> Result<()> {
    match op {
        Op::SetFlag { keys, flag, on } => by_folder(c, keys, |c, _, set| c.uid_store(set, *on, flag.imap())),
        // A keyword that isn't an atom would make the command malformed: the engine never sends one.
        Op::SetKeyword { keys, keyword, on } if mailcraft_model::is_keyword(keyword) => {
            by_folder(c, keys, |c, _, set| c.uid_store(set, *on, keyword))
        }
        Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => by_folder(c, keys, |c, folder, set| if folder == to { Ok(()) } else { c.uid_move(set, to) }),
        Op::Expunge { keys } => expunge_keys(c, keys),
        Op::CreateFolder { path } => c.create(path),
        Op::RenameFolder { from, to } => c.rename(from, to),
        Op::DeleteFolder { path } => c.delete(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_credentials() {
        let m = MemoryCredentials::default();
        let a = AccountConfig { id: "a1".into(), ..Default::default() };
        assert_eq!(m.get(&a), None);
        m.set(&a, "pw").unwrap();
        assert_eq!(m.get(&a).as_deref(), Some("pw"));
        m.delete(&a);
        assert_eq!(m.get(&a), None);
    }

    #[test]
    fn pool_without_password_asks_for_one() {
        let dir = std::env::temp_dir().join(format!("mailcraft-pool-{}", std::process::id()));
        let store = Store::open(&dir).unwrap();
        let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
        let a = AccountConfig { id: "a1".into(), ..Default::default() };
        p.submit(Job::Start { account: a, password: None });
        assert!(matches!(p.poll().as_slice(), [Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }]));
        p.submit(Job::SyncAll { account: "a1".into() });
        assert!(matches!(p.poll().as_slice(), [Event::Notice { .. }]));
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        p.submit(Job::FetchBody { key: key.clone() });
        assert!(matches!(p.poll().as_slice(), [Event::BodyFailed { .. }]));
        p.store.write_body(&key, b"Subject: cached\r\n\r\nx").unwrap();
        p.submit(Job::FetchBody { key });
        assert!(matches!(p.poll().as_slice(), [Event::Body { .. }]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
