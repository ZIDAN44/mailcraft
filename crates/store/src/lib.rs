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

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mailcraft_model::{AccountConfig, Folder, MessageKey, Op, Summary};
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
            Files::Disk => fs::rename(from, to),
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

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    files: Files,
}

impl Store {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(io_err(&root))?;
        Ok(Store { root, files: Files::Disk })
    }

    /// A cache whose accounts are each one data file in `files` (as Outlook keeps a `.pst` per
    /// account), with the account list and the app's own files in `root`. Account folders a
    /// cache at `root` had from before are brought into data files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(io_err(&root))?;
        let packs = packed::Packs::open(&root, files.as_ref())?;
        Ok(Store { root, files: Files::Packed(Arc::new(packs)) })
    }

    /// The data file an account is kept in, when the cache keeps them so.
    pub fn data_file(&self, account: &str) -> Option<PathBuf> {
        #[cfg(target_arch = "wasm32")]
        let _ = account;
        match &self.files {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.file(&safe_segment(account)),
            _ => None,
        }
    }

    /// The folder the data files are in, when the cache keeps them so.
    pub fn data_files_dir(&self) -> Option<PathBuf> {
        match &self.files {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => Some(x.dir().to_path_buf()),
            _ => None,
        }
    }

    /// A cache kept in memory only (clones share it).
    pub fn memory() -> Store {
        Store { root: PathBuf::from("memory"), files: Files::Memory(Arc::default(), Arc::default()) }
    }

    /// A memory cache holding files from an earlier [`Store::snapshot`].
    pub fn memory_from(files: Vec<(String, Vec<u8>)>) -> Store {
        let s = Store::memory();
        if let Files::Memory(m, _) = &s.files {
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
        let Files::Memory(m, _) = &self.files else { return Vec::new() };
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
        matches!(self.files, Files::Memory(..))
    }

    /// Start noting which files of a memory cache change, for [`Store::take_changes`].
    pub fn track_changes(&self) {
        if let Files::Memory(_, changed) = &self.files {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.get_or_insert_with(Default::default);
        }
    }

    /// The files of a memory cache changed since the last call (once [`Store::track_changes`]
    /// started): relative path and the bytes now, `None` for a file removed. Lets the browser
    /// keep the cache between visits.
    pub fn take_changes(&self) -> Vec<(String, Option<Vec<u8>>)> {
        let Files::Memory(m, changed) = &self.files else { return Vec::new() };
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

    /// Where an account's mail is kept (to show the user): its data file, or its folder. Before
    /// anything is written, the folder its data file goes in (its own folder never exists then).
    pub fn account_path(&self, account: &str) -> PathBuf {
        self.data_file(account).or_else(|| self.data_files_dir()).unwrap_or_else(|| self.account_dir(account))
    }

    fn account_dir(&self, account: &str) -> PathBuf {
        self.root.join(safe_segment(account))
    }

    /// Fill an account from an existing MailCraft data file (`.mailcraft`): its folders, mail,
    /// calendar and contacts, as they were. The file is only read (from a copy). Messages are
    /// renumbered as this account's; returns how many there are.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_data_file(&self, account: &str, path: &Path) -> Result<usize> {
        let dir = self.account_dir(account);
        let tmp = self.root.join(format!(".import-{}.tmp", safe_segment(account)));
        packed::read_data_file(path, &tmp, &mut |batch| {
            // Paths come from a file: kept only when they stay inside the account. The old
            // account's changes still to make on its server, and its unsent mail, stay behind.
            let items: Vec<(PathBuf, Vec<u8>)> = batch
                .into_iter()
                .filter(|(k, _)| k != "ops.json" && !k.starts_with("outbox/"))
                .filter_map(|(k, v)| inner_path(&k).map(|rel| (dir.join(rel), v)))
                .collect();
            self.files.write_many(&items)
        })?;
        // Folders and messages name the account they belong to: this one now.
        let mut folders = self.load_folders(account)?;
        if folders.iter().any(|f| f.account != account) {
            for f in &mut folders {
                f.account = account.to_string();
            }
            self.save_folders(account, &folders)?;
        }
        let mut messages = 0;
        for f in folders {
            let mut st = self.load_folder(account, &f.path)?;
            messages += st.messages.len();
            if st.messages.iter().any(|m| m.key.account != account) {
                for m in &mut st.messages {
                    m.key.account = account.to_string();
                }
                self.save_folder(account, &f.path, &st)?;
            }
        }
        Ok(messages)
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

    pub fn load_folder(&self, account: &str, folder: &str) -> Result<FolderState> {
        let mut st: FolderState = self.read_json(&self.folder_dir(account, folder).join("state.json"))?;
        // Keys are rebuilt rather than trusted: a renamed or copied cache stays consistent.
        for m in &mut st.messages {
            m.key.account = account.to_string();
            m.key.folder = folder.to_string();
            m.key.validity = st.validity;
        }
        Ok(st)
    }

    pub fn save_folder(&self, account: &str, folder: &str, state: &FolderState) -> Result<()> {
        self.write_json(&self.folder_dir(account, folder).join("state.json"), state)
    }

    /// Forget a folder's cache (UIDVALIDITY changed, or the folder was deleted on the server).
    pub fn clear_folder(&self, account: &str, folder: &str) -> Result<()> {
        let dir = self.folder_dir(account, folder);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// Bytes an account's cache takes on disk (message sources, summaries, the Outbox).
    pub fn account_size(&self, account: &str) -> u64 {
        self.files.size(&self.account_dir(account))
    }

    /// Forget everything about an account.
    pub fn remove_account(&self, account: &str) -> Result<()> {
        let dir = self.account_dir(account);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    pub fn read_body(&self, key: &MessageKey) -> Option<Vec<u8>> {
        self.files.read(&self.body_path(key)).ok()
    }

    pub fn has_body(&self, key: &MessageKey) -> bool {
        self.files.is_file(&self.body_path(key))
    }

    pub fn write_body(&self, key: &MessageKey, raw: &[u8]) -> Result<()> {
        self.files.write(&self.body_path(key), raw)
    }

    pub fn remove_body(&self, key: &MessageKey) {
        let _ = self.files.remove_file(&self.body_path(key));
    }

    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        self.read_json(&self.account_dir(account).join("ops.json"))
    }

    pub fn save_ops(&self, account: &str, ops: &[Op]) -> Result<()> {
        let path = self.account_dir(account).join("ops.json");
        if ops.is_empty() {
            match self.files.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&path)(e)),
                _ => return Ok(()),
            }
        }
        self.write_json(&path, ops)
    }

    /// Apply a change to the cached folders at once (optimistic update). Returns the folders that
    /// changed. Moves only remove from the source: the destination learns the new UID at its
    /// next sync.
    pub fn apply_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        if op.is_folder_op() {
            return self.apply_folder_op(account, op);
        }
        if let Op::EmptyFolder { path } = op {
            let mut st = self.load_folder(account, path)?;
            for m in &st.messages {
                self.remove_body(&m.key);
            }
            st.messages.clear();
            self.save_folder(account, path, &st)?;
            let mut list = self.load_folders(account)?;
            for f in list.iter_mut().filter(|f| &f.path == path) {
                f.total = 0;
                f.unread = 0;
            }
            self.save_folders(account, &list)?;
            return Ok(vec![path.clone()]);
        }
        let mut folders: Vec<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
        folders.sort();
        folders.dedup();
        for folder in &folders {
            let mut st = self.load_folder(account, folder)?;
            let in_folder = |k: &MessageKey| k.account == account && &k.folder == folder && k.validity == st.validity;
            match op {
                Op::SetFlag { keys, flag, on } => {
                    let uids: Vec<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        flag.set(&mut m.flags, *on);
                    }
                }
                Op::SetKeyword { keys, keyword, on } => {
                    let uids: Vec<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        m.flags.set_keyword(keyword, *on);
                    }
                }
                Op::Move { keys, to } => {
                    if to == folder {
                        continue;
                    }
                    let uids: Vec<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::Expunge { keys } => {
                    let gone: Vec<&MessageKey> = keys.iter().filter(|k| in_folder(k)).collect();
                    for k in &gone {
                        self.remove_body(k);
                    }
                    st.messages.retain(|m| !gone.iter().any(|k| k.uid == m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
            self.save_folder(account, folder, &st)?;
        }
        Ok(folders)
    }

    /// Folder-list changes: `folders.json` is updated and each folder's cache moves with it (or
    /// goes, for a deleted folder). Returns the folders whose contents should be shown afresh.
    fn apply_folder_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        let mut list = self.load_folders(account)?;
        let delimiter = list.iter().find_map(|f| f.delimiter);
        let changed = match op {
            Op::CreateFolder { path } => {
                if !list.iter().any(|f| &f.path == path) {
                    list.push(Folder {
                        account: account.to_string(),
                        path: path.clone(),
                        delimiter,
                        role: mailcraft_model::FolderRole::Other,
                        selectable: true,
                        unread: 0,
                        total: 0,
                    });
                }
                vec![path.clone()]
            }
            Op::RenameFolder { from, to } => {
                let mut moved = Vec::new();
                for f in list.iter_mut() {
                    if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                        let (src, dst) = (self.folder_dir(account, &f.path), self.folder_dir(account, &new));
                        if self.files.exists(&src) {
                            let _ = self.files.remove_dir_all(&dst);
                            self.files.rename(&src, &dst).map_err(io_err(&src))?;
                        }
                        f.path = new.clone();
                        moved.push(new);
                    }
                }
                moved
            }
            Op::DeleteFolder { path } => {
                let gone: Vec<String> =
                    list.iter().filter(|f| mailcraft_model::is_within(&f.path, path, f.delimiter.or(delimiter))).map(|f| f.path.clone()).collect();
                for g in &gone {
                    self.clear_folder(account, g)?;
                }
                list.retain(|f| !gone.contains(&f.path));
                Vec::new()
            }
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => Vec::new(),
        };
        mailcraft_model::sort_folders(&mut list);
        self.save_folders(account, &list)?;
        Ok(changed)
    }

    fn outbox_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join("outbox")
    }

    /// Queue a message to send when the account is back online.
    pub fn outbox_add(&self, account: &str, item: &OutboxItem, raw: &[u8]) -> Result<()> {
        let dir = self.outbox_dir(account);
        self.files.write(&dir.join(format!("{}.eml", item.id)), raw)?;
        self.write_json(&dir.join(format!("{}.json", item.id)), item)
    }

    /// Update a queued message's bookkeeping (attempts, last error).
    pub fn outbox_update(&self, account: &str, item: &OutboxItem) -> Result<()> {
        self.write_json(&self.outbox_dir(account).join(format!("{}.json", item.id)), item)
    }

    /// Queued messages, oldest first.
    pub fn outbox(&self, account: &str) -> Vec<OutboxItem> {
        let dir = self.outbox_dir(account);
        let mut items: Vec<OutboxItem> = self
            .files
            .list(&dir)
            .into_iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .filter_map(|(_, p)| self.files.read(&p).ok())
            .filter_map(|b| serde_json::from_slice::<OutboxItem>(&b).ok())
            .filter(|i| self.files.is_file(&dir.join(format!("{}.eml", i.id))))
            .collect();
        items.sort_by_key(|i| (i.queued, i.id));
        items
    }

    pub fn outbox_raw(&self, account: &str, id: u64) -> Option<Vec<u8>> {
        self.files.read(&self.outbox_dir(account).join(format!("{id}.eml"))).ok()
    }

    pub fn outbox_remove(&self, account: &str, id: u64) {
        let dir = self.outbox_dir(account);
        let _ = self.files.remove_file(&dir.join(format!("{id}.json")));
        let _ = self.files.remove_file(&dir.join(format!("{id}.eml")));
    }

    /// Cached sources across all folders of an account, as (key, path) pairs, for indexing.
    pub fn cached_bodies(&self, account: &str, folder: &str, validity: u32) -> Vec<u32> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        self.files.list(&dir).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
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

/// Write `bytes` to `path` via a temporary file and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(path)(e)
    })
}

#[cfg(test)]
mod tests {
    use mailcraft_model::{Flag, Flags};

    use super::*;

    pub(crate) fn temp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("mailcraft-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Store::open(dir).unwrap()
    }

    fn msg(uid: u32, seen: bool) -> Summary {
        Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
            subject: format!("m{uid}"),
            flags: Flags { seen, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn accounts_folders_and_state_round_trip() {
        let s = temp_store("rt");
        assert!(s.load_accounts().unwrap().is_empty());
        let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
        s.save_accounts(std::slice::from_ref(&acc)).unwrap();
        assert_eq!(s.load_accounts().unwrap(), vec![acc]);

        let f = Folder { account: "a1".into(), path: "Projets/Caf&AOk-".into(), delimiter: Some('/'), ..Default::default() };
        s.save_folders("a1", std::slice::from_ref(&f)).unwrap();
        assert_eq!(s.load_folders("a1").unwrap(), vec![f]);

        let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 77, messages: vec![msg(9, false), msg(4, true)], window: 0 };
        s.save_folder("a1", "INBOX", &st).unwrap();
        let back = s.load_folder("a1", "INBOX").unwrap();
        assert_eq!(back, st);
        assert_eq!(back.unread(), 1);

        let k = &st.messages[0].key;
        assert!(!s.has_body(k));
        s.write_body(k, b"Subject: x\r\n\r\nhi").unwrap();
        assert_eq!(s.read_body(k).unwrap(), b"Subject: x\r\n\r\nhi");
        assert_eq!(s.cached_bodies("a1", "INBOX", 3), vec![9]);
        s.clear_folder("a1", "INBOX").unwrap();
        assert!(!s.has_body(k));
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), FolderState::default());
        s.remove_account("a1").unwrap();
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn damaged_files_start_fresh() {
        let s = temp_store("bad");
        fs::write(s.root().join("accounts.json"), b"{not json").unwrap();
        assert!(s.load_accounts().unwrap().is_empty());
        assert!(s.root().join("accounts.json.bad").exists());
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn ops_apply_locally_and_persist() {
        let s = temp_store("ops");
        let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false), msg(4, false), msg(2, false)], window: 0 };
        s.save_folder("a1", "INBOX", &st).unwrap();
        let k9 = st.messages[0].key.clone();
        let k4 = st.messages[1].key.clone();

        let changed = s.apply_op("a1", &Op::SetFlag { keys: vec![k9.clone()], flag: Flag::Seen, on: true }).unwrap();
        assert_eq!(changed, vec!["INBOX".to_string()]);
        assert!(s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);

        s.apply_op("a1", &Op::Move { keys: vec![k4.clone()], to: "Archive".into() }).unwrap();
        assert!(s.load_folder("a1", "INBOX").unwrap().find(4).is_none());

        s.write_body(&k9, b"x").unwrap();
        s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()] }).unwrap();
        assert!(!s.has_body(&k9));
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

        // A stale key (old UIDVALIDITY) changes nothing.
        let mut stale = st.messages[2].key.clone();
        stale.validity = 1;
        s.apply_op("a1", &Op::Expunge { keys: vec![stale] }).unwrap();
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

        let ops = vec![Op::Move { keys: vec![k4], to: "Archive".into() }];
        s.save_ops("a1", &ops).unwrap();
        assert_eq!(s.load_ops("a1").unwrap(), ops);
        s.save_ops("a1", &[]).unwrap();
        assert!(s.load_ops("a1").unwrap().is_empty());
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn folder_ops_move_caches_with_folders() {
        let s = temp_store("folders");
        let f = |p: &str| Folder {
            account: "a1".into(),
            path: p.into(),
            delimiter: Some('/'),
            role: mailcraft_model::FolderRole::from_name(p, Some('/')),
            selectable: true,
            ..Default::default()
        };
        s.save_folders("a1", &[f("INBOX"), f("Projects"), f("Projects/Film")]).unwrap();
        let film = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 9 };
        s.save_folder("a1", "Projects/Film", &FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false)], window: 0 })
            .unwrap();
        s.write_body(&film, b"Subject: x\r\n\r\nfilm").unwrap();

        assert_eq!(s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap(), vec!["Clients".to_string()]);
        // Creating twice keeps one.
        s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap();
        assert_eq!(s.load_folders("a1").unwrap().iter().filter(|f| f.path == "Clients").count(), 1);

        // Renaming moves the subfolder and its cached mail along.
        let changed = s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Clients/Work".into() }).unwrap();
        assert_eq!(changed, vec!["Clients/Work".to_string(), "Clients/Work/Film".to_string()]);
        let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
        assert!(paths.contains(&"Clients/Work/Film".to_string()) && !paths.contains(&"Projects".to_string()), "{paths:?}");
        let moved = s.load_folder("a1", "Clients/Work/Film").unwrap();
        assert_eq!(moved.messages.len(), 1);
        assert_eq!(s.read_body(&moved.messages[0].key).unwrap(), b"Subject: x\r\n\r\nfilm");
        assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());

        // Deleting takes the subfolders and their caches.
        s.apply_op("a1", &Op::DeleteFolder { path: "Clients/Work".into() }).unwrap();
        let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(paths, vec!["INBOX".to_string(), "Clients".to_string()]);
        assert!(!s.has_body(&moved.messages[0].key));
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn outbox_queues_and_forgets() {
        let s = temp_store("outbox");
        assert!(s.outbox("a1").is_empty());
        let item = |id, queued| OutboxItem { id, from: "z@x".into(), recipients: vec!["a@y".into()], queued, ..Default::default() };
        s.outbox_add("a1", &item(7, 20), b"Subject: later\r\n\r\nx").unwrap();
        s.outbox_add("a1", &item(3, 10), b"Subject: first\r\n\r\nx").unwrap();
        let ids: Vec<u64> = s.outbox("a1").iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![3, 7], "oldest first");
        let mut i = s.outbox("a1")[0].clone();
        i.attempts = 2;
        i.error = Some("offline".into());
        s.outbox_update("a1", &i).unwrap();
        assert_eq!(s.outbox("a1")[0], i);
        assert_eq!(s.outbox_raw("a1", 7).unwrap(), b"Subject: later\r\n\r\nx");
        s.outbox_remove("a1", 3);
        assert_eq!(s.outbox("a1").len(), 1);
        // Damaged bookkeeping is skipped, not fatal.
        fs::write(s.root().join("a1").join("outbox").join("9.json"), b"{nope").unwrap();
        assert_eq!(s.outbox("a1").len(), 1);
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn hostile_names_stay_inside_the_root() {
        let s = temp_store("names");
        let key = MessageKey { account: "../../etc".into(), folder: "..\\..\\x".into(), validity: 1, uid: 1 };
        let p = s.body_path(&key);
        assert!(p.starts_with(s.root()));
        assert!(!p.to_string_lossy().contains(".."));
        let _ = fs::remove_dir_all(s.root());
    }

    #[test]
    fn a_memory_cache_reports_its_changes_once_asked() {
        let s = Store::memory();
        s.save_doc("rules.json", &vec![1]).unwrap();
        assert!(s.take_changes().is_empty(), "not tracked yet");
        s.save_folders("a1", &[Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() }]).unwrap();
        s.track_changes();
        s.save_doc("rules.json", &vec![2]).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "Projects".into(), validity: 1, uid: 4 };
        s.write_body(
            &key,
            b"Subject: x

hi",
        )
        .unwrap();
        let changes = s.take_changes();
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes.contains(&("rules.json".into(), Some(b"[2]".to_vec()))));
        let body = changes.iter().find(|(p, _)| p.ends_with("/4.eml")).unwrap();
        assert!(body.0.starts_with("a1/f"));
        assert!(s.take_changes().is_empty(), "taken");
        // A rename is a removal and a write; removing an account removes each file.
        s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
        let changes = s.take_changes();
        assert!(changes.iter().any(|(p, v)| p == &body.0 && v.is_none()), "{changes:?}");
        assert!(changes.iter().any(|(p, v)| p.ends_with("/4.eml") && p != &body.0 && v.is_some()));
        s.remove_account("a1").unwrap();
        assert!(s.take_changes().iter().filter(|(p, _)| p.ends_with(".eml")).all(|(_, v)| v.is_none()));
        // A store kept from them comes back the same.
        let back = Store::memory_from(s.snapshot(|_| true));
        assert_eq!(back.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![2]);
    }

    #[test]
    fn a_memory_cache_works_like_the_disk_and_keeps_a_snapshot() {
        let s = Store::memory();
        assert!(s.is_memory());
        let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
        s.save_accounts(std::slice::from_ref(&acc)).unwrap();
        let folders = vec![
            Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() },
            Folder { account: "a1".into(), path: "Projects/Film".into(), delimiter: Some('/'), ..Default::default() },
        ];
        s.save_folders("a1", &folders).unwrap();
        let st = FolderState { validity: 3, uid_next: 3, messages: vec![msg(2, false), msg(1, true)], ..Default::default() };
        s.save_folder("a1", "Projects/Film", &st).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 2 };
        s.write_body(
            &key,
            b"Subject: m2

hi",
        )
        .unwrap();
        assert!(s.has_body(&key));
        assert_eq!(s.cached_bodies("a1", "Projects/Film", 3), vec![2]);
        assert!(s.account_size("a1") > 0);

        // A renamed folder takes its cache with it, children too.
        s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
        assert_eq!(s.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
        let moved = MessageKey { folder: "Work/Film".into(), ..key.clone() };
        assert_eq!(
            s.read_body(&moved).unwrap(),
            b"Subject: m2

hi"
        );
        assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());
        s.apply_op("a1", &Op::SetFlag { keys: vec![MessageKey { uid: 2, ..moved.clone() }], flag: Flag::Seen, on: true }).unwrap();
        assert!(s.load_folder("a1", "Work/Film").unwrap().messages.iter().all(|m| m.flags.seen));

        let item = OutboxItem { id: 5, from: "z@x".into(), recipients: vec!["a@y".into()], ..Default::default() };
        s.outbox_add("a1", &item, b"x").unwrap();
        assert_eq!(s.outbox("a1"), vec![item]);
        s.outbox_remove("a1", 5);
        assert!(s.outbox("a1").is_empty());

        // What outlasts the page: here everything but message sources.
        let snap = s.snapshot(|p| !p.ends_with(".eml"));
        assert!(snap.iter().any(|(p, _)| p == "accounts.json"));
        assert!(snap.iter().all(|(p, _)| !p.ends_with(".eml")));
        let back = Store::memory_from(snap);
        assert_eq!(back.load_accounts().unwrap(), vec![acc]);
        assert_eq!(back.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
        assert!(!back.has_body(&moved));
        // Paths from a snapshot stay inside the cache.
        let odd = Store::memory_from(vec![("../accounts.json".into(), b"[]".to_vec()), ("a//b".into(), b"x".to_vec())]);
        assert!(odd.snapshot(|_| true).is_empty());

        s.remove_account("a1").unwrap();
        assert!(s.load_folders("a1").unwrap().is_empty() && s.account_size("a1") == 0);
        // Clones share the cache; a damaged document starts fresh.
        let other = s.clone();
        other.save_doc("rules.json", &vec![1, 2]).unwrap();
        assert_eq!(s.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![1, 2]);
        let bad = Store::memory_from(vec![("rules.json".into(), b"{oops".to_vec())]);
        assert!(bad.load_doc::<Vec<i32>>("rules.json").unwrap().is_empty());
    }
}
