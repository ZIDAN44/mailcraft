//! How far a long piece of background work has come (a data file's import, and later the search
//! index's build and a data file's compaction): one shape for all of them, so the status bar
//! shows each the same way.

use serde::{Deserialize, Serialize};

/// Items and bytes done of those known to be coming; a total of 0 is not known (an
/// indeterminate bar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Work {
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

impl Work {
    /// The work done, each item counting as `weight` bytes besides its own (an item costs time
    /// however small it is). Saturates instead of wrapping.
    pub fn weighted_done(&self, weight: u64) -> u64 {
        self.bytes_done.saturating_add(self.done.saturating_mul(weight))
    }

    /// The work there is, weighed as [`Work::weighted_done`].
    pub fn weighted_total(&self, weight: u64) -> u64 {
        self.bytes_total.saturating_add(self.total.saturating_mul(weight))
    }

    /// Thousandths done, weighed as [`Work::weighted_done`]: see [`permille`].
    pub fn permille(&self, weight: u64) -> Option<u16> {
        permille(self.weighted_done(weight), self.weighted_total(weight))
    }
}

/// Thousandths of `total` that `done` is: 0..=999 (finished is said by the work's own end, never
/// by a full bar while the last item is still being written); `None` when there's no total.
/// Worked out in `u128`, so no total overflows it.
pub fn permille(done: u64, total: u64) -> Option<u16> {
    if total == 0 {
        return None;
    }
    let p = u128::from(done).saturating_mul(1000) / u128::from(total);
    Some(u16::try_from(p.min(999)).unwrap_or(999))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousandths_never_overflow_or_reach_the_end() {
        assert_eq!(permille(0, 0), None, "no total: indeterminate");
        assert_eq!(permille(1, 2), Some(500));
        assert_eq!(permille(2, 2), Some(999), "the end is said by the work, not the bar");
        assert_eq!(permille(5, 2), Some(999), "more done than counted");
        assert_eq!(permille(u64::MAX, u64::MAX), Some(999));
        assert_eq!(permille(u64::MAX / 2, u64::MAX), Some(499));
        let w = Work { done: u64::MAX, total: u64::MAX, bytes_done: u64::MAX, bytes_total: u64::MAX };
        assert_eq!((w.weighted_done(128), w.weighted_total(128)), (u64::MAX, u64::MAX), "saturated, not wrapped");
        assert_eq!(w.permille(128), Some(999));
        let w = Work { done: 1, total: 4, bytes_done: 0, bytes_total: 0 };
        assert_eq!(w.permille(10), Some(250));
    }
}
