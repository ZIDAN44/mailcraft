//! Opening the data files before the window has anything in it, and how that's going.
//!
//! A data file written to when MailCraft ended without closing (a crash, a power cut, a forced
//! quit) after its last seal is checked as it opens: every page read, which took 305 s for a
//! 37 GB file. The app opens its data files on a thread of its own ([`Store::open_data_files`])
//! and draws what [`OpenProgress`] says meanwhile, so the window comes at once, with the check in
//! it, rather than nothing showing until the check ends.

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use super::*;

/// Where opening the data files is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenStage {
    /// Opening one (quick, unless the disk is slow).
    Opening,
    /// Checking one that wasn't closed properly.
    Checking,
    /// All opened (or not: [`OpenProgress::problems`]).
    Done,
}

/// How opening the data files is going: written by the thread opening them, read by the window
/// as it draws. Portable: no clock, no threads.
#[derive(Debug, Default)]
pub struct OpenProgress {
    /// The data file being opened (its file name only).
    file: Mutex<String>,
    stage: AtomicU8,
    /// How far its check is, in thousandths, as the database last said (it says so only as each
    /// of its passes over the file begins: 0, 600 and 900, or 300 too if it had to roll back).
    permille: AtomicU32,
    /// The bytes the data files have read: how far a pass over the one being checked has come.
    read: AtomicU64,
    /// The pass under way, for the part of the way it has come ([`OpenProgress::permille`]).
    pass: Mutex<Pass>,
    /// The most said so far of the file being checked: what's said never goes back.
    shown: AtomicU32,
    done: AtomicU32,
    total: AtomicU32,
    /// How many had to be checked.
    checked: AtomicU32,
    /// Data files that couldn't be opened: (file name, why).
    problems: Mutex<Vec<(String, String)>>,
    /// A check holds here until a test lets it go ([`OpenProgress::hold_checks`]).
    #[cfg(test)]
    pub(crate) hold: (Mutex<bool>, std::sync::Condvar),
}

/// A pass of the database's over a data file it checks: from where it says it is to where its
/// next pass begins, and the bytes read when it began, of a file of `len` bytes.
#[derive(Debug, Default)]
struct Pass {
    to: u32,
    read_at: u64,
    len: u64,
}

/// Where the database's check is next said to be, after it says it's at `permille` (each pass
/// reads the file through; one rolling back to the last commit says 300 after its first).
fn next_said(permille: u32) -> u32 {
    match permille {
        0..600 => 600,
        600..900 => 900,
        _ => 1000,
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl OpenProgress {
    /// Nothing to open (a cache of plain files, or in memory): done already.
    pub fn finished() -> OpenProgress {
        let p = OpenProgress::default();
        p.finish();
        p
    }

    pub fn stage(&self) -> OpenStage {
        match self.stage.load(Ordering::Acquire) {
            0 => OpenStage::Opening,
            1 => OpenStage::Checking,
            _ => OpenStage::Done,
        }
    }

    /// The data file being opened or checked (its file name).
    pub fn file(&self) -> String {
        lock(&self.file).clone()
    }

    /// How far the check of the file is, in thousandths: where the database last said it was,
    /// and as far again towards where it next says as its pass has read of the file (up to just
    /// short of it: a file has room in it the pass doesn't read). Never less than said before.
    pub fn permille(&self) -> u32 {
        let said = self.permille.load(Ordering::Relaxed).min(1000);
        let now = {
            let pass = lock(&self.pass);
            let read = self.read.load(Ordering::Relaxed).saturating_sub(pass.read_at).min(pass.len);
            let part = if pass.len == 0 { 0.0 } else { read as f64 / pass.len as f64 * 0.99 };
            // A float to an integer saturates: within said..to.
            said.saturating_add((f64::from(pass.to.saturating_sub(said)) * part) as u32)
        };
        self.shown.fetch_max(now, Ordering::Relaxed).max(now).min(1000)
    }

    /// The data files read `bytes` more (as they read them).
    pub(crate) fn read(&self, bytes: usize) {
        self.read.fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// The data file about to be checked has `len` bytes (said before [`OpenProgress::checking`]).
    pub(crate) fn sized(&self, len: u64) {
        lock(&self.pass).len = len;
    }

    /// (opened, to open).
    pub fn files(&self) -> (u32, u32) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    /// How many data files had to be checked.
    pub fn checked(&self) -> u32 {
        self.checked.load(Ordering::Relaxed)
    }

    /// Data files that couldn't be opened: (file name, why).
    pub fn problems(&self) -> Vec<(String, String)> {
        lock(&self.problems).clone()
    }

    /// A data file (or the whole opening, with no file name) failed: said, and gone on from.
    pub fn fail(&self, file: &str, why: String) {
        let mut p = lock(&self.problems);
        // A few are told; the rest are in the log.
        if p.len() < 16 {
            p.push((file.to_string(), why));
        }
    }

    /// All of them opened, or given up on.
    pub fn finish(&self) {
        self.stage.store(2, Ordering::Release);
    }

    /// `total` data files to open.
    pub fn start(&self, total: usize) {
        self.total.store(u32::try_from(total).unwrap_or(u32::MAX), Ordering::Relaxed);
    }

    /// Opening `file` now.
    pub fn opening(&self, file: &str) {
        *lock(&self.file) = file.to_string();
        self.permille.store(0, Ordering::Relaxed);
        self.shown.store(0, Ordering::Relaxed);
        *lock(&self.pass) = Pass::default();
        self.stage.store(0, Ordering::Release);
    }

    /// The database is checking `file`, `fraction` of the way: called by it as it goes (on the
    /// thread opening it).
    pub fn checking(&self, file: &str, fraction: f64) {
        if self.stage.swap(1, Ordering::AcqRel) != 1 {
            *lock(&self.file) = file.to_string();
            self.checked.fetch_add(1, Ordering::Relaxed);
        }
        // A float to an integer saturates: NaN is 0, anything past 1 is 1000.
        let said = (fraction * 1000.0).clamp(0.0, 1000.0) as u32;
        {
            let mut pass = lock(&self.pass);
            pass.to = next_said(said);
            pass.read_at = self.read.load(Ordering::Relaxed);
        }
        self.permille.store(said, Ordering::Relaxed);
        #[cfg(test)]
        {
            let (held, wake) = &self.hold;
            let mut h = lock(held);
            while *h {
                h = wake.wait(h).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
    }

    /// A check ended (the file opened, or not): the next one is told and counted, a data file
    /// opened later included (as the command line opens them, where none counts as opened).
    pub fn checked_one(&self) {
        if self.stage.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            // The next check starts from nothing, though no file was said to open before it.
            self.shown.store(0, Ordering::Relaxed);
            *lock(&self.pass) = Pass::default();
        }
    }

    /// One more data file opened.
    pub fn opened(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
        self.stage.store(0, Ordering::Release);
    }

    /// Checks hold (`true`) until let go (`false`): what else goes on meanwhile is tested.
    #[cfg(test)]
    pub(crate) fn hold_checks(&self, on: bool) {
        *lock(&self.hold.0) = on;
        self.hold.1.notify_all();
    }
}

/// Finishes the progress however opening ends, a panic included: the window never waits on it
/// for good.
#[cfg(not(target_arch = "wasm32"))]
struct Finish<'a>(&'a OpenProgress);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Finish<'_> {
    fn drop(&mut self) {
        self.0.finish();
    }
}

impl Store {
    /// How opening the data files is going ([`Store::open_data_files`]).
    pub fn open_progress(&self) -> Arc<OpenProgress> {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.progress.clone(),
            _ => Arc::new(OpenProgress::finished()),
        }
    }

    /// Open every data file, checking any that wasn't closed properly, with account folders from
    /// before brought in first ([`Store::packed`]); [`Store::open_progress`] says how it goes,
    /// and is finished however this ends. One that can't be opened is told there and gone on
    /// from (an account using it says why again); folders that couldn't come in are the error.
    /// Nothing to do for other stores.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_data_files(&self) -> Result<()> {
        let progress = self.open_progress();
        let _finish = Finish(&progress);
        match &self.files.main {
            Files::Packed(x) => x.open_all(),
            _ => Ok(()),
        }
    }
}
