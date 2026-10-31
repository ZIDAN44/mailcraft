//! The session: search, here and on the servers.

use super::*;

/// How many searches the search box remembers.
pub const RECENT_SEARCHES: usize = 10;
/// Sent at least this long ago with nothing back, a message is waiting for a reply
/// (`is:awaiting`); sent longer ago than [`AWAITING_WITHIN_DAYS`], it no longer is.
pub const AWAITING_AFTER_DAYS: i64 = 2;
pub const AWAITING_WITHIN_DAYS: i64 = 30;

impl Session {
    /// The folders a search looks in: the one on view, it and the folders inside it, its
    /// account's (every account's from a unified view), or every account's.
    pub(crate) fn scope_folders(&self, scope: SearchScope) -> Vec<(String, String)> {
        let every = |a: &AccountState| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone())).collect::<Vec<_>>();
        match (scope, &self.view) {
            (SearchScope::Current, _) => self.view_folders(),
            (SearchScope::Subfolders, View::Folder { account, folder }) => {
                let Some(a) = self.account(account) else { return Vec::new() };
                let inside = |p: &str| {
                    a.folders
                        .iter()
                        .find(|f| f.path == *folder)
                        .and_then(|f| f.delimiter)
                        .is_some_and(|d| p.strip_prefix(folder.as_str()).is_some_and(|rest| rest.starts_with(d)))
                };
                a.folders
                    .iter()
                    .filter(|f| f.selectable && (f.path == *folder || inside(&f.path)))
                    .map(|f| (account.clone(), f.path.clone()))
                    .collect()
            }
            (SearchScope::Subfolders, _) => self.view_folders(),
            (SearchScope::Mailbox, View::Folder { account, .. }) => self.account(account).map(every).unwrap_or_default(),
            (SearchScope::Mailbox | SearchScope::All, _) => self.accounts.iter().flat_map(every).collect(),
        }
    }

    /// Search words for each of these folders that has none yet; an importing account's folders
    /// wait for the import to end (searched by their summaries until then).
    pub(super) fn ensure_index(&mut self, folders: &[(String, String)]) {
        let mut built = false;
        for (a, f) in folders {
            let k = (a.clone(), f.clone());
            if self.index.contains_key(&k) || self.importing.contains(a) {
                continue;
            }
            let ix = self.folder_index(a, f);
            self.index.insert(k, ix);
            built = true;
        }
        if built {
            self.index_builds = self.index_builds.wrapping_add(1);
        }
    }

    /// One folder's search words: its summaries' and its downloaded messages'.
    fn folder_index(&mut self, a: &str, f: &str) -> SearchIndex {
        let mut index = Index::new();
        let mut keys = Vec::new();
        let mut has_body = Vec::new();
        // A thousand at a time: a big folder isn't copied whole while it's read.
        let n = self.folder_state(a, f).messages.len();
        let mut at = 0;
        while at < n {
            let msgs: Vec<Summary> =
                self.folder_state(a, f).messages.get(at..at.saturating_add(1000).min(n)).map(<[Summary]>::to_vec).unwrap_or_default();
            if msgs.is_empty() {
                break;
            }
            at += msgs.len();
            for m in msgs {
                let doc = u32::try_from(keys.len()).unwrap_or(u32::MAX);
                index.add(doc, &summary_text(&m));
                let words = self.words_of(&m.key);
                if let Some(words) = &words {
                    index.add(doc, words);
                }
                has_body.push(words.is_some());
                keys.push(m.key);
            }
        }
        SearchIndex { index, keys, has_body, stale: false }
    }

    /// A folder's mail changed: its search words go (read again at the next search), or, while
    /// its account imports, are kept and marked to go when the import ends.
    pub(super) fn index_changed(&mut self, account: &str, folder: &str) {
        let k = (account.to_string(), folder.to_string());
        if self.importing.contains(account) {
            if let Some(ix) = self.index.get_mut(&k) {
                ix.stale = true;
            }
        } else {
            self.index.remove(&k);
        }
    }

    /// An account's folders changed (created, renamed, deleted): all its search words go.
    pub(super) fn forget_account_index(&mut self, account: &str) {
        self.index.retain(|(a, _), _| a != account);
    }

    /// An account's folder list came in: the search words of folders no longer in it go.
    pub(super) fn index_folders_listed(&mut self, account: &str) {
        let listed: HashSet<String> = self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
        self.index.retain(|(a, f), _| a != account || listed.contains(f));
    }

    /// A data file import into `account` begins (or is under way).
    pub(super) fn import_started(&mut self, account: &str) {
        self.importing.insert(account.to_string());
    }

    /// The import into `account` ended: the folders it changed are read again for search, once,
    /// at the next search.
    pub(super) fn import_ended(&mut self, account: &str) {
        if self.importing.remove(account) {
            self.index.retain(|(a, _), ix| a != account || !ix.stale);
        }
    }

    /// How many times search words were read for some folder (one search reading several folders
    /// counts once): tests check what doesn't read them again.
    #[cfg(test)]
    pub(crate) fn index_builds(&self) -> u64 {
        self.index_builds
    }

    /// A downloaded message's words for search, read once (`None`: not downloaded).
    pub(super) fn words_of(&mut self, key: &MessageKey) -> Option<Arc<str>> {
        if let Some(w) = self.body_words.get(key) {
            return Some(w.clone());
        }
        let raw = self.store.as_ref().and_then(|s| s.read_body(key))?;
        let words: Arc<str> = Arc::from(mime::search_text(&raw));
        self.keep_words(key, words.clone());
        Some(words)
    }

    /// Keep a message's body words, the oldest kept going first past [`BODY_WORDS_BYTES`] (all
    /// at once: they're cheap to read again).
    pub(super) fn keep_words(&mut self, key: &MessageKey, words: Arc<str>) {
        if self.body_words_bytes + words.len() > BODY_WORDS_BYTES {
            self.body_words.clear();
            self.body_words_bytes = 0;
        }
        self.body_words_bytes += words.len();
        if let Some(old) = self.body_words.insert(key.clone(), words) {
            self.body_words_bytes = self.body_words_bytes.saturating_sub(old.len());
        }
    }

    pub(super) fn search_results(&mut self, q: &str, scope: SearchScope) -> Vec<Summary> {
        let mut query = Query::parse(q);
        // `category:Work` means the Work category's keyword (`$label2`).
        // `is:categorized` means one of the categories.
        for c in &mut query.clauses {
            if let mailcraft_model::search::Term::Keyword(k) = &mut c.term
                && let Some(cat) = self.category(k)
            {
                *k = cat.keyword.clone();
            }
            if let mailcraft_model::search::Term::Categorized(ks) = &mut c.term {
                *ks = self.prefs.categories.iter().map(|c| c.keyword.clone()).collect();
            }
        }
        if query.is_empty() {
            return Vec::new();
        }
        let awaiting = if query.clauses.iter().any(|c| asks_awaiting(&c.term)) { self.awaiting_replies() } else { HashSet::new() };
        let folders = self.scope_folders(scope);
        let mut out = Vec::new();
        // Flags, sizes, senders, subjects: the summaries answer, no message is read.
        if summary_only(&query) {
            for (a, f) in &folders {
                let st = self.folder_state(a, f);
                let found = st
                    .messages
                    .iter()
                    .filter(|m| query.matches(&Doc { summary: m, body: None, indexed: false, awaiting: awaiting.contains(&m.key) }));
                out.extend(found.cloned());
            }
            return out;
        }
        self.ensure_index(&folders);
        let needs_body = query.needs_body();
        for (a, f) in folders {
            let Some(ix) = self.index.remove(&(a.clone(), f.clone())) else {
                // An importing account's folder not read yet: its summaries alone answer.
                let st = self.folder_state(&a, &f);
                let found = st
                    .messages
                    .iter()
                    .filter(|m| query.matches(&Doc { summary: m, body: None, indexed: false, awaiting: awaiting.contains(&m.key) }));
                out.extend(found.cloned());
                continue;
            };
            let docs = ix.index.candidates(&query);
            if !docs.is_empty() {
                // Where each message is in the folder's list, made once: looked up one by one, a
                // search finding most of a big folder was quadratic.
                let by_uid: HashMap<u32, usize> = self.folder_state(&a, &f).messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect();
                for doc in docs {
                    let Some(key) = ix.keys.get(doc as usize) else { continue };
                    let Some(&at) = by_uid.get(&key.uid) else { continue };
                    let Some(m) = self.folder_state(&a, &f).messages.get(at).filter(|m| m.key == *key).cloned() else { continue };
                    // The body read again only when the index can't settle the query.
                    let body = if needs_body && ix.has_body.get(doc as usize) == Some(&true) { self.words_of(key) } else { None };
                    if query.matches(&Doc { summary: &m, body: body.as_deref(), indexed: true, awaiting: awaiting.contains(&m.key) }) {
                        out.push(m);
                    }
                }
            }
            self.index.insert((a, f), ix);
        }
        out
    }

    /// The user's messages still waiting for a reply: in a Sent folder, sent between
    /// [`AWAITING_AFTER_DAYS`] and [`AWAITING_WITHIN_DAYS`] ago to someone else (not in bulk),
    /// and named by no message from anyone else, in any folder, as the one it answers
    /// (`In-Reply-To`) or follows (`References`).
    pub fn awaiting_replies(&mut self) -> HashSet<MessageKey> {
        const DAY: i64 = 86_400;
        let mine: HashSet<String> = self
            .accounts
            .iter()
            .flat_map(|a| [a.config.email.trim().to_lowercase(), a.config.username.trim().to_lowercase()])
            .filter(|e| e.contains('@'))
            .collect();
        let folders: Vec<(String, String, bool)> = self
            .accounts
            .iter()
            .flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone(), f.role == FolderRole::Sent)))
            .collect();
        let (newest, oldest) = (self.clock.now.saturating_sub(AWAITING_AFTER_DAYS * DAY), self.clock.now.saturating_sub(AWAITING_WITHIN_DAYS * DAY));
        let mut answered: HashSet<String> = HashSet::new();
        let mut sent: Vec<(MessageKey, String)> = Vec::new();
        for (account, folder, is_sent) in folders {
            for m in &self.folder_state(&account, &folder).messages {
                if !mine.contains(&m.from.email.trim().to_lowercase()) {
                    answered.extend(std::iter::once(&m.in_reply_to).chain(&m.references).filter(|id| !id.is_empty()).cloned());
                } else if is_sent
                    && !m.bulk
                    && !m.message_id.is_empty()
                    && (oldest..=newest).contains(&m.date)
                    && m.to.iter().chain(&m.cc).any(|r| !mine.contains(&r.email.trim().to_lowercase()))
                {
                    sent.push((m.key.clone(), m.message_id.clone()));
                }
            }
        }
        sent.into_iter().filter(|(_, id)| !answered.contains(id)).map(|(k, _)| k).collect()
    }

    /// The servers' search is no longer wanted (the search box cleared or changed, issue #4):
    /// the accounts still searching stop; what they found stays in the folders' lists.
    pub(crate) fn stop_server_search(&mut self) {
        if let Some(ss) = self.server_search.take()
            && ss.pending > 0
        {
            self.jobs.push(Job::StopSearch);
        }
    }

    /// The search box's recent searches, newest first: kept with the settings, or, while an
    /// account is kept online only, in memory only.
    pub fn recent_searches(&self) -> &[String] {
        if self.searches_in_memory() { &self.recent_in_memory } else { &self.prefs.recent_searches }
    }

    /// Nothing of an online-only account reaches the disk (issue #4), its searches neither.
    fn searches_in_memory(&self) -> bool {
        self.accounts.iter().any(|a| a.config.keeps_online())
    }

    /// Forget the recent searches.
    pub fn forget_recent_searches(&mut self) {
        self.prefs.recent_searches.clear();
        self.recent_in_memory.clear();
    }

    /// Keep a search among the recent ones (the search box's list): newest first, once, ten.
    pub fn remember_search(&mut self, query: &str) {
        let q: String = query.trim().chars().take(500).collect();
        if q.is_empty() {
            return;
        }
        let recent = if self.searches_in_memory() {
            // Those kept on disk before go too.
            if self.recent_in_memory.is_empty() {
                self.recent_in_memory = std::mem::take(&mut self.prefs.recent_searches);
            }
            self.prefs.recent_searches.clear();
            &mut self.recent_in_memory
        } else {
            &mut self.prefs.recent_searches
        };
        recent.retain(|r| *r != q);
        recent.insert(0, q);
        recent.truncate(RECENT_SEARCHES);
    }

    pub fn run_search(&mut self, query: &str, scope: SearchScope) -> usize {
        self.search_as(query, scope, None)
    }

    /// A search, or (`folder`) a Search Folder's: a search typed in is never one, whatever it
    /// asks.
    pub(crate) fn search_as(&mut self, query: &str, scope: SearchScope, folder: Option<String>) -> usize {
        self.search_folder = folder;
        let q = query.trim();
        self.search = if q.is_empty() { None } else { Some((q.to_string(), scope)) };
        if self.server_search.as_ref().is_some_and(|ss| ss.query != q) {
            self.stop_server_search();
        }
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        self.refresh_list();
        self.list.len()
    }
}

/// Whether a search asks for mail waiting for a reply (`Some(true)`), or for mail that isn't
/// (`Some(false)`, `-is:awaiting`), as one of the terms that must all match.
pub(super) fn awaiting_wanted(q: &str) -> Option<bool> {
    Query::parse(q).clauses.iter().find(|c| c.term == mailcraft_model::search::Term::Awaiting).map(|c| !c.negated)
}

/// Whether the summaries alone answer a query: it looks for no words anywhere or in the text
/// (flags, categories, sizes, senders, recipients, subjects, waiting for a reply).
pub(super) fn summary_only(q: &Query) -> bool {
    q.clauses.iter().all(|c| summary_term(&c.term))
}

fn summary_term(t: &mailcraft_model::search::Term) -> bool {
    use mailcraft_model::search::{Field, Term};
    match t {
        Term::Words(f, _) | Term::Phrase(f, _) => !matches!(f, Field::Any | Field::Body),
        Term::AnyOf(any) => any.iter().all(summary_term),
        Term::HasAttachment(_)
        | Term::Unread(_)
        | Term::Flagged(_)
        | Term::Keyword(_)
        | Term::Important
        | Term::Categorized(_)
        | Term::Size(..)
        | Term::Awaiting => true,
    }
}

/// Whether a search term asks for mail waiting for a reply (on its own or among others).
fn asks_awaiting(t: &mailcraft_model::search::Term) -> bool {
    use mailcraft_model::search::Term;
    match t {
        Term::Awaiting => true,
        Term::AnyOf(any) => any.iter().any(asks_awaiting),
        _ => false,
    }
}
