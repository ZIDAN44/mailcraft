//! The bytes an import may hold between reading an item and having it on the disk.
//!
//! A lane takes a charge for an item before reading it (what the file says the item weighs, times
//! what a rebuilt message holds at once besides), and the writer gives it back once the item's
//! source is written. A lane whose charge doesn't fit waits, except the lane holding the item the
//! writer needs next: that one always goes on, so the import can't wait on itself. At most the
//! cap and one item are held.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::*;

/// What an import may hold when not told otherwise.
pub(super) const CAP: u64 = 384 * 1024 * 1024;
/// A waiting lane looks at the stop flag this often, woken or not.
const RECHECK: Duration = Duration::from_millis(200);

#[derive(Default)]
struct State {
    in_flight: u64,
    /// The item the writer needs next.
    oldest: u64,
    waiting: usize,
}

pub(super) struct Budget {
    cap: u64,
    state: Mutex<State>,
    changed: Condvar,
}

impl Budget {
    /// A budget of `cap` bytes (0: [`CAP`]).
    pub(super) fn new(cap: u64) -> Self {
        Budget { cap: if cap == 0 { CAP } else { cap }, state: Mutex::default(), changed: Condvar::new() }
    }

    pub(super) fn cap(&self) -> u64 {
        self.cap
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take `charge` for item `seq`, waiting while it doesn't fit (unless `seq` is the item the
    /// writer needs next). False, with nothing taken, once `stopped` says to stop. The time it
    /// waited is counted to `tally`.
    pub(super) fn acquire(&self, seq: u64, charge: u64, stopped: &dyn Fn() -> bool, tally: &Tally) -> bool {
        let mut st = self.lock();
        let began = Instant::now();
        let mut waited = false;
        while st.in_flight.saturating_add(charge) > self.cap && seq != st.oldest && !stopped() {
            if !waited {
                waited = true;
                st.waiting = st.waiting.saturating_add(1);
            }
            st = match self.changed.wait_timeout(st, RECHECK) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
        if waited {
            st.waiting = st.waiting.saturating_sub(1);
            tally.waited(began);
        }
        if stopped() {
            return false;
        }
        st.in_flight = st.in_flight.saturating_add(charge);
        true
    }

    /// Give back `charge`.
    pub(super) fn release(&self, charge: u64) {
        if charge == 0 {
            return;
        }
        let mut st = self.lock();
        st.in_flight = st.in_flight.saturating_sub(charge);
        drop(st);
        self.changed.notify_all();
    }

    /// The writer needs item `seq` next (it only goes forward).
    pub(super) fn advance(&self, seq: u64) {
        let mut st = self.lock();
        if seq > st.oldest {
            st.oldest = seq;
            drop(st);
            self.changed.notify_all();
        }
    }

    /// Whether a lane is waiting for room.
    pub(super) fn waiting(&self) -> bool {
        self.lock().waiting > 0
    }

    #[cfg(test)]
    pub(super) fn in_flight(&self) -> u64 {
        self.lock().in_flight
    }

    /// Wake every waiting lane (to see that it should stop).
    pub(super) fn wake(&self) {
        self.changed.notify_all();
    }
}

/// What an item is charged before it's read: the size the file gives it (taken as at least 4 KiB
/// and at most [`MAX_CHARGE`]; 1 MiB when it gives none), times what its message holds at
/// once while it's made (its parts, the message, the compressed copy), 3.5.
pub(super) fn charge_for(size: Option<i64>) -> u64 {
    let est = match size {
        Some(n) if n > 0 => u64::try_from(n).unwrap_or(0).clamp(4 * 1024, MAX_CHARGE),
        _ => 1024 * 1024,
    };
    held(est)
}

/// What `bytes` of an item hold at once while its message is made.
pub(super) fn held(bytes: u64) -> u64 {
    bytes.saturating_mul(7) / 2
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;

    /// Lanes taking charges bigger than the whole budget, in random order: every one gets in (the
    /// one the writer needs next always does), and all of it comes back.
    #[test]
    fn charges_over_the_cap_all_get_in_and_come_back() {
        let budget = Budget::new(1000);
        let stop = AtomicBool::new(false);
        let tally = Tally::default();
        let next = AtomicU64::new(0);
        let n = 400u64;
        std::thread::scope(|s| {
            for lane in 0..8u64 {
                let (budget, stop, tally, next) = (&budget, &stop, &tally, &next);
                s.spawn(move || {
                    let mut seq = lane;
                    while seq < n {
                        let charge = (seq * 7919 + lane * 31) % 2500;
                        assert!(budget.acquire(seq, charge, &|| stop.load(Ordering::Relaxed), tally));
                        // Applied in order, as the writer does.
                        while next.load(Ordering::SeqCst) != seq {
                            std::thread::yield_now();
                        }
                        budget.release(charge);
                        next.store(seq + 1, Ordering::SeqCst);
                        budget.advance(seq + 1);
                        seq += 8;
                    }
                });
            }
        });
        assert_eq!(budget.in_flight(), 0);
    }

    /// A lane waiting for room gives up once told to stop, taking nothing.
    #[test]
    fn a_stop_wakes_a_waiting_lane() {
        let budget = Budget::new(10);
        let stop = AtomicBool::new(false);
        let tally = Tally::default();
        budget.advance(99);
        let stopped = || stop.load(Ordering::Relaxed);
        assert!(budget.acquire(0, 10, &stopped, &tally));
        std::thread::scope(|s| {
            let waiter = s.spawn(|| budget.acquire(1, 5, &stopped, &tally));
            while !budget.waiting() {
                std::thread::yield_now();
            }
            stop.store(true, Ordering::Relaxed);
            budget.wake();
            assert!(!waiter.join().unwrap());
        });
        assert_eq!(budget.in_flight(), 10);
    }

    #[test]
    fn an_item_is_charged_for_what_it_holds() {
        assert_eq!(charge_for(None), 7 * 512 * 1024);
        assert_eq!(charge_for(Some(-5)), 7 * 512 * 1024);
        assert_eq!(charge_for(Some(1)), 7 * 2 * 1024);
        assert_eq!(charge_for(Some(i64::MAX)), held(MAX_CHARGE));
    }
}
