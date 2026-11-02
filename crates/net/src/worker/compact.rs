//! Data files made smaller, each on a thread of its own ("mail-compact"), as
//! `mailcraft_model::job::plan` says for how it was asked: in place (what an earlier build kept
//! raw compressed again first, in the background; then the database's own compaction, which has
//! the file to itself), or by a copy into a new file, in the background; or not at all, told why.
//!
//! While a compaction has the file to itself (in place, or at the end of a copy), the account's
//! jobs wait here rather than go to its threads ([`Pool::held_for_compaction`]): its sync is
//! paused, no message is fetched (so no events pile up for the window to apply after), and nothing
//! on the pool's own thread (the window's) waits on the file. They go on as soon as that part
//! ends. A sync already under way waits for the file in the store, as anything else asking for it
//! does.
//!
//! Progress comes at most every [`PROGRESS_EVERY`]. Stopping ([`Job::CancelCompact`]) is heard
//! between batches; the part that has the file to itself can't be stopped.

use std::sync::atomic::AtomicBool;

use mailcraft_model::job::{CompactMode, CompactPhase, CompactPlan, CompactReport, DataFileHealth, Work, plan};

use super::*;
use crate::space::{LOW_SPACE, Space};

/// Progress is sent at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// The compactions running, by account, and the jobs waiting for one to give its file back.
#[derive(Default)]
pub(super) struct Compacts {
    running: HashMap<String, CompactCtl>,
    held: HashMap<String, Vec<Job>>,
    /// The drive asked for room (a test's, else the computer's own).
    #[cfg(test)]
    pub(super) space: Option<Arc<dyn Space>>,
}

/// A compaction: how it's stopped, whether it has the file to itself now, and its thread.
struct CompactCtl {
    cancel: Arc<AtomicBool>,
    exclusive: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// The file had to itself until this is dropped (however the compaction ends).
struct Exclusive(Arc<AtomicBool>);

impl Exclusive {
    fn begin(flag: &Arc<AtomicBool>) -> Exclusive {
        flag.store(true, Ordering::SeqCst);
        Exclusive(flag.clone())
    }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// What a compaction thread works with.
struct Run {
    store: Store,
    sink: Sink,
    account: String,
    mode: CompactMode,
    cancel: Arc<AtomicBool>,
    exclusive: Arc<AtomicBool>,
    space: Arc<dyn Space>,
}

impl Pool {
    /// Measure an account's data file, on a thread of its own (a cold one takes a moment).
    pub(super) fn data_file_health(&mut self, account: String) {
        let (store, sink) = (self.store.clone(), self.sink.clone());
        let spawned = std::thread::Builder::new().name("mail-health".into()).spawn(move || {
            let measured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.data_file_health(&account)));
            let (health, error) = match measured {
                Ok(Ok(h)) => (h.map(Box::new), None),
                Ok(Err(e)) => (None, Some(e.to_string())),
                Err(_) => (None, Some("the data file couldn't be measured: it may be damaged".into())),
            };
            sink.emit(Event::DataFileHealth { account, health, error });
        });
        if let Err(e) = spawned {
            log::warn!("data file health: {e}");
        }
    }

    /// Compact an account's data file on a thread of its own; one compacting already, or
    /// importing, is told so.
    pub(super) fn compact_data_file(&mut self, account: String, mode: CompactMode) {
        self.prune_compacts();
        let refused = if self.compacts.running.contains_key(&account) {
            Some("its data file is being compacted already")
        } else if self.importing(&account) {
            Some("a data file is being imported into it")
        } else {
            None
        };
        if let Some(why) = refused {
            let report = CompactReport { mode, error: Some(why.into()), ..CompactReport::default() };
            return self.sink.emit(Event::Compacted { account, report: Box::new(report) });
        }
        let (cancel, exclusive) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        #[cfg(test)]
        let space = self.compacts.space.clone().unwrap_or_else(|| Arc::new(crate::space::Drives));
        #[cfg(not(test))]
        let space: Arc<dyn Space> = Arc::new(crate::space::Drives);
        let run = Run {
            store: self.store.clone(),
            sink: self.sink.clone(),
            account: account.clone(),
            mode,
            cancel: cancel.clone(),
            exclusive: exclusive.clone(),
            space,
        };
        let spawned = std::thread::Builder::new().name("mail-compact".into()).spawn(move || {
            let report = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.run())) {
                Ok(r) => r,
                Err(_) => CompactReport {
                    mode: run.mode,
                    error: Some("compacting stopped unexpectedly; the data file is checked as it next opens".into()),
                    ..CompactReport::default()
                },
            };
            // Given back before it's said: the window may ask for the file as soon as it hears.
            run.exclusive.store(false, Ordering::SeqCst);
            run.sink.emit(Event::Compacted { account: run.account.clone(), report: Box::new(report) });
        });
        match spawned {
            Ok(thread) => {
                self.compacts.running.insert(account, CompactCtl { cancel, exclusive, thread });
            }
            Err(e) => {
                let report = CompactReport { mode, error: Some(format!("compact: {e}")), ..CompactReport::default() };
                self.sink.emit(Event::Compacted { account, report: Box::new(report) });
            }
        }
    }

    /// Stop compacting an account's data file where that can be stopped (heard between batches).
    pub(super) fn cancel_compact(&mut self, account: &str) {
        if let Some(c) = self.compacts.running.get(account) {
            c.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Every compaction asked to stop, none waited for (MailCraft closing): one that has its file
    /// to itself ends with the process, and the file is checked as it next opens.
    pub(super) fn cancel_compacts(&mut self) {
        for c in self.compacts.running.values() {
            c.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// `job`, unless its account's data file is being had to itself by a compaction: then it
    /// waits here, and goes on once that's over ([`Pool::release_compacted`]).
    pub(super) fn held_for_compaction(&mut self, job: Job) -> Option<Job> {
        if matches!(job, Job::CancelCompact { .. } | Job::CompactDataFile { .. }) {
            return Some(job);
        }
        let account = job.account();
        let held = !account.is_empty()
            && (self.compacts.held.contains_key(account) || self.compacts.running.get(account).is_some_and(|c| c.exclusive.load(Ordering::SeqCst)));
        if !held {
            return Some(job);
        }
        self.compacts.held.entry(account.to_string()).or_default().push(job);
        None
    }

    /// The jobs held for accounts whose compaction no longer has the file to itself, sent on in
    /// the order they came.
    pub(super) fn release_compacted(&mut self) {
        if self.compacts.held.is_empty() {
            return;
        }
        let free: Vec<String> = self
            .compacts
            .held
            .keys()
            .filter(|a| self.compacts.running.get(*a).is_none_or(|c| !c.exclusive.load(Ordering::SeqCst)))
            .cloned()
            .collect();
        for account in free {
            for job in self.compacts.held.remove(&account).unwrap_or_default() {
                self.submit(job);
            }
        }
    }

    /// Compactions that have ended let go of.
    fn prune_compacts(&mut self) {
        self.compacts.running.retain(|_, c| !c.thread.is_finished());
    }
}

impl Run {
    fn progress(&self, phase: CompactPhase, work: Work, exclusive_s: Option<u64>) {
        self.sink.emit(Event::Compacting { account: self.account.clone(), phase, work, exclusive_s });
    }

    fn size(&self) -> Option<u64> {
        self.store.data_file(&self.account).and_then(|f| std::fs::metadata(f).ok()).map(|m| m.len())
    }

    fn health(&self) -> std::result::Result<Option<DataFileHealth>, String> {
        self.store.data_file_health(&self.account).map_err(|e| e.to_string())
    }

    /// The compaction, start to end: what it came to.
    fn run(&self) -> CompactReport {
        let start = Instant::now();
        let mut report = CompactReport { mode: self.mode, ..CompactReport::default() };
        self.progress(CompactPhase::Measuring, Work::default(), None);
        let health = match self.health() {
            Ok(Some(h)) => h,
            Ok(None) => return report,
            Err(e) => return CompactReport { error: Some(e), ..report },
        };
        (report.before, report.after) = (health.file_bytes, health.file_bytes);
        let free = self.space.free(&crate::space::data_dir(&self.store, &self.account));
        match plan(&health, self.mode, free) {
            CompactPlan::Nothing => {}
            CompactPlan::TooLong { seconds } => report.too_long_s = Some(seconds),
            CompactPlan::Reimport => report.reimport = true,
            CompactPlan::InPlace { recompress } => self.in_place(&health, recompress, &mut report),
            CompactPlan::Copy => self.copy(&mut report),
        }
        report.elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        report
    }

    /// Compressed again first when `recompress`, then compacted, if that still fits how it was
    /// asked.
    fn in_place(&self, health: &DataFileHealth, recompress: bool, report: &mut CompactReport) {
        let mut health = health.clone();
        if recompress {
            let estimate = Some(health.exclusive_ms(true).div_ceil(1000));
            let mut last: Option<Instant> = None;
            let mut progress = |w: Work| {
                if last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    last = Some(Instant::now());
                    self.progress(CompactPhase::Recompress, w, estimate);
                }
            };
            match self.store.recompress_data_file(&self.account, &self.cancel, &mut progress) {
                Ok(Some(r)) => {
                    report.recompressed = r.before.saturating_sub(r.after);
                    if !r.finished {
                        report.cancelled = true;
                        return;
                    }
                }
                Ok(None) => return,
                Err(e) => {
                    report.error = Some(e.to_string());
                    return;
                }
            }
            // Measured again: what compressing freed moves too, and may take it past the limit.
            health = match self.health() {
                Ok(Some(h)) => h,
                Ok(None) => return,
                Err(e) => {
                    report.error = Some(e);
                    return;
                }
            };
            let seconds = health.exclusive_ms(false).div_ceil(1000);
            if self.mode.limit_s().is_some_and(|limit| seconds > limit) {
                report.too_long_s = Some(seconds);
                return;
            }
        }
        if self.cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return;
        }
        let seconds = health.exclusive_ms(false).div_ceil(1000);
        let _exclusive = Exclusive::begin(&self.exclusive);
        self.progress(CompactPhase::Exclusive, Work::default(), Some(seconds));
        match self.store.compact_data_file(&self.account) {
            Ok(Some(s)) => {
                report.compacted = true;
                report.after = s.after;
            }
            Ok(None) => {}
            Err(e) => {
                report.error = Some(e.to_string());
                report.after = self.size().unwrap_or(report.after);
            }
        }
    }

    /// Copied into a new file, stopped when asked or when the drive runs short of room.
    fn copy(&self, report: &mut CompactReport) {
        let dir = crate::space::data_dir(&self.store, &self.account);
        let mut last: Option<Instant> = None;
        let mut short = false;
        let mut go_on = |w: Work| {
            if self.cancel.load(Ordering::Relaxed) {
                return false;
            }
            if last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                last = Some(Instant::now());
                if self.space.free(&dir).is_some_and(|f| f < LOW_SPACE) {
                    short = true;
                    return false;
                }
                self.progress(CompactPhase::Copy, w, None);
            }
            true
        };
        // The end has the file to itself too: said, and the account's jobs held, as in place.
        let mut exclusive: Option<Exclusive> = None;
        let mut swap = || {
            exclusive = Some(Exclusive::begin(&self.exclusive));
            self.progress(CompactPhase::Exclusive, Work::default(), None);
            true
        };
        let copied = self.store.copy_data_file(&self.account, &mut go_on, &mut swap);
        drop(exclusive);
        match copied {
            Ok(Some(s)) if s.cancelled && short => report.error = Some("the drive is running out of room; the copy was thrown away".into()),
            Ok(Some(s)) if s.cancelled => report.cancelled = true,
            Ok(Some(s)) => {
                report.copied = true;
                report.after = s.after;
            }
            Ok(None) => {}
            Err(e) => report.error = Some(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;
