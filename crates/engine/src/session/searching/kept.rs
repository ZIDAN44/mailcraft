//! The session: the folder's list on view when a search started, kept to be shown again when
//! the search is cleared. Building it again copied each of the folder's summaries, filtered and
//! arranged them and worked out their conversations and rows: on the 10,941-message folder of a
//! real file, 18–36 ms of the frame that cleared the search.

use super::*;

/// What a list was built from besides the cached mail: shown again only while all of it is the
/// same.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ListFrom {
    view: View,
    filter: ListFilter,
    arrange: Arrange,
    reverse: bool,
    conversations: bool,
    /// Focused or Other, when the list is split.
    focus: Option<bool>,
    /// The address book (Focused is worked out from it).
    contacts: u64,
    /// [`SearchState::changes`] then.
    changes: u64,
}

/// A list kept, as it was shown.
pub(crate) struct KeptList {
    from: ListFrom,
    list: Vec<SharedSummary>,
    threads: Vec<Thread>,
    nav: mailboxes::ListNav,
}

impl Session {
    /// The list kept while a search shows (for the memory it holds, `bench::big_mailbox`).
    #[cfg(test)]
    pub(crate) fn kept_summaries(&self) -> &[SharedSummary] {
        self.search_state.kept.as_ref().map(|k| k.list.as_slice()).unwrap_or_default()
    }

    /// What the list on view is built from now (`None`: the Outbox's, nothing to keep).
    fn list_from(&self) -> Option<ListFrom> {
        if matches!(self.view, View::Outbox | View::Empty) {
            return None;
        }
        Some(ListFrom {
            view: self.view.clone(),
            filter: self.filter,
            arrange: self.prefs.arrange,
            reverse: self.prefs.reverse_sort,
            conversations: self.prefs.conversations,
            focus: self.focus_split().then_some(self.focus_other),
            contacts: self.contacts_built,
            changes: self.search_state.changes,
        })
    }

    /// Cached mail changed: a list kept is built again rather than shown.
    pub(crate) fn mail_changed(&mut self) {
        self.search_state.changes = self.search_state.changes.wrapping_add(1);
    }

    /// A search starts over the folder's list: keep it (with its conversations and rows) for
    /// when the search is cleared. Not one that shows the message being read only because it
    /// is (filtered out otherwise): built again, it wouldn't.
    pub(crate) fn keep_list(&mut self) {
        if let Some(old) = self.search_state.kept.take() {
            self.let_go(old.list);
        }
        let Some(from) = self.list_from() else { return };
        if self.filter != ListFilter::All && self.reading.is_some() {
            return;
        }
        let list = std::mem::take(&mut self.list);
        let threads = std::mem::take(&mut self.threads);
        let structure = self.nav.structure;
        let mut nav = std::mem::take(&mut self.nav);
        // The list built for the search tells it changed by its structure.
        self.nav.structure = structure;
        nav.structure = structure;
        self.search_state.kept = Some(KeptList { from, list, threads, nav });
    }

    /// Show the list kept from before the search, when nothing it was built from changed since
    /// (a list kept that's out of date is let go of). Whether it's shown.
    pub(crate) fn show_kept_list(&mut self) -> bool {
        let Some(kept) = self.search_state.kept.take() else { return false };
        if self.list_from().as_ref() != Some(&kept.from) {
            self.let_go(kept.list);
            return false;
        }
        let shown = std::mem::replace(&mut self.list, kept.list);
        self.let_go(shown);
        self.threads = kept.threads;
        let structure = self.nav.structure;
        self.nav = kept.nav;
        self.nav.structure = structure.wrapping_add(1);
        // Its date headings, when the day turned meanwhile.
        if mailcraft_model::dates::today(&self.clock) != self.nav.day {
            self.rebuild_rows();
        }
        self.list_taken();
        true
    }
}
