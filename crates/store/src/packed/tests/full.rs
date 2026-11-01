//! A drive that fills up as a data file is written: the write fails saying the drive is full,
//! and once there's room again the file is written to as before, nothing written before lost.

use std::fs::OpenOptions;
use std::io;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use redb::backends::FileBackend;
use redb::{BackendError, Database, DatabaseError, StorageBackend};

use super::super::{disk, from_disk, growth};
use super::commits::inbox;
use super::*;

/// A data file on a drive that is full while `full` is set: growing it fails as the disk would.
#[derive(Debug)]
struct Filling {
    file: FileBackend,
    full: Arc<AtomicBool>,
}

/// The data file at `path` opened as `b` says, on a drive full while `full` is set.
pub(in super::super) fn create(b: &redb::Builder, path: &Path, full: &Arc<AtomicBool>) -> Result<Database, DatabaseError> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    b.create_with_backend(Filling { file: FileBackend::new(file)?, full: full.clone() })
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

/// `n` bytes that don't compress (a data file keeps what it's given compressed).
fn noise(n: usize, mut x: u64) -> Vec<u8> {
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

/// The drive full as a write grows the data file: the write fails as one for want of room (the
/// disk's own error, not the database's word for it); with room again, the file is opened again
/// and written to, and what was written before the full drive is all there.
#[test]
fn a_full_drive_fails_a_write_as_such_and_room_again_writes() {
    let t = temp("full-drive");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let first = noise(1 << 20, 7);
    s.write_body(&inbox(1), &first).unwrap();
    s.seal("a1").unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs.full.store(true, Ordering::Relaxed);
    let big = noise(64 << 20, 11);
    let e = s.write_body(&inbox(2), &big).unwrap_err();
    assert!(matches!(&e, crate::StoreError::Io { source, .. } if source.kind() == io::ErrorKind::StorageFull), "{e}");
    // Still full: refused the same way, not as a database that gave up after a failed write.
    let e = s.write_body(&inbox(3), &big).unwrap_err();
    assert!(matches!(&e, crate::StoreError::Io { source, .. } if source.kind() == io::ErrorKind::StorageFull), "{e}");
    packs.full.store(false, Ordering::Relaxed);
    s.write_body(&inbox(2), &big).unwrap();
    assert_eq!(s.read_body(&inbox(1)).as_deref(), Some(first.as_slice()));
    assert_eq!(s.read_body(&inbox(2)).map(|b| b.len()), Some(big.len()));
    assert_eq!(s.read_body(&inbox(3)), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// How far a data file can grow in one step, as the database grows it: doubling while small,
/// then to the end of its region, or past the next when its own is nearly full.
#[test]
fn a_data_file_grows_at_most_by_a_region_or_two() {
    const GIB: u64 = 1 << 30;
    assert_eq!(growth(0), 64 << 20);
    assert_eq!(growth(GIB), GIB);
    assert_eq!(growth(4 * GIB - 1), 4 * GIB - 1);
    assert_eq!(growth(4 * GIB), 4 * GIB);
    assert_eq!(growth(38 * GIB), 2 * GIB);
    assert_eq!(growth(39 * GIB + GIB / 2), GIB / 2 + 4 * GIB);
    assert!(growth(u64::MAX) <= 8 * GIB);
}

/// A write refused because an earlier one failed at the disk (on another thread) is one to open
/// the data file again for, as the failed one is; one refused over what was written isn't.
#[test]
fn a_database_refusing_after_a_failed_write_is_opened_again() {
    assert!(from_disk(&disk(redb::Error::PreviousIo)));
    assert!(from_disk(&disk(redb::Error::Io(io::Error::from(io::ErrorKind::StorageFull)))));
    assert!(!from_disk(&disk(redb::Error::ValueTooLarge(1))));
}
