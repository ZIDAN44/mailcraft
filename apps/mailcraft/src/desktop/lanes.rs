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

pub(crate) struct Lanes {
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
    pub(crate) fn submit(&self, lane: Lane, work: Work) -> Result<(), Work> {
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
    pub(crate) fn available(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn work(cancelled: &Arc<AtomicBool>, run: impl FnOnce() + Send + 'static) -> Work {
        Work { run: Box::new(run), cancelled: cancelled.clone(), first: false }
    }

    /// Work that panics leaves the lane working: the next runs, on the same thread.
    #[test]
    fn a_panic_leaves_the_lane_working() {
        let lanes = Lanes::default();
        let no = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        assert!(
            lanes
                .submit(
                    Lane::Pictures,
                    work(&no, move || {
                        tx2.send(std::thread::current().id()).unwrap();
                        panic!("a picture that breaks the decoder");
                    })
                )
                .is_ok()
        );
        assert!(lanes.submit(Lane::Pictures, work(&no, move || tx.send(std::thread::current().id()).unwrap())).is_ok());
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let next = rx.recv_timeout(Duration::from_secs(10)).expect("the work after the panic ran");
        assert_eq!(first, next, "on the lane's one thread");
        assert_ne!(first, std::thread::current().id());
        assert!(lanes.available());
    }

    /// A lane runs its work in order, one at a time; work marked first goes before what waits.
    #[test]
    fn work_runs_in_order_with_first_ahead() {
        let lanes = Lanes::default();
        let no = Arc::new(AtomicBool::new(false));
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (tx, rx) = mpsc::channel();
        // The first holds the lane while the others queue up.
        let t = tx.clone();
        lanes
            .submit(
                Lane::Pictures,
                work(&no, move || {
                    t.send(-1).unwrap();
                    gate_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    t.send(0).unwrap();
                }),
            )
            .ok()
            .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), -1, "started");
        for n in 1..=3 {
            let t = tx.clone();
            lanes.submit(Lane::Pictures, Work { first: n == 3, ..work(&no, move || t.send(n).unwrap()) }).ok().unwrap();
        }
        gate_tx.send(()).unwrap();
        let order: Vec<i32> = (0..4).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
        assert_eq!(order, [0, 3, 1, 2]);
    }

    /// Cancelled work that goes on past the limit (a decoder caught in a loop) is left alone:
    /// the work waiting behind it runs on a new thread. Work that wasn't cancelled is waited for.
    #[test]
    fn a_lane_stuck_on_cancelled_work_gets_a_new_thread() {
        let mut lanes = Lanes::default();
        lanes.stuck_after = Duration::from_millis(200);
        let (loop_cancelled, no) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (tx, rx) = mpsc::channel();
        let t = tx.clone();
        lanes
            .submit(
                Lane::Pictures,
                work(&loop_cancelled, move || {
                    // Goes on until the test lets it end (it never looks at its flag).
                    let _ = stop_rx.recv_timeout(Duration::from_secs(30));
                    t.send("loop ended").unwrap();
                }),
            )
            .ok()
            .unwrap();
        lanes.submit(Lane::Pictures, work(&no, move || tx.send("next").unwrap())).ok().unwrap();
        // Long, but not cancelled: waited for.
        std::thread::sleep(Duration::from_millis(400));
        assert!(lanes.available());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "not cancelled: the lane waits");
        // Cancelled late in a long run: given the limit from its cancel, not its start.
        loop_cancelled.store(true, Ordering::Relaxed);
        assert!(lanes.available());
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "just cancelled: the lane still waits");
        // Once it has gone on past the limit after its cancel, the lane carries on without it.
        std::thread::sleep(Duration::from_millis(200));
        assert!(lanes.available());
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), "next");
        // The new thread takes later work while the old one is still caught.
        let (tx2, rx2) = mpsc::channel();
        lanes.submit(Lane::Pictures, work(&no, move || tx2.send(std::thread::current().name().map(str::to_string)).unwrap())).ok().unwrap();
        assert_eq!(rx2.recv_timeout(Duration::from_secs(10)).unwrap().as_deref(), Some("mail-pictures"));
        stop_tx.send(()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), "loop ended");
    }

    /// A thread that can't be started gives the work back, and no more work is offered.
    #[test]
    fn a_thread_that_cannot_start_gives_the_work_back() {
        let mut lanes = Lanes::default();
        lanes.spawn = |_, _, _| Err(std::io::Error::other("no threads"));
        let no = Arc::new(AtomicBool::new(false));
        assert!(lanes.available());
        let (tx, rx) = mpsc::channel();
        let back = lanes.submit(Lane::Pictures, work(&no, move || tx.send(()).unwrap())).expect_err("given back");
        assert!(!lanes.available(), "work is done in the window now");
        (back.run)();
        assert!(rx.try_recv().is_ok(), "the work given back is whole");
    }

    /// Each lane has its own thread: work on one doesn't wait for the other's.
    #[test]
    fn lanes_do_not_wait_for_each_other() {
        let lanes = Lanes::default();
        let no = Arc::new(AtomicBool::new(false));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        lanes.submit(Lane::Pictures, work(&no, move || _ = stop_rx.recv_timeout(Duration::from_secs(30)))).ok().unwrap();
        let (tx, rx) = mpsc::channel();
        lanes.submit(Lane::Reading, work(&no, move || tx.send(std::thread::current().name().map(str::to_string)).unwrap())).ok().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().as_deref(), Some("mail-reading"));
        stop_tx.send(()).unwrap();
    }
}
