//! Data files: MailCraft's copy of an Outlook data file, kept beside it (`beside.rs` in the
//! store). Such a copy is named in `datafiles.json` by its whole path; every other data file by
//! its name in the data files' folder. A copy found there is only read, and read only when it
//! was closed properly (one that would need a repair to open is no copy of this file's, as far
//! as MailCraft can tell, and is left alone).

use std::collections::HashSet;
use std::fs::OpenOptions;

use super::*;
use crate::{Beside, Found};

/// Names tried beside a file: `archive.mailcraft`, then `archive (2).mailcraft` and on.
const TRIES: u32 = 100;
/// A copy's note of the file it read, and its folder list, are read only when this big at most.
const NOTE_MAX: usize = 64 << 20;
/// The import's note in a data file, and the account's folder list.
const NOTE: &str = "import.json";
const FOLDERS: &str = "folders.json";

/// A path as compared for "the same file": case ignored (Windows' and macOS's file systems), and
/// `/` as `\`.
fn same_key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

impl Packs {
    /// Where the copy of `source` goes: the first of its names beside it that's a copy made from
    /// a file of that name and no account's now, else the first name that's free, when its
    /// folder can be written (the file made there and removed at once says so); with the other
    /// data files otherwise. `take`: that account's data file is the one beside from now on.
    pub(crate) fn beside(&self, source: &Path, take: Option<&str>) -> Beside {
        let elsewhere = Beside { path: self.dir.clone(), found: None, elsewhere: true };
        let (Some(dir), Some(stem), Some(file)) = (
            source.parent().filter(|d| d.is_absolute()),
            source.file_stem().and_then(|s| s.to_str()).map(str::trim).filter(|s| !s.is_empty()),
            source.file_name().and_then(|s| s.to_str()),
        ) else {
            return elsewhere;
        };
        if dir.to_str().is_none() {
            return elsewhere;
        }
        let mut names = Packs::lock(&self.names);
        let held: HashSet<String> = names.values().map(|n| same_key(&self.dir.join(n))).collect();
        for i in 1..=TRIES {
            let name = if i == 1 { format!("{stem}.{EXTENSION}") } else { format!("{stem} ({i}).{EXTENSION}") };
            let path = dir.join(name);
            if held.contains(&same_key(&path)) {
                continue;
            }
            let found = match fs::symlink_metadata(&path) {
                Ok(m) if m.is_file() => match note_of(&path, file) {
                    Some(found) => Some(found),
                    // Another file: the next name.
                    None => continue,
                },
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    if !writable(&path) {
                        return elsewhere;
                    }
                    None
                }
                Err(_) => return elsewhere,
            };
            if let (Some(account), Some(p)) = (take, path.to_str()) {
                names.insert(account.to_string(), p.to_string());
                if let Ok(json) = serde_json::to_vec_pretty(&*names) {
                    let _ = write_atomic(&self.root.join(NAMES), &json);
                }
            }
            return Beside { path, found, elsewhere: false };
        }
        elsewhere
    }
}

/// Whether a file can be made at `path` (nothing there): it's made, and removed at once.
fn writable(path: &Path) -> bool {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(f) => {
            drop(f);
            if let Err(e) = fs::remove_file(path) {
                log::warn!("{}: made to see the folder can be written, but not removed: {e}", path.display());
            }
            true
        }
        Err(_) => false,
    }
}

/// What the data file at `path` says of the file it was made from, when it's a copy of a file
/// named `source` (whatever folder that was in then: the two may have moved together). Opened to
/// read only; `None` for a file that isn't one, that's open elsewhere, or that wasn't closed
/// properly.
fn note_of(path: &Path, source: &str) -> Option<Found> {
    let db = redb::ReadOnlyDatabase::open(path).ok()?;
    let tx = db.begin_read().ok()?;
    let table = tx.open_table(FILES).ok()?;
    let read = |key: &str| read_file(&tx, &table, key).ok().flatten().filter(|b| b.len() <= NOTE_MAX);
    let note: serde_json::Value = serde_json::from_slice(&read(NOTE)?).ok()?;
    let src = note.get("source")?;
    let noted = src.get("path")?.as_str()?;
    if !noted.rsplit(['/', '\\']).next().is_some_and(|n| n.eq_ignore_ascii_case(source)) {
        return None;
    }
    let account = read(FOLDERS)
        .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
        .and_then(|list| list.first().and_then(|f| f.get("account")).and_then(|a| a.as_str()).map(str::to_string))
        .unwrap_or_default();
    Some(Found {
        state: note.get("state").and_then(|s| s.as_str()).unwrap_or_default().to_string(),
        size: src.get("size").and_then(serde_json::Value::as_u64).unwrap_or(0),
        modified: src.get("modified").and_then(serde_json::Value::as_i64).unwrap_or(0),
        changed: false,
        account,
    })
}
