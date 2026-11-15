//! A folder's list as the cache keeps it: its sync state, the `state.json` it's kept in (in
//! pages of numbered messages when it's big), and a long list written a page at a time.

use crate::*;

/// What the cache knows about one folder (RFC 4549 §4.3 and RFC 7162 state).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderState {
    /// UIDVALIDITY when the cache was filled; a different value on the server empties it.
    pub validity: u32,
    /// UIDNEXT at the last sync: messages at or above it are new.
    pub uid_next: u32,
    /// HIGHESTMODSEQ at the last sync (0 when the server lacks CONDSTORE).
    pub highest_modseq: u64,
    /// Newest first.
    pub messages: Vec<SharedSummary>,
    /// How many of the newest messages it was filled with (the setting then): raised since,
    /// the older mail is fetched once.
    #[serde(default)]
    pub window: usize,
}

/// Folder lists longer than this are kept in pages of this many messages (by number, so new mail
/// changes the last page only, and a flag one page).
pub const PAGE_MESSAGES: usize = 4096;

/// A folder's `state.json`: the folder's sync state and its list, or for a big folder the hashes
/// of the pages its list is kept in (`state-<n>.json`).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StateFile {
    pub(crate) validity: u32,
    pub(crate) uid_next: u32,
    pub(crate) highest_modseq: u64,
    pub(crate) messages: Vec<SharedSummary>,
    pub(crate) window: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) pages: Vec<u64>,
    /// How many messages its pages hold.
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub(crate) count: usize,
}

fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// A folder's list being written a page at a time (see [`Store::list_writer`]).
pub struct ListWriter {
    pub(super) account: String,
    pub(super) folder: String,
    /// Hashes of the pages written.
    pub(super) pages: Vec<u64>,
    /// Messages not yet in a page.
    pub(super) page: Vec<Summary>,
    pub(super) total: usize,
    pub(super) unread: u32,
}

/// A folder's list part-way through being written ([`ListWriter::tail`]): the hashes of the pages
/// written, the messages in them and how many of those are unread, and the messages not yet in a
/// page. Read back from a file, it's checked before it's used ([`Store::resume_list_writer`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListTail {
    pub pages: Vec<u64>,
    pub in_pages: u64,
    pub unread_in_pages: u32,
    pub page: Vec<Summary>,
}

impl ListTail {
    /// Whether it adds up: only full pages written, no more unread than messages in them, not
    /// too many of either, and the messages not yet in a page in rising number order.
    pub fn is_sound(&self) -> bool {
        let full = u64::try_from(self.pages.len()).ok().and_then(|n| n.checked_mul(PAGE_MESSAGES as u64));
        let rising = self.page.windows(2).all(|w| w.first().map(|m| m.key.uid) < w.get(1).map(|m| m.key.uid));
        self.pages.len() <= TAIL_PAGES_MAX
            && self.page.len() <= TAIL_PAGE_MAX
            && full == Some(self.in_pages)
            && u64::from(self.unread_in_pages) <= self.in_pages
            && rising
    }
}

/// A list's tail names at most this many pages written (4 billion messages), and holds at most
/// two pages' worth not yet in one.
pub const TAIL_PAGES_MAX: usize = 1_000_000;
pub const TAIL_PAGE_MAX: usize = 2 * PAGE_MESSAGES;

impl ListWriter {
    /// Add messages (in rising number order); full pages are written as they fill.
    pub fn push(&mut self, store: &Store, messages: Vec<Summary>) -> Result<()> {
        for m in messages {
            self.total += 1;
            if !m.flags.seen && !m.flags.deleted {
                self.unread = self.unread.saturating_add(1);
            }
            self.page.push(m);
            if self.page.len() == PAGE_MESSAGES {
                self.write_page(store)?;
            }
        }
        Ok(())
    }

    fn write_page(&mut self, store: &Store) -> Result<()> {
        let path = folders::page_path(&store.folder_dir(&self.account, &self.folder), self.pages.len());
        let bytes = to_json(&path, &self.page)?;
        store.files.write(&path, &bytes)?;
        self.pages.push(page_hash(&bytes));
        self.page.clear();
        Ok(())
    }

    /// Where the list has got to, with `more` (messages after those handed over, in rising number
    /// order) at its end: what an import notes to go on writing it later
    /// ([`Store::resume_list_writer`]). Nothing is written.
    pub fn tail(&self, more: &[Summary]) -> ListTail {
        let written = self.page.len();
        let unread_page = self.page.iter().filter(|m| !m.flags.seen && !m.flags.deleted).count();
        ListTail {
            pages: self.pages.clone(),
            in_pages: u64::try_from(self.total.saturating_sub(written)).unwrap_or(u64::MAX),
            unread_in_pages: self.unread.saturating_sub(u32::try_from(unread_page).unwrap_or(u32::MAX)),
            page: self.page.iter().chain(more).cloned().collect(),
        }
    }

    /// Write what's left and the list (with `head`'s sync state): its (unread, total).
    pub fn finish(mut self, store: &Store, head: &FolderState) -> Result<(u32, u32)> {
        let counts = (self.unread, u32::try_from(self.total).unwrap_or(u32::MAX));
        if self.pages.is_empty() {
            // Small: kept whole, as save_folder keeps it.
            let mut st = FolderState { messages: std::mem::take(&mut self.page).into_iter().map(SharedSummary::new).collect(), ..head.clone() };
            sort_newest_first(&mut st.messages);
            store.save_folder(&self.account, &self.folder, &st)?;
            return Ok(counts);
        }
        if !self.page.is_empty() {
            self.write_page(store)?;
        }
        // The head under the lock every head written takes, after the marks handed over before,
        // as save_folder writes it.
        store.flush_marks_of(&self.account, Some(&self.folder));
        let _held = store.hold_changes();
        let dir = store.folder_dir(&self.account, &self.folder);
        // Its pages are written already, over the old list's: the head is written whatever the old
        // one reads as, or the old head would name a mix of both lists' pages. Missing or damaged
        // reads as none; one that can't be read names no pages, so the ones this list no longer
        // has are found by looking for their files.
        let old: StateFile = match store.read_json(&dir.join(folders::HEAD)) {
            Ok(old) => old,
            Err(e) => {
                log::warn!("{}/{}: the list's old head not read: {e}", self.account, self.folder);
                let after = (self.pages.len()..TAIL_PAGES_MAX).take_while(|i| store.files.is_file(&folders::page_path(&dir, *i))).count();
                StateFile { pages: vec![0; self.pages.len().saturating_add(after)], ..StateFile::default() }
            }
        };
        let file = StateFile {
            validity: head.validity,
            uid_next: head.uid_next,
            highest_modseq: head.highest_modseq,
            messages: Vec::new(),
            window: head.window,
            count: self.total,
            pages: self.pages,
        };
        // A new list: no marks from an old one put on it.
        let mut writes = Vec::new();
        let stale = store.head_writes(&dir, &file, &old, &mut writes)?;
        store.write_files(writes)?;
        store.remove_stale(stale);
        Ok(counts)
    }
}

/// A page's hash (FNV-1a over its JSON), to tell which pages changed.
pub(super) fn page_hash(bytes: &[u8]) -> u64 {
    mailcraft_model::hash::fnv1a(bytes)
}

impl FolderState {
    pub fn find(&self, uid: u32) -> Option<&Summary> {
        self.messages.iter().find(|m| m.key.uid == uid).map(|m| &**m)
    }

    pub fn find_mut(&mut self, uid: u32) -> Option<&mut Summary> {
        self.messages.iter_mut().find(|m| m.key.uid == uid).map(SharedSummary::make_mut)
    }

    pub fn unread(&self) -> u32 {
        u32::try_from(self.messages.iter().filter(|m| !m.flags.seen && !m.flags.deleted).count()).unwrap_or(u32::MAX)
    }

    /// The number for the next message filed in `account`'s `folder` (this list): at least
    /// `uid_next`, past every number in the list, never 0; an error once they've run out (a
    /// damaged list at the last one).
    pub fn free_uid(&self, account: &str, folder: &str) -> Result<u32> {
        let past = match self.messages.iter().map(|m| m.key.uid).max() {
            Some(u) => u.checked_add(1).ok_or_else(|| no_uids_left(account, folder))?,
            None => 1,
        };
        Ok(self.uid_next.max(past).max(1))
    }

    /// Give `uid` to a message filed in `account`'s `folder` and move `uid` on: an error, to be
    /// returned before anything is written, when there's no number after it (two messages under
    /// one number are one source written over the other).
    pub fn take_uid(uid: &mut u32, account: &str, folder: &str) -> Result<u32> {
        let next = uid.checked_add(1).ok_or_else(|| no_uids_left(account, folder))?;
        Ok(std::mem::replace(uid, next))
    }
}

pub(crate) fn no_uids_left(account: &str, folder: &str) -> StoreError {
    StoreError::Io { path: format!("{account}/{folder}"), source: io::Error::other("the folder has no message numbers left") }
}
