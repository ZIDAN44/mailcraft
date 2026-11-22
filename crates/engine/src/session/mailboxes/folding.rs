//! Headings of the message list folded away (issue #189), as Outlook's groups collapse: a
//! heading clicked hides its mail, which it counts, and shows it again clicked once more. What's
//! folded is kept for each folder and arrangement, between runs too; a search's list folds
//! nothing, nor does one shown without headings.

use std::collections::BTreeSet;

use super::rows::Row;
use super::*;

/// The most headings kept folded in one folder, and the most folders kept: a list arranged by
/// sender has a heading for each.
const FOLDED_MAX: usize = 500;
const PLACES_MAX: usize = 1000;

impl Session {
    /// Where the list's folded headings are kept: the folder (or the view of every account's)
    /// shown; none for a search's list, or one shown without headings.
    fn fold_place(&self) -> Option<String> {
        if self.search.is_some() || !self.prefs.groups {
            return None;
        }
        Some(match &self.view {
            View::Folder { account, folder } => format!("{account}\u{1}{folder}"),
            View::Unified { role } => format!("\u{1}{role:?}"),
            View::Outbox => "\u{1}outbox".into(),
            View::Empty => return None,
        })
    }

    /// The headings folded in the list shown.
    pub(super) fn folded_here(&self) -> Option<&BTreeSet<String>> {
        self.prefs.folded_groups.get(&self.fold_place()?)
    }

    /// What a heading is kept as when folded: the arrangement and its text (a name's apart from a
    /// fixed heading's, so a sender called "Today" is another).
    pub(super) fn heading_key(&self, row: &Row) -> Option<String> {
        let arrange = self.prefs.arrange.code();
        match row {
            Row::Header(label) => Some(format!("{arrange}\u{1}{label}")),
            Row::Group(name) => Some(format!("{arrange}\u{2}{name}")),
            _ => None,
        }
    }

    /// Whether the heading on row `r` of [`Session::rows`] is folded.
    pub fn heading_folded(&self, r: usize) -> bool {
        matches!(self.nav.rows.get(r.saturating_add(1)), Some(Row::Folded { .. }))
    }

    /// Fold the heading shown as `heading` (a date's, a sender's…), or show its mail again;
    /// `folded` says which (the other way when not given).
    pub fn fold_group(&mut self, heading: &str, folded: Option<bool>) -> CmdResult {
        let place = self.fold_place().ok_or_else(|| CmdError::Disabled("headings fold in a folder's list shown in groups".into()))?;
        let row = self
            .nav
            .rows
            .iter()
            .find(|r| matches!(r, Row::Header(h) if *h == heading) || matches!(r, Row::Group(g) if g == heading))
            .cloned()
            .ok_or_else(|| CmdError::Params(format!("no heading `{heading}` in the list")))?;
        let key = self.heading_key(&row).ok_or_else(|| CmdError::Params(format!("no heading `{heading}` in the list")))?;
        let set = self.prefs.folded_groups.get(&place);
        let fold = folded.unwrap_or_else(|| !set.is_some_and(|s| s.contains(&key)));
        if fold && set.is_some_and(|s| s.len() >= FOLDED_MAX && !s.contains(&key)) {
            return Err(CmdError::Disabled("too many headings are folded here: show some again first".into()));
        }
        self.keep_folded(place, std::iter::once(key), fold);
        Ok(serde_json::json!({ "heading": heading, "folded": fold }))
    }

    /// Fold every heading of the list shown (Outlook's Collapse All Groups), or show all their
    /// mail again.
    pub fn fold_all(&mut self, folded: bool) -> CmdResult {
        let place = self.fold_place().ok_or_else(|| CmdError::Disabled("headings fold in a folder's list shown in groups".into()))?;
        let keys: Vec<String> = self.nav.rows.iter().filter_map(|r| self.heading_key(r)).take(FOLDED_MAX).collect();
        let n = keys.len();
        self.keep_folded(place, keys.into_iter(), folded);
        Ok(serde_json::json!({ "headings": n, "folded": folded }))
    }

    /// Keep headings folded at a place, or not, and lay the list out again.
    fn keep_folded(&mut self, place: String, keys: impl Iterator<Item = String>, fold: bool) {
        let folds = &mut self.prefs.folded_groups;
        if fold && !folds.contains_key(&place) && folds.len() >= PLACES_MAX {
            folds.pop_first();
        }
        let set = folds.entry(place.clone()).or_default();
        for key in keys {
            if fold {
                set.insert(key);
            } else {
                set.remove(&key);
            }
        }
        while set.len() > FOLDED_MAX {
            set.pop_last();
        }
        if set.is_empty() {
            folds.remove(&place);
        }
        self.rebuild_rows();
        self.revision = self.revision.wrapping_add(1);
    }

    /// A folder renamed or moved (`to`), or deleted (none): what was folded in it follows it.
    pub(in crate::session) fn folds_follow_folder(&mut self, account: &str, from: &str, to: Option<&str>, d: Option<char>) {
        let prefix = format!("{account}\u{1}");
        let moved: Vec<String> = self
            .prefs
            .folded_groups
            .keys()
            .filter(|k| k.strip_prefix(&prefix).is_some_and(|f| mailcraft_model::is_within(f, from, d)))
            .cloned()
            .collect();
        for k in moved {
            let Some(set) = self.prefs.folded_groups.remove(&k) else { continue };
            let new = to.zip(k.strip_prefix(&prefix)).and_then(|(to, f)| mailcraft_model::rebase_path(f, from, to, d));
            if let Some(new) = new {
                self.prefs.folded_groups.insert(format!("{prefix}{new}"), set);
            }
        }
    }

    /// Where a message whose heading is folded would be among the message rows: the first row
    /// after its heading (`None` when it isn't under a folded one).
    pub(crate) fn folded_spot(&self, key: &MessageKey) -> Option<usize> {
        let i = self.index_of(key)?;
        let m = self.list.get(i)?;
        // Its conversation's, when the list shows them: pinned with any of it, dated by its newest.
        let thread = self.thread_at(i).filter(|_| self.prefs.conversations);
        let pinned = match thread {
            Some(t) => t.messages.iter().any(|&j| self.list.get(j).is_some_and(|m| m.flags.pinned())),
            None => m.flags.pinned(),
        };
        let heading = if self.search.is_none() && pinned {
            Row::Header(super::rows::PINNED_HEADING)
        } else {
            let date = thread.map_or(m.when(), |t| t.latest);
            let label = DateGroup::of_day(self.clock.local_day(date), self.clock.local_day(self.clock.now)).label();
            super::rows::heading_of(self.prefs.arrange, Some(m), || label)
        };
        let r = self.nav.rows.iter().position(|row| *row == heading).filter(|r| self.heading_folded(*r))?;
        let r = u32::try_from(r).ok()?;
        Some(self.nav.msg_rows.partition_point(|&mr| mr < r))
    }
}
