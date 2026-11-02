//! Leases: a file the one process doing something locks while it does it (an account's search
//! index written), so another process on the same data folder (the CLI and the app, a second
//! copy of the app) finds it's being done elsewhere. The lock is the system's
//! (`File::try_lock`), let go however the process ends: a crash leaves no lease behind, only its
//! file, which the next to take it locks at once.
//!
//! One is shared: every store on the folder layout ([`Store::open`]) holds the folders lease
//! (`folders.lock`) shared for its life, and data files bringing those folders in take it alone
//! (`packed.rs`), so folders another copy of MailCraft is writing are never moved from under it.
//! The app's store on the folder layout ([`Store::open_alone`]) holds it alone, so a second
//! window there (where the single-instance check can't keep it out) doesn't write the folder
//! states and `ops.json` the first writes whole.
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
    lock(path, false)
}

/// Take the lease at `path`, `shared` with others taking it so or alone; `None` when it can't
/// be had (one holds it alone, or any holds it while it's asked for alone).
#[cfg(not(target_arch = "wasm32"))]
fn lock(path: &Path, shared: bool) -> Result<Option<Lease>> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let file = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(io_err(path))?;
    match if shared { file.try_lock_shared() } else { file.try_lock() } {
        Ok(()) => Ok(Some(Lease { file: Some(file) })),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(e)) => {
            no_locks(path, &e);
            Ok(Some(Lease { file: None }))
        }
    }
}

/// The folders lease of the store at `root`: held shared by every store on its folder layout,
/// alone by data files bringing those folders in.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn folders(root: &Path) -> PathBuf {
    root.join(search::LOCKS).join("folders.lock")
}

/// What a store on the folder layout at `root` holds for its life, shared by its clones and let
/// go with the last of them: the folders lease, shared with any other such store, or held
/// `alone` (the app's: no other copy of MailCraft writes its folder states and `ops.json`
/// meanwhile). An error while another holds it alone, or data files are bringing those folders
/// in.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn folder_layout(root: &Path, alone: bool) -> Result<Option<Arc<Lease>>> {
    match lock(&folders(root), !alone)? {
        Some(lease) => Ok(Some(Arc::new(lease))),
        None => Err(in_another_copy(root)),
    }
}

/// The browser's folder layout is in memory: nothing to hold.
#[cfg(target_arch = "wasm32")]
pub(crate) fn folder_layout(_root: &Path, _alone: bool) -> Result<Option<Arc<Lease>>> {
    Ok(None)
}

/// The folders at `root` are another copy of MailCraft's (its folder layout's, or its data files'
/// bringing them in). The CLI tells it apart by "open in another copy of MailCraft".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn in_another_copy(root: &Path) -> StoreError {
    io_err(root)(io::Error::other("its mail is open in another copy of MailCraft"))
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
