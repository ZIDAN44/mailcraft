//! The work the passes over hostile text do (a message's HTML, an invitation's notes, a calendar,
//! a contact card), counted in steps (bytes looked through, mostly), so that a test can hold a
//! pass to work in proportion to its input whatever the machine's load. Timed, a test depended on
//! the load (many builds share a machine) and only caught a pass a hundred times slower; one that
//! looks through its input again from each of thousands of marks takes thousands of times the
//! steps. A pass over hostile input counts what it looks through: its searches through
//! [`scanned`], the rest with [`steps`]. Counted in this crate's tests and with the `count-work`
//! feature (the other crates' tests turn it on); elsewhere these do nothing.

/// `n` more steps.
#[inline]
pub fn steps(n: usize) {
    #[cfg(any(test, feature = "count-work"))]
    tally::add(n);
    #[cfg(not(any(test, feature = "count-work")))]
    let _ = n;
}

/// What a search through `hay` found (where, or `None`), the bytes it looked through counted as
/// steps: up to what it found, or all of `hay`.
#[inline]
pub fn scanned(hay: impl AsRef<[u8]>, found: Option<usize>) -> Option<usize> {
    steps(found.map_or(hay.as_ref().len(), |at| at.saturating_add(1)));
    found
}

#[cfg(any(test, feature = "count-work"))]
pub use tally::linear;

#[cfg(any(test, feature = "count-work"))]
mod tally {
    use std::cell::Cell;

    /// Steps a pass may take for each byte of its input ([`linear`]): a message is read through
    /// a few dozen times at most (each of its passes, a byte looked at by a search or two in
    /// each), where one that looks through it again for each of its tags takes thousands.
    const PER_BYTE: u64 = 64;
    /// Steps a pass may take whatever its input: what it reads once (a short message's style
    /// sheet, its parts).
    const BASE: u64 = 1 << 16;

    thread_local! {
        /// Steps taken on this thread (each test runs on one of its own).
        static TAKEN: Cell<u64> = const { Cell::new(0) };
        /// Where a pass held to [`linear`] stops.
        static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    }

    pub(super) fn add(n: usize) {
        let taken = TAKEN.with(|t| {
            let taken = t.get().saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
            t.set(taken);
            taken
        });
        // Stopped at once rather than at the end: out of proportion, a megabyte would take minutes.
        let limit = LIMIT.with(Cell::get);
        assert!(taken <= limit, "past {limit} steps: work out of proportion to the input");
    }

    /// `f`, run on an input `len` bytes long, held to work in proportion to it: it fails (a panic)
    /// as soon as it takes more than [`PER_BYTE`] steps for each byte, and [`BASE`].
    pub fn linear<T>(len: usize, f: impl FnOnce() -> T) -> T {
        let allowed = u64::try_from(len).unwrap_or(u64::MAX).saturating_mul(PER_BYTE).saturating_add(BASE);
        let start = TAKEN.with(Cell::get);
        let outer = LIMIT.with(|l| l.replace(start.saturating_add(allowed)));
        let out = f();
        LIMIT.with(|l| l.set(outer));
        // A pass stopped inside a `catch_unwind` (the layout's, the text converter's) went on: its
        // steps are counted still.
        let took = TAKEN.with(Cell::get).saturating_sub(start);
        assert!(took <= allowed, "{took} steps for {len} bytes: work out of proportion to the input");
        out
    }
}
