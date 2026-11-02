//! Opening a data file to read, and reopening the handles that read it.
//!
//! The file is opened to read only, and while it's open no other program may open it to write
//! (on Windows; elsewhere nothing stops one): the import never asks to write it, and Outlook
//! can't change it under the reader. A file Outlook has open already fails to open, which is how
//! the import knows to say so.
//!
//! outlook-pst keeps every B-tree page a handle reads for as long as the handle lives: over a
//! whole file, hundreds of megabytes. So the handle that reads items is let go of and opened
//! again (from the same [`Source`], which keeps what was read) once the pages it has read are
//! thought to hold [`HANDLE_CACHE`].

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use super::chunked::{ChunkReader, Pages, Shared, Source};
use super::*;

/// The most a handle's own caches are let hold before it's reopened.
pub(super) const HANDLE_CACHE: u64 = 16 * 1024 * 1024;

/// `path` opened to read only, shared with other readers but not with writers.
pub(super) fn open_read(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        /// `FILE_SHARE_READ`: others may read it while it's open, not write or delete it.
        const FILE_SHARE_READ: u32 = 0x1;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

/// `path` opened to read through a cache of its own, in the chunks `opts` say (`chunk_kb`), its
/// reads failing once the import is asked to stop.
pub(super) fn source(path: &Path, opts: &ImportOptions) -> io::Result<Shared> {
    Source::new(open_read(path)?, opts.chunk_kb, opts.stop.clone())
}

/// When the handle that reads items is opened again.
#[derive(Clone, Copy)]
pub(super) struct Recycle {
    /// After this many items, whatever it holds (0: only when it holds too much): for tests and
    /// measuring.
    pub(super) every: usize,
    /// Once its caches are thought to hold this much.
    pub(super) limit: u64,
}

impl Recycle {
    pub(super) fn of(opts: &ImportOptions) -> Self {
        let limit = if opts.handle_cache_bytes > 0 { opts.handle_cache_bytes } else { HANDLE_CACHE };
        Recycle { every: opts.recycle, limit }
    }

    /// Whether a handle that has read `items` items and these `pages` is due to be reopened.
    pub(super) fn due(&self, items: usize, pages: &Pages) -> bool {
        (self.every > 0 && items >= self.every) || pages.estimate() >= self.limit
    }
}

/// A handle on `source`, new, at its start, and the pages it will read.
pub(super) fn handle(source: &Shared) -> (Box<ChunkReader>, Pages) {
    let (reader, pages) = ChunkReader::new(source);
    (Box::new(reader), pages)
}
