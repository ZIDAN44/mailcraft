//! MailCraft's local-first cache.
//!
//! Everything the app shows comes from here, so mail stays readable and searchable offline. The
//! network layer writes what it syncs; the engine reads. Layout under the data folder:
//!
//! ```text
//! accounts.json                       account settings (never passwords)
//! <account>/folders.json              the folder list with counts
//! <account>/ops.json                  changes made offline, replayed when online (RFC 4549)
//! <account>/<folder-hash>/state.json  UIDVALIDITY, UIDNEXT, HIGHESTMODSEQ and the summaries
//! <account>/<folder-hash>/<validity>/<uid>.eml   downloaded message sources
//! ```
//!
//! Every file is written to a temporary name and renamed into place, so a crash or power cut
//! leaves either the old or the new version, never half of one.
//!
//! [`Store::memory`] keeps the same layout in memory instead, for the browser (which has no
//! files) and for tests; [`Store::snapshot`] and [`Store::memory_from`] carry what should
//! outlast the page.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod location;
#[cfg(not(target_arch = "wasm32"))]
mod packed;
pub mod search;
#[cfg(not(target_arch = "wasm32"))]
pub use packed::EXTENSION as DATA_FILE_EXTENSION;
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub use packed::fuzz_data_file;
#[cfg(not(target_arch = "wasm32"))]
pub use packed::growth as data_file_growth;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use mailcraft_model::{AccountConfig, Folder, MessageKey, Op, Summary, sort_newest_first};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("{path}: {source}")]
    Json { path: String, source: serde_json::Error },
    /// A file MailCraft wrote that doesn't read back as it should (the search index's): set
    /// aside and made again, never trusted.
    #[error("{path}: damaged ({what})")]
    Damaged { path: String, what: String },
    /// Something only one may do at a time is being done already (an account's search index
    /// written by an import and the indexer, or by another copy of MailCraft on the same folder).
    #[error("{path}: in use")]
    Busy { path: String },
}

pub type Result<T> = std::result::Result<T, StoreError>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.display().to_string(), source }
}

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
    pub messages: Vec<Summary>,
    /// How many of the newest messages it was filled with (the setting then): raised since,
    /// the older mail is fetched once.
    #[serde(default)]
    pub window: usize,
}

/// Folder lists longer than this are kept in pages of this many messages (by number, so new mail
/// changes the last page only, and a flag one page).
pub const PAGE_MESSAGES: usize = 4096;

/// Memory for each open data file's cache (an import's is [`BULK_CACHE`]). Outside an import a
/// transaction is small, so nothing is written out and read back whatever the cache: on the
/// 41 GB file's data file, opening its biggest folder, 40 Downs and a mark-read had no page read
/// back with 16, 32 or 48 MiB, and 2,745, 2,395 and 2,111 pages read from the file, in the same
/// time; the first search, which reads every message once, read within 1% as many with each. So
/// the memory isn't spent on every open file.
pub const CACHE: usize = 16 << 20;
/// What a data file's write transaction holds in memory before it has to write pages out and read
/// them back: half its cache (the database's rule).
pub const WRITE_BUFFER: usize = CACHE / 2;
/// The most a writer in the background (an import, the search index, a recompress) puts in one
/// transaction, as kept (compressed, [`Store::prepare`]): a quarter of the write buffer, and never
/// over 8 MiB. Each then holds the data file for a few milliseconds, so a write from the window
/// (a message marked read) waits at most that long; a 16 MiB one took 119 ms on a 37 GB file.
pub const BACKGROUND_COMMIT: usize = if WRITE_BUFFER / 4 < 8 << 20 { WRITE_BUFFER / 4 } else { 8 << 20 };
/// The cache of a data file taking a bulk write (an import, [`Store::bulk_begin`]). Importing a
/// file of 600,000 messages, 32 MiB had 58% fewer pages read back by its transactions than 16 MiB,
/// for about 22 MB more at its peak; 48, 64, 128 and 192 MiB took from 1 to 5 points more off
/// that for up to 225 MB more. On the 41 GB file, 74% fewer, and the import wrote 40.1 GB instead
/// of 45.3 for a 37.7 GB data file, for 4 to 26 MB more at its peak. Its batches stay at
/// [`BACKGROUND_COMMIT`]: 4 and 8 MiB ones with this cache read back 1.7 and 4 times as many
/// pages, and wrote no less.
pub const BULK_CACHE: usize = 32 << 20;

/// How a data file's cache has been used, as the database counts it: numbers only, for measuring,
/// and zeros (but `used_bytes`) unless the store is built with its `cache-metrics` feature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CacheUse {
    /// Pages put out of the cache to make room.
    pub evictions: u64,
    /// Pages read from the file, not found in the cache.
    pub read_misses: u64,
    /// Pages changed in a transaction that had been written out of it already and were read back.
    pub write_misses: u64,
    /// Bytes in the cache now.
    pub used_bytes: u64,
}

/// A folder's `state.json`: the folder's sync state and its list, or for a big folder the hashes
/// of the pages its list is kept in (`state-<n>.json`).
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StateFile {
    validity: u32,
    uid_next: u32,
    highest_modseq: u64,
    messages: Vec<Summary>,
    window: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pages: Vec<u64>,
    /// How many messages its pages hold.
    #[serde(skip_serializing_if = "is_zero_usize")]
    count: usize,
}

fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// A folder's list being written a page at a time (see [`Store::list_writer`]).
pub struct ListWriter {
    account: String,
    folder: String,
    /// Hashes of the pages written.
    pages: Vec<u64>,
    /// Messages not yet in a page.
    page: Vec<Summary>,
    total: usize,
    unread: u32,
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
        let path = store.folder_dir(&self.account, &self.folder).join(format!("state-{}.json", self.pages.len()));
        let p = path.display().to_string();
        let bytes = serde_json::to_vec(&self.page).map_err(|source| StoreError::Json { path: p, source })?;
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
            let mut st = FolderState { messages: std::mem::take(&mut self.page), ..head.clone() };
            sort_newest_first(&mut st.messages);
            store.save_folder(&self.account, &self.folder, &st)?;
            return Ok(counts);
        }
        if !self.page.is_empty() {
            self.write_page(store)?;
        }
        let dir = store.folder_dir(&self.account, &self.folder);
        let path = dir.join("state.json");
        let old: StateFile = store.read_json(&path).unwrap_or_default();
        let file = StateFile {
            validity: head.validity,
            uid_next: head.uid_next,
            highest_modseq: head.highest_modseq,
            messages: Vec::new(),
            window: head.window,
            count: self.total,
            pages: self.pages,
        };
        let p = path.display().to_string();
        let bytes = serde_json::to_vec(&file).map_err(|source| StoreError::Json { path: p, source })?;
        // A new list: no marks from an old one put on it.
        let marks = dir.join(folders::MARKS);
        let mut writes = vec![(path, bytes)];
        if store.has_marks(&marks) {
            writes.push((marks, b"[]".to_vec()));
        }
        store.write_files(writes)?;
        for i in file.pages.len()..old.pages.len() {
            let _ = store.files.remove_file(&dir.join(format!("state-{i}.json")));
        }
        Ok(counts)
    }
}

/// A page's hash (FNV-1a over its JSON), to tell which pages changed.
fn page_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

impl FolderState {
    pub fn find(&self, uid: u32) -> Option<&Summary> {
        self.messages.iter().find(|m| m.key.uid == uid)
    }

    pub fn find_mut(&mut self, uid: u32) -> Option<&mut Summary> {
        self.messages.iter_mut().find(|m| m.key.uid == uid)
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

fn no_uids_left(account: &str, folder: &str) -> StoreError {
    StoreError::Io { path: format!("{account}/{folder}"), source: io::Error::other("the folder has no message numbers left") }
}

/// A message waiting in the Outbox (its source is stored next to it as `<id>.eml`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: u64,
    /// Envelope sender and recipients (Bcc included: it isn't in the message).
    pub from: String,
    pub recipients: Vec<String>,
    /// Where a copy goes once sent.
    #[serde(default)]
    pub sent_folder: Option<String>,
    /// When it was queued (Unix seconds).
    #[serde(default)]
    pub queued: i64,
    #[serde(default)]
    pub attempts: u32,
    /// Why the last attempt failed.
    #[serde(default)]
    pub error: Option<String>,
    /// Held back until then (Unix seconds; 0 = send at once): Outlook's Delay Delivery.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub not_before: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// FNV-1a: a stable, dependency-free hash for folder directory names (folder names can hold
/// characters no file system accepts).
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Keep only characters that are safe in a path segment.
pub(crate) fn safe_segment(s: &str) -> String {
    let v: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    if v.is_empty() { format!("x{:x}", fnv64(s)) } else { v }
}

/// A path inside an account (`a/b/c`) from a data file, when every part of it is a plain name:
/// nothing that climbs out (`..`), no drive or root, no odd characters.
#[cfg(not(target_arch = "wasm32"))]
fn inner_path(key: &str) -> Option<PathBuf> {
    if key.is_empty() || key.len() > 512 {
        return None;
    }
    let mut out = PathBuf::new();
    for part in key.split('/') {
        let plain = !part.is_empty()
            && part != "."
            && part != ".."
            && part.len() <= 128
            && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !plain {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// A document name kept inside the data folder.
fn safe_doc(name: &str) -> String {
    let v: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).take(64).collect();
    let v = v.trim_start_matches('.').to_string();
    if v.is_empty() { "doc.json".into() } else { v }
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    files: Routed,
    /// Serializes read-modify-write of ops.json between the UI-side pool and account threads.
    changes: Arc<Mutex<()>>,
    /// How many message sources were read ([`Store::reads`]).
    reads: Arc<std::sync::atomic::AtomicU64>,
    /// The accounts whose search index is being written ([`search::Writer`]), and whether the
    /// account has gone meanwhile.
    search_writers: Arc<Mutex<search::Writers>>,
    /// How many times each account was removed ([`Store::while_kept`]).
    removals: Arc<RwLock<HashMap<String, u64>>>,
    /// Read marks kept on a thread of their own ([`Store::mark_later`]).
    marking: Arc<marking::Marking>,
}

impl Store {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        Ok(Store {
            files: Routed::new(&root, Files::Disk),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
        })
    }

    /// A cache whose accounts are each one data file in `files` (as Outlook keeps a `.pst` per
    /// account), with the account list and the app's own files in `root`. Account folders a
    /// cache at `root` had from before are brought into data files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        let packs = packed::Packs::open(&root, files.as_ref())?;
        Ok(Store {
            files: Routed::new(&root, Files::Packed(Arc::new(packs))),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
        })
    }

    /// [`Store::open_packed`]'s cache with nothing opened yet: no data file, and no folder from
    /// before brought in. [`Store::open_data_files`] does both, on a thread of its own, while the
    /// window shows how it goes (an account's file opens when first used otherwise).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        let packs = packed::Packs::new(&root, files.as_ref())?;
        Ok(Store {
            files: Routed::new(&root, Files::Packed(Arc::new(packs))),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
        })
    }

    /// A cache kept in memory only (clones share it).
    pub fn memory() -> Store {
        let root = PathBuf::from("memory");
        Store {
            files: Routed::new(&root, Files::Memory(Arc::default(), Arc::default())),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
        }
    }

    /// A memory cache whose files can be made to fail ([`Store::fail`]), for tests of what a
    /// store error loses.
    #[doc(hidden)]
    pub fn failing() -> Store {
        let root = PathBuf::from("memory");
        let faults = files::Faults { files: Files::Memory(Arc::default(), Arc::default()), reads: Mutex::default(), writes: Mutex::default() };
        Store {
            files: Routed::new(&root, Files::Failing(Arc::new(faults))),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
        }
    }

    /// From now on, a [`Store::failing`] cache fails to read the files whose path ends with
    /// `reads` (`state.json`, `.eml`), and to write, rename or remove those ending with `writes`;
    /// `None` lets them work again; a leading `!` makes them damaged rather than failing, and a
    /// leading `#` fails them as a full drive would. Other caches ignore it.
    #[doc(hidden)]
    pub fn fail(&self, reads: Option<&str>, writes: Option<&str>) {
        if let Files::Failing(f) = &self.files.main {
            *f.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = reads.map(str::to_string);
            *f.writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = writes.map(str::to_string);
        }
    }

    /// A memory cache holding files from an earlier [`Store::snapshot`].
    pub fn memory_from(files: Vec<(String, Vec<u8>)>) -> Store {
        let s = Store::memory();
        if let Files::Memory(m, _) = &s.files.main {
            // Paths stay inside the cache.
            let inside = files.into_iter().filter(|(p, _)| !p.split('/').any(|seg| seg.is_empty() || seg == "." || seg == ".."));
            let entries: BTreeMap<PathBuf, Vec<u8>> = inside.map(|(p, v)| (p.split('/').fold(s.root.clone(), |acc, seg| acc.join(seg)), v)).collect();
            Files::with(m, |m| m.extend(entries));
        }
        s
    }

    /// A memory cache's files whose relative path (`a1/folders.json`) `keep` accepts; empty for
    /// a cache on disk.
    pub fn snapshot(&self, keep: impl Fn(&str) -> bool) -> Vec<(String, Vec<u8>)> {
        let Files::Memory(m, _) = &self.files.main else { return Vec::new() };
        Files::with(m, |m| {
            m.iter()
                .filter_map(|(k, v)| {
                    let rel = k.strip_prefix(&self.root).ok()?;
                    let rel: Vec<&str> = rel.iter().map(|s| s.to_str()).collect::<Option<_>>()?;
                    Some((rel.join("/"), v))
                })
                .filter(|(p, _)| keep(p))
                .map(|(p, v)| (p, v.clone()))
                .collect()
        })
    }

    /// Whether the cache lives in memory.
    pub fn is_memory(&self) -> bool {
        matches!(self.files.main, Files::Memory(..))
    }

    /// Start noting which files of a memory cache change, for [`Store::take_changes`].
    pub fn track_changes(&self) {
        if let Files::Memory(_, changed) = &self.files.main {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.get_or_insert_with(Default::default);
        }
    }

    /// The files of a memory cache changed since the last call (once [`Store::track_changes`]
    /// started): relative path and the bytes now, `None` for a file removed. Lets the browser
    /// keep the cache between visits.
    pub fn take_changes(&self) -> Vec<(String, Option<Vec<u8>>)> {
        let Files::Memory(m, changed) = &self.files.main else { return Vec::new() };
        let paths = {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match g.as_mut() {
                Some(set) => std::mem::take(set),
                None => return Vec::new(),
            }
        };
        Files::with(m, |m| {
            paths
                .into_iter()
                .filter_map(|k| {
                    let rel = k.strip_prefix(&self.root).ok()?;
                    let rel: Vec<&str> = rel.iter().map(|s| s.to_str()).collect::<Option<_>>()?;
                    Some((rel.join("/"), m.get(&k).cloned()))
                })
                .collect()
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn folder_dir(&self, account: &str, folder: &str) -> PathBuf {
        self.account_dir(account).join(format!("f{:016x}", fnv64(folder)))
    }

    /// Where a message's source is cached.
    pub fn body_path(&self, key: &MessageKey) -> PathBuf {
        self.folder_dir(&key.account, &key.folder).join(key.validity.to_string()).join(format!("{}.eml", key.uid))
    }

    fn read_json<T: DeserializeOwned + Default>(&self, path: &Path) -> Result<T> {
        match self.files.read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(v) => Ok(v),
                Err(source) => {
                    // A damaged cache file is not worth failing over: it is rebuilt by the next
                    // sync. Keep the bad copy for diagnosis.
                    log::warn!("{}: unreadable, starting fresh: {source}", path.display());
                    let _ = self.files.rename(path, &path.with_extension("json.bad"));
                    Ok(T::default())
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
            Err(e) => Err(io_err(path)(e)),
        }
    }

    fn write_json<T: Serialize + ?Sized>(&self, path: &Path, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value).map_err(|source| StoreError::Json { path: path.display().to_string(), source })?;
        self.files.write(path, &bytes)
    }

    /// A small document at the top of the data folder (`rules.json`, `snoozes.json`); empty when
    /// missing or damaged. `name` is a plain file name.
    pub fn load_doc<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        self.read_json(&self.root.join(safe_doc(name)))
    }

    pub fn save_doc<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<()> {
        self.write_json(&self.root.join(safe_doc(name)), value)
    }

    /// A small document kept with an account's cache (removed with it).
    pub fn load_account_doc<T: DeserializeOwned + Default>(&self, account: &str, name: &str) -> Result<T> {
        self.read_json(&self.account_dir(account).join(safe_doc(name)))
    }

    /// An account's document as it's kept, for a reader that checks it itself (one that can't be
    /// read is left where it is, not set aside); `None` when there's none.
    pub fn load_account_doc_bytes(&self, account: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.account_dir(account).join(safe_doc(name));
        match self.files.read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&path)(e)),
        }
    }

    pub fn save_account_doc<T: Serialize + ?Sized>(&self, account: &str, name: &str, value: &T) -> Result<()> {
        self.write_json(&self.account_dir(account).join(safe_doc(name)), value)
    }

    pub fn load_accounts(&self) -> Result<Vec<AccountConfig>> {
        self.read_json(&self.root.join("accounts.json"))
    }

    pub fn save_accounts(&self, accounts: &[AccountConfig]) -> Result<()> {
        self.write_json(&self.root.join("accounts.json"), accounts)
    }

    /// An account's folder list, once the read marks handed over before (and their counts) are
    /// on disk ([`Store::mark_later`]).
    #[track_caller]
    pub fn load_folders(&self, account: &str) -> Result<Vec<Folder>> {
        self.flush_marks_of(account, None);
        self.read_folders(account)
    }

    /// [`Store::load_folders`] without waiting for the marks: with the lock held, which their
    /// thread takes.
    pub(crate) fn read_folders(&self, account: &str) -> Result<Vec<Folder>> {
        self.read_json(&self.account_dir(account).join("folders.json"))
    }

    /// Keep an account's folder list as it is now. It's written under the same lock as
    /// [`Store::upsert_folder`] and [`Store::update_folder_counts`], so a list read before one of
    /// those and written after can't be half of each.
    #[track_caller]
    pub fn save_folders(&self, account: &str, folders: &[Folder]) -> Result<()> {
        self.flush_marks_of(account, None);
        let _held = self.hold_changes();
        self.write_folders(account, folders)
    }

    /// [`Store::save_folders`] with the lock held already.
    fn write_folders(&self, account: &str, folders: &[Folder]) -> Result<()> {
        self.write_json(&self.account_dir(account).join("folders.json"), folders)
    }
}

/// The calling thread (the window's) is never kept waiting by a compaction that has an account's
/// data file to itself: what it reads of that file fails at once (`WouldBlock`, "being
/// compacted") until it's given back, rather than freezing the window. Writes still wait.
pub fn no_waiting() {
    #[cfg(not(target_arch = "wasm32"))]
    packed::no_waiting();
}

/// The app folder (the account list, the app's state): the per-user one
/// ([`location::base_root`]); `MAILCRAFT_DATA` overrides it. The accounts' data files are in
/// [`location::files_dir`].
pub fn default_root() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("MAILCRAFT_DATA").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    location::base_root()
}

/// A folder of this user's own for files MailCraft hands to other apps (an attachment opened with
/// its app, a video's frames): `name` inside it, made when missing. Not the shared temporary
/// folder on Linux and the BSDs, where `/tmp` is everyone's: a folder of a predictable name there
/// could be made first by another user, who could then read the attachments put in it or plant
/// links that a write would follow (PdfCraft and GridCraft found). There it is under
/// `$XDG_RUNTIME_DIR` (the user's own, 0700) or the user's cache folder, owner-only: the cache
/// also when the runtime folder can't be used (WSL can name one that isn't there, and no
/// attachment opened or dragged out). Windows' and macOS's temporary folders are the user's
/// already. `None` in the browser, or when it can't be made.
#[cfg(not(target_arch = "wasm32"))]
pub fn scratch_dir(name: &str) -> Option<PathBuf> {
    let dir = if cfg!(all(unix, not(target_os = "macos"))) {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute());
        let own = [runtime, dirs::cache_dir()].into_iter().flatten().find_map(|base| {
            let own = base.join("mailcraft");
            fs::create_dir_all(&own).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&own, fs::Permissions::from_mode(0o700)).ok()?;
            }
            Some(own)
        })?;
        own.join(name)
    } else {
        std::env::temp_dir().join(format!("mailcraft-{name}"))
    };
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// No files in the browser.
#[cfg(target_arch = "wasm32")]
pub fn scratch_dir(_name: &str) -> Option<PathBuf> {
    None
}

/// The [`scratch_dir`]s MailCraft puts mail in: attachments opened with other apps, and the
/// pictures and video frames made of them.
pub const SCRATCH: [&str; 3] = ["open", "thumbs", "frames"];

/// Remove what's in the [`SCRATCH`] folders (issue #4: nothing of an online-only account, or a
/// portable copy, stays behind). A file another app still has open stays.
pub fn clear_scratch() {
    for name in SCRATCH {
        if let Some(dir) = scratch_dir(name) {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

pub use accounts::Recompressed;
use atomic::private_dir;
pub use atomic::write_atomic;
pub use bodies::{CommitTimes, LongWrite, Prepared, Timed};
pub use compacting::Shrunk;
use files::{Files, Routed};
pub use folders::FolderLoad;
pub use mailcraft_model::job::DataFileHealth;
pub use marking::{FolderMarks, MarksAfter};
pub use opening::{OpenProgress, OpenStage};

mod accounts;
mod atomic;
mod bodies;
mod compacting;
mod files;
mod folders;
mod leases;
mod marking;
mod opening;

#[cfg(test)]
mod tests;
