//! Search: a search run a bounded unit of work at a time ([`Session::search_step`]), so the
//! window can run a little of it each frame and never wait on it.
//!
//! A unit is one of: a page of a folder's list read; up to [`UNIT_PAGES`] pages of an
//! account's index looked through for a word; [`UNIT_MESSAGES`] summaries checked; a message's
//! text read, or [`VERIFY_BYTES`](super::matching::VERIFY_BYTES) of it checked; (waiting only)
//! a message's source read; a step of putting the list together at the end. The folders a run
//! reads are kept in the cache while it lasts. Nothing in it reads the clock or the network.
//!
//! Each folder in scope is read, its account's index opened and the query's words looked up in
//! it, then its summaries checked: the summary settles flags, dates, senders and subjects; the
//! index settles words; what neither can settle is checked against the message's text as the
//! index keeps it. A message the index doesn't have yet "may be missing" (`missing`), or,
//! waiting (the CLI, tests), is read from its source. What's found streams into the list as it
//! is found; at the end the list is arranged and put into conversations.

use std::collections::VecDeque;

use mailcraft_store::FolderLoad;
use mailcraft_store::search::{Bits, Field, ScanCursor};

use super::finish::Finish;
use super::matching::{Answers, Candidate, Index, Recorder, Verdict, Verifying, classify, from_source};
use super::work::SearchWork;
use super::*;

/// Summaries checked in one unit.
pub(crate) const UNIT_MESSAGES: usize = 512;
/// Index pages looked through for a word in one unit (at least one word's worth).
pub(crate) const UNIT_PAGES: usize = 2;

/// How a search is going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchProgress {
    /// Nothing is left to do (until mail changes).
    pub done: bool,
    /// Messages found so far.
    pub found: usize,
    /// Messages that may match but whose text isn't indexed yet.
    pub missing: usize,
    /// Messages that may match past where their text was cut short (too long to index whole):
    /// not waiting for anything.
    pub cut_short: usize,
    /// The list changed in the last step.
    pub changed: bool,
}

/// What searches have read, all told: what a step's budget is checked against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchStats {
    /// Pages of folders' lists read.
    pub folder_pages: u64,
    /// Pages of an index read (lexicon, postings, texts).
    pub reader_pages: u64,
    /// Summaries checked.
    pub summaries: u64,
    /// Units spent checking a message's text.
    pub texts: u64,
    /// Message sources read (waiting only).
    pub sources: u64,
    /// Units of work done.
    pub units: u64,
    /// Summaries' texts made (folded) for searches of the summaries' words.
    pub summary_texts: u64,
}

/// The words a search looks up in an account's index, and the docs each was found in.
#[derive(Debug, Default)]
pub(crate) struct Terms {
    /// A few (the words typed): looked through in turn, as each summary asks, not hashed.
    bits: Vec<(Field, String, Bits)>,
    todo: Vec<(Field, String)>,
    cursor: ScanCursor,
    scanning: Option<Bits>,
}

impl Terms {
    /// Whether `doc` has a word of `field` starting with `word`; `None`: not looked up.
    pub(crate) fn has(&self, field: Field, word: &str, doc: u32) -> Option<bool> {
        self.bits.iter().find(|(f, w, _)| *f == field && w == word).map(|(_, _, b)| b.get(doc))
    }

    fn ready(&self) -> bool {
        self.todo.is_empty()
    }
}

/// Where a run is in a folder.
#[derive(Debug, Default)]
struct Cursor {
    fi: usize,
    load: Option<FolderLoad>,
    /// Its account's index was looked at.
    indexed: bool,
    at: usize,
    /// Its first scan in this run: nothing of it is found, missing or cut short yet, so what
    /// isn't found needs no looking up (each summary's key hashed three times was a third of
    /// what checking a flag over 50,000 cost).
    first: bool,
}

/// What a folder's list being had in the cache came to.
enum Loaded {
    /// It's there.
    Ready,
    /// A page was read (a unit).
    Read,
    /// Not read yet: read marks of it are still being written (a unit).
    Waits,
    /// It can't be read.
    Failed,
}

/// A search, running or done.
pub(crate) struct SearchRun {
    pub(crate) q: String,
    pub(crate) scope: SearchScope,
    pub(crate) query: Query,
    /// Settled in full before anything is shown: what isn't indexed is read from its source.
    pub(crate) wait: bool,
    needs_text: bool,
    asks_files: bool,
    /// It asks for words in the summaries (senders, subjects, previews).
    summary_words: bool,
    /// The words to look up in each index.
    words: Vec<(Field, String)>,
    pub(crate) folders: Vec<(String, String)>,
    /// How many of `folders` (the first) are on view.
    on_view: usize,
    /// Folders still to scan (indices into `folders`), in order.
    pending: VecDeque<usize>,
    /// Those whose scan began, by index: scanned again, they may hold what was found.
    begun: Vec<bool>,
    cur: Option<Cursor>,
    awaiting: Option<(AwaitingScan, Option<FolderLoad>, usize)>,
    asks_awaiting: bool,
    awaited: HashSet<MessageKey>,
    terms: HashMap<String, Terms>,
    pub(crate) found: HashSet<MessageKey>,
    /// Found since last shown, and found before and no longer.
    pub(crate) fresh: Vec<Summary>,
    pub(crate) gone: HashSet<MessageKey>,
    /// Steps since what was found was last shown.
    pub(crate) unshown_steps: usize,
    /// Found before in a folder being scanned again: let go at its end unless found again.
    recheck: HashSet<MessageKey>,
    verify: VecDeque<Candidate>,
    /// `verify` is in the order of the docs: their texts read a page after another, each once.
    verify_sorted: bool,
    verifying: Option<Verifying>,
    fallback: VecDeque<Candidate>,
    /// What each message checked against its text said (flags never change it).
    pub(crate) known: HashMap<MessageKey, Answers>,
    pub(crate) missing: HashSet<MessageKey>,
    /// Those whose text was cut short ([`Verdict::CutShort`]).
    pub(crate) cut_short: HashSet<MessageKey>,
    /// The list shows this run's finds alone (`refresh_list` put it together for it), each
    /// change since merged in: its end needs only what's left to show.
    pub(crate) owns_list: bool,
    pub(super) finish: Finish,
    /// What the unit under way is doing ([`SearchWork`]).
    pub(super) work: SearchWork,
}

impl SearchRun {
    pub(crate) fn new(q: &str, scope: SearchScope, query: Query, folders: Vec<(String, String)>, wait: bool) -> SearchRun {
        use mailcraft_model::search::Term;
        fn summary_words(t: &Term) -> bool {
            match t {
                Term::Words(..) | Term::Phrase(..) => true,
                Term::AnyOf(any) => any.iter().any(summary_words),
                _ => false,
            }
        }
        fn files(t: &Term) -> bool {
            match t {
                Term::HasAttachment(_) => true,
                Term::AnyOf(any) => any.iter().any(files),
                _ => false,
            }
        }
        let mut words: Vec<(Field, String)> = Vec::new();
        for (tf, w, _) in query.text_words() {
            if w.is_empty() {
                continue;
            }
            let address = w.contains(['@', '.']);
            for f in [Some(Field::from(tf)), address.then_some(Field::Address)].into_iter().flatten() {
                if !words.iter().any(|(g, x)| *g == f && *x == w) {
                    words.push((f, w.clone()));
                }
            }
        }
        SearchRun {
            q: q.to_string(),
            scope,
            needs_text: query.needs_text(),
            asks_files: query.clauses.iter().any(|c| files(&c.term)),
            summary_words: query.clauses.iter().any(|c| summary_words(&c.term)),
            awaiting: None,
            asks_awaiting: false,
            query,
            wait,
            words,
            pending: (0..folders.len()).collect(),
            begun: vec![false; folders.len()],
            folders,
            cur: None,
            awaited: HashSet::new(),
            terms: HashMap::new(),
            found: HashSet::new(),
            fresh: Vec::new(),
            gone: HashSet::new(),
            unshown_steps: 0,
            recheck: HashSet::new(),
            verify: VecDeque::new(),
            verify_sorted: true,
            on_view: 0,
            verifying: None,
            fallback: VecDeque::new(),
            known: HashMap::new(),
            missing: HashSet::new(),
            cut_short: HashSet::new(),
            owns_list: false,
            finish: Finish::Due,
            work: SearchWork::Summaries,
        }
    }

    /// Whether the index is opened for it: it asks for words, or whether there are files.
    pub(crate) fn uses_index(&self) -> bool {
        self.needs_text || self.asks_files
    }

    /// The words looked up in `account`'s index so far.
    pub(crate) fn terms_of(&self, account: &str) -> Option<&Terms> {
        self.terms.get(account)
    }

    /// The mail waiting for a reply, when the query asks (all of it once worked out).
    pub(crate) fn awaited(&self) -> Option<&HashSet<MessageKey>> {
        self.asks_awaiting.then_some(&self.awaited)
    }

    /// Whether folder `fi` was scanned (and isn't to be again).
    pub(crate) fn scanned(&self, fi: usize) -> bool {
        self.awaiting.is_none() && !self.pending.contains(&fi) && self.cur.as_ref().is_none_or(|c| c.fi != fi)
    }

    /// Nothing is left to read: only the list to put together, if that.
    pub(crate) fn idle(&self) -> bool {
        self.awaiting.is_none()
            && self.cur.is_none()
            && self.pending.is_empty()
            && self.verifying.is_none()
            && self.verify.is_empty()
            && self.fallback.is_empty()
    }

    pub(crate) fn done(&self) -> bool {
        matches!(self.finish, Finish::Done)
    }

    /// The list being put together again from the start: what it gathered (copies of the
    /// summaries) is no longer how the folders hold them.
    pub(crate) fn gather_again(&mut self) {
        if !self.done() {
            self.finish = Finish::Due;
        }
    }

    /// Whether the run reads anything of an account `asked` says yes to (its folders, and those
    /// it reads for mail awaiting a reply).
    pub(crate) fn reads_any(&self, asked: impl Fn(&str) -> bool) -> bool {
        self.folders.iter().any(|(a, _)| asked(a)) || self.awaiting.as_ref().is_some_and(|(s, _, _)| s.folders.iter().any(|(a, _, _)| asked(a)))
    }

    pub(crate) fn progress(&self, changed: bool) -> SearchProgress {
        SearchProgress { done: self.done(), found: self.found.len(), missing: self.missing.len(), cut_short: self.cut_short.len(), changed }
    }

    /// A run done with: what it holds of each message (found, answered, waiting to be shown or
    /// checked) moved to `summaries` and `keys`, to be let go of a slice at a time; the rest,
    /// small, goes now.
    pub(crate) fn let_go_into(self, summaries: &mut Vec<Summary>, keys: &mut Vec<MessageKey>) {
        let SearchRun { found, fresh, gone, recheck, verify, fallback, known, missing, cut_short, awaited, .. } = self;
        summaries.extend(fresh);
        summaries.extend(verify.into_iter().chain(fallback).map(|c| c.m));
        keys.extend(found.into_iter().chain(gone).chain(recheck).chain(missing).chain(cut_short).chain(awaited));
        keys.extend(known.into_keys());
    }

    /// Scan folder `fi` again (its list changed): what it found before stays found until the
    /// scan says otherwise.
    pub(crate) fn rescan(&mut self, fi: usize) {
        let Some((a, f)) = self.folders.get(fi).cloned() else { return };
        let here = |k: &MessageKey| k.account == a && k.folder == f;
        self.recheck.extend(self.found.iter().filter(|k| here(k)).cloned());
        // What a message's text said stays known: a message (its key) never changes its text.
        self.missing.retain(|k| !here(k));
        self.cut_short.retain(|k| !here(k));
        self.verify.retain(|c| !here(&c.m.key));
        self.fallback.retain(|c| !here(&c.m.key));
        if self.verifying.as_ref().is_some_and(|v| here(&v.c.m.key)) {
            self.verifying = None;
        }
        if self.cur.as_ref().is_some_and(|c| c.fi == fi) {
            self.cur = None;
        }
        if !self.pending.contains(&fi) {
            self.pending.push_back(fi);
        }
        self.finish = Finish::Due;
    }

    /// An account's index changed: its words are looked up again and its folders scanned again.
    pub(crate) fn rescan_account(&mut self, account: &str) {
        self.terms.remove(account);
        for fi in 0..self.folders.len() {
            if self.folders.get(fi).is_some_and(|(a, _)| a == account) {
                self.rescan(fi);
            }
        }
    }

    /// Where `m` stands now, as `v` says (`last`: nothing more can be read for it). Taken, not
    /// copied: a word found in thousands of messages copied each summary twice.
    pub(crate) fn record(&mut self, m: Summary, v: Verdict, awaiting: bool, last: bool) {
        let key = &m.key;
        forget(&mut self.recheck, key);
        if !matches!(v, Verdict::CutShort) {
            forget(&mut self.cut_short, key);
        }
        match v {
            Verdict::Yes => {
                forget(&mut self.missing, key);
                forget(&mut self.gone, key);
                if self.found.insert(key.clone()) {
                    self.fresh.push(m);
                }
            }
            Verdict::Verify(doc) if !last => {
                self.verify.push_back(Candidate { m, doc: Some(doc), awaiting });
                self.verify_sorted = false;
                self.finish = Finish::Due;
            }
            Verdict::Missing if self.wait && !last => {
                self.fallback.push_back(Candidate { m, doc: None, awaiting });
                self.finish = Finish::Due;
            }
            Verdict::No | Verdict::Verify(_) | Verdict::Missing | Verdict::CutShort => {
                if matches!(v, Verdict::No | Verdict::CutShort) {
                    self.missing.remove(key);
                } else {
                    self.missing.insert(key.clone());
                }
                if matches!(v, Verdict::CutShort) {
                    self.cut_short.insert(key.clone());
                }
                if self.found.remove(key) {
                    self.gone.insert(key.clone());
                }
            }
        }
    }
}

/// `key` no longer in `set`: an empty set (most are, most of the time) isn't asked, which would
/// hash the key.
fn forget(set: &mut HashSet<MessageKey>, key: &MessageKey) {
    if !set.is_empty() {
        set.remove(key);
    }
}

impl Session {
    /// Start a search of `q` over `scope` (replacing any): run by [`Session::search_step`].
    /// `wait`: settled in full before it's shown, what isn't indexed read from its source.
    pub fn start_search(&mut self, q: &str, scope: SearchScope, wait: bool) {
        let query = self.query_of(q);
        let mut folders = self.scope_folders(scope);
        // The folders on view first: what's on screen is found first.
        let shown = self.view_folders();
        folders.sort_by_key(|k| !shown.contains(k));
        let on_view = folders.iter().filter(|k| shown.contains(k)).count();
        let mut run = SearchRun::new(q, scope, query, folders, wait);
        run.on_view = on_view;
        if run.query.is_empty() {
            run.pending.clear();
        } else if run.query.clauses.iter().any(|c| asks_awaiting(&c.term)) {
            run.awaiting = Some((AwaitingScan::new(self), None, 0));
            run.asks_awaiting = true;
        }
        self.search_state.pinned = run.folders.iter().cloned().collect();
        // An account that had no index (or one that couldn't be read) is looked at again; one a
        // search stopped on, once its writer writes it again.
        self.search_state.readers.retain(|_, slot| slot.reader.is_some() || slot.damaged);
        if let Some(old) = self.search_state.run.replace(run) {
            let state = &mut self.search_state;
            old.let_go_into(&mut state.letting_go, &mut state.letting_go_keys);
        }
    }

    /// How the search is going (`None`: no search).
    pub fn search_progress(&self) -> Option<SearchProgress> {
        self.search_state.run.as_ref().map(|r| r.progress(false))
    }

    /// What searches have read so far.
    pub fn search_stats(&self) -> SearchStats {
        SearchStats { summary_texts: self.search_state.texts.made(), ..self.search_state.stats }
    }

    /// Settle a waiting search still running: `true` when it did (and showed what it found).
    pub(crate) fn settle_search(&mut self) -> bool {
        if !self.search_state.run.as_ref().is_some_and(|r| r.wait && !r.done()) {
            return false;
        }
        while self.search_state.run.as_ref().is_some_and(|r| !r.done()) {
            self.search_step(u32::MAX);
        }
        true
    }

    /// At most `units` units, asking `more` after each whether to go on, and telling `done`
    /// what each was ([`SearchWork`]).
    pub(super) fn run_units(&mut self, run: &mut SearchRun, units: u32, more: &mut dyn FnMut() -> bool, done: &mut dyn FnMut(SearchWork)) {
        #[cfg(test)]
        self.search_test_fault();
        let mut left = units.max(1);
        while left > 0 {
            let spent = if run.awaiting.is_some() {
                run.work = SearchWork::Awaiting;
                self.awaiting_unit(run)
            } else if run.cur.is_some() || !run.pending.is_empty() {
                // Which of its parts: said by the unit.
                run.work = SearchWork::Summaries;
                self.folder_unit(run)
            } else if run.verifying.is_some() || !run.verify.is_empty() {
                run.work = SearchWork::Verify;
                self.verify_unit(run)
            } else if !run.fallback.is_empty() {
                run.work = SearchWork::Source;
                self.fallback_unit(run)
            } else if !run.done() {
                run.work = SearchWork::Finish;
                self.finish_unit(run)
            } else {
                break;
            };
            done(run.work);
            self.search_state.stats.units = self.search_state.stats.units.saturating_add(u64::from(spent));
            left = left.saturating_sub(spent);
            if !more() {
                break;
            }
        }
    }

    /// A test's: the next step of the search stops part-way, as a bug would.
    #[cfg(test)]
    pub(crate) fn fail_next_search_step(&mut self) {
        self.search_state.fault = true;
    }

    #[cfg(test)]
    fn search_test_fault(&mut self) {
        if std::mem::take(&mut self.search_state.fault) {
            panic!("a test's search fault");
        }
    }

    /// Have a folder's list in the cache, a page a call (`load` holds the reading between calls).
    fn load_unit(&mut self, k: &(String, String), load: &mut Option<FolderLoad>) -> Loaded {
        let Some(store) = self.store.clone() else {
            return match self.load_cached(&k.0, &k.1) {
                Ok(()) => Loaded::Ready,
                Err(_) => Loaded::Failed,
            };
        };
        let step = match load {
            None if self.cache.contains_key(k) => return Loaded::Ready,
            // Read marks still being written: read once they are, not waited for here (the window's
            // thread).
            None if store.marks_waiting_for(&k.0, Some(&k.1)) => return Loaded::Waits,
            None => store.folder_load(&k.0, &k.1).map(|l| {
                let done = l.done();
                *load = Some(l);
                done
            }),
            Some(l) => l.step(&store),
        };
        self.search_state.stats.folder_pages = self.search_state.stats.folder_pages.saturating_add(1);
        match step {
            Ok(true) => {
                if let Some(l) = load.take()
                    && !self.cache.contains_key(k)
                {
                    self.keep_cached(k.clone(), l.finish());
                }
                Loaded::Read
            }
            Ok(false) => Loaded::Read,
            Err(e) => {
                log::warn!("a folder's list couldn't be read for search: {e}");
                *load = None;
                Loaded::Failed
            }
        }
    }

    /// One unit of working out which of the user's mail waits for a reply (`is:awaiting`).
    fn awaiting_unit(&mut self, run: &mut SearchRun) -> u32 {
        let Some((mut scan, mut load, mut at)) = run.awaiting.take() else { return 0 };
        let Some((a, f, sent)) = scan.folders.first().cloned() else {
            run.awaited = scan.finish();
            return 1;
        };
        let k = (a, f);
        match self.load_unit(&k, &mut load) {
            Loaded::Read | Loaded::Waits => {
                run.awaiting = Some((scan, load, at));
                return 1;
            }
            Loaded::Failed => {
                scan.folders.remove(0);
                run.awaiting = Some((scan, None, 0));
                return 1;
            }
            Loaded::Ready => {}
        }
        let messages = self.cache.get(&k).map(|st| st.messages.as_slice()).unwrap_or_default();
        let end = at.saturating_add(UNIT_MESSAGES).min(messages.len());
        for m in messages.get(at..end).unwrap_or_default() {
            scan.take(m, sent);
        }
        at = end;
        if at >= messages.len() {
            scan.folders.remove(0);
            at = 0;
        }
        run.awaiting = Some((scan, None, at));
        1
    }

    /// One unit of scanning the folders: a page of one read, its index opened, its words
    /// looked up, or a slice of its summaries checked.
    fn folder_unit(&mut self, run: &mut SearchRun) -> u32 {
        let mut cur = match run.cur.take() {
            Some(c) => c,
            None => match run.pending.pop_front() {
                Some(fi) => {
                    let first = run.begun.get(fi).is_some_and(|b| !b);
                    if let Some(b) = run.begun.get_mut(fi) {
                        *b = true;
                    }
                    Cursor { fi, first, ..Default::default() }
                }
                None => return 0,
            },
        };
        let Some(k) = run.folders.get(cur.fi).cloned() else { return 0 };
        match self.load_unit(&k, &mut cur.load) {
            Loaded::Read => {
                run.work = SearchWork::ListPage;
                run.cur = Some(cur);
                return 1;
            }
            // The other folders meanwhile, this one after them.
            Loaded::Waits if !run.pending.is_empty() => {
                run.work = SearchWork::ListPage;
                run.pending.push_back(cur.fi);
                return 1;
            }
            Loaded::Waits => {
                run.work = SearchWork::ListPage;
                run.cur = Some(cur);
                return 1;
            }
            Loaded::Failed => {
                self.folder_scanned(run, &k);
                return 1;
            }
            Loaded::Ready => {}
        }
        let account = k.0.clone();
        if run.uses_index() && !cur.indexed {
            cur.indexed = true;
            if self.open_reader(&account) {
                run.work = SearchWork::OpenIndex;
                run.cur = Some(cur);
                return 1;
            }
        }
        if run.needs_text
            && !run.terms.get(&account).is_some_and(Terms::ready)
            && self.search_state.readers.get(&account).is_some_and(|s| s.reader.is_some())
        {
            run.work = SearchWork::LookUp;
            self.terms_unit(run, &account);
            run.cur = Some(cur);
            return 1;
        }
        // A slice of the summaries.
        let Some(st) = self.cache.get(&k) else {
            self.folder_scanned(run, &k);
            return 1;
        };
        let len = st.messages.len();
        let end = cur.at.saturating_add(UNIT_MESSAGES).min(len);
        let texts: &[SummaryText] = if run.summary_words { self.search_state.texts.texts(&k.0, &k.1, &st.messages, cur.at..end) } else { &[] };
        let reader = self.search_state.readers.get(&account).and_then(|s| s.reader.as_ref());
        let index = reader.filter(|_| run.uses_index()).map(|reader| Index::of(reader, run.terms.get(&account), &k.1));
        let none = SummaryText::default();
        let mut verdicts = Vec::new();
        for (i, m) in st.messages.get(cur.at..end).unwrap_or_default().iter().enumerate() {
            if m.flags.deleted {
                continue;
            }
            let text = texts.get(i).unwrap_or(&none);
            let awaiting = !run.awaited.is_empty() && run.awaited.contains(&m.key);
            let known = if run.known.is_empty() { None } else { run.known.get(&m.key) };
            let v = classify(&run.query, m, text, awaiting, index, known);
            // Unchanged and nothing to do: not cloned.
            let same = match v {
                Verdict::No if cur.first => true,
                _ if cur.first => false,
                Verdict::Yes => run.found.contains(&m.key) && !run.recheck.contains(&m.key),
                Verdict::No => !run.found.contains(&m.key) && !run.missing.contains(&m.key) && !run.cut_short.contains(&m.key),
                Verdict::CutShort => !run.found.contains(&m.key) && run.cut_short.contains(&m.key),
                _ => false,
            };
            if !same {
                verdicts.push((m.clone(), v, awaiting));
            }
        }
        let checked = u64::try_from(end.saturating_sub(cur.at)).unwrap_or(u64::MAX);
        self.search_state.stats.summaries = self.search_state.stats.summaries.saturating_add(checked);
        for (m, v, awaiting) in verdicts {
            run.record(m, v, awaiting, false);
        }
        cur.at = end;
        if end >= len {
            self.folder_scanned(run, &k);
        } else {
            run.cur = Some(cur);
        }
        1
    }

    /// A folder scanned in full: what was found there before and wasn't found again goes.
    fn folder_scanned(&mut self, run: &mut SearchRun, k: &(String, String)) {
        let left: Vec<MessageKey> = run.recheck.iter().filter(|m| m.account == k.0 && m.folder == k.1).cloned().collect();
        for key in left {
            run.recheck.remove(&key);
            if run.found.remove(&key) {
                run.gone.insert(key);
            }
        }
    }

    /// Open `account`'s index for search, once: whether that took a unit.
    fn open_reader(&mut self, account: &str) -> bool {
        if self.search_state.readers.contains_key(account) {
            return false;
        }
        let reader = match &self.store {
            Some(store) => match mailcraft_store::search::Reader::open(store, account) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("{account}: the search index couldn't be read: {e}");
                    None
                }
            },
            None => None,
        };
        self.search_state.readers.insert(account.to_string(), ReaderSlot { reader, checked: self.clock.now, behind: false, damaged: false });
        true
    }

    /// Look up the run's words in `account`'s index, a few pages.
    fn terms_unit(&mut self, run: &mut SearchRun, account: &str) {
        let (Some(store), Some(reader)) = (self.store.clone(), self.search_state.readers.get_mut(account).and_then(|s| s.reader.as_mut())) else {
            return;
        };
        let terms = run.terms.entry(account.to_string()).or_insert_with(|| Terms { todo: run.words.clone(), ..Default::default() });
        let Some((field, word)) = terms.todo.first().cloned() else { return };
        let before = reader.pages_read();
        let mut bits = terms.scanning.take().unwrap_or_else(|| Bits::new(reader.len()));
        let done = reader.scan_prefix(&store, field, &word, &mut terms.cursor, &mut bits, UNIT_PAGES);
        let read = reader.pages_read().saturating_sub(before);
        self.search_state.stats.reader_pages = self.search_state.stats.reader_pages.saturating_add(read);
        match done {
            Ok(false) => terms.scanning = Some(bits),
            Ok(true) => {
                terms.bits.retain(|(f, w, _)| *f != field || *w != word);
                terms.bits.push((field, word, bits));
                terms.todo.remove(0);
                terms.cursor = ScanCursor::default();
            }
            Err(e) => {
                // Read afresh by the next search; this one goes on without it (what it can't
                // tell may be missing).
                log::warn!("{account}: the search index couldn't be read: {e}");
                run.terms.remove(account);
                if let Some(slot) = self.search_state.readers.get_mut(account) {
                    slot.reader = None;
                }
            }
        }
    }

    /// One unit of checking a message against the text the index keeps for it.
    fn verify_unit(&mut self, run: &mut SearchRun) -> u32 {
        self.search_state.stats.texts = self.search_state.stats.texts.saturating_add(1);
        let mut v = match run.verifying.take() {
            Some(v) => v,
            None => {
                if !run.verify_sorted {
                    // Checked in the folders' order, a message after another read a page of texts
                    // afresh (a common phrase over 50,000 messages: 1,015 pages read, each 256 KiB
                    // unpacked); in the docs' order each page is read once (103). The folders on
                    // view still come first, as they're scanned first: what's on screen is found
                    // first. (What's sorted already stays a run the sort takes as it is.)
                    let view = run.folders.get(..run.on_view).unwrap_or_default().to_vec();
                    let off = |c: &Candidate| !view.iter().any(|(a, f)| *a == c.m.key.account && *f == c.m.key.folder);
                    run.verify
                        .make_contiguous()
                        .sort_by(|a, b| off(a).cmp(&off(b)).then(a.m.key.account.cmp(&b.m.key.account)).then(a.doc.cmp(&b.doc)));
                    run.verify_sorted = true;
                }
                let Some(c) = run.verify.pop_front() else { return 0 };
                let reader = self.search_state.readers.get(&c.m.key.account).and_then(|s| s.reader.as_ref());
                match (c.doc, reader) {
                    (Some(doc), Some(r)) if r.info(doc).is_some() => {
                        let info = r.info(doc).unwrap_or_default();
                        Verifying::new(&run.query, c, doc, info)
                    }
                    // Its index went meanwhile: it may be missing.
                    _ => {
                        run.record(c.m, Verdict::Missing, c.awaiting, true);
                        return 1;
                    }
                }
            }
        };
        let account = v.c.m.key.account.clone();
        let (Some(store), Some(reader)) = (self.store.clone(), self.search_state.readers.get_mut(&account).and_then(|s| s.reader.as_mut())) else {
            run.record(v.c.m, Verdict::Missing, v.c.awaiting, true);
            return 1;
        };
        let before = reader.pages_read();
        let said = v.step(reader, &store);
        let read = reader.pages_read().saturating_sub(before);
        self.search_state.stats.reader_pages = self.search_state.stats.reader_pages.saturating_add(read);
        match said {
            None => run.verifying = Some(v),
            Some(answers) => {
                let c = v.c;
                let st = SummaryText::for_query(&c.m, &run.query);
                // A long message's stays cut short whenever it's weighed again from these.
                let verdict = classify(&run.query, &c.m, &st, c.awaiting, None, Some(&answers));
                if answers.said() {
                    run.known.insert(c.m.key.clone(), answers);
                }
                run.record(c.m, verdict, c.awaiting, true);
            }
        }
        1
    }

    /// Waiting: one message the index doesn't have read from its source.
    fn fallback_unit(&mut self, run: &mut SearchRun) -> u32 {
        let Some(c) = run.fallback.pop_front() else { return 0 };
        let atoms = Recorder::atoms(&run.query, &c.m, c.awaiting);
        let raw = self.store.as_ref().and_then(|s| s.read_body(&c.m.key));
        self.search_state.stats.sources = self.search_state.stats.sources.saturating_add(1);
        let st = SummaryText::for_query(&c.m, &run.query);
        let verdict = match raw {
            Some(raw) => {
                let answers = from_source(&raw, &atoms);
                let verdict = classify(&run.query, &c.m, &st, c.awaiting, None, Some(&answers));
                run.known.insert(c.m.key.clone(), answers);
                verdict
            }
            // Not kept here (only its headers came): its summary alone is searched, for now
            // (what it said isn't kept: its text may come).
            None => {
                let doc = Doc { summary: &c.m, summary_text: &st, awaiting: c.awaiting, files: None, text: &super::matching::NoText };
                match run.query.eval(&doc) {
                    Tri::Yes => Verdict::Yes,
                    Tri::No => Verdict::No,
                    Tri::Unknown => Verdict::Missing,
                }
            }
        };
        run.record(c.m, verdict, c.awaiting, true);
        1
    }
}
