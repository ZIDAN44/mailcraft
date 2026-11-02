//! Read marks, flags and categories of a data file's mail kept on a thread of the store's own
//! ([`Store::mark_later`]), in the order they were made.
//!
//! Each was a durable commit of the account's data file made in the window's frame: on a 38 GB
//! file that took 24 ms by median, and the disk's flushes now and then held it for 150 ms to
//! 1.4 s, every Down that opened an unread message. The window changes its lists in memory, then
//! hands the flags over (what each message's flags are now, not how they changed, so a mark
//! written late still says the last word) with the folders' counts, written together in one
//! commit, still durable: a mark is saved at once, only not while the window waits.
//!
//! What reads a folder's list from the disk ([`Store::folder_load`]) waits first for the marks
//! handed over before in that folder, what reads or writes an account's folder list for its
//! marks (their counts), and closing ([`Store::seal_all`]) for all of them: a list read
//! meanwhile would lack them, and a move filed the message with its flags from before. Nothing
//! waits for another folder's or another account's. The thread's own store doesn't wait on
//! itself. A mark that couldn't be kept is told ([`Store::take_marks_failed`]), for the window to
//! read its folders again from the disk. A wait of the window's holds its frame: one longer than
//! [`SLOW_WAIT`] is logged with who waited. The window's own writes of an account's folder list go
//! the same way ([`Store::count_later`], [`Store::save_folders_later`]), after the marks before
//! them, and it reads no folder whose marks are still to be written ([`Store::marks_waiting_for`]):
//! it waits for none of them.

use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use std::sync::mpsc::Sender;
use std::sync::{Condvar, MutexGuard, PoisonError};
use std::time::Duration;

use mailcraft_model::Flags;

use crate::*;

/// How long [`Store::flush_marks`] waits for the marks before it at most.
const FLUSH_WAIT: Duration = Duration::from_secs(10);
/// A wait for the marks, or for the lock every change of a list takes, longer than this is
/// logged with who waited: on the window's thread it held a frame (a hold of Down on a 38 GB
/// data file once had frames of 0.5 and 1.8 s while marks were committed).
pub(crate) const SLOW_WAIT: Duration = Duration::from_millis(50);

/// One folder's messages marked: their number and what their flags are now, in the list of
/// `validity` (a list made anew since isn't marked).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderMarks {
    pub folder: String,
    pub validity: u32,
    pub flags: Vec<(u32, Flags)>,
}

/// What came of marks kept behind the window that it should hear of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarksAfter {
    /// They couldn't be kept: the folders are as the disk has them.
    Failed { account: String, folders: Vec<String>, message: String },
    /// The account's folder list on disk has none of the folders counted (it was damaged and
    /// set aside): the window's is to be written whole.
    Unlisted { account: String },
}

/// Marks handed to the thread, and what's become of them.
#[derive(Default)]
pub(crate) struct Marking {
    /// The thread's queue (none in the browser, which writes them at once): only the stores
    /// handing it marks hold it, so it closes, and the thread ends, once they have all gone.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    tx: Mutex<Option<Sender<Task>>>,
    /// What the thread shares with them.
    state: Arc<MarkState>,
}

#[derive(Default)]
struct MarkState {
    queue: Mutex<Queue>,
    written: Condvar,
    /// Held by a test to keep the thread from writing ([`Store::pause_marks`]).
    pause: Mutex<()>,
}

#[derive(Default)]
struct Queue {
    /// Marks handed over, and of them written (or failed).
    sent: u64,
    done: u64,
    after: Vec<MarksAfter>,
    /// The marks handed over and not written yet, in order: each one's account and the folders
    /// it marks (what waits for them waits only for those of its folder or account).
    pending: VecDeque<(String, Vec<String>)>,
}

impl Queue {
    /// Whether marks of `account` (in `folder`; in any of its folders, or their counts, when
    /// `None`) are still to be written.
    fn waiting_for(&self, account: &str, folder: Option<&str>) -> bool {
        self.pending.iter().any(|(a, fs)| a == account && folder.is_none_or(|f| fs.iter().any(|x| x == f)))
    }
}

struct Task {
    account: String,
    lists: Vec<FolderMarks>,
    counts: Vec<(String, u32, u32)>,
    /// The account's whole folder list, to write instead ([`Store::save_folders_later`]).
    folders: Option<Vec<Folder>>,
    /// How many times the account had been removed when they were made ([`Store::while_kept`]).
    seen: u64,
}

impl std::fmt::Debug for Marking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let q = self.queue();
        f.debug_struct("Marking").field("sent", &q.sent).field("done", &q.done).finish_non_exhaustive()
    }
}

impl Marking {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.state.queue()
    }
}

impl MarkState {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Store {
    /// Keep the flags of `lists`' messages as they are now and the account's folder `counts`
    /// (`path`, unread, total), together, on the store's thread for marks after those handed
    /// over before (written at once where there's no such thread: the browser).
    pub fn mark_later(&self, account: &str, lists: Vec<FolderMarks>, counts: Vec<(String, u32, u32)>) {
        self.write_later(Task { account: account.to_string(), lists, counts, folders: None, seen: self.removals(account) });
    }

    /// Keep the account's whole folder list (`folders`) on the store's thread for marks, after the
    /// marks handed over before: a durable commit of a data file not made in the window's frame,
    /// nor waiting there for those marks. One that couldn't be kept is told
    /// ([`MarksAfter::Failed`], with no folders).
    pub fn save_folders_later(&self, account: &str, folders: Vec<Folder>) {
        self.write_later(Task {
            account: account.to_string(),
            lists: Vec::new(),
            counts: Vec::new(),
            folders: Some(folders),
            seen: self.removals(account),
        });
    }

    /// Hand `task` to the store's thread for marks (written at once where there's none).
    fn write_later(&self, task: Task) {
        #[cfg(not(target_arch = "wasm32"))]
        let task = match self.send_marks(task) {
            Ok(()) => return,
            Err(task) => task,
        };
        let after = self.write_marks(&task);
        self.marking.queue().after.extend(after);
    }

    /// Keep the account's folder `counts` (`path`, unread, total) on the store's thread for
    /// marks, after the marks handed over before (whose counts they follow): a durable commit of
    /// a data file not made in the window's frame. A folder list on disk with none of them is
    /// told ([`MarksAfter::Unlisted`]).
    pub fn count_later(&self, account: &str, counts: Vec<(String, u32, u32)>) {
        if !counts.is_empty() {
            self.mark_later(account, Vec::new(), counts);
        }
    }

    /// Wait (a while at most) until the marks handed over so far are on disk. Whether they are.
    #[track_caller]
    pub fn flush_marks(&self) -> bool {
        let target = self.marking.queue().sent;
        self.wait_marks(|q| q.done < target)
    }

    /// Wait (a while at most) until the marks of `account` handed over so far (in `folder`; in
    /// any of its folders, with their counts, when `None`) are on disk. Whether they are.
    #[track_caller]
    pub fn flush_marks_of(&self, account: &str, folder: Option<&str>) -> bool {
        self.wait_marks(|q| q.waiting_for(account, folder))
    }

    /// Wait while `waiting` says so, [`FLUSH_WAIT`] at most; a wait over [`SLOW_WAIT`] is logged
    /// with its caller.
    #[track_caller]
    fn wait_marks(&self, waiting: impl Fn(&Queue) -> bool) -> bool {
        let mut q = self.marking.queue();
        if !waiting(&q) {
            return true;
        }
        let caller = std::panic::Location::caller();
        let slow = Slow::start();
        while waiting(&q) {
            let left = FLUSH_WAIT.saturating_sub(slow.took());
            q = match self.marking.state.written.wait_timeout(q, left) {
                Ok((q, t)) if t.timed_out() && waiting(&q) => {
                    log::warn!("read marks: still being saved after {} s ({caller})", FLUSH_WAIT.as_secs());
                    return false;
                }
                Ok((q, _)) => q,
                Err(e) => e.into_inner().0,
            };
        }
        slow.log("the read marks", caller);
        true
    }

    /// The lock every change of a list (and of the folder list) is made under; a wait for it over
    /// [`SLOW_WAIT`] is logged with its caller.
    #[track_caller]
    pub(crate) fn hold_changes(&self) -> MutexGuard<'_, ()> {
        match self.changes.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                let slow = Slow::start();
                let held = self.changes.lock().unwrap_or_else(PoisonError::into_inner);
                slow.log("the lock of the lists' changes", std::panic::Location::caller());
                held
            }
        }
    }

    /// Whether marks handed over aren't on disk yet (tests).
    #[doc(hidden)]
    pub fn marks_waiting(&self) -> bool {
        let q = self.marking.queue();
        q.done < q.sent
    }

    /// Whether marks of `account` (in `folder`; in any of its folders, with their counts, when
    /// `None`) aren't on disk yet: reading them from the disk would wait for them
    /// ([`Store::flush_marks_of`]).
    pub fn marks_waiting_for(&self, account: &str, folder: Option<&str>) -> bool {
        self.marking.queue().waiting_for(account, folder)
    }

    /// Keep the thread from writing marks while the guard is held (tests: what the window's thread
    /// writes itself).
    #[doc(hidden)]
    pub fn pause_marks(&self) -> std::sync::MutexGuard<'_, ()> {
        self.marking.state.pause.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold the lock every change of a list is made under, as the thread for marks does through a
    /// commit (tests: what the window's thread would wait for).
    #[doc(hidden)]
    pub fn hold_lists(&self) -> MutexGuard<'_, ()> {
        self.hold_changes()
    }

    /// What came of the marks written since the last call that the window should hear of.
    pub fn take_marks_failed(&self) -> Vec<MarksAfter> {
        std::mem::take(&mut self.marking.queue().after)
    }

    /// Hand `task` to the thread (started with the first): `Err` gives it back when there's none.
    #[cfg(not(target_arch = "wasm32"))]
    fn send_marks(&self, task: Task) -> std::result::Result<(), Task> {
        let mut tx = self.marking.tx.lock().unwrap_or_else(PoisonError::into_inner);
        if tx.is_none() {
            *tx = self.start_marking();
        }
        let Some(sender) = tx.as_ref() else { return Err(task) };
        // Counted before it's sent, so a flush right after waits for it.
        {
            let mut q = self.marking.queue();
            q.sent += 1;
            q.pending.push_back((task.account.clone(), task.lists.iter().map(|l| l.folder.clone()).collect()));
        }
        match sender.send(task) {
            Ok(()) => Ok(()),
            Err(mpsc::SendError(task)) => {
                *tx = None;
                let mut q = self.marking.queue();
                q.sent = q.sent.saturating_sub(1);
                q.pending.pop_back();
                Err(task)
            }
        }
    }

    /// The thread: it holds the store, but not its marks (a store that waits on none) nor the
    /// queue's sending end, so it ends once every store handing it marks has gone, and never
    /// waits on itself.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_marking(&self) -> Option<Sender<Task>> {
        let (tx, rx) = mpsc::channel::<Task>();
        let store = Store { marking: Arc::default(), ..self.clone() };
        let marking = self.marking.state.clone();
        let spawned = std::thread::Builder::new().name("mail-marks".into()).spawn(move || {
            for task in rx {
                drop(marking.pause.lock().unwrap_or_else(PoisonError::into_inner));
                let after = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.write_marks(&task))).unwrap_or_else(|_| {
                    log::error!("{}: read marks failed unexpectedly", task.account);
                    vec![failed(&task, "read marks failed unexpectedly".into())]
                });
                let mut q = marking.queue();
                q.done = q.done.saturating_add(1);
                q.pending.pop_front();
                q.after.extend(after);
                marking.written.notify_all();
            }
        });
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::warn!("read marks will be saved as they're made: {e}");
                None
            }
        }
    }

    /// Write `task` (nothing for an account removed since): what the window should hear of.
    fn write_marks(&self, task: &Task) -> Vec<MarksAfter> {
        if let Some(folders) = &task.folders {
            return match self.while_kept(&task.account, task.seen, || self.save_folders(&task.account, folders)) {
                Some(Err(e)) => vec![failed(task, e.to_string())],
                _ => Vec::new(),
            };
        }
        match self.while_kept(&task.account, task.seen, || self.save_flags(&task.account, &task.lists, &task.counts)) {
            None => Vec::new(),
            Some(Ok(0)) if !task.counts.is_empty() => vec![MarksAfter::Unlisted { account: task.account.clone() }],
            Some(Ok(_)) => Vec::new(),
            // Counts alone ([`Store::count_later`]): the next count puts them right.
            Some(Err(e)) if task.lists.is_empty() => {
                log::warn!("{}: folder counts not saved: {e}", task.account);
                Vec::new()
            }
            Some(Err(e)) => vec![failed(task, e.to_string())],
        }
    }

    /// Keep the flags of `lists`' messages as they are now and the account's folder counts, in one
    /// write (one durable commit in a data file). A list kept in pages takes them beside its pages
    /// ([`folders::MARKS`]) while they're few, and into its pages once they're more; a small list
    /// is written whole. Says how many of the counts' folders the account's list has.
    pub fn save_flags(&self, account: &str, lists: &[FolderMarks], counts: &[(String, u32, u32)]) -> Result<usize> {
        let _held = self.hold_changes();
        let (mut writes, mut stale) = (Vec::new(), Vec::new());
        for marks in lists.iter().filter(|m| !m.flags.is_empty()) {
            let (w, s) = self.flag_writes(account, marks)?;
            writes.extend(w);
            stale.extend(s);
        }
        let mut list = self.read_folders(account)?;
        let (found, recounted) = folders::set_counts(&mut list, counts);
        if recounted {
            let path = self.account_dir(account).join("folders.json");
            let bytes = serde_json::to_vec(&list).map_err(|source| StoreError::Json { path: path.display().to_string(), source })?;
            writes.push((path, bytes));
        }
        if !writes.is_empty() {
            self.write_files(writes)?;
        }
        self.remove_stale(stale);
        Ok(found)
    }

    /// What keeping one folder's marks writes (with the lock held), and the pages it no longer has.
    fn flag_writes(&self, account: &str, marks: &FolderMarks) -> Result<(Vec<(PathBuf, Vec<u8>)>, Vec<PathBuf>)> {
        let dir = self.folder_dir(account, &marks.folder);
        let head: StateFile = self.read_json(&dir.join("state.json"))?;
        if head.validity != marks.validity {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut now: BTreeMap<u32, &Flags> = marks.flags.iter().map(|(uid, f)| (*uid, f)).collect();
        let mark_path = dir.join(folders::MARKS);
        let kept: Vec<(u32, Flags)> = if head.pages.is_empty() { Vec::new() } else { self.read_json(&mark_path)? };
        if !head.pages.is_empty() {
            let mut all: BTreeMap<u32, &Flags> = kept.iter().map(|(uid, f)| (*uid, f)).collect();
            all.extend(now.iter().map(|(uid, f)| (*uid, *f)));
            if all.len() <= folders::MARKS_MAX {
                let list: Vec<(u32, &Flags)> = all.into_iter().collect();
                let bytes = serde_json::to_vec(&list).map_err(|source| StoreError::Json { path: mark_path.display().to_string(), source })?;
                return Ok((vec![(mark_path, bytes)], Vec::new()));
            }
            // Too many: into their pages, as the list has them with these.
            for (uid, f) in &kept {
                now.entry(*uid).or_insert(f);
            }
        }
        let mut st = self.list_from_disk(account, &marks.folder)?;
        for m in &mut st.messages {
            if let Some(f) = now.get(&m.key.uid) {
                m.flags.clone_from(f);
            }
        }
        let changed: std::collections::HashSet<u32> = now.keys().copied().collect();
        self.list_writes(account, &marks.folder, &st, Some(&changed))
    }
}

/// `task`'s marks not kept, for `message`.
fn failed(task: &Task, message: String) -> MarksAfter {
    let mut folders: Vec<String> = task.lists.iter().map(|l| l.folder.clone()).collect();
    folders.sort();
    folders.dedup();
    MarksAfter::Failed { account: task.account.clone(), folders, message }
}

/// How long a wait took, timed only once it's known to wait (the browser, which reads no clock,
/// has no thread to wait on).
struct Slow(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Slow {
    fn start() -> Slow {
        Slow(
            #[cfg(not(target_arch = "wasm32"))]
            std::time::Instant::now(),
        )
    }

    fn took(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        return self.0.elapsed();
        #[cfg(target_arch = "wasm32")]
        Duration::ZERO
    }

    /// Logged when it was over [`SLOW_WAIT`], with the thread and `caller` that waited for `what`.
    fn log(&self, what: &str, caller: &std::panic::Location<'_>) {
        let took = self.took();
        if took > SLOW_WAIT {
            let thread = std::thread::current();
            log::warn!("{} waited {} ms for {what} ({caller})", thread.name().unwrap_or("a thread"), took.as_millis());
        }
    }
}
