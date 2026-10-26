//! Where MailCraft keeps its things, and moving its data files (as Outlook moves a `.pst`).
//!
//! As Outlook keeps each account in a `.pst` in Documents\Outlook Files and its settings
//! elsewhere, MailCraft keeps each account in a data file (`<address>.mailcraft`) in
//! Documents\MailCraft Files ([`files_dir`]), and its account list and window state in the
//! per-user app folder ([`base_root`]). The data files' folder can be moved: the move is asked
//! for while MailCraft runs ([`schedule_move`]) and carried out when it next starts, before
//! anything is open ([`apply_pending_move`]); then `location.json` in the app folder points at
//! the new folder. The old files go only once every file has been copied and checked.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// In the app folder: where the data files are when they've been moved.
const LOCATION: &str = "location.json";
/// In the app folder: a move asked for, carried out at the next start.
const PENDING: &str = "move-pending.json";
/// The data files' folder, in Documents (or wherever it's moved to inside a folder in use).
pub const FILES_FOLDER: &str = "MailCraft Files";

#[derive(serde::Serialize, serde::Deserialize)]
struct Pointer {
    path: PathBuf,
}

fn read_pointer(file: &Path) -> Option<PathBuf> {
    let p: Pointer = serde_json::from_slice(&fs::read(file).ok()?).ok()?;
    p.path.is_absolute().then_some(p.path)
}

fn write_pointer(file: &Path, path: &Path) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(&Pointer { path: path.to_path_buf() }).map_err(|e| e.to_string())?;
    crate::write_atomic(file, &json).map_err(|e| e.to_string())
}

/// The per-user app folder: `%APPDATA%\MailCraft` on Windows, `~/Library/Application
/// Support/MailCraft` on macOS, `$XDG_DATA_HOME/mailcraft` (or `~/.local/share/mailcraft`)
/// elsewhere.
pub fn base_root() -> Option<PathBuf> {
    // A relative value (`APPDATA=data`, an empty `HOME`) would put the mail wherever MailCraft was
    // started from; the XDG spec says to ignore one (craft-libs' app folders found this).
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    if cfg!(target_os = "windows") {
        return var("APPDATA").map(|d| d.join("MailCraft"));
    }
    let home = var("HOME")?;
    if cfg!(target_os = "macos") {
        return Some(home.join("Library").join("Application Support").join("MailCraft"));
    }
    match var("XDG_DATA_HOME") {
        Some(x) => Some(x.join("mailcraft")),
        None => Some(home.join(".local").join("share").join("mailcraft")),
    }
}

/// Where the data files go unless moved: "MailCraft Files" in Documents (wherever the system
/// keeps Documents), or in the app folder when there's no Documents folder.
pub fn default_files_dir(base: &Path) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(docs) = dirs::document_dir() {
        return docs.join(FILES_FOLDER);
    }
    base.join(FILES_FOLDER)
}

/// Where the data files are: where they were moved to, or [`default_files_dir`].
pub fn files_dir(base: &Path) -> PathBuf {
    read_pointer(&base.join(LOCATION)).unwrap_or_else(|| default_files_dir(base))
}

/// The move waiting for the next start, if one is.
pub fn pending_move(base: &Path) -> Option<PathBuf> {
    read_pointer(&base.join(PENDING))
}

fn has_files(p: &Path) -> bool {
    fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())
}

/// Ask for the data files to move into `to` when MailCraft next starts; returns the folder
/// they'll be in. A folder that already has other files gets a "MailCraft Files" folder inside
/// it; moving into, or above, the folder in use is refused.
pub fn schedule_move(base: &Path, to: &Path) -> Result<PathBuf, String> {
    if !to.is_absolute() {
        return Err("choose a folder on this computer".into());
    }
    let current = files_dir(base);
    let target = if to == current || !has_files(to) || to.ends_with(FILES_FOLDER) { to.to_path_buf() } else { to.join(FILES_FOLDER) };
    if target == current {
        cancel_move(base);
        return Ok(target);
    }
    if target.starts_with(&current) || current.starts_with(&target) {
        return Err("choose a folder outside the one your data files are in now".into());
    }
    if has_files(&target) {
        return Err(format!("{} already has files in it: choose another folder", target.display()));
    }
    write_pointer(&base.join(PENDING), &target)?;
    Ok(target)
}

/// Forget a move asked for.
pub fn cancel_move(base: &Path) {
    let _ = fs::remove_file(base.join(PENDING));
}

/// Whether two files hold the same bytes (read a piece at a time: data files can be large).
fn same(a: &Path, b: &Path) -> bool {
    let (Ok(mut fa), Ok(mut fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    if fa.metadata().map(|m| m.len()).ok() != fb.metadata().map(|m| m.len()).ok() {
        return false;
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let (Ok(na), Ok(nb)) = (fa.read(&mut ba), fb.read(&mut bb)) else { return false };
        if na != nb || ba.get(..na) != bb.get(..nb) {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

/// Carry out a move asked for (call before anything opens the data files). `Ok(Some(folder))`
/// when they moved; on an error nothing has changed and the move is no longer pending.
pub fn apply_pending_move(base: &Path) -> Result<Option<PathBuf>, String> {
    let Some(to) = pending_move(base) else { return Ok(None) };
    cancel_move(base);
    let from = files_dir(base);
    if to == from {
        return Ok(Some(to));
    }
    let list: Vec<PathBuf> = match fs::read_dir(&from) {
        Ok(rd) => rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    };
    let created = !to.exists();
    let copy = || -> Result<(), String> {
        fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        for src in &list {
            let dst = to.join(src.file_name().unwrap_or_default());
            if dst.exists() {
                return Err(format!("{} is already there", dst.display()));
            }
            fs::copy(src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            if !same(src, &dst) {
                return Err(format!("{} didn't copy whole", dst.display()));
            }
        }
        Ok(())
    };
    if let Err(e) = copy() {
        // Back as it was: only what this move made goes.
        for src in &list {
            let _ = fs::remove_file(to.join(src.file_name().unwrap_or_default()));
        }
        if created {
            let _ = fs::remove_dir_all(&to);
        }
        return Err(e);
    }
    if to == default_files_dir(base) {
        let _ = fs::remove_file(base.join(LOCATION));
    } else {
        write_pointer(&base.join(LOCATION), &to)?;
    }
    // The copies are checked: the old files go, and their folder when it's left empty.
    for src in &list {
        let _ = fs::remove_file(src);
    }
    let _ = fs::remove_dir(&from);
    Ok(Some(to))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mailcraft-location-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// An app folder whose data files were moved to `files` (as if from Documents).
    fn seed(base: &Path, files: &Path) {
        fs::create_dir_all(files).unwrap();
        fs::write(files.join("zoe@studio.example.mailcraft"), vec![7u8; 3 << 20]).unwrap();
        fs::write(files.join("old.mailcraft"), b"small").unwrap();
        fs::create_dir_all(base).unwrap();
        write_pointer(&base.join(LOCATION), files).unwrap();
    }

    #[test]
    fn the_data_files_move_at_the_next_start() {
        let t = temp("move");
        let (base, first, elsewhere) = (t.join("app"), t.join("C").join(FILES_FOLDER), t.join("D-drive").join("Mail"));
        seed(&base, &first);
        assert_eq!(files_dir(&base), first);
        // Asked for: nothing moves yet.
        assert_eq!(schedule_move(&base, &elsewhere).unwrap(), elsewhere);
        assert_eq!(pending_move(&base), Some(elsewhere.clone()));
        assert!(first.join("old.mailcraft").exists() && !elsewhere.exists());
        // At the next start: copied, checked, pointed at, and the old ones gone.
        assert_eq!(apply_pending_move(&base).unwrap(), Some(elsewhere.clone()));
        assert_eq!(files_dir(&base), elsewhere);
        assert_eq!(fs::read(elsewhere.join("zoe@studio.example.mailcraft")).unwrap(), vec![7u8; 3 << 20]);
        assert!(!first.exists(), "the old folder went once empty");
        assert_eq!(pending_move(&base), None);
        assert_eq!(apply_pending_move(&base).unwrap(), None, "done once");
        let _ = fs::remove_dir_all(t);
    }

    #[test]
    fn a_move_only_goes_where_it_can() {
        let t = temp("refuse");
        let (base, files) = (t.join("app"), t.join("C").join(FILES_FOLDER));
        seed(&base, &files);
        assert!(schedule_move(&base, Path::new("relative")).is_err());
        assert!(schedule_move(&base, &files.join("inner")).is_err(), "not into itself");
        // A folder with other files in it gets a "MailCraft Files" folder inside.
        let docs = t.join("Documents");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("cv.pdf"), b"%PDF").unwrap();
        assert_eq!(schedule_move(&base, &docs).unwrap(), docs.join(FILES_FOLDER));
        cancel_move(&base);
        assert_eq!(pending_move(&base), None);
        // A copy that fails leaves everything as it was.
        let blocked = t.join("blocked");
        fs::write(&blocked, b"a file, not a folder").unwrap();
        write_pointer(&base.join(PENDING), &blocked.join("x")).unwrap();
        assert!(apply_pending_move(&base).is_err());
        assert_eq!(files_dir(&base), files);
        assert!(files.join("old.mailcraft").exists() && pending_move(&base).is_none());
        // A damaged pointer is ignored.
        fs::write(base.join(LOCATION), b"{nope").unwrap();
        assert_eq!(files_dir(&base), default_files_dir(&base));
        let _ = fs::remove_dir_all(t);
    }
}
