//! Data files read into accounts, each on a thread of its own: one import at a time into an
//! account, and each stopped as asked. Cancelled, it keeps what came in; paused (MailCraft
//! closing), the same, less the folder it was part-way through; thrown away (the account
//! removed), the data file it made goes once the import has let go of it, while the account
//! still has it. A MailCraft data file is copied in whole or not at all. A file looked at before
//! it's imported is planned then, and its import goes by that plan.

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
/// An Outlook data file's copy as a share of its messages' own sizes, in percent: 38.65 GB for
/// 39.5 GB of messages, measured on a 41 GB file.
const COPY_PCT: u64 = 98;
/// The room asked for a copy, in percent of its size, and besides it: the file grows past its
/// final size while it's written (pages freed under a reader are taken again only later), and
/// the drive keeps some room after.
const NEED_PCT: u64 = 130;
const NEED_EXTRA: u64 = 2 << 30;

/// The imports running, by account, and the data file looked at last, as planned.
#[derive(Default)]
pub(super) struct Imports {
    running: HashMap<String, ImportCtl>,
    looked: Arc<Mutex<Option<Looked>>>,
}

/// An account's import: how it's asked to stop (the one queued after it, once there is one), the
/// import queued to follow it, and its thread, which ends with the data files of imports thrown
/// away that couldn't be removed yet.
struct ImportCtl {
    stop: Stop,
    queue: Arc<Mutex<Queue>>,
    thread: JoinHandle<Vec<PathBuf>>,
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
    pub(super) fn import_pst(&mut self, account: String, path: String, resume: bool) {
        self.prune_imports();
        if let Some(ctl) = self.imports.running.get_mut(&account) {
            let mut queue = lock(&ctl.queue);
            // The import last asked for (the one queued, when there is one) goes on.
            if ctl.stop.how() != Some(StopHow::Discard) {
                drop(queue);
                return self.sink.emit(Event::Notice { account, message: "a data file is being imported into this account already".into() });
            }
            if !queue.ended {
                // The one queued was thrown away in its turn (its account removed again): it
                // never begins, and is said so now; this one takes its place.
                if let Some((dropped, _)) = queue.next.take() {
                    never_began(&self.sink, &account, &dropped, StopHow::Discard);
                }
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
            let (mut path, mut stop, mut left, mut resume) = (path, asked, Vec::new(), resume);
            loop {
                let planned = planned_for(&looked, &path);
                left.extend(run(&store, &sink, &account, &path, &stop, planned, resume));
                let mut q = lock(&queued);
                match q.next.take() {
                    // One asked for after an import thrown away starts afresh.
                    Some((next, asked)) => (path, stop, resume) = (next, asked, false),
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

    /// Let go of an account's import thread (ended, or ending), and remove the data files of
    /// imports thrown away that couldn't be removed while something held them. Each is removed by
    /// the path the import wrote, never by the account's name, which another account may have by
    /// now; and a file still there is no account's (a data file is never named after one that
    /// exists).
    fn let_go_of_import(&mut self, account: &str) {
        let Some(ctl) = self.imports.running.remove(account) else { return };
        let Ok(files) = ctl.thread.join() else { return };
        for file in files {
            if let Err(e) = remove(&file) {
                let message = format!("the data file of an import stopped couldn't be removed ({}): {e}", file.display());
                self.sink.emit(Event::Notice { account: account.to_string(), message });
            }
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
    /// them to write what they hold (MailCraft closing). The changes to mail made so far are on
    /// disk first (a message marked read just before closing stays read).
    pub fn shutdown(&mut self) {
        self.edits.finish();
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
    /// holds, planned as its import would bring it in; and how big MailCraft's copy will be, and
    /// the room for it where the data files go. That plan is kept, and the file's import goes by
    /// it when the file hasn't changed since. A file that makes the reader panic is said to be
    /// one that couldn't be looked at.
    pub(super) fn inspect_data_file(&mut self, path: String) {
        let (sink, asked, looked) = (self.sink.clone(), path.clone(), self.imports.looked.clone());
        let destination = self.store.data_files_dir().unwrap_or_else(|| self.store.root().to_path_buf());
        let space = drive_for(&path);
        let spawned = std::thread::Builder::new().name("mail-inspect".into()).spawn(move || {
            let looked_at = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| look_at(&path, &looked)));
            let mut facts = looked_at.unwrap_or_else(|_| DataFileFacts {
                path: path.clone(),
                error: Some("the data file couldn't be looked at: it may be damaged".into()),
                ..DataFileFacts::default()
            });
            room_for(&mut facts, &destination, space.as_ref());
            sink.emit(Event::DataFileInspected { path, facts: Box::new(facts) });
        });
        if let Err(e) = spawned {
            let facts = DataFileFacts { path: asked.clone(), error: Some(format!("inspect: {e}")), ..DataFileFacts::default() };
            self.sink.emit(Event::DataFileInspected { path: asked, facts: Box::new(facts) });
        }
    }
}

/// Where the room is asked for a data file looked at: the computer's own drives (a test's, in a
/// test that gave one).
fn drive_for(path: &str) -> Arc<dyn crate::space::Space> {
    #[cfg(test)]
    if let Some(drive) = tests::space_for(path) {
        return drive;
    }
    let _ = path;
    Arc::new(crate::space::Drives)
}

/// What the data file at `path` holds, planned (an Outlook one's plan kept in `looked`).
fn look_at(path: &str, looked: &Mutex<Option<Looked>>) -> DataFileFacts {
    let mut facts = facts_of(path);
    if facts.error.is_none() && facts.kind == "pst" {
        match crate::pst::inspect(Path::new(path)) {
            Ok(planned) => {
                (facts.messages, facts.folders, facts.pim_items, facts.bytes) = (planned.messages, planned.folders, planned.pim_items, planned.bytes);
                facts.record_key.clone_from(&planned.record_key);
                let planned = Arc::new(planned);
                *lock(looked) = Some(Looked { path: path.to_string(), size: facts.size, modified: facts.modified, planned });
            }
            Err(e) => facts.error = Some(e),
        }
    }
    facts
}

/// The size MailCraft's copy of a data file is expected to come to, the room asked for to write
/// it, and the room there is in `destination` (on the drive of the nearest folder of it that's
/// there; none when that can't be told). An Outlook file's copy is about [`COPY_PCT`] of its
/// messages' own sizes; one of MailCraft's is copied as it is.
fn room_for(facts: &mut DataFileFacts, destination: &Path, space: &dyn crate::space::Space) {
    facts.estimate = if facts.kind == "mailcraft" { facts.size } else { facts.bytes / 100 * COPY_PCT };
    facts.need = (facts.estimate / 100).saturating_mul(NEED_PCT).saturating_add(NEED_EXTRA);
    facts.destination = destination.to_string_lossy().into_owned();
    facts.free = space.free(destination);
}

/// Never waits for the imports: they are paused and let go (they end on their own, writing what
/// they hold). Waits for the changes to mail made so far to be on disk (`edits.rs`).
impl Drop for Pool {
    fn drop(&mut self) {
        self.edits.finish();
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
fn run(store: &Store, sink: &Sink, account: &str, path: &str, stop: &Stop, planned: Option<Arc<Planned>>, resume: bool) -> Option<PathBuf> {
    let began = Instant::now();
    // Stopped while it waited behind an import thrown away: it never began.
    if let Some(how) = stop.how() {
        return never_began(sink, account, path, how);
    }
    // The data file the import writes in: the account's own when it had one (never removed for
    // a discard), else the one the import makes, noted once it's there.
    let file = store.data_file(account).filter(|f| f.exists());
    let own = file.is_none();
    let mut told = Told { sink, store, account, stop, began, last: Progress::default(), folders_done: 0, folders_said: None, file, own };
    told.say(ImportPhase::Planning);
    if path.to_ascii_lowercase().ends_with(".mailcraft") {
        return copy_in(&mut told, path);
    }
    let opts = ImportOptions { stop: stop.clone(), planned, resume, ..ImportOptions::default() };
    #[cfg(test)]
    let opts = ImportOptions { fault: tests::fault_for(account), ..opts };
    let result = crate::pst::import_into(Path::new(path), store, account, &opts, &mut told);
    // Thrown away, however far it got (it may have ended on its own just before).
    if stop.how() == Some(StopHow::Discard) {
        let (messages, skipped) = result.as_ref().map_or((0, 0), |d| (d.messages, d.skipped));
        return thrown_away(&told, path, messages, skipped);
    }
    match result {
        // It can't go on from where it stopped (the file changed, went, or is held): it waits
        // on the user, nothing read.
        Ok(done) if done.blocked.is_some() => {
            let progress = ImportProgress { phase: ImportPhase::Blocked, blocked: done.blocked, run_ms: ms(began), ..ImportProgress::default() };
            sink.emit(Event::ImportProgress { account: account.to_string(), progress });
        }
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

/// MailCraft's own data file copied in, asked as it's copied whether to stop. Stopped part-way,
/// nothing of it is kept: a part of a data file, copied in the order it keeps its files, is no
/// account's mail (folder lists naming messages not there, all of them still the old account's).
fn copy_in(told: &mut Told<'_>, path: &str) -> Option<PathBuf> {
    let (store, sink, account, stop) = (told.store, told.sink, told.account, told.stop);
    let file = RefCell::new(told.file.clone());
    let copied = store.import_data_file_until(account, Path::new(path), &|| {
        #[cfg(test)]
        if let Some(crate::pst::Fault::Slow(ms)) = tests::fault_for(account) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        // The data file being written, as the copy goes (it's made by the first batch).
        if file.borrow().is_none() && stop.how() != Some(StopHow::Discard) {
            *file.borrow_mut() = store.data_file(account);
        }
        stop.raised()
    });
    told.file = file.into_inner();
    if stop.how() == Some(StopHow::Discard) {
        return thrown_away(told, path, 0, 0);
    }
    let messages = match copied {
        Ok(n) => n,
        Err(mailcraft_store::StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::Interrupted && stop.raised() => {
            return stopped_copy(told, path);
        }
        // Any other error is said, whatever was asked meanwhile.
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
    // Whole: a stop asked once the last batch was in came too late to matter.
    let folders = store.load_folders(account).unwrap_or_default();
    let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
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

/// A copy cancelled, or paused as MailCraft closes, part-way: what came in goes (the account is
/// still this one: it wasn't removed). Cancelled, it ends as a file that couldn't be opened, none
/// of it kept; paused, only a word that it stopped is said.
fn stopped_copy(told: &Told<'_>, path: &str) -> Option<PathBuf> {
    let (error, left) = remove_written(told, false);
    if told.stop.how() == Some(StopHow::Pause) {
        if let Some(e) = error {
            log::warn!("{}: {e}", told.account);
        }
        told.say(ImportPhase::Paused);
        return left;
    }
    let stopped = "it was stopped before all of it was copied, and none of it was kept";
    let error = Some(error.map_or_else(|| stopped.to_string(), |e| format!("{stopped} ({e})")));
    let report = ImportReport { file: path.to_string(), stopped: told.stop.how(), runs: 1, elapsed_ms: ms(told.began), ..ImportReport::default() };
    told.sink.emit(Event::Imported { account: told.account.to_string(), messages: 0, skipped: 0, done: true, error, report: Some(Box::new(report)) });
    left
}

/// An import thrown away: the data file it made goes, and the end is said, with why the file is
/// still there when it is (it's returned, to be tried again once the import's thread is over).
fn thrown_away(told: &Told<'_>, path: &str, messages: usize, skipped: usize) -> Option<PathBuf> {
    let (error, left) = remove_written(told, true);
    let report = ImportReport { file: path.to_string(), stopped: Some(StopHow::Discard), runs: 1, ..ImportReport::default() };
    told.sink.emit(Event::Imported { account: told.account.to_string(), messages, skipped, done: true, error, report: Some(Box::new(report)) });
    left
}

/// Remove the data file the import wrote in (with `made`, only one it made itself) while the
/// account still has that very file: never by the account's id alone. A removed account lets go
/// of its file before its import sees the stop (the file is kept, as a removed account's is), and
/// its id may be another account's by then, whose file stays. Returns why it couldn't be removed,
/// and the file, to be tried again.
fn remove_written(told: &Told<'_>, made: bool) -> (Option<String>, Option<PathBuf>) {
    let (store, account) = (told.store, told.account);
    let Some(file) = told.file.clone().filter(|f| (told.own || !made) && store.data_file(account).as_ref() == Some(f)) else {
        return (None, None);
    };
    let mut error = None;
    // The store lets go of the file and forgets it, even when it can't be removed yet.
    if let Err(e) = store.delete_account(account) {
        error = Some(format!("its data file couldn't be removed: {e}"));
    }
    if let Err(e) = remove(&file) {
        return (Some(format!("its data file couldn't be removed: {e}")), Some(file));
    }
    (error, None)
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
        elapsed_ms: ms(began).saturating_add(done.earlier_ms),
        runs: done.runs.max(1),
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
/// part-way through aren't in the list, so none can be changed while it's written. Thrown away,
/// it says nothing more but its end: the account is gone, and its id may be another's by then.
/// It notes the data file the import writes in, the one file a stop may remove.
struct Told<'a> {
    sink: &'a Sink,
    store: &'a Store,
    account: &'a str,
    stop: &'a Stop,
    began: Instant,
    last: Progress,
    folders_done: usize,
    folders_said: Option<Instant>,
    file: Option<PathBuf>,
    /// The import made `file` (the account had none).
    own: bool,
}

impl Told<'_> {
    fn say(&self, phase: ImportPhase) {
        if self.thrown_away() {
            return;
        }
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

    fn thrown_away(&self) -> bool {
        self.stop.how() == Some(StopHow::Discard)
    }

    /// The account's data file, noted once the import's first batch has made it: then only, and
    /// never once the import is thrown away (its account may have let go of it by then, and a
    /// file under its id be another account's).
    fn note_file(&mut self) {
        if self.file.is_none() && !self.thrown_away() {
            self.file = self.store.data_file(self.account);
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
        if self.thrown_away() {
            return;
        }
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
