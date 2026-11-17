//! The threads slow work runs on (`Services::background`): one per lane, started when the lane
//! is first given work. Each piece of work runs under `catch_unwind`, so a picture that breaks
//! a decoder leaves the lane working. Work that goes on 30 seconds after it was cancelled (a
//! PDF that sends the renderer round in a loop) is left to finish alone: the lane carries on in
//! a new thread, and the old one ends when that work does.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mailcraft_ui_egui::{Lane, Work};

/// How long cancelled work may go on before its lane is given a new thread.
const STUCK_AFTER: Duration = Duration::from_secs(30);

/// A lock that outlives a panic elsewhere (the queue is whole between its changes).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A lane's work waiting, and the thread doing it.
#[derive(Default)]
struct Queue {
    waiting: VecDeque<Work>,
    /// The thread that takes the work (a number counted up as threads are replaced): one left
    /// behind finds it changed and ends.
    thread: u64,
    /// Whether a thread was started.
    started: bool,
    /// The work running now.
    running: Option<Running>,
    /// The app is closing: the threads end.
    closed: bool,
}

/// The work a lane's thread is doing.
struct Running {
    cancelled: Arc<AtomicBool>,
    /// When its cancel was first seen (the lane is looked in on now and then).
    cancel_seen: Option<Instant>,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    /// Told when work is added, or the lane closes.
    work: Condvar,
}

pub(super) struct Lanes {
    lanes: Mutex<HashMap<Lane, Arc<Shared>>>,
    /// A thread couldn't be started: work is done in the window instead.
    failed: AtomicBool,
    stuck_after: Duration,
    /// Starts a lane's thread (`spawn`; one that fails, in tests).
    spawn: fn(Lane, &Arc<Shared>, u64) -> std::io::Result<()>,
}

impl Default for Lanes {
    fn default() -> Lanes {
        Lanes { lanes: Mutex::new(HashMap::new()), failed: AtomicBool::new(false), stuck_after: STUCK_AFTER, spawn }
    }
}

impl Lanes {
    /// Run `work` on `lane`'s thread (started if it isn't); `Err` gives it back when a thread
    /// couldn't be started.
    pub(super) fn submit(&self, lane: Lane, work: Work) -> Result<(), Work> {
        let shared = lock(&self.lanes).entry(lane).or_default().clone();
        let mut q = lock(&shared.queue);
        if !q.started {
            q.thread = q.thread.wrapping_add(1);
            if (self.spawn)(lane, &shared, q.thread).is_err() {
                self.failed.store(true, Ordering::Relaxed);
                return Err(work);
            }
            q.started = true;
        }
        if work.first {
            q.waiting.push_front(work);
        } else {
            q.waiting.push_back(work);
        }
        drop(q);
        shared.work.notify_all();
        self.replace_stuck();
        Ok(())
    }

    /// Whether work can be given to a thread (no thread failed to start). Looks after the
    /// lanes too: one whose cancelled work has gone on too long gets a new thread.
    pub(super) fn available(&self) -> bool {
        self.replace_stuck();
        !self.failed.load(Ordering::Relaxed)
    }

    /// Each lane whose work running now was cancelled more than `stuck_after` ago (counted
    /// from when the cancel was first seen here) gets a new thread for the work waiting; the
    /// old one ends when that work does.
    fn replace_stuck(&self) {
        let lanes: Vec<(Lane, Arc<Shared>)> = lock(&self.lanes).iter().map(|(l, s)| (*l, s.clone())).collect();
        for (lane, shared) in lanes {
            let mut q = lock(&shared.queue);
            let Some(running) = q.running.as_mut() else { continue };
            if !running.cancelled.load(Ordering::Relaxed) {
                continue;
            }
            let seen = *running.cancel_seen.get_or_insert_with(Instant::now);
            if seen.elapsed() < self.stuck_after {
                continue;
            }
            let old = q.thread;
            q.thread = old.wrapping_add(1);
            if (self.spawn)(lane, &shared, q.thread).is_err() {
                // The stuck thread stays the lane's: its work waits for it.
                q.thread = old;
                continue;
            }
            log::warn!("{lane:?} work went on {}s after it was cancelled; the lane goes on in a new thread", self.stuck_after.as_secs());
            q.running = None;
            drop(q);
            shared.work.notify_all();
        }
    }
}

impl Drop for Lanes {
    fn drop(&mut self) {
        for shared in lock(&self.lanes).values() {
            lock(&shared.queue).closed = true;
            shared.work.notify_all();
        }
    }
}

/// Start thread number `id` of a lane.
fn spawn(lane: Lane, shared: &Arc<Shared>, id: u64) -> std::io::Result<()> {
    let name = match lane {
        Lane::Pictures => "mail-pictures",
        Lane::Reading => "mail-reading",
        Lane::Videos => "mail-videos",
    };
    let shared = shared.clone();
    std::thread::Builder::new().name(name.into()).spawn(move || work_lane(&shared, id)).map(drop)
}

/// A lane's thread: the work waiting, one at a time, until the lane closes or the thread is
/// replaced.
fn work_lane(shared: &Shared, id: u64) {
    loop {
        let work = {
            let mut q = lock(&shared.queue);
            loop {
                if q.closed || q.thread != id {
                    return;
                }
                if let Some(w) = q.waiting.pop_front() {
                    q.running = Some(Running { cancelled: w.cancelled.clone(), cancel_seen: None });
                    break w;
                }
                q = shared.work.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work.run)).is_err() {
            log::warn!("work on a background thread failed (it panicked); the next goes on");
        }
        let mut q = lock(&shared.queue);
        if q.thread == id {
            q.running = None;
        }
    }
}
