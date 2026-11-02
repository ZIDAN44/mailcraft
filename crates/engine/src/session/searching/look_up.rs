//! Search: the query's words looked up in each account's index, a few pages a unit, and the
//! words looked up lately kept for the next search.
//!
//! Looking a word up reads (from the reader's cache, mostly) and decodes a lexicon page of each
//! of the index's segments, and the postings of each word starting so: 9-11 ms of a warm search
//! of the 41 GB file for a word found in 7,654 messages, about a quarter of it. The same word
//! searched again (from another folder, or the query run again) takes what the same generation
//! of the index gave the last time.

use std::collections::VecDeque;

use mailcraft_store::search::{Bits, Field, ScanCursor};

use super::run::{SearchRun, UNIT_PAGES};
use super::*;

/// Words kept looked up, each account's.
const LOOKED_UP: usize = 32;

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

    pub(super) fn ready(&self) -> bool {
        self.todo.is_empty()
    }

    /// `word` of `field` found in `bits`: the next word to look up.
    fn found(&mut self, field: Field, word: String, bits: Bits) {
        self.bits.retain(|(f, w, _)| *f != field || *w != word);
        self.bits.push((field, word, bits));
        if !self.todo.is_empty() {
            self.todo.remove(0);
        }
        self.cursor = ScanCursor::default();
    }
}

/// An account's words looked up lately, and the index they were looked up in (its epoch and
/// generation): another generation has them looked up again.
#[derive(Debug, Default)]
struct Lately {
    of: (u64, u64),
    words: VecDeque<(Field, String, Bits)>,
}

/// The words looked up lately, by account.
#[derive(Debug, Default)]
pub(crate) struct LookedUp {
    accounts: HashMap<String, Lately>,
}

impl LookedUp {
    /// The docs `word` of `field` was found in, in the index as it is (`of`), when it was looked
    /// up lately.
    fn get(&self, account: &str, of: (u64, u64), field: Field, word: &str) -> Option<&Bits> {
        let lately = self.accounts.get(account).filter(|l| l.of == of)?;
        lately.words.iter().find(|(f, w, _)| *f == field && w == word).map(|(_, _, b)| b)
    }

    /// Keep `word` of `field` looked up: the oldest go past [`LOOKED_UP`].
    fn keep(&mut self, account: &str, of: (u64, u64), field: Field, word: &str, bits: &Bits) {
        let lately = self.accounts.entry(account.to_string()).or_default();
        if lately.of != of {
            *lately = Lately { of, words: VecDeque::new() };
        }
        lately.words.retain(|(f, w, _)| *f != field || w != word);
        while lately.words.len() >= LOOKED_UP {
            lately.words.pop_front();
        }
        lately.words.push_back((field, word.to_string(), bits.clone()));
    }

    /// An account gone: its words go.
    pub(crate) fn forget(&mut self, account: &str) {
        self.accounts.remove(account);
    }
}

impl Session {
    /// Open `account`'s index for search, once: whether that took a unit.
    pub(super) fn open_reader(&mut self, account: &str) -> bool {
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

    /// Searching in the background (the window), an account's index opened a tick before anyone
    /// searches it: one account a tick, the first that has none open (or has one written since
    /// it had none). The first search's first steps then have its index open already (the one
    /// that opened it was often the slowest step of a search just after opening). Each index so
    /// opened stays open, and in memory, whether or not anyone searches. One a search stopped on
    /// (damaged) stays unread by ticks: the next search reads it again once its writer has
    /// written it. While a search runs, it opens and reads again what it reads itself, and one
    /// that has ended does so for its own accounts (`follow_indexes`), as it looks
    /// again at what the index then finds.
    pub(crate) fn open_indexes_ahead(&mut self) {
        let run = self.search_state.run.as_ref();
        if !self.search_state.background || self.store.is_none() || run.is_some_and(|r| !r.done()) {
            return;
        }
        let searched = |id: &String| run.is_some_and(|r| r.folders.iter().any(|(a, _)| a == id));
        let readers = &self.search_state.readers;
        let due = |id: &String| readers.get(id).is_none_or(|s| s.reader.is_none() && s.behind && !s.damaged && !searched(id));
        let Some(account) = self.accounts.iter().map(|a| &a.config.id).find(|id| due(id)).cloned() else { return };
        self.search_state.readers.remove(&account);
        // Read on the window's frame, where nothing else would catch it: an index that stops the
        // reading part-way is left unread (damaged) until its writer writes it again, as one a
        // search stopped on is, rather than closing the window at every start.
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            self.open_test_fault();
            self.open_reader(&account);
        }));
        if opened.is_err() {
            log::error!("{account}: the search index couldn't be opened: something in it couldn't be read");
            PANICS_CAUGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.search_state.readers.insert(account, ReaderSlot { reader: None, checked: self.clock.now, behind: false, damaged: true });
        }
    }

    /// A test's: the next index a tick opens ahead stops part-way, as a bug would.
    #[cfg(test)]
    pub(crate) fn fail_next_index_open(&mut self) {
        self.search_state.open_fault = true;
    }

    #[cfg(test)]
    fn open_test_fault(&mut self) {
        if std::mem::take(&mut self.search_state.open_fault) {
            panic!("a test's index fault");
        }
    }

    /// Look up the run's words in `account`'s index, a few pages; those looked up lately in the
    /// index as it is now, at once.
    pub(super) fn terms_unit(&mut self, run: &mut SearchRun, account: &str) {
        let (Some(store), Some(reader)) = (self.store.clone(), self.search_state.readers.get_mut(account).and_then(|s| s.reader.as_mut())) else {
            return;
        };
        let terms = run.terms.entry(account.to_string()).or_insert_with(|| Terms { todo: run.words.clone(), ..Default::default() });
        let of = (reader.epoch(), reader.generation());
        while terms.scanning.is_none()
            && let Some((field, word)) = terms.todo.first().cloned()
            && let Some(bits) = self.search_state.looked_up.get(account, of, field, &word)
        {
            terms.found(field, word, bits.clone());
        }
        let Some((field, word)) = terms.todo.first().cloned() else { return };
        let before = reader.pages_read();
        let mut bits = terms.scanning.take().unwrap_or_else(|| Bits::new(reader.len()));
        let done = reader.scan_prefix(&store, field, &word, &mut terms.cursor, &mut bits, UNIT_PAGES);
        let read = reader.pages_read().saturating_sub(before);
        self.search_state.stats.reader_pages = self.search_state.stats.reader_pages.saturating_add(read);
        match done {
            Ok(false) => terms.scanning = Some(bits),
            Ok(true) => {
                self.search_state.looked_up.keep(account, of, field, &word, &bits);
                terms.found(field, word, bits);
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
}
