//! How the cache puts files on disk: a file is replaced in one step, so a reader sees the old
//! contents or the new ones and never half of either, and what MailCraft writes stays its owner's.

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Result, io_err};

/// Counts the writes this process has made, so that two writes of one file at the same moment
/// each get a scratch file of their own.
static SCRATCH_SERIAL: AtomicUsize = AtomicUsize::new(0);

/// Replaces `path` with `bytes`: they go to a scratch file beside it, are flushed to the disk, and
/// the scratch file is then renamed over `path`. Missing folders are made first. On any failure
/// the scratch file is removed and `path` keeps what it had.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(io_err(folder))?;
    }
    let scratch = scratch_beside(path);
    // Flushed before the rename, so a power cut can't leave the new name on an empty file.
    let filled = owner_only_file(&scratch).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    // A failure while filling is reported against the scratch file, one while renaming against
    // the target; either way the scratch file goes.
    let outcome = match filled {
        Err(e) => Err(io_err(&scratch)(e)),
        Ok(()) => rename(&scratch, path).map_err(io_err(path)),
    };
    if outcome.is_err() {
        let _ = fs::remove_file(&scratch);
    }
    outcome?;
    // Make the rename itself last too. Unix only: Windows can't open a folder to flush it.
    #[cfg(unix)]
    if let Some(folder) = path.parent()
        && let Ok(handle) = fs::File::open(folder)
    {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// Renames `from` to `to`. On Windows a file another program has open for a moment (OneDrive
/// syncing Documents, an antivirus scan, the search indexer) refuses the rename with a sharing
/// or lock violation, or access denied: it's tried again for up to about a second before that
/// counts as a failure (craft-libs' atomic writes found this).
pub(crate) fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    return patiently(|| fs::rename(from, to), |ms| std::thread::sleep(std::time::Duration::from_millis(ms)));
    #[cfg(not(windows))]
    fs::rename(from, to)
}

/// `attempt` until it works, fails for good, or has been held up seven times (waiting 10, 20,
/// 40… ms between, at most 200).
#[cfg(any(windows, test))]
fn patiently(mut attempt: impl FnMut() -> std::io::Result<()>, mut wait: impl FnMut(u64)) -> std::io::Result<()> {
    const SHARING_VIOLATION: i32 = 32;
    const LOCK_VIOLATION: i32 = 33;
    let held =
        |e: &std::io::Error| e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(SHARING_VIOLATION | LOCK_VIOLATION));
    let (mut delay, mut left) = (10u64, 7u32);
    loop {
        match attempt() {
            Err(e) if held(&e) && left > 0 => {
                left -= 1;
                wait(delay);
                delay = delay.saturating_mul(2).min(200);
            }
            done => return done,
        }
    }
}

/// `.<name>.<process>.<serial>.part`, in the same folder as `path` so the rename stays on one
/// file system.
fn scratch_beside(path: &Path) -> PathBuf {
    let serial = SCRATCH_SERIAL.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{}.{serial}.part", std::process::id()))
}

/// Creates (or empties) a file that, on Unix, only its owner may read or write: the cache holds
/// mail, and the computer may be shared.
fn owner_only_file(at: &Path) -> std::io::Result<fs::File> {
    let mut how = fs::File::options();
    #[cfg(unix)]
    how.mode(OWNER_READ_WRITE);
    how.write(true).truncate(true).create(true);
    how.open(at)
}

/// `rw-------`.
#[cfg(unix)]
const OWNER_READ_WRITE: u32 = 0o600;

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
            assert_eq!(bits(&target), OWNER_READ_WRITE);
            fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
            private_dir(&root).unwrap();
            assert_eq!(bits(&root), 0o700, "a folder others could enter is closed");
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// A rename another program holds up a few times goes through; one refused for another
    /// reason, or held up for good, fails, after waiting a second at most.
    #[test]
    fn a_held_rename_is_tried_again() {
        use std::io::{Error, ErrorKind};
        let mut waited = Vec::new();
        let mut tries = 0;
        let r = patiently(
            || {
                tries += 1;
                if tries < 4 { Err(Error::from_raw_os_error(32)) } else { Ok(()) }
            },
            |ms| waited.push(ms),
        );
        assert!(r.is_ok());
        assert_eq!(waited, vec![10, 20, 40]);
        let mut waited = Vec::new();
        let r = patiently(|| Err(Error::from(ErrorKind::PermissionDenied)), |ms| waited.push(ms));
        assert_eq!(r.unwrap_err().kind(), ErrorKind::PermissionDenied);
        assert_eq!(waited, vec![10, 20, 40, 80, 160, 200, 200]);
        assert!(waited.iter().sum::<u64>() < 1000);
        let mut tries = 0;
        let r = patiently(
            || {
                tries += 1;
                Err(Error::from(ErrorKind::NotFound))
            },
            |_| panic!("no wait for a file that isn't there"),
        );
        assert!(r.is_err() && tries == 1);
    }
}
