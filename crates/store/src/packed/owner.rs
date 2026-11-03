//! Data files: which MailCraft data folder each is an account of. A data folder has an id of its
//! own, made once (`datafolder.json` in its root), and every data file it makes says it's that
//! folder's (`owner` in the file's `META`: the id, and the folder's path to show). A copy beside
//! an Outlook file found by another data folder (the CLI's, a `--data` run) is then told apart
//! from one no account has: it's that folder's while the folder is there and still names it as
//! an account's data file (`datafiles.json`, the only thing of it read), and taken on only when
//! the user says so. A copy from before this has no owner, and is anyone's, as it was.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// The data folder's own id, in its root.
pub(crate) const FOLDER_ID: &str = "datafolder.json";
/// In a data file's `META`: the data folder it's an account of (JSON: `id` and `path`).
pub(super) const OWNER: &str = "owner";
/// The most of an owner record, an id file or another folder's list of data files read.
const OWNER_MAX: usize = 8 << 10;
const LIST_MAX: u64 = 16 << 20;
/// Ids made, so two made at once in this process differ whatever the clock says.
static MADE: AtomicUsize = AtomicUsize::new(0);

/// Whether `id` is one as a data folder's is made: 32 lowercase hex digits.
fn usable(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A new id: 128 bits from the system's random keys (`RandomState`), the time and the folder.
fn new_id(root: &Path) -> String {
    let n = MADE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let seed = (n, nanos, std::process::id(), root.to_string_lossy().into_owned());
    let a = std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), &seed);
    let b = std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), (&seed, a));
    format!("{a:016x}{b:016x}")
}

/// The id in the data folder at `root`: `Ok(None)` when it has none; an `InvalidData` error when
/// the file is there but isn't one (damaged, or not MailCraft's).
pub(crate) fn read_folder_id(root: &Path) -> io::Result<Option<String>> {
    let path = root.join(FOLDER_ID);
    let bytes = match fs::metadata(&path) {
        Ok(m) if !m.is_file() || m.len() > OWNER_MAX as u64 => return Err(io::Error::from(io::ErrorKind::InvalidData)),
        Ok(_) => fs::read(&path),
        Err(e) => Err(e),
    };
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    id_from(&bytes).map(Some).ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))
}

/// The id an id file's bytes hold, when they're one.
fn id_from(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("id").and_then(serde_json::Value::as_str).map(str::to_string))
        .filter(|id| usable(id))
}

/// An owner record's id and path, as kept ("" for both when it isn't one).
fn owner_from(bytes: &[u8]) -> (String, String) {
    let record = Some(bytes).filter(|b| b.len() <= OWNER_MAX).and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
    let field = |k: &str| record.as_ref().and_then(|r| r.get(k)).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    let id = field("id");
    if !usable(&id) {
        return (String::new(), String::new());
    }
    (id, field("path"))
}

/// Whether a data folder's list of data files (`datafiles.json`'s bytes) names the copy whose
/// key is `copy` ([`beside::same_key`]); `None` when it isn't a list.
fn names_copy(bytes: &[u8], copy: &str) -> Option<bool> {
    let names: BTreeMap<String, String> = serde_json::from_slice(bytes).ok()?;
    Some(names.values().any(|n| Path::new(n).is_absolute() && beside::same_key(Path::new(n)) == copy))
}

/// For the fuzz target (`fuzz/fuzz_targets/store/data_folder.rs`): `input` read as a data
/// folder's id file, a data file's owner record and a data folder's list of data files. Whatever
/// is taken from them is as it should be.
#[doc(hidden)]
pub fn fuzz_data_folder(input: &[u8]) -> bool {
    let id_ok = id_from(input).is_none_or(|id| usable(&id));
    let (id, path) = owner_from(input);
    let owner_ok = if id.is_empty() { path.is_empty() } else { usable(&id) };
    let _ = names_copy(input, &beside::same_key(Path::new("C:/Mail/Archive.mailcraft")));
    id_ok && owner_ok
}

/// The id of the data folder at `root`, made when it has none: written whole to a file of its
/// own, then linked in under its name, which fails when another process made one first (that
/// one is taken). A damaged file is set aside (`datafolder.json.damaged`) and a new id made.
/// `None` when the folder can't be read or written: nothing is recorded then.
pub(crate) fn folder_id(root: &Path) -> Option<String> {
    for _ in 0..4 {
        match read_folder_id(root) {
            Ok(Some(id)) => return Some(id),
            Ok(None) => {}
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                let path = root.join(FOLDER_ID);
                let aside = root.join(format!("{FOLDER_ID}.damaged"));
                log::warn!("{}: not a data folder's id; set aside as {}", path.display(), aside.display());
                let _ = fs::remove_file(&aside);
                if let Err(e) = fs::rename(&path, &aside) {
                    log::warn!("{}: couldn't be set aside: {e}", path.display());
                    return None;
                }
                continue;
            }
            Err(e) => {
                log::warn!("{}: {e}", root.join(FOLDER_ID).display());
                return None;
            }
        }
        let json = serde_json::to_vec(&serde_json::json!({ "id": new_id(root) })).ok()?;
        let made = root.join(format!("{FOLDER_ID}.{}-{}.new", std::process::id(), MADE.fetch_add(1, Ordering::Relaxed)));
        if let Err(e) = write_atomic(&made, &json) {
            log::warn!("{}: {e}", made.display());
            return None;
        }
        // Linked in, never renamed over: one made meanwhile by another process stays.
        match fs::hard_link(&made, root.join(FOLDER_ID)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            // A file system with no links: renamed in (read back below, whoever's it is).
            Err(_) => {
                if fs::rename(&made, root.join(FOLDER_ID)).is_err() {
                    let _ = fs::remove_file(&made);
                    return None;
                }
            }
        }
        let _ = fs::remove_file(&made);
    }
    None
}

/// Who a data file says it's an account of: the data folder's id and its path ("" for both when
/// it says nothing: a file from before, or one whose record can't be read).
pub(super) fn owner_of(tx: &redb::ReadTransaction) -> (String, String) {
    let Ok(table) = tx.open_table(META) else { return (String::new(), String::new()) };
    match table.get(OWNER) {
        Ok(Some(v)) => owner_from(v.value()),
        _ => (String::new(), String::new()),
    }
}

/// Whether the data folder at `path` whose id is `id` still has the copy at `copy` as one of its
/// accounts' data files: it does unless the folder is gone, is another data folder now, or its
/// list of data files (read only, and nothing else of it) doesn't name the copy. Anything that
/// can't be told says it does.
pub(crate) fn still_has(path: &str, id: &str, copy: &Path) -> bool {
    let root = Path::new(path);
    if path.is_empty() || !root.is_absolute() {
        return true;
    }
    match fs::metadata(root) {
        Ok(m) if !m.is_dir() => return false,
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    }
    // Another data folder there now (this one was moved, or removed and made again).
    if let Ok(Some(now)) = read_folder_id(root)
        && now != id
    {
        return false;
    }
    let list = root.join(NAMES);
    let bytes = match fs::metadata(&list) {
        Ok(m) if m.len() > LIST_MAX => return true,
        Ok(_) => fs::read(&list),
        Err(e) => Err(e),
    };
    match bytes {
        Ok(b) => names_copy(&b, &beside::same_key(copy)).unwrap_or(true),
        // No data files named at all.
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

impl Packs {
    /// This data folder's id ([`folder_id`]), made the first time it's asked for.
    pub(crate) fn folder_id(&self) -> Option<String> {
        if let Some(id) = self.id.get() {
            return Some(id.clone());
        }
        let id = folder_id(&self.root)?;
        Some(self.id.get_or_init(|| id).clone())
    }

    /// The record a data file of this folder's keeps of it (`None`: no id to record).
    pub(super) fn owner_record(&self) -> Option<Vec<u8>> {
        let id = self.folder_id()?;
        serde_json::to_vec(&serde_json::json!({ "id": id, "path": self.root.to_string_lossy() })).ok()
    }

    /// What a data file made new says in `META` from the start: everything in it is kept as this
    /// build keeps it, and it's this folder's.
    pub(super) fn write_fresh_meta(&self, tx: &redb::WriteTransaction) -> io::Result<()> {
        let deflate = DEFLATE_NOW.to_le_bytes();
        match self.owner_record() {
            Some(r) => write_meta(tx, &[(DEFLATE, deflate.as_slice()), (OWNER, r.as_slice())]),
            None => write_meta(tx, &[(DEFLATE, deflate.as_slice())]),
        }
    }

    /// `account`'s data file is this folder's (`ours`), or no folder's: its owner record written,
    /// or taken out. Nothing to do when it has no file.
    pub(crate) fn set_owner(&self, account: &str, ours: bool) -> io::Result<()> {
        let Some(db) = self.db(account, false)? else { return Ok(()) };
        let record = if ours { self.owner_record() } else { None };
        seal_with(&db, &self.seals, account, |tx| match &record {
            Some(r) => write_meta(tx, &[(OWNER, r.as_slice())]),
            None => tx.open_table(META).map_err(disk)?.remove(OWNER).map(drop).map_err(disk),
        })
    }
}
