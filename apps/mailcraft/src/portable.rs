//! Portable mode (issue #4): a marker file beside the executable ([`MARKERS`]) keeps everything
//! MailCraft writes in `<exe dir>/MailCraftData`: the account list, the window's state, the log
//! and the accounts' data files, so a copy on a USB stick leaves nothing in `%APPDATA%`,
//! Documents or the system's keychain. Passwords and sign-ins are then held in memory only and
//! asked for again at the next start, and new-mail notifications name neither sender nor subject
//! (the system keeps them). The Windows portable zip ships with `portable.txt`; the MSI doesn't.
//! The Craft apps share the scheme (PdfCraft's `portable.rs`, PhotoCraft's `app_dirs.rs`,
//! craft-libs' `craft-settings`); on macOS the marker goes beside the `.app`, since writing
//! inside a signed bundle breaks its signature.
//!
//! If that folder can't be written (a read-only medium, Program Files), portable mode stays off,
//! the per-user folders are used and the app says why.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Files beside the executable that switch on portable mode (either one; contents are ignored).
pub const MARKERS: [&str; 2] = ["portable.txt", "MailCraft.portable"];
/// The data folder created beside the executable in portable mode.
pub const DATA_DIR: &str = "MailCraftData";

/// The outcome of looking for a portable marker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Portable {
    /// `<exe dir>/MailCraftData` when portable mode is on.
    pub dir: Option<PathBuf>,
    /// Why portable mode is off although a marker was found.
    pub unwritable: Option<String>,
}

/// Portable mode for this process, resolved once (the check probes the disk).
pub fn current() -> &'static Portable {
    static CURRENT: OnceLock<Portable> = OnceLock::new();
    CURRENT.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        resolve(exe.as_deref().and_then(Path::parent))
    })
}

/// Look for a marker beside the executable in `exe_dir` and, when one is there, make its data
/// folder.
pub fn resolve(exe_dir: Option<&Path>) -> Portable {
    let Some(base) = exe_dir.map(|d| bundle_parent(d).unwrap_or(d)) else { return Portable::default() };
    let Some(marker) = MARKERS.iter().map(|m| base.join(m)).find(|p| p.is_file()) else { return Portable::default() };
    let folder = base.join(DATA_DIR);
    match writable(&folder) {
        Ok(()) => Portable { dir: Some(folder), unwritable: None },
        Err(e) => Portable {
            dir: None,
            unwritable: Some(format!(
                "portable mode is off: {} was found, but {} can't be written ({e}); MailCraft keeps its data in your user folder instead",
                marker.display(),
                folder.display()
            )),
        },
    }
}

/// On macOS the executable is in `X.app/Contents/MacOS`: the folder the bundle is in.
fn bundle_parent(exe_dir: &Path) -> Option<&Path> {
    if !cfg!(target_os = "macos") || exe_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = exe_dir.parent()?;
    let bundle = contents.parent()?;
    (contents.file_name()? == "Contents" && bundle.extension()? == "app").then_some(bundle.parent()?)
}

/// Make `dir` and prove a file can be written in it (a folder can exist yet be read-only).
fn writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    probe(dir, ".write-test")
}

/// Names a probe tries before the folder counts as unwritable.
const PROBE_TRIES: u32 = 64;

/// Prove a file can be made in `dir`: one is made and removed again. It's made new
/// (`create_new`), so whatever is already at its name (a file, a link someone left there
/// pointing elsewhere) is never opened, written through or removed: the next name is tried
/// instead (as PhotoCraft's portable check). Only the probe made here is removed.
pub(crate) fn probe(dir: &Path, stem: &str) -> std::io::Result<()> {
    let pid = std::process::id();
    for n in 0..PROBE_TRIES {
        let path = dir.join(format!("{stem}-{pid}-{n}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "every name for a test file is taken"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mailcraft-portable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// No marker: off, and nothing made. Either marker: the data folder beside it, made (and
    /// what's in it kept). A folder named as a marker isn't one; a data folder that can't be made
    /// turns it off and says why.
    #[test]
    fn a_marker_beside_the_app_keeps_its_data_there() {
        let exe = temp("off");
        assert_eq!(resolve(Some(&exe)), Portable::default());
        assert!(!exe.join(DATA_DIR).exists());
        assert_eq!(resolve(None), Portable::default());
        let _ = std::fs::remove_dir_all(&exe);

        for marker in MARKERS {
            let exe = temp("on");
            std::fs::write(exe.join(marker), b"").unwrap();
            let p = resolve(Some(&exe));
            assert_eq!(p, Portable { dir: Some(exe.join(DATA_DIR)), unwritable: None }, "{marker}");
            assert!(exe.join(DATA_DIR).is_dir());
            assert_eq!(std::fs::read_dir(exe.join(DATA_DIR)).unwrap().count(), 0, "the write probe is removed");
            std::fs::write(exe.join(DATA_DIR).join("accounts.json"), b"[]").unwrap();
            assert_eq!(resolve(Some(&exe)).dir, Some(exe.join(DATA_DIR)));
            assert_eq!(std::fs::read(exe.join(DATA_DIR).join("accounts.json")).unwrap(), b"[]", "what's there is kept");
            let _ = std::fs::remove_dir_all(&exe);
        }

        let exe = temp("dir-marker");
        std::fs::create_dir_all(exe.join("portable.txt")).unwrap();
        assert_eq!(resolve(Some(&exe)), Portable::default());
        let _ = std::fs::remove_dir_all(&exe);

        let exe = temp("unwritable");
        std::fs::write(exe.join("portable.txt"), b"").unwrap();
        std::fs::write(exe.join(DATA_DIR), b"a file in the way").unwrap();
        let p = resolve(Some(&exe));
        assert_eq!(p.dir, None);
        assert!(p.unwritable.is_some_and(|w| w.contains("portable mode is off") && w.contains("can't be written")));
        let _ = std::fs::remove_dir_all(&exe);
    }

    /// What's already at a probe's name is never written through or removed: files there (and,
    /// on Unix, links to a file elsewhere) stay as they were, the next free name is used, and
    /// with every name taken the folder counts as unwritable, still touching nothing.
    #[test]
    fn the_write_probe_never_goes_through_what_is_at_its_name() {
        let root = temp("probe-taken");
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let other = root.join("elsewhere.txt");
        std::fs::write(&other, b"keep me").unwrap();
        let pid = std::process::id();
        let name = |n: u32| data.join(format!(".write-test-{pid}-{n}"));
        let take = |n: u32| {
            #[cfg(unix)]
            if n.is_multiple_of(2) {
                std::os::unix::fs::symlink(&other, name(n)).unwrap();
                return;
            }
            std::fs::write(name(n), b"someone's").unwrap();
        };
        for n in 0..PROBE_TRIES - 1 {
            take(n);
        }
        writable(&data).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"keep me");
        assert!(std::fs::symlink_metadata(name(0)).is_ok() && std::fs::symlink_metadata(name(1)).is_ok(), "what was there stays");
        assert!(!name(PROBE_TRIES - 1).exists(), "the probe made is removed");
        take(PROBE_TRIES - 1);
        assert!(writable(&data).is_err());
        assert_eq!(std::fs::read(&other).unwrap(), b"keep me");
        assert_eq!(std::fs::read(name(1)).unwrap(), b"someone's");
        let _ = std::fs::remove_dir_all(&root);
    }
}
