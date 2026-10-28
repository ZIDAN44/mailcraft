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
#[cfg(not(target_arch = "wasm32"))]
pub use packed::EXTENSION as DATA_FILE_EXTENSION;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mailcraft_model::{AccountConfig, Folder, MessageKey, Op, Summary, sort_newest_first};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io { path: String, source: io::Error },
    #[error("{path}: {source}")]
    Json { path: String, source: serde_json::Error },
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
        store.files.write(&path, &bytes)?;
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

/// Where the files live.
#[derive(Clone, Debug)]
enum Files {
    Disk,
    /// Each account in one data file (`packed.rs`), the rest on disk.
    #[cfg(not(target_arch = "wasm32"))]
    Packed(Arc<packed::Packs>),
    /// The files, and the paths changed since they were last taken (when that's tracked).
    Memory(Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>, Arc<Mutex<Option<std::collections::BTreeSet<PathBuf>>>>),
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

impl Files {
    fn with<T>(m: &Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>, f: impl FnOnce(&mut BTreeMap<PathBuf, Vec<u8>>) -> T) -> T {
        let mut g = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }

    /// Note changed paths, when changes are tracked.
    fn mark<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        if let Files::Memory(_, changed) = self {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(set) = g.as_mut() {
                set.extend(paths.into_iter().map(Path::to_path_buf));
            }
        }
    }

    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        match self {
            Files::Disk => fs::read(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.read(p),
            Files::Memory(m, _) => Files::with(m, |m| m.get(p).cloned().ok_or_else(not_found)),
        }
    }

    fn write(&self, p: &Path, bytes: &[u8]) -> Result<()> {
        match self {
            Files::Disk => write_atomic(p, bytes),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.write(p, bytes),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.insert(p.to_path_buf(), bytes.to_vec()));
                self.mark([p]);
                Ok(())
            }
        }
    }

    /// Many files at once (importing a data file: desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    fn write_many(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        match self {
            Files::Packed(x) => x.write_many(items),
            _ => {
                for (p, bytes) in items {
                    self.write(p, bytes)?;
                }
                Ok(())
            }
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Files::Disk => atomic::rename(from, to),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.rename(from, to),
            Files::Memory(m, _) => {
                let moved = Files::with(m, |m| {
                    let moving: Vec<PathBuf> = m.keys().filter(|k| k.starts_with(from)).cloned().collect();
                    let mut moved = Vec::new();
                    for k in moving {
                        if let (Some(v), Ok(rest)) = (m.remove(&k), k.strip_prefix(from)) {
                            let new = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                            m.insert(new.clone(), v);
                            moved.extend([k, new]);
                        }
                    }
                    moved
                });
                if moved.is_empty() {
                    return Err(not_found());
                }
                self.mark(moved.iter().map(PathBuf::as_path));
                Ok(())
            }
        }
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        match self {
            Files::Disk => fs::remove_file(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.remove_file(p),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.remove(p).map(|_| ()).ok_or_else(not_found))?;
                self.mark([p]);
                Ok(())
            }
        }
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        match self {
            Files::Disk => fs::remove_dir_all(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.remove_dir_all(p),
            Files::Memory(m, _) => {
                let gone: Vec<PathBuf> = Files::with(m, |m| {
                    let gone: Vec<PathBuf> = m.keys().filter(|k| k.starts_with(p)).cloned().collect();
                    for k in &gone {
                        m.remove(k);
                    }
                    gone
                });
                if gone.is_empty() {
                    return Err(not_found());
                }
                self.mark(gone.iter().map(PathBuf::as_path));
                Ok(())
            }
        }
    }

    fn is_file(&self, p: &Path) -> bool {
        match self {
            Files::Disk => p.is_file(),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.is_file(p),
            Files::Memory(m, _) => Files::with(m, |m| m.contains_key(p)),
        }
    }

    fn exists(&self, p: &Path) -> bool {
        match self {
            Files::Disk => p.exists(),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.exists(p),
            Files::Memory(m, _) => Files::with(m, |m| m.keys().any(|k| k.starts_with(p))),
        }
    }

    /// The files directly in a folder, by name.
    fn list(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.list(dir),
            Files::Disk => {
                let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                    .collect()
            }
            Files::Memory(m, _) => Files::with(m, |m| {
                m.keys().filter(|k| k.parent() == Some(dir)).filter_map(|k| Some((k.file_name()?.to_str()?.to_string(), k.clone()))).collect()
            }),
        }
    }

    /// Every file under a folder, at any depth.
    fn walk(&self, dir: &Path) -> Vec<PathBuf> {
        fn disk(p: &Path, depth: usize, out: &mut Vec<PathBuf>) {
            let Ok(rd) = fs::read_dir(p) else { return };
            for e in rd.filter_map(|e| e.ok()) {
                match e.file_type() {
                    Ok(t) if t.is_dir() && depth < 8 => disk(&e.path(), depth + 1, out),
                    Ok(t) if t.is_file() => out.push(e.path()),
                    _ => {}
                }
            }
        }
        match self {
            Files::Disk => {
                let mut out = Vec::new();
                disk(dir, 0, &mut out);
                out
            }
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.walk(dir),
            Files::Memory(m, _) => Files::with(m, |m| m.keys().filter(|k| k.starts_with(dir)).cloned().collect()),
        }
    }

    /// Bytes under a folder.
    fn size(&self, dir: &Path) -> u64 {
        fn walk(p: &Path, depth: usize) -> u64 {
            let Ok(rd) = fs::read_dir(p) else { return 0 };
            rd.filter_map(|e| e.ok())
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() && depth < 8 => walk(&e.path(), depth + 1),
                    Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                    _ => 0,
                })
                .fold(0u64, u64::saturating_add)
        }
        match self {
            Files::Disk => walk(dir, 0),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.size(dir).unwrap_or_else(|| walk(dir, 0)),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.iter().filter(|(k, _)| k.starts_with(dir)).map(|(_, v)| v.len() as u64).fold(0u64, u64::saturating_add))
            }
        }
    }
}

/// Where each path goes: an account kept online only ([`Store::set_online_only`]) in memory,
/// everything else in the store's own files.
#[derive(Clone, Debug)]
struct Routed {
    root: PathBuf,
    main: Files,
    /// The online-only accounts' files: never written anywhere.
    held: Files,
    /// Those accounts (their folder names, `safe_segment`).
    online: Arc<Mutex<std::collections::BTreeSet<String>>>,
}

impl Routed {
    fn new(root: &Path, main: Files) -> Routed {
        Routed { root: root.to_path_buf(), main, held: Files::Memory(Arc::default(), Arc::default()), online: Arc::default() }
    }

    fn online(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<String>> {
        self.online.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The files a path is in: an online-only account's are held in memory.
    fn at(&self, p: &Path) -> &Files {
        let account = p.strip_prefix(&self.root).ok().and_then(|r| r.iter().next()).and_then(|a| a.to_str());
        match account {
            Some(a) if self.online().contains(a) => &self.held,
            _ => &self.main,
        }
    }

    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        self.at(p).read(p)
    }

    fn write(&self, p: &Path, bytes: &[u8]) -> Result<()> {
        self.at(p).write(p, bytes)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_many(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        let (held, main): (Vec<_>, Vec<_>) = items.iter().cloned().partition(|(p, _)| std::ptr::eq(self.at(p), &self.held));
        if !held.is_empty() {
            self.held.write_many(&held)?;
        }
        if main.is_empty() { Ok(()) } else { self.main.write_many(&main) }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.at(from).rename(from, to)
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.at(p).remove_file(p)
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        self.at(p).remove_dir_all(p)
    }

    fn is_file(&self, p: &Path) -> bool {
        self.at(p).is_file(p)
    }

    fn exists(&self, p: &Path) -> bool {
        self.at(p).exists(p)
    }

    fn list(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        self.at(dir).list(dir)
    }

    fn size(&self, dir: &Path) -> u64 {
        self.at(dir).size(dir)
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    files: Routed,
    /// Serializes read-modify-write of ops.json between the UI-side pool and account threads.
    changes: Arc<Mutex<()>>,
}

impl Store {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        Ok(Store { files: Routed::new(&root, Files::Disk), root, changes: Arc::default() })
    }

    /// A cache whose accounts are each one data file in `files` (as Outlook keeps a `.pst` per
    /// account), with the account list and the app's own files in `root`. Account folders a
    /// cache at `root` had from before are brought into data files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        let packs = packed::Packs::open(&root, files.as_ref())?;
        Ok(Store { files: Routed::new(&root, Files::Packed(Arc::new(packs))), root, changes: Arc::default() })
    }

    /// A cache kept in memory only (clones share it).
    pub fn memory() -> Store {
        let root = PathBuf::from("memory");
        Store { files: Routed::new(&root, Files::Memory(Arc::default(), Arc::default())), root, changes: Arc::default() }
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

    pub fn save_account_doc<T: Serialize + ?Sized>(&self, account: &str, name: &str, value: &T) -> Result<()> {
        self.write_json(&self.account_dir(account).join(safe_doc(name)), value)
    }

    pub fn load_accounts(&self) -> Result<Vec<AccountConfig>> {
        self.read_json(&self.root.join("accounts.json"))
    }

    pub fn save_accounts(&self, accounts: &[AccountConfig]) -> Result<()> {
        self.write_json(&self.root.join("accounts.json"), accounts)
    }

    pub fn load_folders(&self, account: &str) -> Result<Vec<Folder>> {
        self.read_json(&self.account_dir(account).join("folders.json"))
    }

    pub fn save_folders(&self, account: &str, folders: &[Folder]) -> Result<()> {
        self.write_json(&self.account_dir(account).join("folders.json"), folders)
    }
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

use atomic::private_dir;
pub use atomic::write_atomic;

mod accounts;
mod atomic;
mod bodies;
mod folders;

#[cfg(test)]
mod tests;
