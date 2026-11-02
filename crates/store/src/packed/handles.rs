//! A data file as its database reads and writes it: the database's own handle for writes, its
//! flushes and its locks, and on Windows handles of their own for reads.
//!
//! On Windows every read and write through one handle opened for ordinary (synchronous) use
//! waits for the one before it on that handle: the index's eight readers, the marks' commits and
//! their flushes and the window's reads all queued on the one handle. Reading the 41 GB file's
//! sources for its index took 110 s with eight readers, 119 s with sixteen and 164 s with two
//! (each source read 6.5 ms by itself, 26 ms with sixteen others), and eight threads reading
//! 4 KiB pieces of the file through one handle read no faster than one, where each through its
//! own read 17 times as fast. Writes through one handle are seen at once through the others (the
//! system's cache is the file's, whatever the handle).
//!
//! The database keeps another process out with byte-range locks taken through its own handle,
//! and Windows' locks hold back every other handle's reads of what they cover: the writer's
//! reaches from the file's start to its lock bytes, far past any data. With handles of its own
//! for reads, a lock is taken from the first of those bytes ([`LOCKS_FROM`]) instead, so another
//! copy of MailCraft opening the file still meets it there. Elsewhere a file's reads at an offset
//! don't wait on each other, and closing a second handle could let go of the process's locks on
//! the file, so reads go through the database's handle there.

use std::fs::{File, OpenOptions};
use std::io;
use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use redb::backends::FileBackend;
use redb::{BackendError, DatabaseError, StorageBackend};

use crate::OpenProgress;

/// Handles of its own a data file reads through (Windows): more than the index's readers, so a
/// read from the window finds one free while they all read.
#[cfg(windows)]
const READ_HANDLES: usize = 10;
/// Where the database's lock bytes start (redb's `LOCK_BASE`, `=4.4.0`): no data is there.
const LOCKS_FROM: u64 = 1 << 62;

/// The threads that have read so far, counted: each one's first handle.
static THREADS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The handle this thread reads through when it's free. One thread's reads stay on one
    /// handle, so the system reads ahead of a thread going through the file (a check after a
    /// crash took 176 s rather than 98 s with each read on the next handle in turn).
    static FIRST: usize = THREADS.fetch_add(1, Ordering::Relaxed);
}

/// A data file's handles.
#[derive(Debug)]
pub(super) struct DataFile {
    file: FileBackend,
    /// Each used by one read at a time (none: reads go through `file`).
    reads: Vec<Mutex<File>>,
    /// Told of the bytes read: how far a check of the file has come.
    counted: Option<Arc<OpenProgress>>,
}

impl DataFile {
    /// The data file at `path` (made when it isn't there), with its handles for reads; one that
    /// can't be opened leaves it fewer (none: its reads go through the database's handle).
    pub(super) fn open(path: &Path) -> Result<DataFile, DatabaseError> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        let file = FileBackend::new(file)?;
        #[cfg(windows)]
        let reads = (0..READ_HANDLES)
            .map_while(|_| File::open(path).inspect_err(|e| log::warn!("{}: read on one handle: {e}", path.display())).ok())
            .map(Mutex::new)
            .collect();
        #[cfg(not(windows))]
        let reads = Vec::new();
        Ok(DataFile { file, reads, counted: None })
    }

    /// Its reads told to `progress` ([`OpenProgress::read`]).
    pub(super) fn counted(mut self, progress: &Arc<OpenProgress>) -> DataFile {
        self.counted = Some(progress.clone());
        self
    }

    /// How many handles of its own it reads through.
    #[cfg(test)]
    pub(super) fn read_handles(&self) -> usize {
        self.reads.len()
    }
}

impl DataFile {
    /// The range a lock over `start..end` takes: from [`LOCKS_FROM`] when it reaches there from
    /// the data before it (with reads through handles of their own, which it would hold back).
    fn locked(&self, start: Bound<u64>, end: Bound<u64>) -> (Bound<u64>, Bound<u64>) {
        let from_data = match start {
            Bound::Unbounded => true,
            Bound::Included(s) => s < LOCKS_FROM,
            Bound::Excluded(s) => s < LOCKS_FROM.saturating_sub(1),
        };
        let reaches = match end {
            Bound::Unbounded => true,
            Bound::Included(e) => e >= LOCKS_FROM,
            Bound::Excluded(e) => e > LOCKS_FROM,
        };
        if !self.reads.is_empty() && from_data && reaches { (Bound::Included(LOCKS_FROM), end) } else { (start, end) }
    }
}

/// Whether a database has the file at `path` open, in this MailCraft or another: its lock bytes
/// are held. Asked through a handle of its own, as another copy opening it would, since with
/// reads through handles of their own nothing else stops the file being read while it's written.
pub(super) fn in_use(path: &Path) -> io::Result<bool> {
    let probe = FileBackend::new(File::open(path)?).map_err(io::Error::other)?;
    probe.query_lock_range(Bound::Included(LOCKS_FROM), Bound::Included(LOCKS_FROM.saturating_add(1))).map_err(io::Error::other)
}

/// Fill `out` from `file` at `offset`; a short file is an error (as the database's own reads).
fn read_at(file: &File, mut offset: u64, out: &mut [u8]) -> io::Result<()> {
    let mut at = 0usize;
    while let Some(rest) = out.get_mut(at..).filter(|r| !r.is_empty()) {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, rest, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, rest, offset)?;
        #[cfg(not(any(windows, unix)))]
        let n = {
            let _ = (file, rest);
            return Err(io::Error::from(io::ErrorKind::Unsupported));
        };
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "failed to fill whole buffer"));
        }
        at = at.saturating_add(n);
        offset = offset.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    }
    Ok(())
}

impl StorageBackend for DataFile {
    fn len(&self) -> io::Result<u64> {
        self.file.len()
    }

    /// Through the thread's own handle, or another no other read is using when it's busy (its
    /// own, waited for, when all are).
    fn read(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        if let Some(p) = &self.counted {
            p.read(out.len());
        }
        let n = self.reads.len();
        if n == 0 {
            return self.file.read(offset, out);
        }
        let first = FIRST.with(|f| *f);
        for i in 0..n {
            if let Some(Ok(f)) = self.reads.get(first.wrapping_add(i) % n).map(Mutex::try_lock) {
                return read_at(&f, offset, out);
            }
        }
        match self.reads.get(first % n) {
            Some(m) => read_at(&m.lock().unwrap_or_else(std::sync::PoisonError::into_inner), offset, out),
            None => self.file.read(offset, out),
        }
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
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
        let (start, end) = self.locked(start, end);
        self.file.try_lock_range(start, end)
    }

    fn try_lock_shared_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<bool, BackendError> {
        let (start, end) = self.locked(start, end);
        self.file.try_lock_shared_range(start, end)
    }

    fn lock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        let (start, end) = self.locked(start, end);
        self.file.lock_range(start, end)
    }

    fn lock_shared_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        let (start, end) = self.locked(start, end);
        self.file.lock_shared_range(start, end)
    }

    fn unlock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<(), BackendError> {
        let (start, end) = self.locked(start, end);
        self.file.unlock_range(start, end)
    }

    fn query_lock_range(&self, start: Bound<u64>, end: Bound<u64>) -> Result<bool, BackendError> {
        let (start, end) = self.locked(start, end);
        self.file.query_lock_range(start, end)
    }
}
