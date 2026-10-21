//! The session: search, here and on the servers.

use super::*;

impl Session {
    pub(super) fn ensure_index(&mut self, scope: SearchScope) {
        if self.index.as_ref().is_some_and(|ix| ix.scope == scope && (scope == SearchScope::All || ix.view == self.view)) {
            return;
        }
        let folders: Vec<(String, String)> = match scope {
            SearchScope::Current => self.view_folders(),
            SearchScope::All => {
                self.accounts.iter().flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone()))).collect()
            }
        };
        let mut index = Index::new();
        let mut keys = Vec::new();
        let mut has_body = Vec::new();
        for (a, f) in folders {
            // A thousand at a time: a big folder isn't copied whole while it's read.
            let n = self.folder_state(&a, &f).messages.len();
            let mut at = 0;
            while at < n {
                let msgs: Vec<Summary> = self.folder_state(&a, &f).messages.get(at..(at + 1000).min(n)).map(<[Summary]>::to_vec).unwrap_or_default();
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
        }
        self.index = Some(SearchIndex { index, keys, has_body, scope, view: self.view.clone() });
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
        self.ensure_index(scope);
        let Some(ix) = self.index.take() else { return Vec::new() };
        let mut out = Vec::new();
        // Each folder's messages by number, made once: looked up one by one in the list, a search
        // finding most of a big folder was quadratic.
        let mut by_uid: HashMap<(String, String), HashMap<u32, usize>> = HashMap::new();
        let needs_body = query.needs_body();
        for doc in ix.index.candidates(&query) {
            let Some(key) = ix.keys.get(doc as usize) else { continue };
            let folder = (key.account.clone(), key.folder.clone());
            if !by_uid.contains_key(&folder) {
                // Where each message is in its folder's list (not a copy of every summary).
                let map = self.folder_state(&key.account, &key.folder).messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect();
                by_uid.insert(folder.clone(), map);
            }
            let Some(&at) = by_uid.get(&folder).and_then(|f| f.get(&key.uid)) else { continue };
            let Some(m) = self.folder_state(&key.account, &key.folder).messages.get(at).filter(|m| m.key.uid == key.uid).cloned() else { continue };
            // The body read again only when the index can't settle the query.
            let body = if needs_body && ix.has_body.get(doc as usize) == Some(&true) { self.words_of(key) } else { None };
            if query.matches(&Doc { summary: &m, body: body.as_deref(), indexed: true }) {
                out.push(m);
            }
        }
        self.index = Some(ix);
        out
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
            self.server_search = None;
        }
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        self.refresh_list();
        self.list.len()
    }
}
