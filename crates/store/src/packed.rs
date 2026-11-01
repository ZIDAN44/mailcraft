//! Each account in one data file, as Outlook keeps each account in a `.pst`.
//!
//! The store's paths stay the same: `<root>/<account>/<path>`. Under this backend everything in
//! an account's folder lives in `<files>/<name>.mailcraft`, a crash-safe single-file database
//! (redb) with one table from path to bytes; files directly in `<root>` (the account list, the
//! app's state) stay ordinary files there. `<files>` is "MailCraft Files" in Documents unless
//! moved (`location.rs`); which file is whose is kept in `<root>/datafiles.json`.
//!
//! Data files are hostile input like everything else: one that can't be read is set aside
//! (`.damaged`) and a new one started, never a crash. Folders from before (an account's folder
//! of files in `<root>`) are brought into its data file when the store opens, and removed only
//! once every file is in and read back. The app opens its data files on a thread of its own
//! (`open.rs`), as one not closed properly is checked as it opens, which can take minutes.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::{CACHE, OpenProgress, Prepared, Result, StoreError, io_err, write_atomic};

mod commit;
mod kept;
mod open;
use commit::*;
use kept::*;
pub(crate) use kept::{pack_owned, unpack};
use open::*;

/// A data file's extension.
pub const EXTENSION: &str = "mailcraft";
/// Which data file is whose (in the root).
const NAMES: &str = "datafiles.json";

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Where a store path lives.
enum Place {
    /// An ordinary file (directly in the root, or outside it).
    Disk,
    /// An account's folder itself.
    Account(String),
    /// A path inside an account's folder.
    In(String, String),
}

pub(crate) struct Packs {
    root: PathBuf,
    dir: PathBuf,
    open: Mutex<Open>,
    /// An account's data file opened (or not), for those waiting on it.
    opened: Condvar,
    names: Mutex<BTreeMap<String, String>>,
    /// Data files written since they were sealed.
    seals: Arc<Seals>,
    /// How opening the data files goes (and a check of one, whoever opens it).
    pub(crate) progress: Arc<OpenProgress>,
}

impl std::fmt::Debug for Packs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packs").field("root", &self.root).field("dir", &self.dir).finish_non_exhaustive()
    }
}

impl Packs {
    /// Data files for the store at `root`, kept in `dir`; folders from before are brought in.
    pub(crate) fn open(root: &Path, dir: &Path) -> Result<Packs> {
        let p = Packs::new(root, dir)?;
        p.bring_in_folders()?;
        Ok(p)
    }

    /// Data files for the store at `root`, kept in `dir`, none of them opened yet.
    pub(crate) fn new(root: &Path, dir: &Path) -> Result<Packs> {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let names = fs::read(root.join(NAMES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Ok(Packs {
            root: root.to_path_buf(),
            dir: dir.to_path_buf(),
            open: Mutex::default(),
            opened: Condvar::new(),
            names: Mutex::new(names),
            seals: Arc::default(),
            progress: Arc::default(),
        })
    }

    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    fn place(&self, p: &Path) -> Place {
        let Ok(rel) = p.strip_prefix(&self.root) else { return Place::Disk };
        let parts: Vec<&str> = rel.iter().filter_map(|s| s.to_str()).collect();
        match parts.as_slice() {
            [] => Place::Disk,
            // Account folders are plain names (`safe_segment`); the root's own files have a dot.
            [one] if !one.contains('.') => Place::Account((*one).to_string()),
            [_] => Place::Disk,
            [account, rest @ ..] => Place::In((*account).to_string(), rest.join("/")),
        }
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The data file of an account, when it has one (or is named already).
    pub(crate) fn file(&self, account: &str) -> Option<PathBuf> {
        Packs::lock(&self.names).get(account).map(|n| self.dir.join(n))
    }

    /// The name of an account's data file, chosen the first time: its address, as Outlook names
    /// a `.pst` (an opened data file keeps that file's name).
    fn name(&self, account: &str) -> String {
        if let Some(n) = Packs::lock(&self.names).get(account) {
            return n.clone();
        }
        let email = fs::read(self.root.join("accounts.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<mailcraft_model::AccountConfig>>(&b).ok())
            .and_then(|list| list.into_iter().find(|a| crate::safe_segment(&a.id) == account).map(|a| a.email));
        let stem = email.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or(account);
        let stem = stem.strip_suffix(".pst").or_else(|| stem.strip_suffix(".ost")).or_else(|| stem.strip_suffix(".mailcraft")).unwrap_or(stem);
        let stem: String = stem
            .chars()
            .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
            .take(80)
            .collect();
        let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
        let stem = if stem.is_empty() { account } else { stem };
        let mut names = Packs::lock(&self.names);
        let taken = |n: &str| names.values().any(|v| v.eq_ignore_ascii_case(n)) || self.dir.join(n).exists();
        let mut name = format!("{stem}.{EXTENSION}");
        let mut i = 2;
        while taken(&name) {
            name = format!("{stem} ({i}).{EXTENSION}");
            i += 1;
        }
        names.insert(account.to_string(), name.clone());
        if let Ok(json) = serde_json::to_vec_pretty(&*names) {
            let _ = write_atomic(&self.root.join(NAMES), &json);
        }
        name
    }

    fn get(&self, account: &str, key: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(db) = self.db(account, false)? else { return Ok(None) };
        let tx = db.begin_read().map_err(other)?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(other(e)),
        };
        read_file(&tx, &table, key)
    }

    /// Whether a file is there (without reading it).
    fn has(&self, account: &str, key: &str) -> io::Result<bool> {
        let Some(db) = self.db(account, false)? else { return Ok(false) };
        let tx = db.begin_read().map_err(other)?;
        match tx.open_table(FILES) {
            Ok(t) => Ok(t.get(key).map_err(other)?.is_some()),
            Err(redb::TableError::TableDoesNotExist(_)) => Ok(false),
            Err(e) => Err(other(e)),
        }
    }

    /// Whether any file is under `prefix` (a folder), from the first key alone.
    fn has_under(&self, account: &str, prefix: &str) -> io::Result<bool> {
        let Some(db) = self.db(account, false)? else { return Ok(false) };
        let tx = db.begin_read().map_err(other)?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(false),
            Err(e) => return Err(other(e)),
        };
        let start = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let first = table.range(start.as_str()..).map_err(other)?.next();
        match first {
            Some(item) => Ok(item.map_err(other)?.0.value().starts_with(start.as_str())),
            None => Ok(false),
        }
    }

    /// The names of the files directly in `prefix` (a folder: `""` for the account's own), by key
    /// alone: what they hold isn't read (`cached_bodies` lists a validity folder of messages this
    /// way), and a folder inside is passed over in one step, not walked file by file (no listing
    /// today is of a folder with others inside; this keeps one from costing what they hold).
    /// Also how many keys it looked at.
    fn children(&self, account: &str, prefix: &str) -> io::Result<(Vec<String>, usize)> {
        let Some(db) = self.db(account, false)? else { return Ok((Vec::new(), 0)) };
        let tx = db.begin_read().map_err(other)?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok((Vec::new(), 0)),
            Err(e) => return Err(other(e)),
        };
        let start = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let (mut out, mut looked) = (Vec::new(), 0usize);
        let mut from = start.clone();
        'seek: loop {
            for item in table.range(from.as_str()..).map_err(other)? {
                let (k, _) = item.map_err(other)?;
                looked = looked.saturating_add(1);
                let Some(name) = k.value().strip_prefix(start.as_str()) else { break 'seek };
                match name.split_once('/') {
                    None => out.push(name.to_string()),
                    // A folder: on past everything in it ('0' comes right after '/').
                    Some((inner, _)) => {
                        from = format!("{start}{inner}0");
                        continue 'seek;
                    }
                }
            }
            break;
        }
        Ok((out, looked))
    }

    /// Keys under `prefix` (a folder: `""` for the account's own), with their sizes.
    fn keys(&self, account: &str, prefix: &str) -> io::Result<Vec<(String, u64)>> {
        let Some(db) = self.db(account, false)? else { return Ok(Vec::new()) };
        let tx = db.begin_read().map_err(other)?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(other(e)),
        };
        let start = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let mut out = Vec::new();
        for item in table.range(start.as_str()..).map_err(other)? {
            let (k, v) = item.map_err(other)?;
            let k = k.value();
            if !k.starts_with(&start) {
                break;
            }
            let v = v.value();
            out.push((k.to_string(), pieced_len(v).unwrap_or(v.len() as u64)));
        }
        Ok(out)
    }

    /// Change an account's data file in one transaction, durable at once (sealed once quiet).
    fn change(&self, account: &str, f: impl FnOnce(&mut Tables<'_>) -> io::Result<()>) -> io::Result<()> {
        self.change_with(account, Commit::PLAIN, 0, f)
    }

    /// Write many files at once: one transaction per data file (a commit each was slow).
    pub(crate) fn write_many(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        let mut by_account: std::collections::BTreeMap<String, Vec<(String, &[u8])>> = std::collections::BTreeMap::new();
        for (p, bytes) in items {
            match self.place(p) {
                Place::In(a, k) => by_account.entry(a).or_default().push((k, bytes.as_slice())),
                Place::Account(_) | Place::Disk => write_atomic(p, bytes)?,
            }
        }
        for (a, files) in by_account {
            self.change(&a, |t| {
                for (k, bytes) in &files {
                    t.put(k.as_str(), bytes)?;
                }
                Ok(())
            })
            .map_err(|source| StoreError::Io { path: a.clone(), source })?;
        }
        Ok(())
    }

    pub(crate) fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        match self.place(p) {
            Place::In(a, k) => self.get(&a, &k)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound)),
            Place::Account(_) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
            Place::Disk => fs::read(p),
        }
    }

    pub(crate) fn write(&self, p: &Path, bytes: &[u8]) -> Result<()> {
        match self.place(p) {
            Place::In(a, k) => {
                self.change(&a, |t| t.put(k.as_str(), bytes)).map_err(|source| StoreError::Io { path: p.display().to_string(), source })
            }
            Place::Account(_) | Place::Disk => write_atomic(p, bytes),
        }
    }

    pub(crate) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match (self.place(from), self.place(to)) {
            (Place::In(a, k), Place::In(b, k2)) if a == b => {
                let moving: Vec<String> = self.keys(&a, &k)?.into_iter().map(|(n, _)| n).chain(self.has(&a, &k)?.then(|| k.clone())).collect();
                if moving.is_empty() {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                self.change(&a, |t| {
                    for old in &moving {
                        let new = format!("{k2}{}", old.strip_prefix(k.as_str()).unwrap_or(""));
                        // Moved as kept: not uncompressed and compressed again.
                        if let Some(v) = t.take_kept(old.as_str())? {
                            t.put_kept(new.as_str(), &v)?;
                        }
                    }
                    Ok(())
                })
            }
            (Place::Disk, Place::Disk) => crate::atomic::rename(from, to),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "can't move between data files")),
        }
    }

    pub(crate) fn remove_file(&self, p: &Path) -> io::Result<()> {
        match self.place(p) {
            Place::In(a, k) => {
                if !self.has(&a, &k)? {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                self.change(&a, |t| t.take_kept(k.as_str()).map(drop))
            }
            Place::Account(_) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
            Place::Disk => fs::remove_file(p),
        }
    }

    /// An account let go but its data file kept where it is, as Outlook keeps a removed account's
    /// `.pst`: closed and no longer anyone's, so it can be opened again (File › Open & Export) and
    /// the same address added again gets a file of its own beside it. Returns the file.
    pub(crate) fn let_go(&self, account: &str) -> Option<PathBuf> {
        self.close(account);
        self.seals.forget(account);
        let name = Packs::lock(&self.names).remove(account);
        if let Ok(json) = serde_json::to_vec_pretty(&*Packs::lock(&self.names)) {
            let _ = write_atomic(&self.root.join(NAMES), &json);
        }
        name.map(|n| self.dir.join(n)).filter(|f| f.exists())
    }

    pub(crate) fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        match self.place(p) {
            // The account goes: its data file with it.
            Place::Account(a) => {
                let had = self.close(&a);
                self.seals.forget(&a);
                let name = Packs::lock(&self.names).remove(&a);
                if let Ok(json) = serde_json::to_vec_pretty(&*Packs::lock(&self.names)) {
                    let _ = write_atomic(&self.root.join(NAMES), &json);
                }
                let legacy = fs::remove_dir_all(p).is_ok();
                match name.map(|n| fs::remove_file(self.dir.join(n))) {
                    Some(r) => r.or(Ok(())),
                    None if had || legacy => Ok(()),
                    None => Err(io::Error::from(io::ErrorKind::NotFound)),
                }
            }
            Place::In(a, k) => {
                let gone: Vec<String> = self.keys(&a, &k)?.into_iter().map(|(n, _)| n).collect();
                if gone.is_empty() {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                self.change(&a, |t| {
                    for g in &gone {
                        t.take_kept(g.as_str())?;
                    }
                    Ok(())
                })
            }
            Place::Disk => fs::remove_dir_all(p),
        }
    }

    pub(crate) fn is_file(&self, p: &Path) -> bool {
        match self.place(p) {
            Place::In(a, k) => self.has(&a, &k).unwrap_or(false),
            Place::Account(_) => false,
            Place::Disk => p.is_file(),
        }
    }

    pub(crate) fn exists(&self, p: &Path) -> bool {
        match self.place(p) {
            Place::In(a, k) => self.has(&a, &k).unwrap_or(false) || self.has_under(&a, &k).unwrap_or(false),
            Place::Account(a) => self.file(&a).is_some_and(|f| f.exists()),
            Place::Disk => p.exists(),
        }
    }

    /// The files directly in a folder, by name.
    pub(crate) fn list(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        let (a, k) = match self.place(dir) {
            Place::In(a, k) => (a, k),
            Place::Account(a) => (a, String::new()),
            Place::Disk => {
                let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
                return rd
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                    .collect();
            }
        };
        self.children(&a, &k).map(|(names, _)| names).unwrap_or_default().into_iter().map(|name| (name.clone(), dir.join(name))).collect()
    }

    /// Every file under a folder (an account's: all of its data file), at any depth.
    pub(crate) fn walk(&self, dir: &Path) -> Vec<PathBuf> {
        let (a, k) = match self.place(dir) {
            Place::In(a, k) => (a, k),
            Place::Account(a) => (a, String::new()),
            Place::Disk => return Vec::new(),
        };
        let base = self.root.join(&a);
        self.keys(&a, &k).unwrap_or_default().into_iter().map(|(key, _)| key.split('/').fold(base.clone(), |p, seg| p.join(seg))).collect()
    }

    /// Bytes under a folder: an account's is its data file's size.
    pub(crate) fn size(&self, dir: &Path) -> Option<u64> {
        match self.place(dir) {
            Place::Account(a) => Some(self.file(&a).and_then(|f| fs::metadata(f).ok()).map_or(0, |m| m.len())),
            Place::In(a, k) => Some(self.keys(&a, &k).unwrap_or_default().iter().map(|(_, n)| n).fold(0u64, |s, n| s.saturating_add(*n))),
            Place::Disk => None,
        }
    }

    /// Account folders from before (files in `<root>/<account>`) come into their data files;
    /// each folder goes once all its files are in and read back.
    fn bring_in_folders(&self) -> Result<()> {
        let Ok(rd) = fs::read_dir(&self.root) else { return Ok(()) };
        let accounts: Vec<String> = fs::read(self.root.join("accounts.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<mailcraft_model::AccountConfig>>(&b).ok())
            .map(|l| l.into_iter().map(|a| crate::safe_segment(&a.id)).collect())
            .unwrap_or_default();
        for entry in rd.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
            if !entry.file_type().is_ok_and(|t| t.is_dir()) || !accounts.contains(&name) {
                continue;
            }
            let folder = entry.path();
            let mut files = Vec::new();
            let mut stack = vec![(folder.clone(), String::new())];
            while let Some((dir, rel)) = stack.pop() {
                for e in fs::read_dir(&dir).map_err(io_err(&dir))?.flatten() {
                    let Some(n) = e.file_name().to_str().map(str::to_string) else { continue };
                    let key = if rel.is_empty() { n } else { format!("{rel}/{n}") };
                    match e.file_type() {
                        Ok(t) if t.is_dir() => stack.push((e.path(), key)),
                        Ok(t) if t.is_file() => files.push((key, e.path())),
                        _ => {}
                    }
                }
            }
            let had_file = self.file(&name).is_some_and(|f| f.exists());
            let r = (|| -> io::Result<()> {
                self.change(&name, |t| {
                    for (key, path) in &files {
                        // A file already in the data file is newer than one left in a folder.
                        if !t.has(key.as_str())? {
                            t.put(key.as_str(), &fs::read(path)?)?;
                        }
                    }
                    Ok(())
                })?;
                for (key, path) in &files {
                    let back = self.get(&name, key)?;
                    if back.is_none() || (!had_file && back != fs::read(path).ok()) {
                        return Err(other(format!("{key} didn't come in whole")));
                    }
                }
                Ok(())
            })();
            match r {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&folder);
                }
                Err(e) => {
                    if !had_file {
                        self.close(&name);
                        self.seals.forget(&name);
                        if let Some(f) = self.file(&name) {
                            let _ = fs::remove_file(f);
                        }
                    }
                    return Err(io_err(&folder)(e));
                }
            }
        }
        Ok(())
    }
}

/// Batches of a data file read for copying, at most this many bytes each.
const BATCH: usize = 32 << 20;

/// Every file in the data file at `path` (its path inside the account, its bytes), handed to
/// `each` in batches. It's read from a copy at `tmp`, so the original is never changed (opening a
/// database may write to it) and one another MailCraft has open can still be read.
pub(crate) fn read_data_file(path: &Path, tmp: &Path, each: &mut dyn FnMut(Vec<(String, Vec<u8>)>) -> Result<()>) -> Result<usize> {
    let io_err = |source: io::Error| StoreError::Io { path: path.display().to_string(), source };
    fs::copy(path, tmp).map_err(|e| match e.raw_os_error() {
        // Windows' sharing and lock violations: an account here (or another copy) has it open.
        Some(32 | 33) => io_err(io::Error::other("it's in use: an account in MailCraft has it open")),
        _ => io_err(e),
    })?;
    let read = (|| -> Result<usize> {
        let not_ours = |e: &dyn std::fmt::Display| io_err(io::Error::new(io::ErrorKind::InvalidData, format!("not a MailCraft data file ({e})")));
        let db = Database::builder().set_cache_size(CACHE).open(tmp).map_err(|e| not_ours(&e))?;
        let tx = db.begin_read().map_err(|e| not_ours(&e))?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(0),
            Err(e) => return Err(not_ours(&e)),
        };
        let (mut batch, mut size, mut n) = (Vec::new(), 0usize, 0usize);
        for item in table.iter().map_err(|e| not_ours(&e))? {
            let (k, _) = item.map_err(|e| not_ours(&e))?;
            let Some(v) = read_file(&tx, &table, k.value()).map_err(|e| not_ours(&e))? else { continue };
            size = size.saturating_add(v.len());
            batch.push((k.value().to_string(), v));
            n += 1;
            if size >= BATCH {
                each(std::mem::take(&mut batch))?;
                size = 0;
            }
        }
        if !batch.is_empty() {
            each(batch)?;
        }
        Ok(n)
    })();
    let _ = fs::remove_file(tmp);
    read
}

#[cfg(test)]
mod tests;
