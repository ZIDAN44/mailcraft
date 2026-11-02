//! The bytes an import may hold between reading an item and having it on the disk.
//!
//! A lane takes a charge for an item before reading it (what the file says the item weighs, times
//! what a rebuilt message holds at once besides), and the writer gives it back once the item's
//! source is written. A lane whose charge doesn't fit waits, except the lane holding the item the
//! writer needs next: that one always goes on, so the import can't wait on itself. At most the
//! cap and one item are held.
//!
//! Room is given in the items' order: a lane waits, too, while one with an item before its own
//! does (one whose charge could fit at all: an item charged more than the cap waits for its turn
//! without holding up the others, as it can't go before then anyway). Otherwise the small items
//! after a big one took the room as it came back, a little at a time, and the big one waited
//! until the writer needed it, then held up every lane and the writer while it alone was read:
//! on a 41 GB file the writer stood idle for 40 to 44% of the import (10 to 22% with room given
//! in order), and the lanes waited for room half their time.

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::*;

/// What an import may hold when not told otherwise. With room given in order the lanes keep the
/// writer busy with less: on the 41 GB file, in the app, 288 MiB kept it 84 to 90% busy at peaks
/// of 818 to 860 MB, where 384 MiB peaked at 950 to 972 MB (850 MB aimed at).
pub(super) const CAP: u64 = 288 * 1024 * 1024;
/// A waiting lane looks at the stop flag this often, woken or not.
const RECHECK: Duration = Duration::from_millis(200);

#[derive(Default)]
struct State {
    in_flight: u64,
    /// The item the writer needs next.
    oldest: u64,
    /// The items whose lanes wait, in order (those whose charge could fit at all).
    queued: BTreeSet<u64>,
    /// The lanes waiting because their charge doesn't fit now, not only for their turn.
    short: usize,
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

    /// Take `charge` for item `seq`, waiting while it doesn't fit or an item before it waits
    /// (unless `seq` is the item the writer needs next). False, with nothing taken, once `stopped`
    /// says to stop. The time it waited is counted to `tally`.
    pub(super) fn acquire(&self, seq: u64, charge: u64, stopped: &dyn Fn() -> bool, tally: &Tally) -> bool {
        self.acquire_more(seq, 0, charge, stopped, tally)
    }

    /// Take `charge` more for item `seq`, whose lane holds `held` for it already (an item read
    /// bigger than the file said): as [`Budget::acquire`], the item weighing the two together.
    pub(super) fn acquire_more(&self, seq: u64, held: u64, charge: u64, stopped: &dyn Fn() -> bool, tally: &Tally) -> bool {
        let mut st = self.lock();
        let began = Instant::now();
        let (mut waited, mut counted) = (false, false);
        loop {
            let short = st.in_flight.saturating_add(charge) > self.cap;
            let behind = st.queued.first().is_some_and(|first| *first < seq);
            if !(short || behind) || seq == st.oldest || stopped() {
                break;
            }
            if !waited {
                waited = true;
                // Charged more than the cap in all, it can only go as the writer's next (what its
                // lane holds counts in `in_flight` already): none waits for it.
                if held.saturating_add(charge) <= self.cap {
                    st.queued.insert(seq);
                }
            }
            if short != counted {
                counted = short;
                st.short = if short { st.short.saturating_add(1) } else { st.short.saturating_sub(1) };
            }
            st = match self.changed.wait_timeout(st, RECHECK) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
        if counted {
            st.short = st.short.saturating_sub(1);
        }
        if waited {
            // The next in line may go now.
            st.queued.remove(&seq);
            self.changed.notify_all();
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

    /// Whether a lane is waiting for room (not only for its turn).
    pub(super) fn waiting(&self) -> bool {
        self.lock().short > 0
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

    /// A big item waiting for room isn't passed by the small ones after it, though they'd fit:
    /// it gets the room first, as it comes back.
    #[test]
    fn room_is_given_in_order() {
        let budget = Budget::new(100);
        let stop = AtomicBool::new(false);
        let tally = Tally::default();
        let stopped = || stop.load(Ordering::Relaxed);
        // The item the writer needs holds most of the room.
        assert!(budget.acquire(0, 90, &stopped, &tally));
        let order = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|s| {
            let big = s.spawn(|| {
                assert!(budget.acquire(1, 50, &stopped, &tally));
                order.lock().unwrap().push(1);
            });
            while !budget.waiting() {
                std::thread::yield_now();
            }
            let small = s.spawn(|| {
                assert!(budget.acquire(2, 5, &stopped, &tally));
                order.lock().unwrap().push(2);
            });
            // Looked at the room more than once meanwhile, the small one still waits.
            std::thread::sleep(RECHECK * 3);
            let passed = order.lock().unwrap().clone();
            budget.release(90);
            big.join().unwrap();
            small.join().unwrap();
            assert!(passed.is_empty(), "a small item went before the big one waiting");
        });
        assert_eq!(order.lock().unwrap().len(), 2);
        assert_eq!(budget.in_flight(), 55);
    }

    /// An item charged more than the whole budget waits for its turn, but the small ones after it
    /// that fit aren't held up by it.
    #[test]
    fn an_item_over_the_cap_holds_no_one_up() {
        let budget = Budget::new(100);
        let stop = AtomicBool::new(false);
        let tally = Tally::default();
        let stopped = || stop.load(Ordering::Relaxed);
        assert!(budget.acquire(0, 10, &stopped, &tally));
        std::thread::scope(|s| {
            let big = s.spawn(|| budget.acquire(1, 500, &stopped, &tally));
            while !budget.waiting() {
                std::thread::yield_now();
            }
            // Later items that fit go, though the big one waits (held up, they'd be stopped).
            let small = s.spawn(|| budget.acquire(2, 20, &stopped, &tally) && budget.acquire(3, 20, &stopped, &tally));
            let began = Instant::now();
            while !small.is_finished() && began.elapsed() < Duration::from_secs(2) {
                std::thread::yield_now();
            }
            let went = small.is_finished();
            if !went {
                stop.store(true, Ordering::Relaxed);
                budget.wake();
            }
            assert!(small.join().unwrap() && went, "small items waited behind one over the cap");
            assert_eq!(budget.in_flight(), 50);
            assert!(budget.waiting(), "the big one waits for room");
            // Its turn: it goes.
            budget.advance(1);
            assert!(big.join().unwrap());
        });
        assert!(!budget.waiting());
        assert_eq!(budget.in_flight(), 550);
    }

    /// An item read bigger than the file said, over the cap in all though the more it asks for
    /// isn't, doesn't hold up the small ones after it either.
    #[test]
    fn a_top_up_over_the_cap_holds_no_one_up() {
        let budget = Budget::new(100);
        let stop = AtomicBool::new(false);
        let tally = Tally::default();
        let stopped = || stop.load(Ordering::Relaxed);
        assert!(budget.acquire(0, 10, &stopped, &tally));
        assert!(budget.acquire(1, 60, &stopped, &tally));
        std::thread::scope(|s| {
            // Read at 150: 90 more, under the cap, but 150 in all.
            let big = s.spawn(|| budget.acquire_more(1, 60, 90, &stopped, &tally));
            while !budget.waiting() {
                std::thread::yield_now();
            }
            let small = s.spawn(|| budget.acquire(2, 10, &stopped, &tally) && budget.acquire(3, 10, &stopped, &tally));
            let began = Instant::now();
            while !small.is_finished() && began.elapsed() < Duration::from_secs(2) {
                std::thread::yield_now();
            }
            let went = small.is_finished();
            if !went {
                stop.store(true, Ordering::Relaxed);
                budget.wake();
            }
            assert!(small.join().unwrap() && went, "small items waited behind a top-up over the cap");
            assert_eq!(budget.in_flight(), 90);
            budget.advance(1);
            assert!(big.join().unwrap());
        });
        assert_eq!(budget.in_flight(), 180);
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
