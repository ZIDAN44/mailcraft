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
//! once every file is in and read back.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::{Result, StoreError, io_err, write_atomic};

/// The one table: path inside the account's folder → bytes.
const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
/// A data file's extension.
pub const EXTENSION: &str = "mailcraft";
/// Which data file is whose (in the root).
const NAMES: &str = "datafiles.json";
/// Memory for each open data file's cache.
const CACHE: usize = 16 << 20;

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
    open: Mutex<HashMap<String, Arc<Database>>>,
    names: Mutex<BTreeMap<String, String>>,
}

impl std::fmt::Debug for Packs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packs").field("root", &self.root).field("dir", &self.dir).finish_non_exhaustive()
    }
}

impl Packs {
    /// Data files for the store at `root`, kept in `dir`; folders from before are brought in.
    pub(crate) fn open(root: &Path, dir: &Path) -> Result<Packs> {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let names = fs::read(root.join(NAMES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let p = Packs { root: root.to_path_buf(), dir: dir.to_path_buf(), open: Mutex::default(), names: Mutex::new(names) };
        p.bring_in_folders()?;
        Ok(p)
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

    /// An account's open data file; `create` makes it when there's none.
    fn db(&self, account: &str, create: bool) -> io::Result<Option<Arc<Database>>> {
        if let Some(db) = Packs::lock(&self.open).get(account) {
            return Ok(Some(db.clone()));
        }
        let known = Packs::lock(&self.names).contains_key(account);
        if !create && !known {
            return Ok(None);
        }
        let path = self.dir.join(self.name(account));
        if !create && !path.exists() {
            return Ok(None);
        }
        let db = match Database::builder().set_cache_size(CACHE).create(&path) {
            Ok(db) => db,
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => {
                return Err(other(format!("{} is open in another copy of MailCraft", path.display())));
            }
            // The disk failing isn't damage (a file that isn't a data file is: InvalidData).
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(e))) if e.kind() != io::ErrorKind::InvalidData => return Err(e),
            Err(e) => {
                // Damaged: set aside, and start again (the server has the mail of most accounts).
                let aside = path.with_extension(format!("{EXTENSION}.damaged"));
                log::warn!("{}: {e}; set aside as {}", path.display(), aside.display());
                fs::rename(&path, &aside)?;
                Database::builder().set_cache_size(CACHE).create(&path).map_err(other)?
            }
        };
        let db = Arc::new(db);
        Packs::lock(&self.open).insert(account.to_string(), db.clone());
        Ok(Some(db))
    }

    fn get(&self, account: &str, key: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(db) = self.db(account, false)? else { return Ok(None) };
        let tx = db.begin_read().map_err(other)?;
        let table = match tx.open_table(FILES) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(other(e)),
        };
        Ok(table.get(key).map_err(other)?.map(|v| v.value().to_vec()))
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
            out.push((k.to_string(), v.value().len() as u64));
        }
        Ok(out)
    }

    /// Change an account's data file in one transaction.
    fn change(&self, account: &str, f: impl FnOnce(&mut redb::Table<&str, &[u8]>) -> io::Result<()>) -> io::Result<()> {
        let Some(db) = self.db(account, true)? else { return Err(io::Error::from(io::ErrorKind::NotFound)) };
        let tx = db.begin_write().map_err(other)?;
        {
            let mut table = tx.open_table(FILES).map_err(other)?;
            f(&mut table)?;
        }
        tx.commit().map_err(other)
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
                    t.insert(k.as_str(), *bytes).map_err(other)?;
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
            Place::In(a, k) => self
                .change(&a, |t| t.insert(k.as_str(), bytes).map(drop).map_err(other))
                .map_err(|source| StoreError::Io { path: p.display().to_string(), source }),
            Place::Account(_) | Place::Disk => write_atomic(p, bytes),
        }
    }

    pub(crate) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match (self.place(from), self.place(to)) {
            (Place::In(a, k), Place::In(b, k2)) if a == b => {
                let moving: Vec<String> = self.keys(&a, &k)?.into_iter().map(|(n, _)| n).chain(self.get(&a, &k)?.map(|_| k.clone())).collect();
                if moving.is_empty() {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                self.change(&a, |t| {
                    for old in &moving {
                        let new = format!("{k2}{}", old.strip_prefix(k.as_str()).unwrap_or(""));
                        let v = t.remove(old.as_str()).map_err(other)?.map(|g| g.value().to_vec());
                        if let Some(v) = v {
                            t.insert(new.as_str(), v.as_slice()).map_err(other)?;
                        }
                    }
                    Ok(())
                })
            }
            (Place::Disk, Place::Disk) => fs::rename(from, to),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "can't move between data files")),
        }
    }

    pub(crate) fn remove_file(&self, p: &Path) -> io::Result<()> {
        match self.place(p) {
            Place::In(a, k) => {
                if self.get(&a, &k)?.is_none() {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
                self.change(&a, |t| t.remove(k.as_str()).map(drop).map_err(other))
            }
            Place::Account(_) => Err(io::Error::from(io::ErrorKind::IsADirectory)),
            Place::Disk => fs::remove_file(p),
        }
    }

    pub(crate) fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        match self.place(p) {
            // The account goes: its data file with it.
            Place::Account(a) => {
                let had = Packs::lock(&self.open).remove(&a).is_some();
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
                        t.remove(g.as_str()).map_err(other)?;
                    }
                    Ok(())
                })
            }
            Place::Disk => fs::remove_dir_all(p),
        }
    }

    pub(crate) fn is_file(&self, p: &Path) -> bool {
        match self.place(p) {
            Place::In(a, k) => self.get(&a, &k).ok().flatten().is_some(),
            Place::Account(_) => false,
            Place::Disk => p.is_file(),
        }
    }

    pub(crate) fn exists(&self, p: &Path) -> bool {
        match self.place(p) {
            Place::In(a, k) => self.get(&a, &k).ok().flatten().is_some() || self.keys(&a, &k).is_ok_and(|v| !v.is_empty()),
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
        let start = if k.is_empty() { String::new() } else { format!("{k}/") };
        self.keys(&a, &k)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(key, _)| {
                let name = key.strip_prefix(&start)?;
                (!name.contains('/')).then(|| (name.to_string(), dir.join(name)))
            })
            .collect()
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
                        if t.get(key.as_str()).map_err(other)?.is_none() {
                            t.insert(key.as_str(), fs::read(path)?.as_slice()).map_err(other)?;
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
                        Packs::lock(&self.open).remove(&name);
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
            let (k, v) = item.map_err(|e| not_ours(&e))?;
            let v = v.value().to_vec();
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
mod tests {
    use mailcraft_model::{AccountConfig, Flags, MessageKey, Summary};

    use crate::Store;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mailcraft-packed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn account(id: &str, email: &str) -> AccountConfig {
        AccountConfig { id: id.into(), email: email.into(), ..Default::default() }
    }

    /// A new account starts from an existing data file: its folders, mail and documents come in,
    /// the messages as this account's; junk is refused and paths that climb out are dropped.
    #[test]
    fn an_account_starts_from_an_existing_data_file() {
        use mailcraft_model::{Folder, FolderRole};
        let t = temp("from-file");
        let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
        old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
        old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
        old.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
        let mut st = old.load_folder("a1", "INBOX").unwrap();
        st.validity = 1;
        st.messages.push(Summary { key: k.clone(), subject: "hi".into(), ..Default::default() });
        old.save_folder("a1", "INBOX", &st).unwrap();
        old.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
        // Changes still to make on its server, and mail still to send: they stay behind.
        old.save_ops("a1", &[mailcraft_model::Op::Expunge { keys: vec![k.clone()] }]).unwrap();
        old.outbox_add("a1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nx").unwrap();
        let file = old.data_file("a1").unwrap();
        drop(old);

        let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
        s.save_accounts(&[account("p7", "ana@example.org")]).unwrap();
        assert_eq!(s.import_data_file("p7", &file).unwrap(), 1);
        let folders = s.load_folders("p7").unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].account, "p7", "its folders are this account's (opening one named the old account)");
        assert!(s.load_ops("p7").unwrap().is_empty() && s.outbox("p7").is_empty(), "the old account's pending work stays behind");
        let st = s.load_folder("p7", "INBOX").unwrap();
        let mine = MessageKey { account: "p7".into(), ..k.clone() };
        assert_eq!(st.messages[0].key, mine, "renumbered as this account's");
        assert_eq!(s.read_body(&mine).unwrap(), b"Subject: hi\r\n\r\nhello");
        assert_eq!(s.load_account_doc::<serde_json::Value>("p7", "pim.json").unwrap(), serde_json::json!({"notes": []}));
        assert!(file.exists(), "the file it came from stays");
        assert!(!t.join("app").join(".import-p7.tmp").exists(), "the copy read from is gone");

        // Junk: refused, and nothing written.
        let junk = t.join("junk.mailcraft");
        std::fs::write(&junk, b"not a database at all").unwrap();
        assert!(s.import_data_file("p8", &junk).is_err());
        assert!(s.load_folders("p8").unwrap().is_empty());

        // A crafted file: paths that climb out of the account are left out.
        let crafted = t.join("crafted.mailcraft");
        {
            let db = redb::Database::create(&crafted).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut table = tx.open_table(super::FILES).unwrap();
                for key in ["../escape.json", "a/../../b", "C:/x", "/root", "ok/fine.json"] {
                    table.insert(key, b"x".as_slice()).unwrap();
                }
            }
            tx.commit().unwrap();
        }
        s.import_data_file("p9", &crafted).unwrap();
        assert!(s.files.exists(&t.join("app").join("p9").join("ok").join("fine.json")));
        assert!(!t.join("app").join("escape.json").exists() && !t.join("escape.json").exists() && !t.join("b").exists());
        let _ = std::fs::remove_dir_all(&t);
    }
    #[test]
    fn each_account_is_one_data_file_named_after_it() {
        let t = temp("one-file");
        let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
        s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "old.pst")]).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
        s.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
        let mut st = s.load_folder("a1", "INBOX").unwrap();
        st.validity = 1;
        st.messages.push(Summary { key: k.clone(), subject: "hi".into(), flags: Flags::default(), ..Default::default() });
        s.save_folder("a1", "INBOX", &st).unwrap();
        s.save_account_doc("a2", "pim.json", &serde_json::json!({"x": 1})).unwrap();
        // One file per account, named as Outlook names a .pst; nothing loose in the app folder.
        let mut files: Vec<String> =
            std::fs::read_dir(t.join("MailCraft Files")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        files.sort();
        assert_eq!(files, ["old.mailcraft", "zoe@studio.example.mailcraft"]);
        assert!(!t.join("app").join("a1").exists());
        assert_eq!(s.data_file("a1"), Some(t.join("MailCraft Files").join("zoe@studio.example.mailcraft")));
        // Read back, also after opening again.
        drop(s);
        let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
        assert_eq!(s.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nhello");
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
        assert!(s.account_size("a1") > 0);
        // Folder renames move inside the file; the account going takes its file.
        s.save_folders("a1", &[mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }])
            .unwrap();
        s.apply_op("a1", &mailcraft_model::Op::RenameFolder { from: "INBOX".into(), to: "Kept".into() }).unwrap();
        assert_eq!(s.load_folder("a1", "Kept").unwrap().messages.len(), 1);
        assert!(s.read_body(&MessageKey { folder: "Kept".into(), ..k.clone() }).is_some(), "its messages moved too");
        s.remove_account("a1").unwrap();
        assert!(!t.join("MailCraft Files").join("zoe@studio.example.mailcraft").exists());
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn folders_from_before_come_into_the_data_file() {
        let t = temp("bring-in");
        let app = t.join("app");
        let old = Store::open(&app).unwrap();
        old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 1 };
        old.write_body(&k, b"Subject: kept\r\n\r\nbody").unwrap();
        old.save_account_doc("a1", "pop-seen.json", &vec!["u1".to_string()]).unwrap();
        assert!(app.join("a1").is_dir());
        let s = Store::open_packed(&app, t.join("Files")).unwrap();
        assert!(!app.join("a1").exists(), "the folder went once it was in");
        assert_eq!(s.read_body(&k).unwrap(), b"Subject: kept\r\n\r\nbody");
        assert_eq!(s.load_account_doc::<Vec<String>>("a1", "pop-seen.json").unwrap(), vec!["u1".to_string()]);
        assert!(t.join("Files").join("zoe@studio.example.mailcraft").is_file());
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn a_damaged_data_file_is_set_aside() {
        let t = temp("damaged");
        let files = t.join("Files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("zoe@studio.example.mailcraft"), b"not a database at all, just junk bytes").unwrap();
        std::fs::create_dir_all(t.join("app")).unwrap();
        std::fs::write(t.join("app").join("datafiles.json"), br#"{"a1": "zoe@studio.example.mailcraft"}"#).unwrap();
        let s = Store::open_packed(t.join("app"), &files).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        assert!(s.read_body(&k).is_none());
        s.write_body(&k, b"x").unwrap();
        assert!(files.join("zoe@studio.example.mailcraft.damaged").exists());
        let _ = std::fs::remove_dir_all(t);
    }
}
