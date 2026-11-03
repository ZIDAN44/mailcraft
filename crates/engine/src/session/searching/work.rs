//! Search: a step of a run, and what each part of it does ([`SearchWork`]), for the window to
//! time.
//!
//! A frame of 2.9 s was once seen inside a step of a search typed into the box, which runs units
//! for 6 ms a frame: one unit didn't stop. Each part of a step now says what it was as it ends,
//! and the window logs one that takes longer than a frame should, by its kind.

use std::cell::Cell;
use std::time::Duration;

use super::*;

/// What a part of a search's step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchWork {
    /// The indexes the run reads read again, where a pass has written to them.
    FollowIndexes,
    /// What's found merged into the list shown.
    Show,
    /// A page of a folder's list read.
    ListPage,
    /// An account's index opened.
    OpenIndex,
    /// Pages of an index looked through for a word.
    LookUp,
    /// Summaries checked.
    Summaries,
    /// A message's kept text read, or a slice of it checked.
    Verify,
    /// A message's source read (a search that waits).
    Source,
    /// Mail waiting for a reply worked out.
    Awaiting,
    /// A step of putting the list together at the end.
    Finish,
}

impl SearchWork {
    /// Its name, for the log.
    pub fn name(self) -> &'static str {
        match self {
            SearchWork::FollowIndexes => "follow the indexes",
            SearchWork::Show => "show what's found",
            SearchWork::ListPage => "a page of a folder's list",
            SearchWork::OpenIndex => "open an index",
            SearchWork::LookUp => "look up a word",
            SearchWork::Summaries => "check summaries",
            SearchWork::Verify => "check a kept text",
            SearchWork::Source => "read a source",
            SearchWork::Awaiting => "mail awaiting a reply",
            SearchWork::Finish => "put the list together",
        }
    }
}

impl Session {
    /// Do at most `units` units of the search's work: no clock, no network, each unit bounded
    /// (see the module's notes). What's found is in the list when it returns.
    pub fn search_step(&mut self, units: u32) -> SearchProgress {
        self.search_step_while(units, &mut || true)
    }

    /// [`Session::search_step`], asking `more` after each unit whether to go on (the window
    /// asks its clock: the engine reads none). What's found is shown once, at the end.
    pub fn search_step_while(&mut self, units: u32, more: &mut dyn FnMut() -> bool) -> SearchProgress {
        self.search_step_timed(units, more, &mut |_| {})
    }

    /// [`Session::search_step_timed`] within `budget` of a frame, `elapsed` telling how long the
    /// frame has run so far (the window's clock: the engine reads none): a unit is begun only
    /// while one as long as the longest of the frame's so far still fits in it. Units differ (a
    /// look-up of two index pages took up to 3.5 ms on a real file, most well under 1 ms): asked
    /// only whether the budget was spent, a long one begun near its end held a frame to 8.5 ms.
    /// The step that first shows what the run found does so at its start, its units then within
    /// a third of the budget: the window lays out a list of new rows in that frame (2.3–4.6 ms
    /// on a real file), and shown after a whole budget's units it held the frame to 8.9–11.3 ms.
    pub fn search_step_within(
        &mut self,
        units: u32,
        budget: Duration,
        elapsed: &mut dyn FnMut() -> Duration,
        done: &mut dyn FnMut(SearchWork),
    ) -> SearchProgress {
        let mut last = elapsed();
        let mut longest = Duration::ZERO;
        let first_shown = Cell::new(false);
        let mut more = || {
            let now = elapsed();
            longest = longest.max(now.saturating_sub(last));
            last = now;
            let budget = if first_shown.get() { budget / 3 } else { budget };
            now.saturating_add(longest) < budget
        };
        self.step_showing(units, &mut more, done, &first_shown)
    }

    /// [`Session::search_step_while`], telling `done` what each part of the step was as it ends
    /// (its kind of unit, or the indexes followed and what's found shown around them), so the
    /// window can time each against its clock.
    pub fn search_step_timed(&mut self, units: u32, more: &mut dyn FnMut() -> bool, done: &mut dyn FnMut(SearchWork)) -> SearchProgress {
        self.step_showing(units, more, done, &Cell::new(false))
    }

    /// [`Session::search_step_timed`], `first_shown` set when the step's start first shows what
    /// the run found; what a run found before anything of it was shown waits for the next step's
    /// start when the step stopped for the frame's time.
    fn step_showing(
        &mut self,
        units: u32,
        more: &mut dyn FnMut() -> bool,
        done: &mut dyn FnMut(SearchWork),
        first_shown: &Cell<bool>,
    ) -> SearchProgress {
        self.let_go_some();
        let Some(mut run) = self.search_state.run.take() else { return SearchProgress { done: true, ..Default::default() } };
        // A data file it reads is had to itself by a compaction: it waits (on the window's
        // thread, nothing of that file is read meanwhile), and goes on once it's given back.
        if run.reads_any(|a| self.compaction.exclusive(a)) {
            let progress = run.progress(false);
            self.search_state.run = Some(run);
            return progress;
        }
        let before = self.revision;
        let went = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.follow_indexes(&mut run);
            done(SearchWork::FollowIndexes);
            // What earlier steps found, shown before this one's units (where the window's
            // clock counts it), taking as many of its units as its rows ask, all but one; then
            // what this one found, if that's cheap.
            let first = !run.shown_any;
            let shown = self.publish_due(&mut run, true).min(units.saturating_sub(1));
            first_shown.set(first && run.shown_any);
            self.search_state.stats.units = self.search_state.stats.units.saturating_add(u64::from(shown));
            done(SearchWork::Show);
            let out_of_time = self.run_units(&mut run, units.saturating_sub(shown), more, done);
            if run.shown_any || !out_of_time {
                self.publish_due(&mut run, false);
            }
            done(SearchWork::Show);
        }));
        if went.is_err() {
            // The search goes. What it was reading may be what's wrong: each index it read is
            // left unread (damaged) until its writer writes it again.
            log::error!("a search stopped: something it read couldn't be searched");
            PANICS_CAUGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let accounts: HashSet<&String> = run.folders.iter().map(|(a, _)| a).collect();
            for (account, slot) in &mut self.search_state.readers {
                if accounts.contains(account) {
                    *slot = ReaderSlot { reader: None, checked: self.clock.now, behind: false, damaged: true };
                }
            }
            self.search_state.readers.retain(|_, slot| slot.damaged);
            self.search_state.pinned.clear();
            return SearchProgress { done: true, ..Default::default() };
        }
        if run.done() {
            run.fresh.clear();
            run.gone.clear();
            // Shown in full: its folders may leave the cache (read again when it scans again).
            self.search_state.pinned.clear();
        } else if self.search_state.pinned.is_empty() {
            self.search_state.pinned = run.folders.iter().cloned().collect();
        }
        let progress = run.progress(self.revision != before);
        self.search_state.run = Some(run);
        progress
    }
}
