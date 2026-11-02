//! Leases: a file the one process doing something locks while it does it (an account's search
//! index written), so another process on the same data folder (the CLI and the app, a second
//! copy of the app) finds it's being done elsewhere. The lock is the system's
//! (`File::try_lock`), let go however the process ends: a crash leaves no lease behind, only its
//! file, which the next to take it locks at once.
//!
//! A lease's file is never deleted: one unlinked while another process opens it leaves that
//! process locking a file no longer at its name, and the next to open the name locks a new one,
//! two holders at once. Its contents are never read or written.
//!
//! Where the file system has no locks (some network shares), a lease holds nothing and the log
//! says so once for its file: the in-process guard alone applies then, as a launch goes on
//! without its election lock (`single_instance`). The browser has no other process: its lease is
//! a stand-in.

use super::*;

/// A lease taken: let go when dropped.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(crate) struct Lease {
    /// The locked file; `None` where the file system has no locks.
    file: Option<fs::File>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Lease {
    fn drop(&mut self) {
        // Closing the file lets the lock go too: an error here is no matter.
        if let Some(f) = &self.file {
            let _ = f.unlock();
        }
    }
}

/// Take the lease at `path`; `None` when another holds it (another process, or another store on
/// the same folder in this one). Its folder is made when it isn't there.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn take(path: &Path) -> Result<Option<Lease>> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(io_err(path))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lease { file: Some(file) })),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(e)) => {
            no_locks(path, &e);
            Ok(Some(Lease { file: None }))
        }
    }
}

/// Said once a file: the file system can't lock it.
#[cfg(not(target_arch = "wasm32"))]
fn no_locks(path: &Path, e: &io::Error) {
    static SAID: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let mut said = SAID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if said.get_or_insert_with(HashSet::new).insert(path.to_path_buf()) {
        log::warn!("{}: can't be locked ({e}): another copy of MailCraft isn't kept from writing at the same time", path.display());
    }
}

/// The browser's lease: there is no other process to keep out.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(crate) struct Lease;

#[cfg(target_arch = "wasm32")]
pub(crate) fn take(_path: &Path) -> Result<Option<Lease>> {
    Ok(Some(Lease))
}
