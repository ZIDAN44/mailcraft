//! Opening a data file to read.
//!
//! The file is opened to read only, and while it's open no other program may open it to write
//! (on Windows; elsewhere nothing stops one): the import never asks to write it, and Outlook
//! can't change it under the reader. A file Outlook has open already fails to open, which is how
//! the import knows to say so.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use super::chunked::{ChunkReader, Shared, Source};

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

/// `path` opened to read through a cache of its own, in chunks of `chunk_kb` KiB (0: the default).
pub(super) fn source(path: &Path, chunk_kb: usize) -> io::Result<Shared> {
    Source::new(open_read(path)?, chunk_kb)
}

/// A handle on `source`, new, at its start.
pub(super) fn handle(source: &Shared) -> Box<ChunkReader> {
    Box::new(ChunkReader::new(source))
}
