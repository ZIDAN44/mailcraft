//! The browser's network side: the engine's jobs run over JMAP and `fetch`, one account's at a
//! time in order (as the desktop's worker threads do), and their events wait for the next frame.
//! New mail comes by push: the session's event stream, read through `fetch` (an `EventSource`
//! can't send the `Authorization` header), with polling as the fallback.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use mailcraft_engine::model::job::{CompactReport, DataFileFacts};
use mailcraft_engine::model::{AccountConfig, Event, Folder, FolderRole, Job, Op, SyncStatus, sort_folders};
use mailcraft_engine::store::Store;
use mailcraft_jmap::{Client, Error, State, SyncOptions};
use mailcraft_ui_egui::Backend;

use crate::fetch::{Fetch, events, sleep};

mod indexing;
mod jobs;
mod smime;

/// How often the Inbox is checked for new mail, and every how many checks everything is.
const POLL_MS: i32 = 120_000;
const FULL_EVERY: u32 = 8;
/// How long before a push stream that ended is opened again (and how often a stream waits for
/// its account to connect).
const PUSH_RETRY_MS: i32 = 30_000;
const PUSH_WAIT_MS: i32 = 3_000;
/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

/// What an account's runner does next.
enum Task {
    Connect,
    SyncAll,
    SyncFolder(String),
    /// Boxed: a job is far larger than the other tasks, and accounts queue many.
    Job(Box<Job>),
}

struct Account {
    config: AccountConfig,
    /// Kept for this page only.
    password: Option<String>,
    client: Option<Rc<Client<Fetch>>>,
    state: State,
    folders: Vec<Folder>,
    queue: VecDeque<Task>,
    running: bool,
    /// Bumped on every start, so an old poller stops.
    generation: u32,
    /// The push stream is open: the Inbox needn't be polled.
    pushing: bool,
    /// How many messages folders asked to show more keep (issue #4), by path.
    windows: HashMap<String, usize>,
}

/// The account document with [`Account::windows`] (the desktop's, too).
const WINDOWS: &str = "windows.json";

struct Shared {
    store: Store,
    events: Vec<Event>,
    accounts: HashMap<String, Account>,
    ctx: egui::Context,
    opts: SyncOptions,
    /// The server search still wanted (`Job::SearchServer`'s `search`; 0: none).
    search: u64,
    /// Each account's certificates unlocked with their password, their keys: this page only.
    smime: HashMap<String, Vec<mailcraft_engine::smime::Identity>>,
    /// The passwords that opened them, for keeping them unlocked when that's asked (gap 7):
    /// in memory, and sealed with the browser's key only then.
    smime_passwords: HashMap<String, Vec<String>>,
    /// Each account's sealing of them: only the latest seal is kept, none once keeping is off.
    smime_sealing: HashMap<String, crate::sealing::Sealing>,
    /// The search indexes asked to be brought up to the mail.
    indexing: indexing::Indexing,
}

type Rcs = Rc<RefCell<Shared>>;

pub struct WebBackend {
    shared: Rcs,
}

impl WebBackend {
    pub fn new(store: Store, ctx: egui::Context) -> WebBackend {
        let opts = SyncOptions { window: 300, prefetch: 20, ..SyncOptions::default() };
        WebBackend {
            shared: Rc::new(RefCell::new(Shared {
                store,
                events: Vec::new(),
                accounts: HashMap::new(),
                ctx,
                opts,
                search: 0,
                smime: HashMap::new(),
                smime_passwords: HashMap::new(),
                smime_sealing: HashMap::new(),
                indexing: indexing::Indexing::default(),
            })),
        }
    }
}

fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

fn emit(sh: &Rcs, e: Event) {
    let ctx = {
        let mut s = sh.borrow_mut();
        s.events.push(e);
        s.ctx.clone()
    };
    ctx.request_repaint();
}

fn status(sh: &Rcs, id: &str, status: SyncStatus) {
    emit(sh, Event::Status { account: id.to_string(), status });
}

fn notice(sh: &Rcs, id: &str, message: String) {
    emit(sh, Event::Notice { account: id.to_string(), message });
}

fn store(sh: &Rcs) -> Store {
    sh.borrow().store.clone()
}

fn client(sh: &Rcs, id: &str) -> Option<Rc<Client<Fetch>>> {
    sh.borrow().accounts.get(id).and_then(|a| a.client.clone())
}

fn state(sh: &Rcs, id: &str) -> State {
    sh.borrow().accounts.get(id).map(|a| a.state.clone()).unwrap_or_default()
}

fn put_state(sh: &Rcs, id: &str, st: State) {
    let store = store(sh);
    if let Err(e) = st.save(&store, id) {
        log::warn!("{id}: {e}");
    }
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.state = st;
    }
}

/// Queue work for an account and make sure its runner is going.
fn enqueue(sh: &Rcs, id: &str, task: Task) {
    let start = {
        let mut s = sh.borrow_mut();
        let Some(a) = s.accounts.get_mut(id) else { return };
        a.queue.push_back(task);
        !std::mem::replace(&mut a.running, true)
    };
    if start {
        wasm_bindgen_futures::spawn_local(run(sh.clone(), id.to_string()));
    }
}

async fn run(sh: Rcs, id: String) {
    loop {
        let task = {
            let mut s = sh.borrow_mut();
            let Some(a) = s.accounts.get_mut(&id) else { return };
            match a.queue.pop_front() {
                Some(t) => t,
                None => {
                    a.running = false;
                    return;
                }
            }
        };
        match task {
            Task::Connect => connect(&sh, &id).await,
            Task::SyncAll => sync_all(&sh, &id).await,
            Task::SyncFolder(f) => sync_one(&sh, &id, &f).await,
            Task::Job(j) => jobs::job(&sh, &id, *j).await,
        }
    }
}

/// A failure: signing in again, reconnecting later, or just a notice.
fn failed(sh: &Rcs, id: &str, e: Error) {
    log::warn!("{id}: {e}");
    match e {
        Error::Auth(m) => {
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
                a.password = None;
            }
            status(sh, id, SyncStatus::NeedsPassword { error: m });
        }
        e @ (Error::Io(_) | Error::Closed(_) | Error::Protocol(_) | Error::Tls(_)) => {
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
            }
            status(sh, id, SyncStatus::error(e.to_string()));
        }
        e => notice(sh, id, e.to_string()),
    }
}

async fn connect(sh: &Rcs, id: &str) {
    let Some((config, password)) = sh.borrow().accounts.get(id).map(|a| (a.config.clone(), a.password.clone())) else { return };
    let Some(url) = config.jmap.clone() else {
        return status(sh, id, SyncStatus::error("only JMAP accounts work in the browser; use the desktop app for this one"));
    };
    let Some(password) = password else {
        return status(sh, id, SyncStatus::NeedsPassword { error: "enter the password: the browser doesn't keep it".into() });
    };
    status(sh, id, SyncStatus::Connecting);
    match Client::connect(Fetch, &url, config.login_name(), &password, false).await {
        Ok(c) => {
            {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                a.client = Some(Rc::new(c));
            }
            // What waited is only sent: the sync after it reads back all it touched.
            replay(sh, id, false).await;
            sync_all(sh, id).await;
        }
        Err(e) => failed(sh, id, e),
    }
}

/// Refresh the folder list; `false` when that failed.
async fn relist(sh: &Rcs, id: &str) -> bool {
    let Some(c) = client(sh, id) else { return false };
    let mut st = state(sh, id);
    let r = mailcraft_jmap::list_folders(&c, &mut st, id).await;
    put_state(sh, id, st);
    match r {
        Ok(mut list) => {
            sort_folders(&mut list);
            let store = store(sh);
            let old: Vec<Folder> = sh.borrow().accounts.get(id).map(|a| a.folders.clone()).unwrap_or_default();
            for f in old.iter().filter(|o| !list.iter().any(|f| f.path == o.path)) {
                if let Err(e) = store.clear_folder(id, &f.path) {
                    log::warn!("{id}: {}: {e}", f.path);
                }
            }
            if let Err(e) = store.save_folders(id, &list) {
                log::warn!("{id}: {e}");
            }
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.folders = list.clone();
            }
            emit(sh, Event::Folders { account: id.to_string(), folders: list });
            true
        }
        Err(e) => {
            failed(sh, id, e);
            false
        }
    }
}

async fn sync_all(sh: &Rcs, id: &str) {
    if !relist(sh, id).await {
        return;
    }
    let paths: Vec<String> =
        sh.borrow().accounts.get(id).map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect()).unwrap_or_default();
    // The Inbox first.
    let inbox = inbox(sh, id);
    for p in inbox.iter().chain(paths.iter().filter(|p| Some(*p) != inbox.as_ref())) {
        if client(sh, id).is_none() {
            return;
        }
        sync_one(sh, id, p).await;
    }
    if let Some(c) = client(sh, id) {
        if let Ok(Some((used, limit))) = c.quota().await {
            emit(sh, Event::Quota { account: id.to_string(), used, limit });
        }
        status(sh, id, SyncStatus::Online { at: now() });
    }
}

async fn sync_one(sh: &Rcs, id: &str, path: &str) {
    let Some(c) = client(sh, id) else { return };
    status(sh, id, SyncStatus::Syncing { folder: path.to_string() });
    let (store, opts) = {
        let s = sh.borrow();
        // A folder asked to show more keeps more.
        let window = s.accounts.get(id).and_then(|a| a.windows.get(path)).copied().unwrap_or(0).max(s.opts.window);
        (s.store.clone(), SyncOptions { window, ..s.opts })
    };
    let mut st = state(sh, id);
    let r = mailcraft_jmap::sync_folder(&c, &mut st, &store, id, path, &opts).await;
    put_state(sh, id, st);
    match r {
        Ok(r) => {
            let (first, counts) = {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                let Some(f) = a.folders.iter_mut().find(|f| f.path == path) else { return };
                let first = f.total == 0 && r.total > 0;
                let changed = f.unread != r.unread || f.total != r.total;
                (f.unread, f.total) = (r.unread, r.total);
                (first, changed.then(|| a.folders.clone()))
            };
            if let Some(folders) = counts {
                if let Err(e) = store.save_folders(id, &folders) {
                    log::warn!("{id}: {e}");
                }
                emit(sh, Event::Folders { account: id.to_string(), folders });
            }
            if r.touched() || first {
                emit(sh, Event::FolderChanged { account: id.to_string(), folder: path.to_string() });
            }
            status(sh, id, SyncStatus::Online { at: now() });
        }
        Err(e) => failed(sh, id, e),
    }
}

/// Search folders on the server (issue #4): JMAP's `Email/query`, the messages found brought
/// into their folders' lists and sent a folder at a time; stops when the search is no longer
/// wanted (a newer one, or the search box cleared).
async fn search_server(sh: &Rcs, id: &str, folders: &[String], query: String, search: u64) {
    let answer = |keys, error, done| Event::ServerSearched { account: id.to_string(), query: query.clone(), keys, error, done };
    let Some(c) = client(sh, id) else {
        emit(sh, answer(Vec::new(), Some("the account isn't connected".into()), true));
        return;
    };
    // Its dates as days around now, a day wider each side (issue #179), as the desktop asks.
    let period = mailcraft_engine::model::search::Query::parse(&query).server_period(&mailcraft_engine::model::dates::Clock::new(now(), 0));
    let Some(filter) = mailcraft_jmap::search_filter(&query, period) else {
        emit(sh, answer(Vec::new(), None, true));
        return;
    };
    let store = store(sh);
    let mut st = state(sh, id);
    let (mut any, mut failed, seen) = (false, None, store.removals(id));
    for f in folders.iter().take(200) {
        if sh.borrow().search != search {
            break;
        }
        match mailcraft_jmap::find_on_server(&c, &mut st, &store, (id, seen), f, &filter, mailcraft_jmap::FOUND_MAX).await {
            Ok((found, new)) => {
                if new > 0 {
                    emit(sh, Event::FolderChanged { account: id.to_string(), folder: f.clone() });
                }
                if !found.is_empty() {
                    any = true;
                    emit(sh, answer(found, None, false));
                }
            }
            Err(e) => failed = Some(e.to_string()),
        }
    }
    put_state(sh, id, st);
    let error = if any { None } else { failed };
    emit(sh, answer(Vec::new(), error, true));
}

/// A change for the server (the cache has it already). It's queued with the cache, which is
/// kept in IndexedDB, as the desktop keeps it on disk: held only by this page, it was lost to a
/// reload or a closed tab before the account was signed in again (the browser never keeps the
/// password), and the next sync put back what the server still had. Then the queue is sent.
async fn remote(sh: &Rcs, id: &str, op: Op) {
    if let Err(e) = store(sh).add_op(id, op.clone()) {
        notice(sh, id, format!("a change couldn't be kept to send later: {e}"));
        // Sent now if it can be; otherwise it's lost, as said.
        if let Some(c) = client(sh, id)
            && let Err(e) = send(sh, id, &c, &op, true).await
        {
            emit(sh, Event::OpFailed { account: id.to_string(), op, error: e.to_string() });
        }
        return;
    }
    replay(sh, id, true).await;
}

/// The changes waiting for the server, sent oldest first. Each leaves the queue once the server
/// has taken it or refused it (a refusal is said); the one a lost connection or sign-in stopped
/// stays, with those after it, for the next connection. `refresh`: the folders each touched are
/// read back after it (not when a full sync follows anyway: each was a folder's sync, then all of
/// them again).
async fn replay(sh: &Rcs, id: &str, refresh: bool) {
    let store = store(sh);
    // Bounded, in case taking one off the queue keeps failing unseen.
    for _ in 0..100_000 {
        let Some(c) = client(sh, id) else { return };
        let Some(op) = store.load_ops(id).unwrap_or_default().into_iter().next() else { return };
        match send(sh, id, &c, &op, refresh).await {
            Err(e) if e.stops_the_queue() => return failed(sh, id, e),
            sent => {
                if let Err(e) = store.remove_op(id, &op) {
                    return notice(sh, id, e.to_string());
                }
                if let Err(e) = sent {
                    emit(sh, Event::OpFailed { account: id.to_string(), op, error: e.to_string() });
                }
            }
        }
    }
}

/// One change made on the server; then, with `refresh`, the folders it touched are shown afresh,
/// with the server's counts. A change to the folders is listed again either way: the next change
/// may name a folder it made or renamed.
async fn send(sh: &Rcs, id: &str, c: &Client<Fetch>, op: &Op, refresh: bool) -> mailcraft_jmap::Result<()> {
    mailcraft_jmap::apply_remote(c, &state(sh, id), op).await?;
    if op.is_folder_op() {
        relist(sh, id).await;
    }
    if !refresh {
        return Ok(());
    }
    let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
    match op {
        Op::Move { to, .. } => touched.push(to.clone()),
        Op::EmptyFolder { path } | Op::MarkFolderRead { path } => touched.push(path.clone()),
        Op::CreateFolder { .. }
        | Op::RenameFolder { .. }
        | Op::DeleteFolder { .. }
        | Op::SetFlag { .. }
        | Op::SetFollowUp { .. }
        | Op::SetReminder { .. }
        | Op::SetKeyword { .. }
        | Op::Expunge { .. } => {}
    }
    touched.sort();
    touched.dedup();
    for f in touched {
        sync_one(sh, id, &f).await;
    }
    Ok(())
}

fn sent_folder(sh: &Rcs, id: &str, given: Option<String>) -> Option<String> {
    given.or_else(|| sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone())))
}

/// What S/MIME can't do in the browser: smart cards and the system's keychain.
const BROWSER_SMIME: &str = "smart cards and the system's keychain are used by the MailCraft desktop app, not in the browser";
/// Revocation servers don't let a web page ask them.
const BROWSER_REVOCATION: &str = "the browser can't ask a certificate's authority";
/// OpenPGP keys of one's own are the desktop app's for now (docs/gaps.md).
const BROWSER_PGP: &str = "your OpenPGP keys are used by the MailCraft desktop app, not in the browser";

/// OpenPGP work with the account's own keys: not in the browser (a draft is saved as it is).
fn pgp(sh: &Rcs, id: &str, work: mailcraft_engine::model::PgpWork) {
    use mailcraft_engine::model::PgpWork;
    match work {
        PgpWork::Decrypt { key, .. } => emit(sh, Event::PgpDecrypted { key, content: None, signature: None, error: Some(BROWSER_PGP.into()) }),
        PgpWork::Protect { id: msg, .. } => {
            emit(sh, Event::Protected { account: id.to_string(), id: msg, raw: None, error: Some(BROWSER_PGP.into()) })
        }
        PgpWork::SealDraft { raw, folder, replaces, .. } => {
            notice(sh, id, format!("the draft couldn't be encrypted, so it's saved as it is: {BROWSER_PGP}"));
            enqueue(sh, id, Task::Job(Box::new(Job::SaveDraft { account: id.to_string(), raw: raw.0, folder, replaces })));
        }
        PgpWork::Import { .. } | PgpWork::Use { .. } | PgpWork::Forget { .. } => {
            emit(sh, Event::PgpKeys { account: id.to_string(), keys: Vec::new(), imported: None, error: Some(BROWSER_PGP.into()) })
        }
    }
}

/// Check the Inbox every few minutes (and everything now and then), reconnecting when needed.
async fn poll(sh: Rcs, id: String, generation: u32) {
    let mut tick = 0u32;
    loop {
        sleep(POLL_MS).await;
        let state = sh.borrow().accounts.get(&id).map(|a| (a.generation, a.client.is_some(), a.password.is_some()));
        let Some((g, connected, password)) = state else { return };
        if g != generation {
            return;
        }
        tick = tick.wrapping_add(1);
        let inbox = inbox(&sh, &id);
        let pushing = sh.borrow().accounts.get(&id).is_some_and(|a| a.pushing);
        match (connected, inbox) {
            (false, _) if password => enqueue(&sh, &id, Task::Connect),
            (false, _) => {}
            (true, _) if tick.is_multiple_of(FULL_EVERY) => enqueue(&sh, &id, Task::SyncAll),
            (true, Some(f)) if !pushing => enqueue(&sh, &id, Task::SyncFolder(f)),
            (true, _) => {}
        }
    }
}

fn inbox(sh: &Rcs, id: &str) -> Option<String> {
    sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()))
}

fn current(sh: &Rcs, id: &str, generation: u32) -> bool {
    sh.borrow().accounts.get(id).is_some_and(|a| a.generation == generation)
}

fn set_pushing(sh: &Rcs, id: &str, on: bool) {
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.pushing = on;
    }
}

/// Hold the account's push stream open while the account is (RFC 8620 §7.3): a `state` event
/// means something changed, and the Inbox syncs.
async fn push(sh: Rcs, id: String, generation: u32) {
    let mut wait = 0;
    loop {
        if wait > 0 {
            sleep(wait).await;
        }
        if !current(&sh, &id, generation) {
            return;
        }
        let Some(c) = client(&sh, &id) else {
            wait = PUSH_WAIT_MS;
            continue;
        };
        // No push from this server: polling it is.
        let Some(url) = c.events_url() else { return };
        let mut opened = false;
        let mut on_event = |e: mailcraft_jmap::sse::Sse| {
            if !current(&sh, &id, generation) {
                return false;
            }
            if !std::mem::replace(&mut opened, true) {
                set_pushing(&sh, &id, true);
            }
            // Mail that came while the stream was closed sends no event: catch up once it's open.
            if (e.event == "state" || e.event == "open")
                && let Some(f) = inbox(&sh, &id)
            {
                enqueue(&sh, &id, Task::SyncFolder(f));
            }
            true
        };
        let r = events(&url, c.authorization(), &mut on_event).await;
        set_pushing(&sh, &id, false);
        if let Err(e) = r {
            log::info!("{id}: push: {e}");
        }
        wait = PUSH_RETRY_MS;
    }
}

/// Download a picture a message links to (once the user asked to see pictures).
async fn fetch_url(sh: Rcs, url: String) {
    use mailcraft_jmap::Http as _;
    let r = Fetch.send(mailcraft_jmap::Request::get(&url, MAX_PICTURE)).await;
    let e = match r {
        Ok(resp) if (200..300).contains(&resp.status) => Event::Fetched { url, data: Some(resp.body), error: None },
        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
    };
    emit(&sh, e);
}

/// A calendar subscribed to by its link (issue #7), read by the page: only from servers that
/// let web pages read their calendars (CORS); the desktop app reads any.
async fn fetch_calendar(sh: Rcs, account: String, calendar: String, url: String) {
    use mailcraft_jmap::Http as _;
    let r = Fetch.send(mailcraft_jmap::Request::get(&url, MAX_CALENDAR)).await;
    let (ics, error) = match r {
        Ok(resp) if (200..300).contains(&resp.status) => (Some(String::from_utf8_lossy(&resp.body).into_owned()), None),
        Ok(resp) => (None, Some(format!("the calendar's server answered HTTP {}", resp.status))),
        Err(_) => (None, Some("the calendar's server doesn't let web pages read it; the desktop app can".to_string())),
    };
    let e = Event::CalendarFetched { account, calendar, ics, etag: String::new(), modified: String::new(), unchanged: false, error };
    emit(&sh, e);
}

/// Largest calendar read in the browser.
const MAX_CALENDAR: usize = 20 << 20;
/// Why an Outlook data file isn't opened here.
const DESKTOP_PST: &str = "opening Outlook data files needs the desktop app";
/// Why message files aren't read here.
const DESKTOP_FILES: &str = "reading message files from the computer needs the desktop app";

impl Backend for WebBackend {
    fn submit(&mut self, job: Job) {
        let sh = &self.shared;
        match job {
            Job::FetchUrl { url } => wasm_bindgen_futures::spawn_local(fetch_url(sh.clone(), url)),
            Job::FetchCalendar { account, calendar, url, .. } => {
                wasm_bindgen_futures::spawn_local(fetch_calendar(sh.clone(), account, calendar, url))
            }
            // No autoconfiguration in the browser: the user gives the JMAP address.
            Job::Discover { email } => emit(sh, Event::Discovered { email, config: None, source: String::new() }),
            Job::SignIn { account } => {
                status(sh, &account.id, SyncStatus::NeedsPassword { error: "signing in through Google or Microsoft needs the desktop app".into() })
            }
            Job::ImportPst { account, .. } | Job::RereadDataFile { account, .. } => {
                emit(sh, Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(DESKTOP_PST.into()), report: None })
            }
            Job::InspectDataFile { path } => {
                let facts = Box::new(DataFileFacts { path: path.clone(), error: Some(DESKTOP_PST.into()), ..DataFileFacts::default() });
                emit(sh, Event::DataFileInspected { path, facts })
            }
            // A page reads no files by their paths (issue #38).
            Job::FileMessages { account, folder, paths } => {
                let failed = paths.into_iter().map(|p| (p, DESKTOP_FILES.to_string())).collect();
                emit(sh, Event::MessagesFiled { account, folder, added: 0, failed })
            }
            Job::ReadMessageFile { path } => emit(sh, Event::MessageFileRead { path, raw: Err(DESKTOP_FILES.into()) }),
            // No import runs here to stop; nothing is left of one to keep.
            Job::StopImport { .. } => {}
            Job::KeepImport { account } => emit(sh, Event::ImportKept { account, error: None }),
            // The browser keeps mail in its own storage, not in data files.
            Job::DataFileHealth { account } => emit(sh, Event::DataFileHealth { account, health: None, error: None }),
            Job::CompactDataFile { account, mode } => {
                let report = CompactReport {
                    mode,
                    error: Some("data files are kept by the desktop app, not in the browser".into()),
                    ..CompactReport::default()
                };
                emit(sh, Event::Compacted { account, report: Box::new(report) })
            }
            Job::CancelCompact { .. } => {}
            Job::ExportPst { account, .. } => emit(
                sh,
                Event::Exported {
                    account,
                    messages: 0,
                    skipped: 0,
                    already: 0,
                    occurrences: 0,
                    replaced: 0,
                    folders_skipped: 0,
                    done: true,
                    error: Some("writing Outlook data files needs the desktop app".into()),
                },
            ),
            Job::ArchivePst { account, path, .. } => emit(
                sh,
                Event::Archived {
                    account,
                    path,
                    messages: 0,
                    archived: Vec::new(),
                    skipped: 0,
                    folders_skipped: 0,
                    done: true,
                    error: Some("writing Outlook data files needs the desktop app".into()),
                },
            ),
            Job::Start { account, password } => {
                let id = account.id.clone();
                let generation = {
                    let mut s = sh.borrow_mut();
                    let store = s.store.clone();
                    let old = s.accounts.remove(&id);
                    let generation = old.as_ref().map_or(1, |a| a.generation.wrapping_add(1));
                    let password = password.map(|p| p.0).or_else(|| old.as_ref().and_then(|a| a.password.clone()));
                    s.accounts.insert(
                        id.clone(),
                        Account {
                            state: State::load(&store, &id),
                            folders: store.load_folders(&id).unwrap_or_default(),
                            config: account,
                            password,
                            client: None,
                            queue: VecDeque::new(),
                            running: false,
                            generation,
                            pushing: false,
                            windows: store.load_account_doc(&id, WINDOWS).unwrap_or_default(),
                        },
                    );
                    generation
                };
                // Kept unlocked between visits: its certificates opened again.
                smime::restore(sh, &id);
                enqueue(sh, &id, Task::Connect);
                wasm_bindgen_futures::spawn_local(poll(sh.clone(), id.clone(), generation));
                wasm_bindgen_futures::spawn_local(push(sh.clone(), id, generation));
            }
            Job::Remove { account } => {
                sh.borrow_mut().accounts.remove(&account);
                indexing::job(sh, Job::StopIndex { account });
            }
            Job::StopSearch => sh.borrow_mut().search = 0,
            j @ (Job::Index { .. } | Job::StopIndex { .. }) => indexing::job(sh, j),
            other => {
                if let Job::SearchServer { search, .. } = &other {
                    sh.borrow_mut().search = *search;
                }
                let id = other.account().to_string();
                if sh.borrow().accounts.contains_key(&id) {
                    enqueue(sh, &id, Task::Job(Box::new(other)));
                } else if let Job::FetchBody { key } = other {
                    // The sample mailbox and other accounts with no server: the cache has it or nobody does.
                    let raw = store(sh).read_body(&key);
                    match raw {
                        Some(raw) => emit(sh, Event::Body { key, raw }),
                        None => emit(sh, Event::BodyFailed { key, error: "not downloaded".into() }),
                    }
                } else if let Job::Apply { account, op } = other {
                    jobs::apply_local(sh, &account, &op);
                } else if let Job::Send { account, id, .. } = other {
                    emit(sh, Event::SendFailed { account, id, error: "this mailbox has no server to send through".into() });
                }
            }
        }
    }

    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.shared.borrow_mut().events)
    }
}
