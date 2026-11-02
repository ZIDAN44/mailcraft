//! The session: finding a message in the list without walking it. The list's index is built
//! with the list (`refresh_list`), so what's drawn each frame (the selection, the message read,
//! its conversation) is a lookup, not a pass over every message: on a folder of ten thousand
//! messages, each Down walked the list several times over.

use std::hash::{BuildHasherDefault, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// No conversation (in [`ListNav::thread_of_list`]).
const NO_THREAD: u32 = u32::MAX;

/// No row (in [`ListNav::msg_pos_of_list`] and [`ListNav::msg_rows`]).
pub(super) const NO_ROW: u32 = u32::MAX;

/// Where each message of [`Session::list`] is, and the conversation it's in.
///
/// Keys are hashed whole, and the index is built again with the list. Keys interned as (folder,
/// uid) and an index updated in place as a search's server hits come in are for when those hits
/// are merged into the list as they stream, rather than the list built again.
#[derive(Default)]
pub(crate) struct ListNav {
    /// A hash of each key, to the first place in the list with that key. Each lookup is checked
    /// against the list, so a hash shared by two keys, or a list changed by hand since, is
    /// looked through instead.
    pos: HashMap<u64, u32, BuildHasherDefault<Hashed>>,
    /// The list's length when this was built: another means the list changed since.
    len: usize,
    /// The hash of each key, by its place in the list when this was built: a conversation is
    /// trusted only while each of its messages is still where it was (a list changed by hand
    /// can keep its length).
    hashes: Vec<u64>,
    /// Each message's conversation, by its place in the list: an index into
    /// [`Session::threads`].
    thread_of_list: Vec<u32>,
    /// Conversations were on when this was built.
    conversations: bool,
    /// Conversations are on but the list shows none (arranged other than by date): the reading
    /// pane still shows the open message's, so they're worked out on the first look after the
    /// list is built ([`Session::thread_at`]), not by every rebuild a sync or a flag change makes.
    lazy: bool,
    /// Those conversations, once looked for, and each message's (as `thread_of_list`). Working
    /// them out for each message opened cost a pass over the list every Down.
    kept: OnceLock<(Vec<Thread>, Vec<u32>)>,
    /// How many times the list's conversations were worked out (for tests).
    threadings: AtomicU64,
    /// Summaries copied out of the list ([`Session::selected`]): what's drawn each frame should
    /// copy none (a select-all copied every message, three times a frame).
    clones: AtomicU64,
    /// The list as rows, with its headings (and conversations, when shown): built with the list
    /// and when the day turns, not for each step through it (each Down built them all again).
    pub(crate) rows: Vec<Row>,
    /// The message rows: where each is among `rows`.
    pub(crate) msg_rows: Vec<u32>,
    /// Each message's row, by its place in the list: its place in `msg_rows` (a conversation's
    /// messages all have its row).
    pub(crate) msg_pos_of_list: Vec<u32>,
    /// The local day the rows' date headings were made on.
    pub(crate) day: i64,
    /// Counted up each time the rows are built: what's laid out from them (the list's row
    /// tops) is made again only when this changes.
    pub(crate) structure: u64,
}

/// The keys of [`ListNav::pos`] are hashes already: taken as they are, not hashed again (half
/// of building the index of a list of tens of thousands).
#[derive(Default)]
pub(crate) struct Hashed(u64);

impl Hasher for Hashed {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// The hash a key is found by: the same on every run and in the browser. Its words mixed by
/// multiplying (as FxHash does), then spread (MurmurHash3's finish): each lookup is checked
/// against the list, so it needs to spread keys, not to resist anyone. SipHash
/// (`DefaultHasher`) took 130-150 ns a key, hashing the 35,000 a search found 5 ms of the frame
/// that showed them. Made from its folder's ([`folder_hash`]), which a search keeps for each
/// folder it reads: what it finds is hashed without its strings.
pub(crate) fn key_hash(k: &MessageKey) -> u64 {
    key_hash_in(folder_hash(&k.account, &k.folder), k.validity, k.uid)
}

/// The part of [`key_hash`] a folder's messages share.
pub(crate) fn folder_hash(account: &str, folder: &str) -> u64 {
    let mut h = Mix::default();
    h.text(account);
    h.text(folder);
    h.0
}

/// [`key_hash`] of a message of the folder whose [`folder_hash`] is `folder`.
pub(crate) fn key_hash_in(folder: u64, validity: u32, uid: u32) -> u64 {
    let mut h = Mix(folder);
    h.word(u64::from(validity) << 32 | u64::from(uid));
    h.finish()
}

/// [`key_hash`]'s state.
#[derive(Default)]
struct Mix(u64);

impl Mix {
    fn word(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    /// A string's bytes, 8 at a time, then its length (so where one ends is part of it).
    fn text(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let (words, rest) = bytes.as_chunks::<8>();
        for w in words {
            self.word(u64::from_le_bytes(*w));
        }
        let rest = rest.iter().rev().fold(0u64, |n, &b| n << 8 | u64::from(b));
        self.word(rest);
        self.word(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    }

    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^ (h >> 33)
    }
}

impl ListNav {
    /// Index `list`, whose conversations are `threads` (none: conversations off, or `lazy`: on
    /// but not shown, worked out when first needed).
    #[cfg(test)]
    pub(crate) fn build(&mut self, list: &[SharedSummary], threads: &[Thread], conversations: bool, lazy: bool) {
        let hashes = list.iter().map(|m| key_hash(&m.key)).collect();
        self.build_hashed(list, hashes, threads, conversations, lazy);
    }

    /// [`ListNav::build`], each key's hash given (`hashes`, in the list's order: those kept
    /// from before need no hashing again); hashed here when it isn't one a message.
    pub(crate) fn build_hashed(&mut self, list: &[SharedSummary], hashes: Vec<u64>, threads: &[Thread], conversations: bool, lazy: bool) {
        self.pos.clear();
        self.pos.reserve(list.len());
        self.hashes = if hashes.len() == list.len() { hashes } else { list.iter().map(|m| key_hash(&m.key)).collect() };
        for (i, &h) in self.hashes.iter().enumerate() {
            // Past u32: not indexed, looked for instead.
            let Ok(i) = u32::try_from(i) else { break };
            self.pos.entry(h).or_insert(i);
        }
        self.len = list.len();
        self.thread_of_list = thread_index(list.len(), threads);
        self.conversations = conversations;
        self.lazy = conversations && lazy;
        self.kept = OnceLock::new();
    }

    /// The list's conversations were worked out once more.
    pub(super) fn threaded(&self) {
        self.threadings.fetch_add(1, Ordering::Relaxed);
    }

    /// How many times the list's conversations were worked out.
    #[cfg(test)]
    pub(crate) fn threadings(&self) -> u64 {
        self.threadings.load(Ordering::Relaxed)
    }
}

/// Each message's conversation, by its place in a list of `len`: an index into `threads`.
fn thread_index(len: usize, threads: &[Thread]) -> Vec<u32> {
    let mut of = vec![NO_THREAD; len];
    for (t, thread) in threads.iter().enumerate() {
        let Ok(t) = u32::try_from(t) else { break };
        for &i in &thread.messages {
            if let Some(slot) = of.get_mut(i) {
                *slot = t;
            }
        }
    }
    of
}

impl Session {
    /// Where a message is in the list (the first place, should it be there twice).
    pub fn index_of(&self, key: &MessageKey) -> Option<usize> {
        self.index_of_hashed(key, key_hash(key))
    }

    /// [`Session::index_of`], the key's hash given (`h`, [`key_hash`]).
    fn index_of_hashed(&self, key: &MessageKey, h: u64) -> Option<usize> {
        if self.nav.len == self.list.len()
            && let Some(&i) = self.nav.pos.get(&h)
            && let Ok(i) = usize::try_from(i)
            && self.list.get(i).is_some_and(|m| &m.key == key)
        {
            return Some(i);
        }
        self.list.iter().position(|m| &m.key == key)
    }

    /// Whether a message is in the list: [`Session::index_of`], but a key the list's index
    /// doesn't have is taken as not there, rather than looked for over the whole list (the list
    /// is only set with its index built, by `take_list`). Asked for each of the thousands of
    /// messages a search finds in a step, the looking over cost more than the rest of showing them.
    pub(crate) fn listed(&self, key: &MessageKey) -> bool {
        self.listed_hashed(key, key_hash(key))
    }

    /// [`Session::listed`], the key's hash given (`h`, [`key_hash`]).
    pub(crate) fn listed_hashed(&self, key: &MessageKey, h: u64) -> bool {
        if self.nav_fresh() && !self.nav.pos.contains_key(&h) {
            return false;
        }
        self.index_of_hashed(key, h).is_some()
    }

    /// The hash of each key in the list, in its order, as its index was built: taken (to build it
    /// again with), so asked only when it's built again at once. `None` when it wasn't built for
    /// the list as it is.
    pub(crate) fn take_hashes(&mut self) -> Option<Vec<u64>> {
        self.nav_fresh().then(|| std::mem::take(&mut self.nav.hashes))
    }

    /// Whether the list's index was built for the list as it is (not one changed by hand since).
    pub(super) fn nav_fresh(&self) -> bool {
        self.nav.len == self.list.len()
    }

    pub fn summary(&self, key: &MessageKey) -> Option<&Summary> {
        self.index_of(key).and_then(|i| self.list.get(i)).map(|m| &**m)
    }

    /// The selected messages (in list order), copied. What's drawn each frame uses
    /// [`Session::selected_iter`] or [`Session::first_selected`], which copy nothing.
    pub fn selected(&self) -> Vec<Summary> {
        let out: Vec<Summary> = if self.selection.len() < 8 {
            let mut at: Vec<usize> = self.selection.iter().filter_map(|k| self.index_of(k)).collect();
            at.sort_unstable();
            at.dedup();
            at.into_iter().filter_map(|i| self.list.get(i)).map(|m| Summary::clone(m)).collect()
        } else {
            let chosen: HashSet<&MessageKey> = self.selection.iter().collect();
            self.list.iter().filter(|m| chosen.contains(&m.key)).map(|m| Summary::clone(m)).collect()
        };
        self.nav.clones.fetch_add(u64::try_from(out.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        out
    }

    /// The selected messages in the list, in the order they were selected, without copying them.
    pub fn selected_iter(&self) -> impl Iterator<Item = &Summary> + '_ {
        self.selection.iter().filter_map(|k| self.summary(k))
    }

    /// The selected message highest in the list.
    pub fn first_selected(&self) -> Option<&Summary> {
        self.selection.iter().filter_map(|k| self.index_of(k)).min().and_then(|i| self.list.get(i)).map(|m| &**m)
    }

    /// How many summaries [`Session::selected`] has copied out of the list (debug stats). Only
    /// those: a copy made any other way (`summary(k).cloned()`) isn't counted.
    pub fn summary_clones(&self) -> u64 {
        self.nav.clones.load(Ordering::Relaxed)
    }

    /// The conversation the message at `i` in the list is in, as the list was arranged.
    pub(crate) fn thread_at(&self, i: usize) -> Option<&Thread> {
        let fresh = self.nav.len == self.list.len() && self.nav.conversations;
        let (shown, of_list): (&[Thread], &[u32]) = if !fresh {
            (&[], &[])
        } else if self.nav.lazy {
            let (t, of) = self.nav.kept.get_or_init(|| {
                self.nav.threaded();
                let t = threads(&self.list);
                let of = thread_index(self.list.len(), &t);
                (t, of)
            });
            (t, of)
        } else {
            (&self.threads, &self.nav.thread_of_list)
        };
        // Each of its messages (`i` among them) still where it was when the list was indexed.
        let unmoved =
            |t: &&Thread| t.messages.iter().all(|&j| self.list.get(j).zip(self.nav.hashes.get(j)).is_some_and(|(m, &h)| key_hash(&m.key) == h));
        let indexed = of_list.get(i).and_then(|&t| shown.get(usize::try_from(t).ok()?)).filter(unmoved);
        indexed.or_else(|| self.threads.iter().find(|t| t.messages.contains(&i)))
    }

    /// The conversation a message is in, shown above it in the reading pane: only while the list
    /// is grouped by conversation (as Outlook's reading pane shows one message otherwise).
    pub(crate) fn thread_of(&self, key: &MessageKey) -> Vec<MessageKey> {
        if !self.prefs.conversations {
            return Vec::new();
        }
        let Some(i) = self.index_of(key) else { return Vec::new() };
        let members = |t: &Thread| -> Vec<MessageKey> { t.messages.iter().filter_map(|&j| self.list.get(j).map(|m| m.key.clone())).collect() };
        if let Some(t) = self.thread_at(i) {
            return members(t);
        }
        // Not worked out with the list (it was changed by hand since): worked out now.
        self.nav.threaded();
        threads(&self.list).iter().find(|t| t.messages.contains(&i)).map(members).unwrap_or_default()
    }
}

impl Session {
    /// The list as rows with date headings (and conversations, when on), as it was built.
    pub fn rows(&self) -> &[Row] {
        &self.nav.rows
    }

    /// Counted up each time the rows are built (the list built again, a new day): what the
    /// interface lays out from them is kept until it changes.
    pub fn nav_structure(&self) -> u64 {
        self.nav.structure
    }

    /// Build the rows again from the list (`refresh_list`, a new day).
    pub(crate) fn rebuild_rows(&mut self) {
        let (rows, msg_rows, msg_pos_of_list) = self.build_rows();
        self.nav.rows = rows;
        self.nav.msg_rows = msg_rows;
        self.nav.msg_pos_of_list = msg_pos_of_list;
        self.nav.day = mailcraft_model::dates::today(&self.clock);
        self.nav.structure = self.nav.structure.wrapping_add(1);
    }

    /// The day turned since the rows were built: their date headings move with it (what was
    /// Today is Yesterday). Cheap otherwise; [`Session::tick`] asks.
    pub(crate) fn follow_the_day(&mut self) {
        if mailcraft_model::dates::today(&self.clock) != self.nav.day {
            self.rebuild_rows();
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// How many message rows the list shows.
    pub(crate) fn row_count(&self) -> usize {
        self.nav.msg_rows.len()
    }

    /// The key of message row `p` (a conversation's: its newest message's).
    pub(crate) fn row_key(&self, p: usize) -> Option<&MessageKey> {
        let r = usize::try_from(*self.nav.msg_rows.get(p)?).ok()?;
        match self.nav.rows.get(r)? {
            Row::Message { index, .. } => self.list.get(*index).map(|m| &m.key),
            _ => None,
        }
    }

    /// The message row a message is on: its own, or its conversation's (one read from the
    /// conversation shown above it in the reading pane has no row of its own).
    pub(crate) fn msg_row_of(&self, key: &MessageKey) -> Option<usize> {
        let i = self.index_of(key)?;
        if self.nav.len == self.list.len() && self.nav.hashes.get(i) == Some(&key_hash(key)) {
            let p = *self.nav.msg_pos_of_list.get(i)?;
            return if p == NO_ROW { None } else { usize::try_from(p).ok() };
        }
        // The list was changed by hand since its rows were built: looked for among them, its
        // own row or else its conversation's (an older message has none of its own).
        (0..self.row_count()).find(|&p| self.row_key(p) == Some(key)).or_else(|| {
            let members = self.thread_of(key);
            (0..self.row_count()).find(|&p| self.row_key(p).is_some_and(|k| members.contains(k)))
        })
    }

    /// Where a message's row is among [`Session::rows`] (its conversation's, for one in a
    /// conversation), to scroll to.
    pub fn row_index_of(&self, key: &MessageKey) -> Option<usize> {
        let p = self.msg_row_of(key)?;
        self.nav.msg_rows.get(p).and_then(|&r| usize::try_from(r).ok())
    }
}

/// A key's hash made from its folder's, as a search makes it for what it finds, is the one made
/// from the key; where one string ends and the next begins is part of it.
#[test]
fn a_key_hashes_alike_from_its_folder() {
    let k = |account: &str, folder: &str, uid| MessageKey { account: account.into(), folder: folder.into(), validity: 7, uid };
    for (a, f) in [("a1", "INBOX"), ("", ""), ("an account name past eight", "Folder/Sub ü")] {
        assert_eq!(key_hash(&k(a, f, 9)), key_hash_in(folder_hash(a, f), 7, 9));
    }
    let hashes: HashSet<u64> =
        [k("ab", "c", 1), k("a", "bc", 1), k("abc", "", 1), k("", "abc", 1), k("a1", "INBOX", 1), k("a1", "INBOX", 2)].iter().map(key_hash).collect();
    assert_eq!(hashes.len(), 6);
    let spread: HashSet<u64> = (0..10_000).map(|uid| key_hash(&k("a1", "INBOX", uid)) >> 52).collect();
    assert!(spread.len() > 3500, "{} of 4,096 top bits' values", spread.len());
}
