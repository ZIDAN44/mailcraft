//! How long a long piece of work has left, from the progress it reports (pure: the rate comes
//! from the worker's own times, the countdown and stalls from the interface's clock, which it
//! passes in; nothing here reads one).

use super::import::ImportProgress;
use super::progress::permille;

/// How fast the rate follows a change: about two thirds of the way in this long.
const TAU_MS: f64 = 30_000.0;
/// Samples closer than this are noise (one big item takes most of a second).
const SAMPLE_MS: u64 = 1_000;
/// Nothing is said before this long into a run, and this far into the work (thousandths).
const SHOW_AFTER_MS: u64 = 20_000;
const SHOW_AFTER_PERMILLE: u16 = 20;
/// No progress for this long (by the interface's clock): the time left isn't known.
const STALL_MS: u64 = 30_000;
/// The time shown is revised only when off by more than this share of it, or this long.
const REVISE: f64 = 0.10;
const REVISE_MIN_S: u64 = 15;
/// The most ever said.
pub const MAX_S: u64 = 99 * 3600;

/// The estimate: a time-based moving average of the work rate, counted down between revisions.
#[derive(Clone, Debug, Default)]
pub struct Eta {
    started: bool,
    /// The newest progress seen (worker time and work), and the run's start.
    run_ms: u64,
    done: u64,
    total: u64,
    run_work0: u64,
    /// The run's first progress (worker time and work), the rate's origin; `None`: none yet.
    origin: Option<(u64, u64)>,
    /// The last sample the rate was taken at.
    sample_ms: u64,
    sample_work: u64,
    /// Work per millisecond; 0: none yet.
    rate: f64,
    /// Seconds left as last shown, and when (interface time, ms).
    shown_s: Option<u64>,
    shown_at: u64,
    /// When the work last moved (interface time, ms).
    moved_at: u64,
}

impl Eta {
    /// Take in an import's progress, as it arrives at `now_ui` (the interface's clock, ms).
    pub fn update(&mut self, p: &ImportProgress, now_ui: u64) {
        self.sample(p.run_ms, p.work_done(), p.work_total(), p.run_start_work, now_ui);
    }

    /// Take in `done` of `total` work at `run_ms` into a run that began with `run_work0` done,
    /// arriving at `now_ui`.
    pub fn sample(&mut self, run_ms: u64, done: u64, total: u64, run_work0: u64, now_ui: u64) {
        // A new run (an import that goes on after a stop) starts the estimate again: its clock
        // went back, it began from other work, or the work went back.
        if !self.started || run_ms < self.run_ms || run_work0 != self.run_work0 || done < self.done {
            *self = Eta { started: true, run_work0, sample_work: run_work0, done: run_work0, moved_at: now_ui, ..Eta::default() };
        }
        if done > self.done {
            self.moved_at = now_ui;
        }
        (self.run_ms, self.done, self.total) = (run_ms, done, total);
        // The rate is timed from the run's first progress: the walk for the totals before it does
        // no work. Seeded with a first second that held the walk, the moving average was still
        // half way from it when first shown, 20 s in: a real file's import said 4 min with 96 s left.
        let Some((origin_ms, origin_work)) = self.origin else {
            if done > self.run_work0 {
                self.origin = Some((run_ms, done));
                (self.sample_ms, self.sample_work) = (run_ms, done);
            }
            return;
        };
        let dt = run_ms.saturating_sub(self.sample_ms);
        if dt < SAMPLE_MS {
            return;
        }
        let delta = done.saturating_sub(self.sample_work);
        let since = run_ms.saturating_sub(origin_ms);
        if self.rate > 0.0 && since as f64 >= TAU_MS {
            // Weighted by the time between samples, so irregular ones count for what they cover.
            let alpha = 1.0 - (-(dt as f64) / TAU_MS).exp();
            self.rate += alpha * (delta as f64 / dt as f64 - self.rate);
        } else {
            // The plain average since the work began, until the moving average has had a time
            // constant to go by.
            self.rate = done.saturating_sub(origin_work) as f64 / since.max(1) as f64;
        }
        (self.sample_ms, self.sample_work) = (run_ms, done);
        if !self.rate.is_finite() || self.rate <= 0.0 {
            self.rate = 0.0;
            self.shown_s = None;
            return;
        }
        let raw = total.saturating_sub(done) as f64 / self.rate / 1000.0;
        // `as` saturates a float; the bound keeps it in range besides.
        let raw = if raw.is_finite() { raw.clamp(0.0, MAX_S as f64) as u64 } else { MAX_S };
        let revise = match self.counted_down(now_ui) {
            None => true,
            Some(now_s) => raw.abs_diff(now_s) as f64 > (REVISE * now_s as f64).max(REVISE_MIN_S as f64),
        };
        if revise {
            (self.shown_s, self.shown_at) = (Some(raw), now_ui);
        }
    }

    /// Seconds left at `now_ui` (the interface's clock, ms): `None` too early in the run, while
    /// the work hasn't moved for a while, or with no rate to go by; else 1..=[`MAX_S`].
    pub fn remaining_s(&self, now_ui: u64) -> Option<u64> {
        if !self.started || self.run_ms < SHOW_AFTER_MS || now_ui.saturating_sub(self.moved_at) >= STALL_MS {
            return None;
        }
        if permille(self.done, self.total).is_none_or(|p| p < SHOW_AFTER_PERMILLE) {
            return None;
        }
        self.counted_down(now_ui).map(|s| s.clamp(1, MAX_S))
    }

    /// The time last shown, less the time since.
    fn counted_down(&self, now_ui: u64) -> Option<u64> {
        self.shown_s.map(|s| s.saturating_sub(now_ui.saturating_sub(self.shown_at) / 1000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `eta` one sample a second from `from_s` to `to_s` (run seconds) at `rate` work a
    /// second; the interface's clock runs 3 s behind the worker's. Returns the work done.
    fn run(eta: &mut Eta, from_s: u64, to_s: u64, mut done: u64, rate: u64, total: u64) -> u64 {
        for t in from_s..to_s {
            done += rate;
            eta.sample((t + 1) * 1000, done, total, 0, (t + 1) * 1000 + 3000);
        }
        done
    }

    fn close(shown: Option<u64>, truth: f64, within: f64) -> bool {
        shown.is_some_and(|s| (s as f64 - truth).abs() <= truth * within)
    }

    /// A run going on from 40% done, at the rate the first run went, is told as closely as a
    /// first run: its rate goes by the work of this run alone.
    #[test]
    fn a_run_gone_on_with_goes_by_its_own_rate() {
        let (rate, total) = (1_000_000, 1_000_000_000);
        let start = total * 4 / 10;
        let mut eta = Eta::default();
        let mut done = start;
        for t in 0..60u64 {
            done += rate;
            eta.sample((t + 1) * 1000, done, total, start, (t + 1) * 1000 + 3000);
            let truth = (total - done) as f64 / rate as f64;
            if let Some(s) = eta.remaining_s((t + 1) * 1000 + 3000) {
                assert!(close(Some(s), truth, 0.20), "{s} s at {t} s vs {truth}");
            }
        }
        assert!(eta.remaining_s(63_000).is_some(), "said by then");
    }

    /// A run going on walks for its totals too, reporting the work it began from: what it says
    /// it began from is what keeps that walk out of its rate.
    #[test]
    fn a_run_gone_on_times_its_rate_from_its_own_first_progress() {
        let (rate, total) = (1_000_000, 1_000_000_000);
        let start = total * 4 / 10;
        let first_said = |run_work0: u64| {
            let mut eta = Eta::default();
            for t in 1..=5u64 {
                eta.sample(t * 1000, start, total, run_work0, t * 1000 + 3000);
            }
            let mut done = start;
            for t in 6..=20u64 {
                done += rate;
                eta.sample(t * 1000, done, total, run_work0, t * 1000 + 3000);
            }
            (eta.remaining_s(23_000), (total - done) as f64 / rate as f64)
        };
        let (said, truth) = first_said(start);
        assert!(close(said, truth, 0.05), "{said:?} vs {truth}");
        // Told it began at nothing, the work it began from is its first progress, and the walk
        // counts against its rate.
        let (said, truth) = first_said(0);
        assert!(said.is_some() && !close(said, truth, 0.20), "{said:?} vs {truth}");
    }

    #[test]
    fn a_steady_rate_is_told_closely() {
        let (rate, total) = (1_000_000, 1_000_000_000);
        let mut eta = Eta::default();
        let done = run(&mut eta, 0, 100, 0, rate, total);
        let truth = (total - done) as f64 / rate as f64;
        assert!(close(eta.remaining_s(103_000), truth, 0.01), "{:?} vs {truth}", eta.remaining_s(103_000));
        // Between samples it counts down.
        assert!(close(eta.remaining_s(113_000), truth - 10.0, 0.01));
    }

    /// The walk for the totals takes the run's first seconds, then the work goes at a steady
    /// rate: the first time said, 20 s in, is within 20% (it was twice the truth), and so is
    /// each one after.
    #[test]
    fn the_walk_before_the_work_doesnt_count_against_its_rate() {
        let (rate, total) = (1_000_000, 100_000_000);
        for walk_s in [1, 2, 5] {
            let mut eta = Eta::default();
            for t in 1..=walk_s {
                eta.sample(t * 1000, 0, total, 0, t * 1000);
            }
            let mut done = 0;
            let mut first = None;
            for t in walk_s + 1..=60 {
                done += rate;
                eta.sample(t * 1000, done, total, 0, t * 1000);
                let truth = (total - done) as f64 / rate as f64;
                if let Some(s) = eta.remaining_s(t * 1000) {
                    first.get_or_insert(t);
                    assert!(close(Some(s), truth, 0.20), "walk {walk_s} s: {s} s at {t} s vs {truth}");
                }
            }
            assert_eq!(first, Some(20), "walk {walk_s} s: said from 20 s in");
        }
    }

    #[test]
    fn a_slower_rate_is_followed() {
        let (rate, total) = (1_000_000, 1_000_000_000);
        let mut eta = Eta::default();
        let done = run(&mut eta, 0, 100, 0, rate, total);
        // Half as fast from here on: after four time constants the estimate has followed.
        let done = run(&mut eta, 100, 220, done, rate / 2, total);
        let truth = (total - done) as f64 / (rate / 2) as f64;
        assert!(close(eta.remaining_s(223_000), truth, 0.10), "{:?} vs {truth}", eta.remaining_s(223_000));
    }

    #[test]
    fn nothing_is_said_too_early() {
        let mut eta = Eta::default();
        assert_eq!(eta.remaining_s(0), None);
        // 19 s in: too soon, however far along.
        run(&mut eta, 0, 19, 0, 10_000_000, 1_000_000_000);
        assert_eq!(eta.remaining_s(22_000), None);
        // Long enough, but under 2% done.
        let mut eta = Eta::default();
        run(&mut eta, 0, 60, 0, 1_000, 1_000_000_000);
        assert_eq!(eta.remaining_s(63_000), None);
        // No total: nothing to count down to.
        let mut eta = Eta::default();
        run(&mut eta, 0, 60, 0, 1_000, 0);
        assert_eq!(eta.remaining_s(63_000), None);
    }

    #[test]
    fn a_stall_is_seen_without_any_progress_arriving() {
        let mut eta = Eta::default();
        let done = run(&mut eta, 0, 60, 0, 1_000_000, 1_000_000_000);
        assert!(eta.remaining_s(63_000).is_some());
        // No event for 30 s by the interface's clock: not known any more.
        assert!(eta.remaining_s(92_000).is_some());
        assert_eq!(eta.remaining_s(93_000), None);
        // Events that say nothing moved don't count as moving.
        eta.sample(61_000, done, 1_000_000_000, 0, 64_000);
        assert_eq!(eta.remaining_s(94_000), None);
        // It moves again: told again.
        eta.sample(100_000, done + 1_000_000, 1_000_000_000, 0, 103_000);
        assert!(eta.remaining_s(103_000).is_some());
    }

    #[test]
    fn a_new_run_starts_over() {
        let mut eta = Eta::default();
        let done = run(&mut eta, 0, 60, 0, 1_000_000, 1_000_000_000);
        assert!(eta.remaining_s(63_000).is_some());
        // Continued after a stop: the run's clock starts again, from the work already done.
        eta.sample(500, done, 1_000_000_000, done, 500_000);
        assert_eq!(eta.remaining_s(500_000), None);
        for t in 1..=30u64 {
            eta.sample(t * 1000, done + t * 2_000_000, 1_000_000_000, done, 500_000 + t * 1000);
        }
        let truth = (1_000_000_000 - (done + 60_000_000)) as f64 / 2_000_000.0;
        assert!(close(eta.remaining_s(530_000), truth, 0.01), "the earlier run's rate is forgotten");
    }

    #[test]
    fn a_new_run_whose_clock_is_ahead_still_starts_over() {
        // A short first run, then one that goes on from half way and whose first progress comes
        // later in its run than the first run's last did.
        let total = 1_000_000;
        let mut eta = Eta::default();
        eta.sample(900, 50, total, 0, 900);
        let mut done = 500_000;
        for t in 0..40u64 {
            done += 1000;
            eta.sample(1500 + t * 1000, done, total, 500_000, 10_000 + t * 1000);
        }
        let truth = (total - done) as f64 / 1000.0;
        assert!(close(eta.remaining_s(49_000), truth, 0.10), "{:?} vs {truth}", eta.remaining_s(49_000));
        // Work that goes back is a new run too, whatever it says it began from.
        let at = eta.done;
        eta.sample(50_000, at / 2, total, 500_000, 50_000);
        assert_eq!(eta.done, at / 2);
        assert_eq!(eta.remaining_s(50_000), None, "nothing known about the new run yet");
    }

    #[test]
    fn odd_numbers_never_give_odd_answers() {
        // More done than the total, no rate at all, totals at the limit.
        let mut eta = Eta::default();
        run(&mut eta, 0, 60, 0, 1_000_000, 10_000_000);
        assert_eq!(eta.remaining_s(63_000), Some(1), "past the total: at least a second, never negative");
        let mut eta = Eta::default();
        for t in 1..60 {
            eta.sample(t * 1000, 5_000_000, 10_000_000, 5_000_000, t * 1000);
        }
        assert_eq!(eta.remaining_s(60_000), None, "nothing moved this run: no rate");
        let mut eta = Eta::default();
        run(&mut eta, 0, 60, u64::MAX / 2, 1, u64::MAX);
        assert!(eta.remaining_s(63_000).is_none_or(|s| (1..=MAX_S).contains(&s)));
    }
}
