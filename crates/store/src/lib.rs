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
pub use packed::growth as data_file_growth;
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub use packed::{fuzz_data_file, fuzz_data_folder};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use mailcraft_model::{AccountConfig, Folder, MessageKey, Op, SharedSummary, Summary, sort_newest_first};
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

impl StoreError {
    /// Whether the mail is open in another copy of MailCraft (a data file it has open, or its
    /// folder layout), which has to be closed first. Told by the error's kind, not its words: the
    /// CLI matched "another copy of MailCraft", which rewording here would have undone unseen.
    pub fn is_open_elsewhere(&self) -> bool {
        matches!(self, StoreError::Io { source, .. } if source.get_ref().is_some_and(|e| e.is::<OpenElsewhere>()))
    }
}

/// The error inside an [`io::Error`] saying the mail is open in another copy of MailCraft.
#[derive(Debug)]
struct OpenElsewhere(String);

impl std::fmt::Display for OpenElsewhere {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OpenElsewhere {}

/// `what` (words saying it's open in another copy of MailCraft) as the error
/// [`StoreError::is_open_elsewhere`] knows (only a copy on disk can be another's).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_elsewhere(what: String) -> io::Error {
    io::Error::other(OpenElsewhere(what))
}

/// A lock that outlives a panic elsewhere (what it guards is whole between steps).
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.display().to_string(), source }
}

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

/// An account's folder list with its counts, in the account's folder.
pub(crate) const FOLDERS: &str = "folders.json";

/// `value` as the JSON a cache file keeps, or an error naming the file it was for.
pub(crate) fn to_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|source| StoreError::Json { path: path.display().to_string(), source })
}

/// FNV-1a: a stable, dependency-free hash for folder directory names (folder names can hold
/// characters no file system accepts).
pub fn fnv64(s: &str) -> u64 {
    mailcraft_model::hash::fnv1a(s.as_bytes())
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
    /// The folder layout's folders lease, held while any clone lives (`leases.rs`): another
    /// copy's data files don't bring its folders in meanwhile.
    _folders: Option<Arc<leases::Lease>>,
}

impl Store {
    /// Open (creating if needed) a cache rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        Store::on_folders(root.into(), false)
    }

    /// [`Store::open`], kept from any other copy of MailCraft while this one has it (the app's):
    /// an error, "open in another copy of MailCraft", while another has it open.
    pub fn open_alone(root: impl Into<PathBuf>) -> Result<Store> {
        Store::on_folders(root.into(), true)
    }

    fn on_folders(root: PathBuf, alone: bool) -> Result<Store> {
        private_dir(&root)?;
        let folders = leases::folder_layout(&root, alone)?;
        Ok(Store::with(root, Files::Disk, folders))
    }

    /// A cache at `root` whose files are `files`, holding the folder layout's lease `folders`
    /// (when it has one): what every way of opening one shares.
    fn with(root: PathBuf, files: Files, folders: Option<Arc<leases::Lease>>) -> Store {
        Store {
            files: Routed::new(&root, files),
            root,
            changes: Arc::default(),
            reads: Arc::default(),
            search_writers: Arc::default(),
            removals: Arc::default(),
            marking: Arc::default(),
            _folders: folders,
        }
    }

    /// A cache whose accounts are each one data file in `files` (as Outlook keeps a `.pst` per
    /// account), with the account list and the app's own files in `root`. Account folders a
    /// cache at `root` had from before are brought into data files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        let packs = packed::Packs::open(&root, files.as_ref())?;
        Ok(Store::with(root, Files::Packed(Arc::new(packs)), None))
    }

    /// [`Store::open_packed`]'s cache with nothing opened yet: no data file, and no folder from
    /// before brought in. [`Store::open_data_files`] does both, on a thread of its own, while the
    /// window shows how it goes (an account's file opens when first used otherwise).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn packed(root: impl Into<PathBuf>, files: impl AsRef<Path>) -> Result<Store> {
        let root = root.into();
        private_dir(&root)?;
        let packs = packed::Packs::new(&root, files.as_ref())?;
        Ok(Store::with(root, Files::Packed(Arc::new(packs)), None))
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
        self.files.write(path, &to_json(path, value)?)
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
        self.read_json(&self.folders_path(account))
    }

    /// Where `account`'s folder list is kept.
    fn folders_path(&self, account: &str) -> PathBuf {
        self.account_dir(account).join(FOLDERS)
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
        self.write_json(&self.folders_path(account), folders)
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
#[cfg(not(target_arch = "wasm32"))]
use beside::IN_PLACE;
use beside::merging;
pub use beside::{Beside, Found, stamp as file_stamp};
use bodies::OUTBOX_REFUSED;
pub use bodies::{CommitTimes, Filing, LongWrite, OutboxItem, Prepared, Timed, Waiting};
pub use compacting::Shrunk;
use files::{Files, Routed};
use files::{atomic, leases};
pub use folders::{FolderLoad, FolderState, ListTail, ListWriter, PAGE_MESSAGES, TAIL_PAGE_MAX, TAIL_PAGES_MAX};
use folders::{StateFile, no_uids_left};
pub use mailcraft_model::job::DataFileHealth;
pub use marking::{FolderMarks, MarksAfter};
pub use merging::{Merged, SEEN_DOC};
pub use opening::{OpenProgress, OpenStage};

mod accounts;
mod beside;
mod bodies;
mod compacting;
mod files;
mod folders;
mod marking;
mod opening;

#[cfg(test)]
mod tests;
