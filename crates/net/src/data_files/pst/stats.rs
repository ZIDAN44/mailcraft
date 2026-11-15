//! Where an import's time goes, and how much it holds and writes: the numbers it measures of
//! itself, summed over its threads.

use super::*;

/// Where an import's time went, and how much it held and wrote: numbers only, for measuring.
/// Times are summed over the threads that did the work; with one lane they all run on one, and
/// nothing waits for another.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportStats {
    /// The whole import, from opening the file to the seal at its end.
    pub total_ms: u64,
    /// Opening the file and finding its named properties, then opening each folder (which loads
    /// its tables of subfolders and items).
    pub plan_ms: u64,
    /// Reading items (their properties, recipients and attachments) from the file, summed over the
    /// lanes (so more than the import's own time with more than one).
    pub read_ms: u64,
    /// Rebuilding them as messages, with their summaries, and as contacts and events.
    pub build_ms: u64,
    /// Compressing the sources for the data file.
    pub pack_ms: u64,
    /// Writing the sources.
    pub write_ms: u64,
    /// Writing the folder lists, the folders, the contacts and events, and sealing the file.
    pub commit_ms: u64,
    /// The writer waiting for items to be read, and readers waiting for room in the budget (and
    /// how many times one waited, however briefly).
    pub writer_idle_ms: u64,
    pub budget_wait_ms: u64,
    pub budget_waits: u64,
    /// Taking the messages' text and words out for search (on the lanes), and putting them in the
    /// account's search index and writing it (on a thread of its own), summed: mostly alongside
    /// the import, not added to it (that's measured against an import without the index). And
    /// the messages put in it and written.
    pub index_ms: u64,
    pub indexed: u64,
    /// The most source bytes read and not yet written at once (the batch waiting for the disk
    /// with the one just added), counted as rebuilt, before compression: the size a budget
    /// charges, so later figures compare with this one.
    pub peak_budget_bytes: u64,
    /// The sources' bytes as rebuilt, and as kept in the cache.
    pub raw_bytes: u64,
    pub kept_bytes: u64,
    /// Items read at once.
    pub lanes: u64,
    /// Bytes read from the data file, in so many reads.
    pub read_bytes: u64,
    pub reads: u64,
    /// Sent items that came in with the id the replies to them answer, and those given one a
    /// message before them had, which kept the id made up for them (`pst/out/sent_ids.rs`): this
    /// run's.
    pub ids_given: u64,
    pub ids_taken_back: u64,
    /// Batches of sources written without a checkpoint, and the time they took; and checkpoints
    /// written (each durable and sealed, most with a batch of sources), and the time they took,
    /// their batch's included: what checkpoints cost, by the two.
    pub batches: u64,
    pub batch_ms: u64,
    pub checkpoints: u64,
    pub checkpoint_ms: u64,
}

/// A stage of an import, timed.
#[derive(Clone, Copy)]
pub(super) enum Phase {
    Plan,
    Read,
    Build,
    Pack,
    Write,
    Commit,
    /// The writer waiting for something to apply.
    Idle,
    /// Lanes waiting for room in the budget.
    Wait,
    /// Making messages' words for search, and writing them.
    Index,
}

/// An import's running counts, which any of its threads may add to.
#[derive(Default)]
pub(super) struct Tally {
    /// Nanoseconds in each [`Phase`].
    nanos: [AtomicU64; 9],
    /// Writes without a checkpoint and with one: how many, and their nanoseconds.
    batches: [AtomicU64; 2],
    checkpoints: [AtomicU64; 2],
    /// Times a lane waited for room.
    waits: AtomicU64,
    peak: AtomicU64,
    raw: AtomicU64,
    kept: AtomicU64,
    read_bytes: AtomicU64,
    reads: AtomicU64,
}

impl Tally {
    /// `f`, its time counted to `phase`.
    pub(super) fn time<T>(&self, phase: Phase, f: impl FnOnce() -> T) -> T {
        let began = Instant::now();
        let r = f();
        self.since(phase, began);
        r
    }

    /// The time since `began`, counted to `phase`.
    pub(super) fn since(&self, phase: Phase, began: Instant) {
        if let Some(n) = self.nanos.get(phase as usize) {
            n.fetch_add(nanos(began.elapsed()), Ordering::Relaxed);
        }
    }

    /// A write since `began`: a batch of sources, or a checkpoint (with its batch, if any).
    pub(super) fn wrote(&self, began: Instant, checkpoint: bool) {
        let [count, time] = if checkpoint { &self.checkpoints } else { &self.batches };
        count.fetch_add(1, Ordering::Relaxed);
        time.fetch_add(nanos(began.elapsed()), Ordering::Relaxed);
    }

    /// The writer waited since `began`.
    pub(super) fn idle(&self, began: Instant) {
        self.since(Phase::Idle, began);
    }

    /// A lane waited for room since `began`.
    pub(super) fn waited(&self, began: Instant) {
        self.waits.fetch_add(1, Ordering::Relaxed);
        self.since(Phase::Wait, began);
    }

    /// A source of `raw` bytes kept in `kept`, with `held` bytes (as rebuilt) now waiting to be written.
    pub(super) fn source(&self, raw: usize, kept: usize, held: usize) {
        let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        self.raw.fetch_add(wide(raw), Ordering::Relaxed);
        self.kept.fetch_add(wide(kept), Ordering::Relaxed);
        self.peak.fetch_max(wide(held), Ordering::Relaxed);
    }

    /// What was read of the data file through `source`.
    pub(super) fn reading(&self, source: &Shared) {
        if let Ok(s) = source.try_borrow() {
            self.read_bytes.fetch_add(s.bytes_read, Ordering::Relaxed);
            self.reads.fetch_add(s.reads, Ordering::Relaxed);
        }
    }

    pub(super) fn snapshot(&self, total: Duration, lanes: u64) -> ImportStats {
        let ms = |phase: Phase| self.nanos.get(phase as usize).map_or(0, |n| n.load(Ordering::Relaxed) / 1_000_000);
        ImportStats {
            total_ms: nanos(total) / 1_000_000,
            plan_ms: ms(Phase::Plan),
            read_ms: ms(Phase::Read),
            build_ms: ms(Phase::Build),
            pack_ms: ms(Phase::Pack),
            write_ms: ms(Phase::Write),
            commit_ms: ms(Phase::Commit),
            writer_idle_ms: ms(Phase::Idle),
            budget_wait_ms: ms(Phase::Wait),
            budget_waits: self.waits.load(Ordering::Relaxed),
            index_ms: ms(Phase::Index),
            indexed: 0,
            peak_budget_bytes: self.peak.load(Ordering::Relaxed),
            raw_bytes: self.raw.load(Ordering::Relaxed),
            kept_bytes: self.kept.load(Ordering::Relaxed),
            lanes,
            read_bytes: self.read_bytes.load(Ordering::Relaxed),
            reads: self.reads.load(Ordering::Relaxed),
            ids_given: 0,
            ids_taken_back: 0,
            batches: self.batches[0].load(Ordering::Relaxed),
            batch_ms: self.batches[1].load(Ordering::Relaxed) / 1_000_000,
            checkpoints: self.checkpoints[0].load(Ordering::Relaxed),
            checkpoint_ms: self.checkpoints[1].load(Ordering::Relaxed) / 1_000_000,
        }
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}
