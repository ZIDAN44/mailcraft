//! Search: what a run found, into the list; and the run kept right as mail changes.
//!
//! While a run streams, what it finds is merged into the list as it stands (newest first, the
//! list's own index built again, its conversations left for the end); at the end the list is
//! put together in full ([`Session::search_list`]). A flag or category changed on mail is weighed
//! again for those messages alone ([`Session::search_patched`]): flags are never text, so what
//! their text said is kept. Mail coming into or leaving a folder has that folder scanned again;
//! folders appearing or going start the search again.

use std::cmp::Ordering;

use super::matching::{Index, Verdict, classify};
use super::run::SearchRun;
use super::*;

/// Most messages whose flags changed that are weighed again one by one; more (a select-all
/// marked read) and their folders are scanned again instead.
const REEVAL_MAX: usize = 512;

/// The order the list is in by date (the order a search streams in), as `arrange_list` makes it.
fn date_order(a: &Summary, b: &Summary, reverse: bool) -> Ordering {
    let newest = b.when().cmp(&a.when()).then_with(|| b.key.cmp(&a.key));
    if reverse { a.when().cmp(&b.when()).then(newest) } else { newest }
}

impl Session {
    /// What the run found, as the folders hold it now (a folder let go of meanwhile: as the list
    /// shows it), with what the servers found for the same query, newest first.
    pub(crate) fn search_list(&mut self, run: &SearchRun) -> Vec<Summary> {
        let mut by_folder: HashMap<(&str, &str), HashSet<u32>> = HashMap::new();
        for k in &run.found {
            by_folder.entry((k.account.as_str(), k.folder.as_str())).or_default().insert(k.uid);
        }
        let mut found = Vec::with_capacity(run.found.len());
        for ((a, f), uids) in by_folder {
            match self.cache.get(&(a.to_string(), f.to_string())) {
                Some(st) => found.extend(st.messages.iter().filter(|m| uids.contains(&m.key.uid) && !m.flags.deleted).cloned()),
                None => found.extend(self.list.iter().filter(|m| m.key.account == a && m.key.folder == f && uids.contains(&m.key.uid)).cloned()),
            }
        }
        // What the servers found: its words may be in mail not downloaded here.
        let q = run.q.clone();
        let have: HashSet<&MessageKey> = run.found.iter().collect();
        let hits: Vec<MessageKey> =
            self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter()).filter(|k| !have.contains(k)).cloned().collect();
        // The servers can't tell dates or what's waiting for a reply: checked here.
        let awaited = run.awaited();
        found.extend(self.server_hits(&q, &hits, awaited));
        // A Search Folder leaves out Deleted Items and Junk, as Outlook's.
        if self.open_search_folder_name().is_some() {
            found.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
        }
        sort_newest_first(&mut found);
        found
    }

    /// Show what the run found and let go of since the list was last shown: merged into the list
    /// by date, the list's index built again, its conversations left for the run's end.
    pub(crate) fn publish_stream(&mut self, run: &mut SearchRun) {
        let filter = self.filter;
        let reverse = self.prefs.reverse_sort;
        let gone = std::mem::take(&mut run.gone);
        let mut fresh: Vec<Summary> =
            std::mem::take(&mut run.fresh).into_iter().filter(|m| self.filter_keeps(filter, m) && self.index_of(&m.key).is_none()).collect();
        if self.open_search_folder_name().is_some() {
            fresh.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
        }
        if gone.is_empty() && fresh.is_empty() {
            return;
        }
        fresh.sort_by(|a, b| date_order(a, b, reverse));
        let old = std::mem::take(&mut self.list);
        let mut list = Vec::with_capacity(old.len() + fresh.len());
        let mut fresh = fresh.into_iter().peekable();
        for m in old.into_iter().filter(|m| !gone.contains(&m.key)) {
            while let Some(f) = fresh.next_if(|f| date_order(f, &m, reverse) == Ordering::Less) {
                list.push(f);
            }
            list.push(m);
        }
        list.extend(fresh);
        self.take_list_with(list, false);
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
            let index = reader.filter(|_| run.uses_index()).map(|reader| Index { reader, terms: run.terms_of(&k.0) });
            let mut verdicts = Vec::new();
            for m in st.messages.iter().filter(|m| uids.contains(&m.key.uid)) {
                let text = SummaryText::for_query(m, &run.query);
                let awaiting = run.awaited().is_some_and(|a| a.contains(&m.key));
                let v = if m.flags.deleted { Verdict::No } else { classify(&run.query, m, &text, awaiting, index, run.known.get(&m.key)) };
                verdicts.push((m.clone(), v, awaiting));
            }
            for (m, v, awaiting) in verdicts {
                let was = run.found.contains(&m.key);
                run.record(&m, v, awaiting, false);
                // Still to be read: the list is put together once it is.
                if run.found.contains(&m.key) != was || !run.idle() {
                    same = false;
                }
            }
        }
        // Nothing new to show: what the list shows of them is patched in place.
        if same {
            run.fresh.clear();
            run.gone.clear();
        }
        same
    }

    /// A folder's mail changed (read again, mail moved in or out): its summaries' text goes, and
    /// a search reading it scans it again.
    pub(crate) fn search_folder_changed(&mut self, account: &str, folder: &str) {
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
        let listed: HashSet<String> = self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
        self.search_state.texts.retain(|a, f| a != account || listed.contains(f));
        if self.account(account).is_none() {
            self.search_state.readers.remove(account);
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
