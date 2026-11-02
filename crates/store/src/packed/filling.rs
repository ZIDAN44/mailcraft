//! A drive a test can fill ([`Packs::full`](super::Packs)): a data file on it fails to grow as on
//! a full drive while it's set. Built for the store's own tests and, with the `test-drive`
//! feature, for those of the crates over it (`Store::fill_drive`).

use std::io;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use redb::{BackendError, Database, DatabaseError, StorageBackend};

use super::handles::DataFile;

/// A data file on a drive that is full while `full` is set: growing it fails as the disk would.
#[derive(Debug)]
struct Filling {
    file: DataFile,
    full: Arc<AtomicBool>,
}

/// The data file at `path` opened as `b` says, on a drive full while `full` is set.
pub(super) fn create(b: &redb::Builder, path: &Path, full: &Arc<AtomicBool>) -> Result<Database, DatabaseError> {
    b.create_with_backend(Filling { file: DataFile::open(path)?, full: full.clone() })
}

impl StorageBackend for Filling {
    fn len(&self) -> io::Result<u64> {
        self.file.len()
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        self.file.read(offset, out)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        if self.full.load(Ordering::Relaxed) && len > self.file.len()? {
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        self.file.set_len(len)
    }

    fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.file.write(offset, data)
    }

    fn close(&self) -> io::Result<()> {
        self.file.close()
    }

    fn try_lock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<bool, BackendError> {
        self.file.try_lock_range(start, end)
    }

    fn try_lock_shared_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<bool, BackendError> {
        self.file.try_lock_shared_range(start, end)
    }

    fn lock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        self.file.lock_range(start, end)
    }

    fn lock_shared_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        self.file.lock_shared_range(start, end)
    }

    fn unlock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        self.file.unlock_range(start, end)
    }

    fn query_lock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<bool, BackendError> {
        self.file.query_lock_range(start, end)
    }
}
