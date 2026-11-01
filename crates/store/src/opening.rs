//! Opening the data files before the window has anything in it, and how that's going.
//!
//! A data file written to when MailCraft ended without closing (a crash, a power cut, a forced
//! quit) after its last seal is checked as it opens: every page read, which took 305 s for a
//! 37 GB file. The app opens its data files on a thread of its own ([`Store::open_data_files`])
//! and draws what [`OpenProgress`] says meanwhile, so the window comes at once, with the check in
//! it, rather than nothing showing until the check ends.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

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
    /// How far its check is, in thousandths (the database says so only a few times).
    permille: AtomicU32,
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

    /// How far the check of the file is, in thousandths: 0 until the database first says.
    pub fn permille(&self) -> u32 {
        self.permille.load(Ordering::Relaxed).min(1000)
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
        self.permille.store((fraction * 1000.0).clamp(0.0, 1000.0) as u32, Ordering::Relaxed);
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
        let _ = self.stage.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire);
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
