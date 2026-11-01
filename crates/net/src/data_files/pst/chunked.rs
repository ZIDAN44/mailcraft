//! A data file read through a cache of the ranges read from it.
//!
//! outlook-pst reads one B-tree page or block at a time, and the root page of a B-tree again
//! before each: on a plain `File` that was millions of tiny reads, one after another. Here each
//! range read stays in a cache (the root pages, once read, are never read again while they're in
//! it), a read that follows on from the one before reads ahead (the blocks of a big attachment
//! mostly lie in order), and any other reads exactly what was asked, so scattered pages don't
//! read a chunk each for a few hundred bytes they use.
//!
//! The cache is the file's, not a handle's ([`Source`], shared through an `Rc`): every handle
//! outlook-pst reads through starts with what the others read. A `BufReader` wouldn't do: it forgets its buffer
//! at every seek, and outlook-pst seeks before every read.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::rc::Rc;

/// The reader's chunk: what a read that follows on from the last one reads ahead in, four at once.
pub(super) const CHUNK: usize = 64 * 1024;
const READ_AHEAD: usize = 4;
/// The cache holds at most this many chunks' worth of ranges (4 MiB at the default chunk).
const SLOTS: usize = 64;

/// A data file opened to read, and what has been read of it.
pub(super) struct Source {
    file: File,
    /// Its length when opened: reads past it are its end.
    len: u64,
    chunk: usize,
    /// Ranges read, by where they start: they never overlap.
    slots: BTreeMap<u64, Slot>,
    /// The same, by when each was last used (for letting go of the least recently used).
    by_use: BTreeMap<u64, u64>,
    held: usize,
    tick: u64,
    /// Where the last range read from the file ended: a miss there (or just past) reads ahead.
    last_miss: Option<u64>,
    /// Bytes read from the file, and reads made.
    pub(super) bytes_read: u64,
    pub(super) reads: u64,
}

struct Slot {
    data: Vec<u8>,
    used: u64,
}

pub(super) type Shared = Rc<RefCell<Source>>;

impl Source {
    /// `file`, read in chunks of `chunk_kb` KiB (0: [`CHUNK`]).
    pub(super) fn new(file: File, chunk_kb: usize) -> io::Result<Shared> {
        let len = file.metadata()?.len();
        let chunk = match chunk_kb {
            0 => CHUNK,
            kb => kb.clamp(4, 4096).saturating_mul(1024),
        };
        Ok(Rc::new(RefCell::new(Source {
            file,
            len,
            chunk,
            slots: BTreeMap::new(),
            by_use: BTreeMap::new(),
            held: 0,
            tick: 0,
            last_miss: None,
            bytes_read: 0,
            reads: 0,
        })))
    }

    pub(super) fn len(&self) -> u64 {
        self.len
    }

    /// The most the cache holds.
    fn cap(&self) -> usize {
        self.chunk.saturating_mul(SLOTS)
    }

    /// Bytes from `pos` into `buf`, as many as one cached range (or one read) gives: 0 only at
    /// the end of the file.
    fn read_at(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || pos >= self.len {
            return Ok(0);
        }
        if let Some(n) = self.hit(pos, buf) {
            return Ok(n);
        }
        // A miss: read up to the next range already here (they never overlap), or the end.
        let next = self.slots.range(pos..).next().map_or(self.len, |(start, _)| *start).min(self.len);
        let room = next.saturating_sub(pos);
        let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        let follows = self.last_miss.is_some_and(|end| pos >= end && pos - end < wide(self.chunk));
        if buf.len() >= self.chunk {
            // A big read is big enough already: straight into `buf`, not kept.
            let want = usize::try_from(room.min(wide(buf.len()))).unwrap_or(buf.len());
            let got = self.fill(pos, buf.get_mut(..want).unwrap_or_default())?;
            return Ok(got);
        }
        let want = if follows { buf.len().max(self.chunk.saturating_mul(READ_AHEAD)) } else { buf.len() };
        let want = usize::try_from(room.min(wide(want))).unwrap_or(0);
        let mut data = vec![0; want];
        let got = self.fill(pos, &mut data)?;
        data.truncate(got);
        let n = got.min(buf.len());
        if let (Some(to), Some(from)) = (buf.get_mut(..n), data.get(..n)) {
            to.copy_from_slice(from);
        }
        if got > 0 {
            self.keep(pos, data);
        }
        Ok(n)
    }

    /// From the cached range holding `pos`, if one does.
    fn hit(&mut self, pos: u64, buf: &mut [u8]) -> Option<usize> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let (start, slot) = self.slots.range_mut(..=pos).next_back()?;
        let from = usize::try_from(pos - *start).ok()?;
        let have = slot.data.get(from..).filter(|d| !d.is_empty())?;
        let n = have.len().min(buf.len());
        buf.get_mut(..n)?.copy_from_slice(have.get(..n)?);
        self.by_use.remove(&slot.used);
        slot.used = tick;
        self.by_use.insert(tick, *start);
        Some(n)
    }

    /// `buf` filled from the file at `pos`, as far as the file goes: how much was.
    fn fill(&mut self, pos: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.file.seek(SeekFrom::Start(pos))?;
        let mut got = 0;
        while let Some(rest) = buf.get_mut(got..).filter(|r| !r.is_empty()) {
            match self.file.read(rest) {
                Ok(0) => break,
                Ok(n) => got = got.saturating_add(n),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.reads = self.reads.saturating_add(1);
        self.bytes_read = self.bytes_read.saturating_add(u64::try_from(got).unwrap_or(u64::MAX));
        self.last_miss = pos.checked_add(u64::try_from(got).unwrap_or(u64::MAX));
        Ok(got)
    }

    /// `data`, read at `pos`, kept; the least recently used ranges let go to make room.
    fn keep(&mut self, pos: u64, data: Vec<u8>) {
        self.held = self.held.saturating_add(data.len());
        self.tick = self.tick.wrapping_add(1);
        self.by_use.insert(self.tick, pos);
        if let Some(old) = self.slots.insert(pos, Slot { data, used: self.tick }) {
            self.by_use.remove(&old.used);
            self.held = self.held.saturating_sub(old.data.len());
        }
        while self.held > self.cap() {
            let Some((&used, &start)) = self.by_use.iter().next() else { break };
            if start == pos {
                break;
            }
            self.by_use.remove(&used);
            if let Some(gone) = self.slots.remove(&start) {
                self.held = self.held.saturating_sub(gone.data.len());
            }
        }
    }

    /// What the cache holds, in bytes and ranges.
    #[cfg(test)]
    pub(super) fn held(&self) -> (usize, usize) {
        (self.held, self.slots.len())
    }
}

/// One handle's view of a [`Source`]: its own position.
pub(super) struct ChunkReader {
    source: Shared,
    pos: u64,
}

impl ChunkReader {
    /// A new handle on `source`, at its start.
    pub(super) fn new(source: &Shared) -> Self {
        ChunkReader { source: source.clone(), pos: 0 }
    }
}

impl Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.source.try_borrow_mut().map_err(io::Error::other)?.read_at(self.pos, buf)?;
        self.pos = self.pos.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
        Ok(n)
    }
}

impl Seek for ChunkReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "a seek before the start of the file, or past what a position can hold");
        self.pos = match to {
            SeekFrom::Start(n) => n,
            SeekFrom::End(d) => self.source.try_borrow().map_err(io::Error::other)?.len().checked_add_signed(d).ok_or_else(invalid)?,
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).ok_or_else(invalid)?,
        };
        Ok(self.pos)
    }
}
