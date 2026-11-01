//! Data files read into accounts, each on a thread of its own: one import at a time into an
//! account, and each stopped as asked. Cancelled, it keeps what came in; paused (MailCraft
//! closing), the same, less the folder it was part-way through; thrown away (the account
//! removed), the data file it wrote goes once the import has let go of it. A file looked at
//! before it's imported is planned then, and its import goes by that plan.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use mailcraft_model::job::{DataFileFacts, ImportPhase, ImportProgress, ImportReport, StopHow, Work};
use mailcraft_model::pim::{PIM_DOC, Pim};

use super::*;
use crate::pst::{ImportOptions, ImportSink, Imported, Planned, Progress, Stop};

/// How long closing MailCraft waits for its imports to pause: each finishes the item in hand and
/// writes what it holds. One that takes longer (a reader stuck on a slow drive) is let go.
pub(super) const IMPORT_STOP_WAIT: Duration = Duration::from_secs(10);
/// The account's folder list is sent at most this often while an import finishes folders.
const FOLDERS_EVERY: Duration = Duration::from_millis(500);
/// A data file that can't be removed at once (something letting go of it) is tried again this
/// many times, a little later each time.
const REMOVE_TRIES: u32 = 4;

/// The imports running, by account, and the data file looked at last, as planned.
#[derive(Default)]
pub(super) struct Imports {
    running: HashMap<String, ImportCtl>,
    looked: Arc<Mutex<Option<Looked>>>,
}

/// An account's import: how it's asked to stop (the one queued after it, once there is one), the
/// import queued to follow it, and its thread, which ends with the data file of an import thrown
/// away that couldn't be removed yet.
struct ImportCtl {
    stop: Stop,
    queue: Arc<Mutex<Queue>>,
    thread: JoinHandle<Option<PathBuf>>,
}

/// An import asked for while the account's last one was being thrown away: it starts on the same
/// thread once that one is over (its data file gone), not beside it.
#[derive(Default)]
struct Queue {
    next: Option<(String, Stop)>,
    /// The thread is ending: nothing more is taken from here.
    ended: bool,
}

/// A data file as it was when it was looked at ([`Job::InspectDataFile`]), and its plan.
struct Looked {
    path: String,
    size: u64,
    modified: i64,
    planned: Arc<Planned>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pool {
    /// Import a data file on a thread of its own: an Outlook one (`.pst`), or MailCraft's own
    /// (`.mailcraft`, copied in as it is). An account already importing isn't given a second;
    /// one whose import is being thrown away gets it once that's over.
    pub(super) fn import_pst(&mut self, account: String, path: String) {
        self.prune_imports();
        if let Some(ctl) = self.imports.running.get_mut(&account) {
            let mut queue = lock(&ctl.queue);
            if ctl.stop.how() != Some(StopHow::Discard) || queue.next.is_some() {
                drop(queue);
                return self.sink.emit(Event::Notice { account, message: "a data file is being imported into this account already".into() });
            }
            if !queue.ended {
                let stop = Stop::default();
                queue.next = Some((path, stop.clone()));
                drop(queue);
                ctl.stop = stop;
                return;
            }
            // Its thread is ending as this is asked: let go of it, and start afresh.
            drop(queue);
            self.let_go_of_import(&account);
        }
        let queue: Arc<Mutex<Queue>> = Arc::default();
        let (sink, store, looked, stop) = (self.sink.clone(), self.store.clone(), self.imports.looked.clone(), Stop::default());
        let (id, asked, queued) = (account.clone(), stop.clone(), Arc::clone(&queue));
        // No index pass for the account until the import thread ends.
        let importing = self.indexer.importing(&account);
        let spawned = std::thread::Builder::new().name("mail-import".into()).spawn(move || {
            let _importing = importing;
            let (mut path, mut stop, mut left) = (path, asked, None);
            loop {
                let planned = planned_for(&looked, &path);
                if let Some(file) = run(&store, &sink, &account, &path, &stop, planned) {
                    left = Some(file);
                }
                let mut q = lock(&queued);
                match q.next.take() {
                    Some((next, asked)) => (path, stop) = (next, asked),
                    None => {
                        q.ended = true;
                        return left;
                    }
                }
            }
        });
        match spawned {
            Ok(thread) => {
                self.imports.running.insert(id, ImportCtl { stop, queue, thread });
            }
            // No thread, no import: it ends here, as a failed one (the account isn't left importing).
            Err(e) => self.sink.emit(Event::Imported {
                account: id,
                messages: 0,
                skipped: 0,
                done: true,
                error: Some(format!("import: {e}")),
                report: None,
            }),
        }
    }

    /// Stop an account's import as `how` says. With none running, the answer comes at once: it's
    /// over, and nothing more will come of it (thrown away, as far as anything was left of it).
    pub(super) fn stop_import(&mut self, account: &str, how: StopHow) {
        self.prune_imports();
        match self.imports.running.get(account) {
            Some(ctl) => ctl.stop.raise(how),
            None => {
                let report = ImportReport { stopped: Some(StopHow::Discard), ..ImportReport::default() };
                self.sink.emit(Event::Imported {
                    account: account.to_string(),
                    messages: 0,
                    skipped: 0,
                    done: true,
                    error: None,
                    report: Some(Box::new(report)),
                });
            }
        }
    }

    /// Let go of the imports that have ended.
    fn prune_imports(&mut self) {
        let ended: Vec<String> = self.imports.running.iter().filter(|(_, c)| c.thread.is_finished()).map(|(a, _)| a.clone()).collect();
        for account in ended {
            self.let_go_of_import(&account);
        }
    }

    /// Let go of an account's import thread (ended, or ending), and remove the data file of an
    /// import thrown away that couldn't be removed while something held it. It's removed by the
    /// path the import wrote, never by the account's name, which another account may have by now;
    /// and a file still there is no account's (a data file is never named after one that exists).
    fn let_go_of_import(&mut self, account: &str) {
        let Some(ctl) = self.imports.running.remove(account) else { return };
        let Ok(Some(file)) = ctl.thread.join() else { return };
        if let Err(e) = remove(&file) {
            let message = format!("the data file of an import stopped couldn't be removed ({}): {e}", file.display());
            self.sink.emit(Event::Notice { account: account.to_string(), message });
        }
    }

    /// Pause every import. With `wait`, wait up to that long for them to stop; those still going
    /// after it, and all of them without it, are let go (they end on their own).
    fn pause_imports(&mut self, wait: Option<Duration>) {
        for ctl in self.imports.running.values() {
            ctl.stop.raise(StopHow::Pause);
        }
        let Some(wait) = wait else {
            self.imports.running.clear();
            return;
        };
        let end = Instant::now() + wait;
        while self.imports.running.values().any(|c| !c.thread.is_finished()) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.prune_imports();
        if !self.imports.running.is_empty() {
            log::warn!("closing: {} import(s) didn't stop in time; left to end on their own", self.imports.running.len());
        }
    }

    /// Stop every account's threads, and pause the imports, waiting a few seconds at most for
    /// them to write what they hold (MailCraft closing).
    pub fn shutdown(&mut self) {
        self.indexer.shutdown();
        self.stop_all();
        self.pause_imports(Some(IMPORT_STOP_WAIT));
    }

    /// Whether an account has an import running.
    #[cfg(test)]
    pub(super) fn importing(&self, account: &str) -> bool {
        self.imports.running.get(account).is_some_and(|c| !c.thread.is_finished())
    }

    /// Look at a data file before it's imported, on a thread of its own (it may be on a slow
    /// drive): what kind it is, how big, when it last changed, and for an Outlook one what it
    /// holds, planned as its import would bring it in. That plan is kept, and the file's import
    /// goes by it when the file hasn't changed since.
    pub(super) fn inspect_data_file(&mut self, path: String) {
        let (sink, asked, looked) = (self.sink.clone(), path.clone(), self.imports.looked.clone());
        let spawned = std::thread::Builder::new().name("mail-inspect".into()).spawn(move || {
            let mut facts = facts_of(&path);
            if facts.error.is_none() && facts.kind == "pst" {
                match crate::pst::inspect(Path::new(&path)) {
                    Ok(planned) => {
                        (facts.messages, facts.folders, facts.pim_items, facts.bytes) =
                            (planned.messages, planned.folders, planned.pim_items, planned.bytes);
                        let planned = Arc::new(planned);
                        *lock(&looked) = Some(Looked { path: path.clone(), size: facts.size, modified: facts.modified, planned });
                    }
                    Err(e) => facts.error = Some(e),
                }
            }
            sink.emit(Event::DataFileInspected { path, facts: Box::new(facts) });
        });
        if let Err(e) = spawned {
            let facts = DataFileFacts { path: asked.clone(), error: Some(format!("inspect: {e}")), ..DataFileFacts::default() };
            self.sink.emit(Event::DataFileInspected { path: asked, facts: Box::new(facts) });
        }
    }
}

/// Never waits: the imports are paused and let go (they end on their own, writing what they hold).
impl Drop for Pool {
    fn drop(&mut self) {
        self.stop_all();
        self.pause_imports(None);
    }
}

/// The plan made of the data file at `path` when it was looked at, while the file is as it was.
fn planned_for(looked: &Mutex<Option<Looked>>, path: &str) -> Option<Arc<Planned>> {
    let now = facts_of(path);
    let looked = lock(looked);
    looked.as_ref().filter(|l| l.path == path && now.error.is_none() && (l.size, l.modified) == (now.size, now.modified)).map(|l| l.planned.clone())
}

/// An import, start to end, on its thread. Returns the data file of one thrown away that
/// couldn't be removed yet.
fn run(store: &Store, sink: &Sink, account: &str, path: &str, stop: &Stop, planned: Option<Arc<Planned>>) -> Option<PathBuf> {
    let began = Instant::now();
    // Stopped while it waited behind an import thrown away: it never began.
    if let Some(how) = stop.how() {
        return never_began(sink, account, path, how);
    }
    let mut told = Told { sink, store, account, stop, began, last: Progress::default(), folders_done: 0, folders_said: None, files: Vec::new() };
    told.say(ImportPhase::Planning);
    if path.to_ascii_lowercase().ends_with(".mailcraft") {
        return copy_in(&mut told, path);
    }
    let opts = ImportOptions { stop: stop.clone(), planned, ..ImportOptions::default() };
    #[cfg(test)]
    let opts = ImportOptions { fault: tests::fault_for(account), ..opts };
    let result = crate::pst::import_into(Path::new(path), store, account, &opts, &mut told);
    // Thrown away, however far it got (it may have ended on its own just before).
    if stop.how() == Some(StopHow::Discard) {
        let (messages, skipped) = result.as_ref().map_or((0, 0), |d| (d.messages, d.skipped));
        told.note_file();
        return thrown_away(&told, path, messages, skipped);
    }
    match result {
        // Paused: what came in is kept, and only a word that it stopped is said, with the folders
        // finished (the last of them may not have been said yet).
        Ok(done) if done.stopped == Some(StopHow::Pause) => {
            sink.emit(Event::Folders { account: account.to_string(), folders: done.folders });
            told.last.imported = done.messages;
            told.last.skipped = done.skipped;
            told.say(ImportPhase::Paused);
        }
        Ok(done) => {
            let mut report = report_of(&done, path, store, account, began);
            report.stopped = done.stopped;
            report.stopped_in.clone_from(&done.stopped_in);
            let paths: Vec<String> = done.folders.iter().map(|f| f.path.clone()).collect();
            sink.emit(Event::Folders { account: account.to_string(), folders: done.folders });
            for folder in paths {
                sink.emit(Event::FolderChanged { account: account.to_string(), folder });
            }
            // A last save that failed is said with what came in (the account stays).
            let (messages, skipped, error) = (done.messages, done.skipped, done.last_save);
            sink.emit(Event::Imported { account: account.to_string(), messages, skipped, done: true, error, report: Some(Box::new(report)) });
        }
        Err(e) => sink.emit(Event::Imported { account: account.to_string(), messages: 0, skipped: 0, done: true, error: Some(e), report: None }),
    }
    None
}

/// An import stopped before it began: said as one stopped so would be, with nothing in it.
fn never_began(sink: &Sink, account: &str, path: &str, how: StopHow) -> Option<PathBuf> {
    if how == StopHow::Pause {
        let progress = ImportProgress { phase: ImportPhase::Paused, ..ImportProgress::default() };
        sink.emit(Event::ImportProgress { account: account.to_string(), progress });
    } else {
        let report = ImportReport { file: path.to_string(), stopped: Some(how), runs: 1, ..ImportReport::default() };
        sink.emit(Event::Imported { account: account.to_string(), messages: 0, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
    }
    None
}

/// MailCraft's own data file copied in, asked before each batch whether to stop.
fn copy_in(told: &mut Told<'_>, path: &str) -> Option<PathBuf> {
    let (store, sink, account, stop) = (told.store, told.sink, told.account, told.stop);
    let files = RefCell::new(Vec::new());
    let copied = store.import_data_file_until(account, Path::new(path), &|| {
        // The data file being written, as the copy goes (it's made by the first batch).
        if let Some(f) = store.data_file(account)
            && !files.borrow().contains(&f)
        {
            files.borrow_mut().push(f);
        }
        stop.raised()
    });
    told.files = files.into_inner();
    told.note_file();
    if stop.how() == Some(StopHow::Discard) {
        return thrown_away(told, path, copied.as_ref().map_or(0, |n| *n), 0);
    }
    let folders = store.load_folders(account).unwrap_or_default();
    let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
    let messages = match copied {
        Ok(n) => n,
        // Stopped part-way: what was copied stays, as a cancelled import's does. Any other
        // error is said, whatever was asked meanwhile.
        Err(mailcraft_store::StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::Interrupted && stop.raised() => {
            folders.iter().fold(0usize, |n, f| n.saturating_add(usize::try_from(f.total).unwrap_or(usize::MAX)))
        }
        Err(e) => {
            sink.emit(Event::Imported {
                account: account.to_string(),
                messages: 0,
                skipped: 0,
                done: true,
                error: Some(e.to_string()),
                report: None,
            });
            return None;
        }
    };
    if stop.how() == Some(StopHow::Pause) {
        sink.emit(Event::Folders { account: account.to_string(), folders });
        told.last.imported = messages;
        told.say(ImportPhase::Paused);
        return None;
    }
    // Its contacts, calendar and notes came with it.
    let pim = store.load_account_doc::<Pim>(account, PIM_DOC).unwrap_or_default();
    let report = ImportReport {
        file: path.to_string(),
        data_file: data_file(store, account),
        messages: count(messages),
        folders: u32::try_from(paths.len()).unwrap_or(u32::MAX),
        contacts: count(pim.cards.len()),
        events: count(pim.events.len()),
        notes: count(pim.notes.len()),
        stopped: stop.how(),
        runs: 1,
        elapsed_ms: ms(told.began),
        ..Default::default()
    };
    sink.emit(Event::Folders { account: account.to_string(), folders });
    for folder in paths {
        sink.emit(Event::FolderChanged { account: account.to_string(), folder });
    }
    sink.emit(Event::Imported { account: account.to_string(), messages, skipped: 0, done: true, error: None, report: Some(Box::new(report)) });
    None
}

/// An import thrown away: the data file it wrote goes, and the end is said, with why the file is
/// still there when it is (it's returned, to be tried again once the import's thread is over).
/// Only a file the import itself wrote, and the account still has, is removed: one the account
/// let go of meanwhile (kept, as a removed account's is) stays.
fn thrown_away(told: &Told<'_>, path: &str, messages: usize, skipped: usize) -> Option<PathBuf> {
    let (store, account) = (told.store, told.account);
    let mut error = None;
    let mut left = None;
    if let Some(file) = store.data_file(account).filter(|f| told.files.contains(f)) {
        // The store lets go of the file and forgets it, even when it can't be removed yet.
        if let Err(e) = store.delete_account(account) {
            error = Some(format!("its data file couldn't be removed: {e}"));
        }
        if let Err(e) = remove(&file) {
            error = Some(format!("its data file couldn't be removed: {e}"));
            left = Some(file);
        }
    }
    let report = ImportReport { file: path.to_string(), stopped: Some(StopHow::Discard), runs: 1, ..ImportReport::default() };
    told.sink.emit(Event::Imported { account: account.to_string(), messages, skipped, done: true, error, report: Some(Box::new(report)) });
    left
}

/// Remove a data file, trying again a little later while something still holds it.
fn remove(file: &Path) -> std::io::Result<()> {
    let mut n = 0u32;
    loop {
        match std::fs::remove_file(file) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if n.saturating_add(1) >= REMOVE_TRIES => return Err(e),
            Err(_) => {
                n = n.saturating_add(1);
                std::thread::sleep(Duration::from_millis(100).saturating_mul(n));
            }
        }
    }
}

/// What an import came to, as the report says it.
fn report_of(done: &Imported, path: &str, store: &Store, account: &str, began: Instant) -> ImportReport {
    let mut report = ImportReport {
        file: path.to_string(),
        data_file: data_file(store, account),
        messages: count(done.messages),
        folders: u32::try_from(done.folders.len()).unwrap_or(u32::MAX),
        contacts: count(done.contacts),
        events: count(done.events),
        notes: count(done.notes),
        elapsed_ms: ms(began),
        runs: 1,
        ..ImportReport::default()
    };
    for (reason, n) in &done.skips {
        report.skip(*reason, count(*n));
    }
    report
}

/// The file the account's mail was copied into ("" for a cache that isn't kept in one).
fn data_file(store: &Store, account: &str) -> String {
    store.data_file(account).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The window, told how an import goes: its progress, and each folder as it's finished (the
/// account's folder list with it, now and then), so it shows while the rest comes in. Folders
/// part-way through aren't in the list, so none can be changed while it's written. It notes the
/// data file the import writes in as it goes, the one file a discard may remove.
struct Told<'a> {
    sink: &'a Sink,
    store: &'a Store,
    account: &'a str,
    stop: &'a Stop,
    began: Instant,
    last: Progress,
    folders_done: usize,
    folders_said: Option<Instant>,
    files: Vec<PathBuf>,
}

impl Told<'_> {
    fn say(&self, phase: ImportPhase) {
        let t = &self.last;
        let run_ms = ms(self.began);
        let phase = if phase == ImportPhase::Importing && self.stop.raised() { ImportPhase::Stopping } else { phase };
        let progress = ImportProgress {
            phase,
            // Items read, which only go up: a batch that can't be written is still read.
            work: Work { done: count(t.read), total: count(t.total), bytes_done: t.bytes_done, bytes_total: t.bytes_total },
            folders_done: u32::try_from(self.folders_done).unwrap_or(u32::MAX),
            folders_total: u32::try_from(t.folders).unwrap_or(u32::MAX),
            imported: count(t.imported),
            skipped: count(t.skipped),
            elapsed_ms: run_ms,
            run_ms,
            ..ImportProgress::default()
        };
        self.sink.emit(Event::ImportProgress { account: self.account.to_string(), progress });
    }

    /// The account's data file, as the import has written it so far: noted as it goes, and at
    /// its end (a removed account lets go of its file, and the import's last batch, written
    /// before it saw the stop, makes another).
    fn note_file(&mut self) {
        if let Some(f) = self.store.data_file(self.account)
            && !self.files.contains(&f)
        {
            self.files.push(f);
        }
    }
}

impl ImportSink for Told<'_> {
    fn progress(&mut self, p: Progress) {
        self.last = p;
        self.note_file();
        self.say(ImportPhase::Importing);
    }

    fn folder(&mut self, done: &Folder) {
        self.folders_done = self.folders_done.saturating_add(1);
        self.note_file();
        // Folders finished together are in the list on disk by the time the first is said.
        if self.folders_said.is_none_or(|t| t.elapsed() >= FOLDERS_EVERY) {
            self.folders_said = Some(Instant::now());
            match self.store.load_folders(self.account) {
                Ok(folders) => self.sink.emit(Event::Folders { account: self.account.to_string(), folders }),
                Err(e) => log::warn!("{}: {e}", self.account),
            }
        }
        // Counted in the next word of progress (a word for each folder would be one too many).
        self.sink.emit(Event::FolderChanged { account: self.account.to_string(), folder: done.path.clone() });
    }
}

/// What the file system says of a data file (nothing read from inside it).
fn facts_of(path: &str) -> DataFileFacts {
    let lower = path.to_ascii_lowercase();
    let kind = [".pst", ".ost", ".mailcraft"].iter().find(|k| lower.ends_with(*k)).map_or("", |k| k.trim_start_matches('.'));
    let mut facts = DataFileFacts { path: path.to_string(), kind: kind.to_string(), ..DataFileFacts::default() };
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {
            facts.size = m.len();
            facts.modified =
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        }
        Ok(_) => facts.error = Some("not a file".into()),
        Err(e) => facts.error = Some(e.to_string()),
    }
    facts
}

/// A count as the model keeps it.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Milliseconds since `began`.
fn ms(began: Instant) -> u64 {
    u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
