//! Data files: which MailCraft data folder each is an account of. A data folder has an id of its
//! own, made once (`datafolder.json` in its root), and every data file it makes says it's that
//! folder's (`owner` in the file's `META`: the id, and the folder's path to show). A copy beside
//! an Outlook file found by another data folder (the CLI's, a `--data` run) is then told apart
//! from one no account has: it's that folder's while the folder is there and still names it as
//! an account's data file (`datafiles.json`, the only thing of it read), and taken on only when
//! the user says so. A copy from before this has no owner, and is anyone's, as it was. Taken over,
//! a copy is never opened again by the folder that had it ([`Packs::held_elsewhere`]), and the
//! folder a record names is looked at only on this computer's own drives ([`local_root`]).

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// The data folder's own id, in its root.
pub(crate) const FOLDER_ID: &str = "datafolder.json";
/// In a data file's `META`: the data folder it's an account of (JSON: `id` and `path`).
pub(super) const OWNER: &str = "owner";
/// The most of an owner record, an id file or another folder's list of data files read.
const OWNER_MAX: usize = 8 << 10;
const LIST_MAX: u64 = 16 << 20;
/// The longest owner path looked at (bytes).
const PATH_MAX: usize = 4096;
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
    // A folder looked at is never a share's or a device's.
    let path_ok = local_root(&path).is_none_or(|p| {
        let p = p.to_string_lossy();
        let shared = [r"\\", "//", r"\/", r"/\"].iter().any(|s| p.starts_with(s));
        p.len() <= PATH_MAX && !p.contains('\0') && !shared && p.as_ref() == path
    });
    let _ = names_copy(input, &beside::same_key(Path::new("C:/Mail/Archive.mailcraft")));
    id_ok && owner_ok && path_ok
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

/// The folder an owner record names, when it's one to look at: an absolute path on one of this
/// computer's drives, as this system writes one (`X:\…` or `X:/…` on Windows, `/…` elsewhere);
/// never two slashes at its start (a share, `\\?\…` or `\\.\…`), no NUL, and not over-long.
pub(super) fn local_root(path: &str) -> Option<&Path> {
    if path.len() > PATH_MAX || path.contains('\0') || [r"\\", "//", r"\/", r"/\"].iter().any(|p| path.starts_with(p)) {
        return None;
    }
    let b = path.as_bytes();
    let local = if cfg!(windows) { matches!(b, [d, b':', b'\\' | b'/', ..] if d.is_ascii_alphabetic()) } else { b.first() == Some(&b'/') };
    // Whatever the text says, only a drive's own path or a plain root is gone to.
    let plain = Path::new(path).components().next().is_some_and(|c| match c {
        std::path::Component::Prefix(p) => matches!(p.kind(), std::path::Prefix::Disk(_)),
        std::path::Component::RootDir => true,
        _ => false,
    });
    (local && plain).then(|| Path::new(path))
}

/// Whether the data folder at `path` whose id is `id` still has the copy at `copy` as one of its
/// accounts' data files: it does unless the folder is gone, is another data folder now, or its
/// list of data files (read only, and nothing else of it) doesn't name the copy. Anything that
/// can't be told says it does, and so does a folder that isn't on this computer's own drives
/// ([`local_root`]): the path comes from a file found beside another (from a zip, a USB stick, a
/// share), and a read from a share could hand the user's sign-in to its server, so it isn't
/// looked at. The user can still take the copy over.
pub(crate) fn still_has(path: &str, id: &str, copy: &Path) -> bool {
    let Some(root) = local_root(path) else { return true };
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

    /// `account`'s data file is this folder's (`ours`: taken on, whoever's it was), or no
    /// folder's: its owner record written, or taken out, but only when it names this folder (or
    /// no folder): another's, it stays theirs (one that took the copy over, its account here
    /// removed after). Nothing to do when it has no file.
    pub(crate) fn set_owner(&self, account: &str, ours: bool) -> io::Result<()> {
        let db = if ours { TakingOver::with(|| self.db(account, false))? } else { self.db(account, false)? };
        let Some(db) = db else { return Ok(()) };
        let record = if ours { self.owner_record() } else { None };
        let me = self.folder_id().unwrap_or_default();
        seal_with(&db, &self.seals, account, |tx| match &record {
            Some(r) => write_meta(tx, &[(OWNER, r.as_slice())]),
            None => {
                let mut table = tx.open_table(META).map_err(disk)?;
                let now = table.get(OWNER).map_err(disk)?.map(|v| owner_from(v.value()).0).unwrap_or_default();
                if now.is_empty() || now == me { table.remove(OWNER).map(drop).map_err(disk) } else { Ok(()) }
            }
        })
    }

    /// The data folder that has the copy beside an Outlook file at `path` (`name`d by its whole
    /// path in this folder's list) as an account's data file, when that's another than this one:
    /// it took the copy over. Read before the copy is opened to write (opened read only, as a copy
    /// found beside a file is), so nothing of it is written here; `None` when it's this folder's,
    /// no folder's, or can't be told (a copy not closed properly is told once it's open:
    /// [`Packs::foreign_owner`]). The data folder's path is the one its record names.
    pub(super) fn held_elsewhere(&self, name: &str, path: &Path) -> Option<String> {
        if !Path::new(name).is_absolute() || TakingOver::now() {
            return None;
        }
        let db = redb::ReadOnlyDatabase::open(path).ok()?;
        let tx = db.begin_read().ok()?;
        self.foreign_owner(&tx)
    }

    /// [`Packs::held_elsewhere`] for a copy open already (one that couldn't be read before it
    /// opened: it wasn't closed properly, and was checked as it opened).
    pub(super) fn held_elsewhere_open(&self, name: &str, db: &Database) -> Option<String> {
        if !Path::new(name).is_absolute() || TakingOver::now() {
            return None;
        }
        self.foreign_owner(&db.begin_read().ok()?)
    }

    /// The path of the other data folder a data file's record (in `tx`) says it's an account of:
    /// not this folder's id, nor this folder's path (its id set aside as damaged and made again,
    /// it's still the folder that made the file). `None` when it's this one's or no folder's, or
    /// this folder has no id to tell by.
    pub(super) fn foreign_owner(&self, tx: &redb::ReadTransaction) -> Option<String> {
        let (id, path) = owner_of(tx);
        let me = self.folder_id()?;
        let here = beside::same_key(&self.root);
        (!id.is_empty() && id != me && beside::same_key(Path::new(&path)) != here).then_some(path)
    }
}

thread_local! {
    /// This thread takes a copy over ([`TakingOver`]): it's opened whoever's it was.
    static TAKING_OVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A copy another data folder has, opened on this thread to be taken over (its record then made
/// this folder's), for as long as `f` runs.
struct TakingOver(bool);

impl TakingOver {
    fn with<T>(f: impl FnOnce() -> T) -> T {
        let _taking = TakingOver(TAKING_OVER.with(|t| t.replace(true)));
        f()
    }

    fn now() -> bool {
        TAKING_OVER.with(std::cell::Cell::get)
    }
}

impl Drop for TakingOver {
    fn drop(&mut self) {
        TAKING_OVER.with(|t| t.set(self.0));
    }
}
