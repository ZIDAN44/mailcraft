//! Search: what a run found, into the list; and the run kept right as mail changes.
//!
//! While a run streams, what it finds is merged into the list as it stands (newest first, the
//! list's own index built again, its conversations left for the end); at the end the list is
//! put together in full ([`Session::search_list`]). A flag or category changed on mail is weighed
//! again for those messages alone ([`Session::search_patched`]): flags are never text, so what
//! their text said is kept. Mail coming into or leaving a folder has that folder scanned again;
//! folders appearing or going start the search again.

use super::found::RunKey;
use super::matching::{Index, Verdict, classify};
use super::merge::{date_order, when_order};
use super::run::SearchRun;
use super::*;

/// Most messages whose flags changed that are weighed again one by one; more (a select-all
/// marked read) and their folders are scanned again instead.
const REEVAL_MAX: usize = 512;

/// A list of at most this many messages takes what a step found at the step's end.
const SHOWN_AT_ONCE: usize = 4096;
/// A longer list is put together again (in time with its length) only every so many steps: one
/// a step for each this many messages it holds. What a step finds then shows at the start of a
/// later step, before its units, where the window's clock counts it. Shown at every step, a
/// common phrase found a few messages a step over tens of thousands built a list of tens of
/// thousands each time (a cold search of 50,000 messages took 14 s, nearly all of it that).
const ROWS_A_STEP: usize = 2048;
/// Rows of the list shown merged in about the time of a unit (a unit making 128 summaries' texts
/// took 0.15 ms; merging 12,000 found into 27,000 shown, 1.8 ms): what a step shows at its start
/// takes as many of its units. Taking none, the step that showed them went on to its 8 units
/// whatever they cost.
const ROWS_A_UNIT: usize = 4096;

impl Session {
    /// What the run found, as the folders hold it now (a folder let go of meanwhile: as the list
    /// shows it), with what the servers found for the same query, newest first.
    pub(crate) fn search_list(&mut self, run: &SearchRun) -> Vec<SharedSummary> {
        let mut by_folder: HashMap<usize, HashSet<u32>> = HashMap::new();
        for k in &run.found {
            by_folder.entry(k.folder()).or_default().insert(k.uid());
        }
        let mut found = Vec::with_capacity(run.found.len());
        for (fi, uids) in by_folder {
            let Some(k) = run.folders.get(fi) else { continue };
            match self.cache.get(k) {
                Some(st) => found.extend(st.messages.iter().filter(|m| uids.contains(&m.key.uid) && !m.flags.deleted).cloned()),
                None => found.extend(self.list.iter().filter(|m| m.key.account == k.0 && m.key.folder == k.1 && uids.contains(&m.key.uid)).cloned()),
            }
        }
        found.extend(self.server_found(run).into_iter().map(SharedSummary::new));
        // A Search Folder leaves out Deleted Items and Junk, as Outlook's.
        if self.open_search_folder_name().is_some() {
            found.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
        }
        sort_newest_first(&mut found);
        found
    }

    /// What the servers found for the run's query that it didn't find here: its words may be in
    /// mail not downloaded here.
    pub(super) fn server_found(&mut self, run: &SearchRun) -> Vec<Summary> {
        let q = run.q.clone();
        let hits: Vec<MessageKey> =
            self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter()).filter(|k| !run.has_found(k)).cloned().collect();
        // The servers can't tell dates or what's waiting for a reply: checked here.
        self.server_hits(&q, &hits, run.awaited())
    }

    /// Show what the run found and let go of since the list was last shown, when it's due: at a
    /// step's end (`start` false) only while the list is short, else at the start of a step once
    /// enough steps went by ([`ROWS_A_STEP`]). The units it took ([`ROWS_A_UNIT`]).
    pub(crate) fn publish_due(&mut self, run: &mut SearchRun, start: bool) -> u32 {
        // Nothing left to read: the run's end shows it (one list built, not two).
        if run.done() || run.wait || (run.fresh.is_empty() && run.gone.is_empty()) || (start && run.idle()) {
            return 0;
        }
        let shown = self.list.len();
        let due = shown.saturating_add(run.fresh.len()) <= SHOWN_AT_ONCE || (start && run.unshown_steps >= shown / ROWS_A_STEP);
        if due {
            self.publish_stream(run);
            run.unshown_steps = 0;
            return u32::try_from(self.list.len() / ROWS_A_UNIT).unwrap_or(u32::MAX);
        }
        if !start {
            run.unshown_steps = run.unshown_steps.saturating_add(1);
        }
        0
    }

    /// Show what the run found and let go of since the list was last shown: merged into the list
    /// by date, the list's index built again, its conversations left for the run's end.
    pub(crate) fn publish_stream(&mut self, run: &mut SearchRun) {
        let filter = self.filter;
        let reverse = self.prefs.reverse_sort;
        let gone: HashSet<MessageKey> = std::mem::take(&mut run.gone).into_iter().filter_map(|k| run.message_key(k)).collect();
        // Each with its key's hash, made as it was found: for the list's index, and to ask
        // whether it's listed; and when it came, to sort by without reading it again.
        let mut fresh: Vec<(i64, u64, SharedSummary)> = std::mem::take(&mut run.fresh)
            .into_iter()
            .filter(|f| self.filter_keeps(filter, &f.m) && !self.listed_hashed(&f.m.key, f.hash))
            .map(|f| (f.m.when(), f.hash, f.m))
            .collect();
        if self.open_search_folder_name().is_some() {
            fresh.retain(|(_, _, m)| {
                self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk))
            });
        }
        if gone.is_empty() && fresh.is_empty() {
            return;
        }
        fresh.sort_by(|a, b| when_order(a.0, b.0, reverse).then_with(|| date_order(&a.2, &b.2, reverse)));
        let merged = self.merge_shown(run, fresh, &gone);
        self.take_merged(run, merged);
    }

    /// Flags or categories changed on `keys` (in the cache already): weighed again against the
    /// search for those messages alone. `true` when what it finds is the same messages (the
    /// list can be patched as it stands); `false` when the list must be put together again.
    pub(crate) fn search_patched(&mut self, keys: &[MessageKey]) -> bool {
        let Some(mut run) = self.search_state.run.take() else { return false };
        let same = self.reeval(&mut run, keys);
        self.search_state.run = Some(run);
        same
    }

    fn reeval(&mut self, run: &mut SearchRun, keys: &[MessageKey]) -> bool {
        let mut by_folder: HashMap<usize, HashSet<u32>> = HashMap::new();
        for k in keys {
            if let Some(fi) = run.folders.iter().position(|(a, f)| *a == k.account && *f == k.folder) {
                by_folder.entry(fi).or_default().insert(k.uid);
            }
        }
        // Changed while the list is put together: gathered again, with the flags as they are.
        if !by_folder.is_empty() {
            run.gather_again();
        }
        if keys.len() > REEVAL_MAX {
            for fi in by_folder.into_keys() {
                run.rescan(fi);
            }
            return false;
        }
        let mut same = true;
        for (fi, uids) in by_folder {
            // A folder not scanned yet is weighed with its flags as they are when it is.
            if !run.scanned(fi) {
                continue;
            }
            let Some(k) = run.folders.get(fi).cloned() else { continue };
            let Some(st) = self.cache.get(&k) else { continue };
            let reader = self.search_state.readers.get(&k.0).and_then(|s| s.reader.as_ref());
            let index = reader.filter(|_| run.uses_index()).map(|reader| Index::of(reader, run.terms_of(&k.0), &k.1));
            let mut verdicts = Vec::new();
            for m in st.messages.iter().filter(|m| uids.contains(&m.key.uid)) {
                let text = SummaryText::for_query(m, &run.query);
                let awaiting = run.awaited().is_some_and(|a| a.contains(&m.key));
                let known = run.known.get(&RunKey::of(fi, &m.key));
                let v = if m.flags.deleted { Verdict::No } else { classify(&run.query, m, &text, awaiting, index, known) };
                verdicts.push((SharedSummary::clone(m), v, awaiting));
            }
            for (m, v, awaiting) in verdicts {
                let key = RunKey::of(fi, &m.key);
                let was = run.found.contains(&key);
                run.record(fi, m, v, awaiting, false);
                // Still to be read: the list is put together once it is.
                if run.found.contains(&key) != was || !run.idle() {
                    same = false;
                }
            }
        }
        // Nothing new to show: what the list shows of them is patched in place. What was found
        // and isn't shown yet (a long list takes it a few steps later, or at the end) stays to be
        // shown, with the flags as they are now (the copies it holds were taken before).
        if same && !run.fresh.is_empty() {
            let changed: HashSet<&MessageKey> = keys.iter().collect();
            let mut at: HashMap<&MessageKey, usize> = HashMap::new();
            let mut folders: HashSet<(String, String)> = HashSet::new();
            for (i, f) in run.fresh.iter().enumerate() {
                if changed.contains(&f.m.key) {
                    at.insert(&f.m.key, i);
                    folders.insert((f.m.key.account.clone(), f.m.key.folder.clone()));
                }
            }
            let mut now: Vec<(usize, SharedSummary)> = Vec::new();
            for k in &folders {
                let Some(st) = self.cache.get(k) else { continue };
                now.extend(st.messages.iter().filter_map(|m| at.get(&m.key).map(|i| (*i, SharedSummary::clone(m)))));
            }
            for (i, m) in now {
                if let Some(slot) = run.fresh.get_mut(i) {
                    slot.m = m;
                }
            }
        }
        same
    }

    /// A folder's mail changed (read again, mail moved in or out): its summaries' text goes, and
    /// a search reading it scans it again.
    pub(crate) fn search_folder_changed(&mut self, account: &str, folder: &str) {
        self.mail_changed();
        self.search_state.texts.forget(account, folder);
        if let Some(run) = self.search_state.run.as_mut()
            && let Some(fi) = run.folders.iter().position(|(a, f)| a == account && f == folder)
        {
            run.rescan(fi);
        }
    }

    /// An account's folders changed (created, renamed, deleted, listed again, the account
    /// gone): the texts of folders it no longer has go, and a search whose folders aren't the
    /// same any more starts again.
    pub(crate) fn search_folders_changed(&mut self, account: &str) {
        self.mail_changed();
        let listed: HashSet<String> = self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
        self.search_state.texts.retain(|a, f| a != account || listed.contains(f));
        self.search_state.folded.retain(|a, f| a != account || listed.contains(f));
        if self.account(account).is_none() {
            self.search_state.readers.remove(account);
            self.search_state.looked_up.forget(account);
        }
        let Some((q, scope, wait, folders)) = self.search_state.run.as_ref().map(|r| (r.q.clone(), r.scope, r.wait, r.folders.clone())) else {
            return;
        };
        let now = self.scope_folders(scope);
        let (a, b): (HashSet<_>, HashSet<_>) = (now.iter().collect(), folders.iter().collect());
        if a != b {
            self.start_search(&q, scope, wait);
        }
    }
}
