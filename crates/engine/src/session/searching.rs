//! The session: search, here and on the servers.
//!
//! A search is a [run](run::SearchRun) the window steps a little each frame (the CLI and tests
//! wait for it): the summaries in memory answer flags, dates, senders and subjects; each
//! account's index, kept up in the background ([`Job::Index`]), answers words; the text the
//! index keeps settles phrases. Nothing of it reads a message's source unless it waits.

use mailcraft_store::search::Reader;

use super::*;

mod finish;
mod folded;
mod folder_text;
mod kept;
mod look_up;
mod matching;
mod publish;
mod run;
mod work;

pub use run::{SearchProgress, SearchStats};
pub use work::SearchWork;

/// How many searches the search box remembers.
pub const RECENT_SEARCHES: usize = 10;
/// Sent at least this long ago with nothing back, a message is waiting for a reply
/// (`is:awaiting`); sent longer ago than [`AWAITING_WITHIN_DAYS`], it no longer is.
pub const AWAITING_AFTER_DAYS: i64 = 2;
pub const AWAITING_WITHIN_DAYS: i64 = 30;

/// How an account's last pass of its search index went ([`Event::Indexed`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexPass {
    /// The messages read of those it found to add.
    pub work: mailcraft_model::job::Work,
    /// The index's generation it wrote last.
    pub generation: u64,
    /// Messages that couldn't be read for search.
    pub failed: u64,
    pub finished: bool,
}

/// The search indexes kept up in the background, as the session asks for them and hears of them.
#[derive(Debug, Default)]
pub(crate) struct Indexing {
    /// Something keeps the indexes ([`Session::set_indexing`]).
    on: bool,
    passes: HashMap<String, IndexPass>,
}

/// Most often an account's index is read again for what its writer added (seconds): a pass
/// writes a generation every few seconds, and each read of it costs the doc pages it changed.
const READER_REFRESH: i64 = 2;
/// Units a search steps when it starts in the background: a small mailbox's search is there at
/// once. Few: the command takes them whatever the time (sixteen of a cold mailbox's, their
/// summaries' text made, held the frame searching for 20 ms); the window steps the rest.
const FIRST_UNITS: u32 = 4;
/// Summaries of a list a search replaced let go of a step (or a tick).
const LET_GO: usize = 4096;

/// An account's index as search reads it.
#[derive(Debug)]
pub(crate) struct ReaderSlot {
    /// `None`: it has none yet, or it couldn't be read.
    pub(crate) reader: Option<Reader>,
    /// When it was last read again (the session's clock).
    pub(crate) checked: i64,
    /// A newer generation was heard of ([`Event::Indexed`]).
    pub(crate) behind: bool,
    /// A search stopped while reading it: not read again until its writer writes it again.
    pub(crate) damaged: bool,
}

/// What search keeps between searches, and the one running.
#[derive(Default)]
pub(crate) struct SearchState {
    /// The search running, or done and still shown.
    pub(crate) run: Option<run::SearchRun>,
    /// Each folder's summaries' text, folded.
    pub(crate) texts: folder_text::FolderTexts,
    /// The texts of messages checked for a phrase, folded.
    pub(crate) folded: folded::FoldedTexts,
    /// The words looked up lately in each account's index.
    pub(crate) looked_up: look_up::LookedUp,
    /// Each account's index, by account.
    pub(crate) readers: HashMap<String, ReaderSlot>,
    /// The folders the search reads: kept in the cache while it runs (not once it's done).
    pub(crate) pinned: HashSet<(String, String)>,
    /// Searches don't wait: the window steps them each frame ([`Session::set_background_search`]).
    background: bool,
    pub(crate) stats: SearchStats,
    /// The list a search started over, let go of [`LET_GO`] summaries a step: letting go of tens
    /// of thousands at once (each with its strings) held the frame that searched for 10–20 ms.
    letting_go: Vec<Summary>,
    /// What a search cleared held of messages (found, answered), let go of as `letting_go` is:
    /// a run that found tens of thousands, dropped whole, held the frame that cleared it.
    letting_go_keys: Vec<MessageKey>,
    /// What its messages' texts said, let go of as `letting_go` is.
    letting_go_answers: Vec<matching::Answers>,
    /// The folder's list on view when the search started, shown again when it's cleared.
    pub(crate) kept: Option<kept::KeptList>,
    /// Bumped whenever cached mail changes: a kept list older than that is built again.
    pub(crate) changes: u64,
    /// A test's: the next step stops part-way, as a bug would.
    #[cfg(test)]
    pub(crate) fault: bool,
}

/// Working out which of the user's messages wait for a reply ([`Session::awaiting_replies`]),
/// a message at a time.
pub(crate) struct AwaitingScan {
    mine: HashSet<String>,
    /// Each selectable folder still to read, and whether it's a Sent folder.
    pub(crate) folders: Vec<(String, String, bool)>,
    newest: i64,
    oldest: i64,
    answered: HashSet<String>,
    sent: Vec<(MessageKey, String)>,
}

impl AwaitingScan {
    pub(crate) fn new(s: &Session) -> AwaitingScan {
        const DAY: i64 = 86_400;
        let mine = s
            .accounts
            .iter()
            .flat_map(|a| [a.config.email.trim().to_lowercase(), a.config.username.trim().to_lowercase()])
            .filter(|e| e.contains('@'))
            .collect();
        let folders = s
            .accounts
            .iter()
            .flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone(), f.role == FolderRole::Sent)))
            .collect();
        let (newest, oldest) = (s.clock.now.saturating_sub(AWAITING_AFTER_DAYS * DAY), s.clock.now.saturating_sub(AWAITING_WITHIN_DAYS * DAY));
        AwaitingScan { mine, folders, newest, oldest, answered: HashSet::new(), sent: Vec::new() }
    }

    /// Take a message of a folder into account (`sent`: a Sent folder's).
    pub(crate) fn take(&mut self, m: &Summary, sent: bool) {
        if !self.mine.contains(&m.from.email.trim().to_lowercase()) {
            self.answered.extend(std::iter::once(&m.in_reply_to).chain(&m.references).filter(|id| !id.is_empty()).cloned());
        } else if sent
            && !m.bulk
            && !m.message_id.is_empty()
            && (self.oldest..=self.newest).contains(&m.date)
            && m.to.iter().chain(&m.cc).any(|r| !self.mine.contains(&r.email.trim().to_lowercase()))
        {
            self.sent.push((m.key.clone(), m.message_id.clone()));
        }
    }

    pub(crate) fn finish(self) -> HashSet<MessageKey> {
        let answered = self.answered;
        self.sent.into_iter().filter(|(_, id)| !answered.contains(id)).map(|(k, _)| k).collect()
    }
}

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

    /// Searches started from now on don't wait: the window steps them a little each frame
    /// ([`Session::search_step`]) and shows what they find as they find it. Off, the default,
    /// a search is settled before it returns (the CLI, scripts and tests).
    pub fn set_background_search(&mut self, on: bool) {
        self.search_state.background = on;
    }

    /// Whether searches run in the background.
    pub fn background_search(&self) -> bool {
        self.search_state.background
    }

    /// A data file import into `account` is read now (begun, or under way).
    pub(super) fn import_started(&mut self, account: &str) {
        self.importing.set_live(account, true);
    }

    /// The import into `account` ended: its index is brought up to all it brought.
    pub(super) fn import_ended(&mut self, account: &str) {
        self.importing.set_live(account, false);
        self.want_index(account, None);
    }

    /// Keep each account's search index up to its mail in the background ([`Job::Index`] asked
    /// as mail changes; the window's network side runs them). Off, the default, none is asked
    /// for (the CLI and tests). Turned on, every account is brought up.
    pub fn set_indexing(&mut self, on: bool) {
        self.indexing.on = on;
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        for id in ids {
            self.want_index(&id, None);
        }
    }

    /// How `account`'s last index pass went, as far as it was heard of.
    pub fn index_pass(&self, account: &str) -> Option<IndexPass> {
        self.indexing.passes.get(account).copied()
    }

    /// The index passes under way, all told (`None`: none is): what the status bar shows, and
    /// how far a search that may be missing mail has to wait. An account being imported into
    /// has none (its import's end asks for one).
    pub fn indexing_work(&self) -> Option<mailcraft_model::job::Work> {
        let mut under_way = self
            .indexing
            .passes
            .iter()
            .filter(|(id, p)| !p.finished && !self.importing.running.contains_key(*id) && self.account(id).is_some())
            .map(|(_, p)| p.work)
            .peekable();
        under_way.peek()?;
        Some(under_way.fold(mailcraft_model::job::Work::default(), |all, w| mailcraft_model::job::Work {
            done: all.done.saturating_add(w.done),
            total: all.total.saturating_add(w.total),
            bytes_done: all.bytes_done.saturating_add(w.bytes_done),
            bytes_total: all.bytes_total.saturating_add(w.bytes_total),
        }))
    }

    /// What the search couldn't read of what was typed (a date that isn't one), in the order
    /// typed: left out of it, and said under the search box.
    pub fn search_problems(&self) -> &[mailcraft_model::search::Problem] {
        self.search_state.run.as_ref().map_or(&[], |r| r.query.problems.as_slice())
    }

    /// Ask for `account`'s index to be brought up to `folder` (`None`: every folder). Asks while
    /// jobs wait to be taken are one: folders add up, every folder takes them all. Not while a
    /// data file is imported into the account (its end asks for every folder).
    pub(super) fn want_index(&mut self, account: &str, folder: Option<&str>) {
        if !self.indexing.on || self.importing.live(account) {
            return;
        }
        let waiting = self.jobs.iter_mut().find_map(|j| match j {
            Job::Index { account: a, folders } if a == account => Some(folders),
            _ => None,
        });
        match (waiting, folder) {
            // Every folder already.
            (Some(folders), _) if folders.is_empty() => {}
            (Some(folders), None) => folders.clear(),
            (Some(folders), Some(f)) => {
                if !folders.iter().any(|x| x == f) {
                    folders.push(f.to_string());
                }
            }
            (None, folder) => self.jobs.push(Job::Index { account: account.to_string(), folders: folder.map(str::to_string).into_iter().collect() }),
        }
    }

    /// How an index pass is going, heard: a search that may be missing what it wrote reads
    /// it (at once when the pass is over, else at most every [`READER_REFRESH`] seconds).
    pub(super) fn indexed(&mut self, account: &str, pass: IndexPass) {
        self.indexing.passes.insert(account.to_string(), pass);
        let generation = self.search_state.readers.get(account).map(|s| s.reader.as_ref().map(Reader::generation));
        match generation {
            // Not read by any search yet: the next opens it as it is.
            None => return,
            Some(Some(g)) if g == pass.generation => return,
            Some(_) => {}
        }
        if let Some(slot) = self.search_state.readers.get_mut(account) {
            slot.behind = true;
            if pass.finished {
                slot.checked = i64::MIN;
            }
        }
        if let Some(mut run) = self.search_state.run.take() {
            self.follow_indexes(&mut run);
            self.search_state.run = Some(run);
            self.settle_search();
        }
    }

    /// A query as typed, made the user's: categories named, dates where the user is.
    fn query_of(&self, q: &str) -> Query {
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
        // `received:2024` is the year where the user is.
        query.resolve(&self.clock);
        query
    }

    /// The user's messages still waiting for a reply: in a Sent folder, sent between
    /// [`AWAITING_AFTER_DAYS`] and [`AWAITING_WITHIN_DAYS`] ago to someone else (not in bulk),
    /// and named by no message from anyone else, in any folder, as the one it answers
    /// (`In-Reply-To`) or follows (`References`).
    pub fn awaiting_replies(&mut self) -> HashSet<MessageKey> {
        let mut scan = AwaitingScan::new(self);
        for (account, folder, sent) in std::mem::take(&mut scan.folders) {
            for m in &self.folder_state(&account, &folder).messages {
                scan.take(m, sent);
            }
        }
        scan.finish()
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

    /// What the servers found for a search, here: each checked against what its summary can
    /// tell (dates, flags, waiting for a reply), which the servers weren't all asked; its words,
    /// which they were, are taken on trust. Words wanted in the senders, recipients or subject
    /// are the server's to match too: IMAP finds them inside words, as the list doesn't, and a
    /// find left out for it would be lost. (Unwanted ones, `-from:ana`, the server left out
    /// more widely than the list would.)
    /// `awaited`: the mail waiting for a reply, when the search worked it out.
    pub(super) fn server_hits(&mut self, q: &str, keys: &[MessageKey], awaited: Option<&HashSet<MessageKey>>) -> Vec<Summary> {
        if keys.is_empty() {
            return Vec::new();
        }
        let mut query = self.query_of(q);
        query.clauses.retain(|c| c.negated || !asks_field_words(&c.term));
        let awaiting = match awaited {
            Some(a) => a.clone(),
            None if query.clauses.iter().any(|c| asks_awaiting(&c.term)) => self.awaiting_replies(),
            None => HashSet::new(),
        };
        // Each folder's list looked through once for all its hits.
        let mut by_folder: HashMap<(String, String), HashSet<u32>> = HashMap::new();
        for k in keys {
            by_folder.entry((k.account.clone(), k.folder.clone())).or_default().insert(k.uid);
        }
        let mut out = Vec::new();
        for ((a, f), uids) in by_folder {
            for m in self.folder_state(&a, &f).messages.iter().filter(|m| uids.contains(&m.key.uid)) {
                let st = SummaryText::for_query(m, &query);
                let doc = Doc { summary: m, summary_text: &st, awaiting: awaiting.contains(&m.key), files: None, text: &matching::Unindexed };
                if query.eval(&doc) != Tri::No {
                    out.push(m.clone());
                }
            }
        }
        out
    }

    /// Search (an empty query: no search): how many messages the list shows. In the
    /// background ([`Session::set_background_search`]) that's what's found so far.
    pub fn run_search(&mut self, query: &str, scope: SearchScope) -> usize {
        self.search_as(query, scope, None)
    }

    /// [`Session::run_search`], waiting for it or not whatever searches do (`search.run`'s
    /// `background`).
    pub fn run_search_with(&mut self, query: &str, scope: SearchScope, background: bool) -> usize {
        let was = std::mem::replace(&mut self.search_state.background, background);
        let n = self.search_as(query, scope, None);
        self.search_state.background = was;
        n
    }

    /// A search, or (`folder`) a Search Folder's: a search typed in is never one, whatever it
    /// asks.
    pub(crate) fn search_as(&mut self, query: &str, scope: SearchScope, folder: Option<String>) -> usize {
        self.search_folder = folder;
        let q = query.trim();
        // The folder's list stays, to be shown again when the search is cleared.
        if self.search.is_none() && !q.is_empty() {
            self.keep_list();
        }
        self.search = if q.is_empty() { None } else { Some((q.to_string(), scope)) };
        if self.server_search.as_ref().is_some_and(|ss| ss.query != q) {
            self.stop_server_search();
        }
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        if q.is_empty() {
            // What the search showed and held is let go of a slice a tick, and the folder's
            // list shown again as it was when nothing changed since: clearing a search of
            // twenty or thirty thousand found held the frame for 50–90 ms, dropping them and
            // copying and arranging the folder's eleven thousand again.
            let shown = std::mem::take(&mut self.list);
            self.let_go(shown);
            self.drop_search();
            self.refresh_list();
            return self.list.len();
        }
        let wait = !self.search_state.background;
        self.start_search(q, scope, wait);
        if wait {
            self.settle_search();
        } else {
            // Nothing of the last search's results stays shown meanwhile (let go of a slice a
            // step).
            let shown = std::mem::take(&mut self.list);
            self.let_go(shown);
            self.refresh_list();
            self.search_step(FIRST_UNITS);
        }
        self.list.len()
    }

    /// Let go of `list` a slice a step and a tick, not at once.
    pub(crate) fn let_go(&mut self, list: Vec<Summary>) {
        if self.search_state.letting_go.is_empty() {
            self.search_state.letting_go = list;
        } else {
            self.search_state.letting_go.extend(list);
        }
    }

    /// Let go of a slice of the list a search replaced (each step and each tick).
    pub(crate) fn let_go_some(&mut self) {
        let_go_slice(&mut self.search_state.letting_go, LET_GO);
        let_go_slice(&mut self.search_state.letting_go_keys, LET_GO);
        // Each with its words, a few strings: fewer a slice.
        let_go_slice(&mut self.search_state.letting_go_answers, LET_GO / 4);
        fn let_go_slice<T>(v: &mut Vec<T>, n: usize) {
            let left = v.len().saturating_sub(n);
            v.truncate(left);
            if left == 0 && v.capacity() > 0 {
                *v = Vec::new();
            }
        }
    }

    /// How many summaries of a list a search replaced are still to be let go of.
    #[cfg(test)]
    pub(crate) fn letting_go(&self) -> usize {
        self.search_state.letting_go.len() + self.search_state.letting_go_keys.len() + self.search_state.letting_go_answers.len()
    }

    /// What their texts said, of the messages a search that ended checked, still to be let go
    /// of.
    #[cfg(test)]
    pub(crate) fn answers_letting_go(&self) -> usize {
        self.search_state.letting_go_answers.len()
    }

    /// The summaries and keys of lists a search replaced, still to be let go of (for the memory
    /// they hold, `bench::big_mailbox`).
    #[cfg(test)]
    pub(crate) fn letting_go_parts(&self) -> (&[Summary], &[MessageKey]) {
        (&self.search_state.letting_go, &self.search_state.letting_go_keys)
    }

    /// What the run found that the list doesn't show yet.
    #[cfg(test)]
    pub(crate) fn search_unshown(&self) -> Vec<MessageKey> {
        self.search_state.run.as_ref().map(|r| r.fresh.iter().map(|m| m.key.clone()).collect()).unwrap_or_default()
    }

    /// How many folders the search keeps in the cache.
    #[cfg(test)]
    pub(crate) fn search_pinned(&self) -> usize {
        self.search_state.pinned.len()
    }

    /// Whether `account`'s index is open for search.
    #[cfg(test)]
    pub(crate) fn search_index_open(&self, account: &str) -> bool {
        self.search_state.readers.get(account).is_some_and(|s| s.reader.is_some())
    }

    /// No search any more: what it held goes, its folders free to leave the cache.
    pub(crate) fn drop_search(&mut self) {
        if let Some(run) = self.search_state.run.take() {
            let state = &mut self.search_state;
            run.let_go_into(&mut state.letting_go, &mut state.letting_go_keys, &mut state.letting_go_answers);
        }
        self.search_state.pinned.clear();
    }

    /// Read again each index the run reads that a pass has written to since: at most every
    /// [`READER_REFRESH`] seconds, at once when the pass is over. One that changed has its
    /// words looked up again and its folders scanned again (what other accounts found stays).
    pub(crate) fn follow_indexes(&mut self, run: &mut run::SearchRun) {
        let Some(store) = self.store.clone() else { return };
        let now = self.clock.now;
        let accounts: HashSet<String> = run.folders.iter().map(|(a, _)| a.clone()).collect();
        for account in accounts {
            let Some(slot) = self.search_state.readers.get_mut(&account) else { continue };
            if !slot.behind || now.saturating_sub(slot.checked) < READER_REFRESH {
                continue;
            }
            slot.behind = false;
            slot.checked = now;
            slot.damaged = false;
            let changed = match slot.reader.as_mut() {
                Some(r) => match r.refresh(&store) {
                    Ok(changed) => changed,
                    Err(e) => {
                        log::warn!("{account}: the search index couldn't be read again: {e}");
                        slot.reader = Reader::open(&store, &account).ok().flatten();
                        true
                    }
                },
                None => {
                    slot.reader = Reader::open(&store, &account).ok().flatten();
                    slot.reader.is_some()
                }
            };
            if changed {
                run.rescan_account(&account);
            }
        }
    }
}

/// Whether a search term looks for words in the senders, recipients or subject (alone or among
/// others): what a server matches its own way (IMAP's FROM finds `ana` in "Joana").
fn asks_field_words(t: &mailcraft_model::search::Term) -> bool {
    use mailcraft_model::search::{Field, Term};
    match t {
        Term::Words(f, _) | Term::Phrase(f, _) => matches!(f, Field::From | Field::To | Field::Subject),
        Term::AnyOf(any) => !any.is_empty() && any.iter().all(asks_field_words),
        _ => false,
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
