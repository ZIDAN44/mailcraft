//! Search: what a run found merged into the list shown, by date, and its rows with it.
//!
//! The list a search streams into is newest first, a row a message under its day's heading (no
//! conversations, no pinned mail, arranged by date). Merging what a step found into it built its
//! rows again from the summaries, and asked each listed one when it came: each of the 47,387 a
//! one-letter word finds is a summary of its own in memory, and reading them all, twice, was most
//! of the 3–7 ms of the step that showed them. When each message came is kept with the list
//! ([`Shown`]), and the rows it had are merged with the new ones', so a merge reads no summary
//! but those it adds. The buffers a merge leaves are the next one's: made afresh, the megabytes
//! of a list of tens of thousands and its rows were memory met for the first time, every merge.

use std::cmp::Ordering;

use super::super::mailboxes::{NO_ROW, key_hash};
use super::run::SearchRun;
use super::*;

/// [`date_order`] of two messages that came at `a` and `b` (`Equal`: at the same time, their
/// keys tell).
pub(super) fn when_order(a: i64, b: i64, reverse: bool) -> Ordering {
    if reverse { a.cmp(&b) } else { b.cmp(&a) }
}

/// The order the list is in by date (the order a search streams in), as `arrange_list` makes it.
pub(super) fn date_order(a: &Summary, b: &Summary, reverse: bool) -> Ordering {
    let newest = b.when().cmp(&a.when()).then_with(|| b.key.cmp(&a.key));
    if reverse { a.when().cmp(&b.when()).then(newest) } else { newest }
}

/// What a run keeps of the list it showed: when each of its messages came, in its order, good
/// while the list's rows weren't built again since (`structure`, [`Session::nav_structure`];
/// nothing that keeps them changes when a message came: a flag marked patches the list in
/// place); and the buffers the last merge left.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    structure: u64,
    whens: Vec<i64>,
    spare: Spare,
}

/// Buffers a merge left (emptied), for the next to fill.
#[derive(Debug, Default)]
struct Spare {
    list: Vec<SharedSummary>,
    hashes: Vec<u64>,
    whens: Vec<i64>,
    rows: Vec<Row>,
    msg_rows: Vec<u32>,
    of_list: Vec<u32>,
}

/// `v` emptied, with room for `n`.
fn reuse<T>(mut v: Vec<T>, n: usize) -> Vec<T> {
    v.clear();
    v.reserve(n);
    v
}

/// What a merge made: the list, each key's hash and when each came, and its rows, message rows
/// and each message's row (`None`: built again from the list); and the buffers it left.
pub(super) struct Merged {
    pub(super) list: Vec<SharedSummary>,
    pub(super) hashes: Vec<u64>,
    whens: Vec<i64>,
    pub(super) rows: Option<(Vec<Row>, Vec<u32>, Vec<u32>)>,
    spare: Spare,
}

/// The rows of the list as shown, read along as their messages are merged.
struct OldRows {
    rows: Vec<Row>,
    at: usize,
    heading: &'static str,
    /// Each was a message's own row under a date heading, in the list's order.
    ok: bool,
}

impl OldRows {
    /// The heading the list's message `i` (the next) was under, and its unread count.
    fn next(&mut self, i: usize) -> (&'static str, usize) {
        while self.ok {
            let Some(row) = self.rows.get(self.at) else { break };
            self.at = self.at.saturating_add(1);
            match row {
                Row::Header(h) => self.heading = h,
                Row::Message { index, count: 1, unread } if *index == i => return (self.heading, *unread),
                _ => break,
            }
        }
        self.ok = false;
        (self.heading, 0)
    }

    /// Every message's row was read, and only headings are left.
    fn all_read(&self) -> bool {
        self.ok && self.rows.get(self.at..).is_some_and(|rest| rest.iter().all(|r| matches!(r, Row::Header(_))))
    }
}

/// The rows as they're merged: a heading where the day's changes, a row a message.
struct NewRows {
    rows: Vec<Row>,
    msg_rows: Vec<u32>,
    last: Option<&'static str>,
}

impl NewRows {
    fn push(&mut self, heading: &'static str, index: usize, unread: usize) {
        if self.last != Some(heading) {
            self.rows.push(Row::Header(heading));
            self.last = Some(heading);
        }
        self.msg_rows.push(u32::try_from(self.rows.len()).unwrap_or(NO_ROW));
        self.rows.push(Row::Message { index, count: 1, unread });
    }
}

/// The date heading of a time, the last day asked kept (messages come a day after another).
struct Headings {
    clock: Clock,
    today: i64,
    last: Option<(i64, &'static str)>,
}

impl Headings {
    fn of(&mut self, when: i64) -> &'static str {
        let day = self.clock.local_day(when);
        match self.last {
            Some((d, label)) if d == day => label,
            _ => {
                let label = DateGroup::of_day(day, self.today).label();
                self.last = Some((day, label));
                label
            }
        }
    }
}

impl Session {
    /// The rows shown, to merge into, when the list's are as a search streaming builds them (and
    /// still today's): taken from the list's nav (its message rows and each message's row into
    /// `spare`), which is built again with the merged list.
    fn rows_to_merge(&mut self, len: usize, spare: &mut Spare) -> Option<OldRows> {
        let plain = self.prefs.arrange == Arrange::Date && self.threads.is_empty() && self.search.is_some();
        let today = mailcraft_model::dates::today(&self.clock);
        if !plain || self.nav.day != today || self.nav.msg_rows.len() != len {
            return None;
        }
        spare.msg_rows = std::mem::take(&mut self.nav.msg_rows);
        spare.of_list = std::mem::take(&mut self.nav.msg_pos_of_list);
        Some(OldRows { rows: std::mem::take(&mut self.nav.rows), at: 0, heading: "", ok: true })
    }

    /// Merge `fresh` (when each came, its key's hash and it, sorted by date as the list is) into
    /// the list shown, less `gone`: the list's index and rows to be built with what it gives.
    pub(super) fn merge_shown(&mut self, run: &mut SearchRun, fresh: Vec<(i64, u64, SharedSummary)>, gone: &HashSet<MessageKey>) -> Merged {
        let reverse = self.prefs.reverse_sort;
        // The hashes the list's index was built of, kept with the messages they're of: only
        // what's new is hashed (each key hashed again was half of building the index again).
        let kept = self.take_hashes().unwrap_or_default();
        let known = kept.len() == self.list.len();
        let Shown { structure, whens, mut spare } = std::mem::take(&mut run.shown);
        let whens = if structure == self.nav.structure && whens.len() == self.list.len() { whens } else { Vec::new() };
        let mut old_rows = if known { self.rows_to_merge(self.list.len(), &mut spare) } else { None };
        let mut old = std::mem::take(&mut self.list);
        let gone_hashes: HashSet<u64> = gone.iter().map(key_hash).collect();
        let n = old.len().saturating_add(fresh.len());
        let mut list = reuse(std::mem::take(&mut spare.list), n);
        let mut hashes = reuse(std::mem::take(&mut spare.hashes), n);
        let mut new_whens = reuse(std::mem::take(&mut spare.whens), n);
        // A row a message, and a heading for each of what's new at most.
        let headed = old_rows.as_ref().map_or(0, |r| r.rows.len()).saturating_add(fresh.len().saturating_mul(2));
        let mut rows =
            NewRows { rows: reuse(std::mem::take(&mut spare.rows), headed), msg_rows: reuse(std::mem::take(&mut spare.msg_rows), n), last: None };
        let mut headings = Headings { clock: self.clock, today: mailcraft_model::dates::today(&self.clock), last: None };
        let mut fresh = fresh.into_iter().peekable();
        for (i, m) in old.drain(..).enumerate() {
            let h = kept.get(i).copied().filter(|_| known).unwrap_or_else(|| key_hash(&m.key));
            let (heading, unread) = old_rows.as_mut().map_or(("", 0), |r| r.next(i));
            if !gone_hashes.is_empty() && gone_hashes.contains(&h) && gone.contains(&m.key) {
                continue;
            }
            let when = whens.get(i).copied().unwrap_or_else(|| m.when());
            while let Some((fw, fh, f)) =
                fresh.next_if(|(fw, _, f)| when_order(*fw, when, reverse).then_with(|| date_order(f, &m, reverse)) == Ordering::Less)
            {
                if old_rows.is_some() {
                    rows.push(headings.of(fw), list.len(), usize::from(!f.flags.seen));
                }
                list.push(f);
                hashes.push(fh);
                new_whens.push(fw);
            }
            if old_rows.is_some() {
                rows.push(heading, list.len(), unread);
            }
            list.push(m);
            hashes.push(h);
            new_whens.push(when);
        }
        for (fw, fh, f) in fresh {
            if old_rows.is_some() {
                rows.push(headings.of(fw), list.len(), usize::from(!f.flags.seen));
            }
            list.push(f);
            hashes.push(fh);
            new_whens.push(fw);
        }
        let merged = old_rows.as_ref().is_some_and(OldRows::all_read);
        // What this merge read from is the next one's to fill.
        spare.list = old;
        spare.hashes = kept;
        spare.whens = whens;
        if let Some(r) = old_rows {
            spare.rows = r.rows;
        }
        let rows = if merged {
            // Each message on its own row, in the list's order.
            let mut of_list = reuse(std::mem::take(&mut spare.of_list), list.len());
            of_list.extend((0..list.len()).map(|i| u32::try_from(i).unwrap_or(NO_ROW)));
            Some((rows.rows, rows.msg_rows, of_list))
        } else {
            None
        };
        Merged { list, hashes, whens: new_whens, rows, spare }
    }

    /// Show what [`Session::merge_shown`] made, keeping when each came for the next merge.
    pub(super) fn take_merged(&mut self, run: &mut SearchRun, merged: Merged) {
        let Merged { list, hashes, whens, rows, spare } = merged;
        match rows {
            Some(rows) => {
                #[cfg(test)]
                {
                    self.search_state.rows_merged = self.search_state.rows_merged.saturating_add(1);
                }
                self.take_list_rows(list, hashes, rows);
            }
            None => self.take_list_with(list, hashes, false),
        }
        run.shown = Shown { structure: self.nav.structure, whens, spare };
    }
}
