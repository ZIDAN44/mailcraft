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

/// Files beside the executable that make a copy portable (issue #4): either one, whatever it
/// holds.
pub const PORTABLE_MARKERS: [&str; 2] = ["portable.txt", "MailCraft.portable"];
/// The folder a portable copy keeps everything in, beside its executable.
pub const PORTABLE_DATA: &str = "MailCraftData";

/// The folder a copy's markers go in: the executable's, or on macOS the folder its `.app`
/// bundle is in (writing inside a signed bundle breaks its signature).
pub fn portable_base(exe_dir: &Path) -> &Path {
    let bundle = || {
        if !cfg!(target_os = "macos") || exe_dir.file_name()? != "MacOS" {
            return None;
        }
        let contents = exe_dir.parent()?;
        let bundle = contents.parent()?;
        (contents.file_name()? == "Contents" && bundle.extension()? == "app").then_some(bundle.parent()?)
    };
    bundle().unwrap_or(exe_dir)
}

/// A portable copy's data folder, when the executable in `exe_dir` is one (a marker beside it):
/// the app's, and the command line's beside it in the portable zip (issue #255: its MCP server
/// looked in the user's folder, and didn't find the window).
pub fn portable_data(exe_dir: &Path) -> Option<PathBuf> {
    let base = portable_base(exe_dir);
    PORTABLE_MARKERS.iter().any(|m| base.join(m).is_file()).then(|| base.join(PORTABLE_DATA))
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
    let (Ok(fa), Ok(fb)) = (fs::File::open(a), fs::File::open(b)) else { return false };
    if fa.metadata().map(|m| m.len()).ok() != fb.metadata().map(|m| m.len()).ok() {
        return false;
    }
    same_bytes(fa, fb)
}

/// Whether two readers give the same bytes to their ends. Each piece is read whole before the
/// two are compared: a read may give fewer bytes than asked on one side only (a network share),
/// which says nothing about what the files hold.
fn same_bytes(mut a: impl Read, mut b: impl Read) -> bool {
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let (Ok(na), Ok(nb)) = (fill(&mut a, &mut ba), fill(&mut b, &mut bb)) else { return false };
        if na != nb || ba.get(..na) != bb.get(..nb) {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

/// `buf` filled from `r`, short only at its end: how many bytes it got.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while let Some(rest) = buf.get_mut(got..).filter(|rest| !rest.is_empty()) {
        match r.read(rest) {
            Ok(0) => break,
            Ok(n) => got = got.saturating_add(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
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
    // The files this move makes, each noted before it's written: on failure only they go, never
    // one that was there already.
    let mut made: Vec<PathBuf> = Vec::new();
    let mut copy = || -> Result<(), String> {
        fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        for src in &list {
            let dst = to.join(src.file_name().unwrap_or_default());
            if dst.exists() {
                return Err(format!("{} is already there", dst.display()));
            }
            made.push(dst.clone());
            fs::copy(src, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            if !same(src, &dst) {
                return Err(format!("{} didn't copy whole", dst.display()));
            }
        }
        Ok(())
    };
    if let Err(e) = copy() {
        // Back as it was: only what this move made goes.
        for dst in &made {
            let _ = fs::remove_file(dst);
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
    use crate::tests::scratch;

    fn temp(name: &str) -> PathBuf {
        let d = scratch(&format!("location-{name}"));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Issue #255: a portable copy's data folder, beside it, for the app and the command line
    /// alike; none without a marker, and a folder named so isn't one.
    #[test]
    fn a_portable_copys_folder_is_found_beside_it() {
        let exe = temp("portable");
        assert_eq!(portable_data(&exe), None);
        fs::create_dir_all(exe.join("portable.txt")).unwrap();
        assert_eq!(portable_data(&exe), None, "a folder isn't a marker");
        fs::write(exe.join("MailCraft.portable"), b"").unwrap();
        assert_eq!(portable_data(&exe), Some(exe.join(PORTABLE_DATA)));
        assert_eq!(portable_base(&exe), exe.as_path());
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
    }

    /// Bytes given a few at a time, as a network share may: `steps` says how many each read
    /// gives, in turn.
    struct Short<'a> {
        bytes: &'a [u8],
        steps: &'a [usize],
        at: usize,
    }

    impl Read for Short<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let step = self.steps[self.at % self.steps.len()];
            self.at += 1;
            let n = step.min(buf.len()).min(self.bytes.len());
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }

    /// A copy read back in pieces of other sizes than the original's (a short read on one side)
    /// is the same copy; one that differs, or is cut short, isn't. Comparing what each read gave
    /// took the first short read for a copy that didn't come whole, and the move was undone.
    #[test]
    fn a_copy_read_back_in_short_pieces_is_the_same() {
        let bytes: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let whole = |b| Short { bytes: b, steps: &[usize::MAX], at: 0 };
        let short = |b| Short { bytes: b, steps: &[1 << 20, 4096, 1, 700_000], at: 0 };
        assert!(same_bytes(whole(&bytes), short(&bytes)));
        assert!(same_bytes(short(&bytes), whole(&bytes)));
        let mut other = bytes.clone();
        other[2_999_999] ^= 1;
        assert!(!same_bytes(whole(&bytes), short(&other)));
        assert!(!same_bytes(short(&bytes), whole(&bytes[..2_999_999])));
        assert!(same_bytes(short(&[]), whole(&[])));
    }

    /// A move that stops at a file already in the target folder (put there after the move was
    /// asked for, an older copy say) leaves that file alone, and takes back only what it copied.
    /// The undoing removed every file of the move's names from the target, that one too.
    #[test]
    fn a_move_that_stops_keeps_what_was_there() {
        let t = temp("there");
        let (base, files, target) = (t.join("app"), t.join("C").join(FILES_FOLDER), t.join("D").join("Mail"));
        seed(&base, &files);
        assert_eq!(schedule_move(&base, &target).unwrap(), target);
        // Meanwhile, an older copy of one of them turns up there.
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old.mailcraft"), b"an older copy").unwrap();
        assert!(apply_pending_move(&base).is_err());
        assert_eq!(fs::read(target.join("old.mailcraft")).unwrap(), b"an older copy", "what was there stays");
        assert!(!target.join("zoe@studio.example.mailcraft").exists(), "what the move copied goes");
        assert_eq!(files_dir(&base), files);
        assert!(files.join("old.mailcraft").exists() && files.join("zoe@studio.example.mailcraft").exists(), "nothing moved");
    }
}
