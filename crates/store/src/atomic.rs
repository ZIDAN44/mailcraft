//! How the cache puts files on disk: a file is replaced in one step, so a reader sees the old
//! contents or the new ones and never half of either, and what MailCraft writes stays its owner's.
//! The writing itself is craft-libs' `craft-atomic`, which the other Craft apps' saves use: the
//! new contents go to a file made new beside the target (never through one already at its name),
//! flushed to the disk (on a Mac fully, or plainly where a network share can't), and renamed over
//! it, tried again while another program holds the file a moment on Windows.

use std::fs;
use std::path::Path;

use craft_atomic::{NewFilePermissions, RenameRetry, WriteOptions};

use crate::{Result, io_err};

/// Replaces `path` with `bytes`: they go to a scratch file beside it, are flushed to the disk, and
/// the scratch file is then renamed over `path`. Missing folders are made first. On any failure
/// the scratch file is removed and `path` keeps what it had. A new file, on Unix, is its owner's
/// only (the cache holds mail, and the computer may be shared).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        fs::create_dir_all(folder).map_err(io_err(folder))?;
    }
    WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private).atomic_write(path, bytes).map_err(io_err(path))
}

/// Make a rename to `path` last too: its folder flushed to the disk, so a power cut can't bring
/// back the old name. Unix only (Windows can't open a folder to flush it); best effort. (Data
/// files compact natively only.)
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sync_folder_of(path: &Path) {
    #[cfg(unix)]
    if let Some(folder) = path.parent()
        && let Ok(handle) = fs::File::open(folder)
    {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Renames `from` to `to`. On Windows a file another program has open for a moment (OneDrive
/// syncing Documents, an antivirus scan, the search indexer) refuses the rename with a sharing
/// or lock violation, or access denied: it's tried again for up to about a second before that
/// counts as a failure (craft-atomic's `retry_rename`).
pub(crate) fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    craft_atomic::retry_rename(RenameRetry::platform(), || fs::rename(from, to))
}

/// Makes the cache's folder if it isn't there. On Unix a folder that others can get into is
/// closed to everyone but its owner (0700); one already closed is left as it is.
pub(crate) fn private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(io_err(dir))?;
    #[cfg(unix)]
    close_to_others(dir);
    Ok(())
}

/// Takes away group and world access to `dir`, when it has any. Best effort.
#[cfg(unix)]
fn close_to_others(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    const OTHERS: u32 = 0o077;
    let open_to_others = fs::metadata(dir).map(|m| m.permissions().mode() & OTHERS != 0).unwrap_or(false);
    if open_to_others {
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written whole or not at all, readable by its owner only on Unix.
    #[test]
    fn written_whole_and_private() {
        let root = std::env::temp_dir().join(format!("mailcraft-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let target = root.join("nested").join("state.json");
        for contents in [&b"first"[..], b"second, longer", b"third"] {
            write_atomic(&target, contents).unwrap();
            assert_eq!(fs::read(&target).unwrap(), contents);
        }
        let names: Vec<_> = fs::read_dir(root.join("nested")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")], "scratch files are gone");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bits = |at: &Path| 0o777 & fs::metadata(at).map(|m| m.permissions().mode()).unwrap();
            assert_eq!(bits(&target), 0o600);
            fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
            private_dir(&root).unwrap();
            assert_eq!(bits(&root), 0o700, "a folder others could enter is closed");
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// A link at the target's name is kept, and the file it names written (as the other Craft
    /// apps' saves do); nothing is written through a file waiting at a scratch name.
    #[cfg(unix)]
    #[test]
    fn a_link_is_kept_and_its_file_written() {
        let root = std::env::temp_dir().join(format!("mailcraft-replace-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let real = root.join("real.json");
        fs::write(&real, b"old").unwrap();
        let link = root.join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&real).unwrap(), b"new");
        let _ = fs::remove_dir_all(&root);
    }

    /// On Windows itself: a file held open by another program (no delete sharing, as OneDrive or
    /// an antivirus holds one) refuses a plain rename; held for 150 ms, it's written once let go.
    #[cfg(windows)]
    #[test]
    fn a_file_held_for_a_moment_is_still_written() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = std::env::temp_dir().join(format!("mailcraft-held-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let target = d.join("accounts.json");
        fs::write(&target, b"old").unwrap();
        const SHARE_READ: u32 = 1;
        let held = fs::OpenOptions::new().read(true).share_mode(SHARE_READ).open(&target).unwrap();
        let scratch = d.join("scratch.part");
        fs::write(&scratch, b"new").unwrap();
        assert!(fs::rename(&scratch, &target).is_err(), "Windows refuses a plain rename over it");
        let letting_go = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(held);
        });
        write_atomic(&target, b"new").unwrap();
        letting_go.join().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        let _ = fs::remove_dir_all(&d);
    }
}
