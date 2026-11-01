//! How a file is kept in a data file: compressed when that helps, whole or in pieces.

use super::*;

/// The main table: path inside the account's folder → bytes.
pub(super) const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
/// Big files in pieces: (path, piece) → bytes. The database gives a value whole runs of pages,
/// doubling (a 300 KB message took 512 KB, and imported mail half again its size in waste), so a
/// big file goes in pieces that each fill their pages, and `FILES` keeps a note of it.
pub(super) const PIECES: TableDefinition<(&str, u32), &[u8]> = TableDefinition::new("pieces");
/// The note in `FILES` for a file kept in pieces: this, then its length (8 bytes, little-endian).
/// Nothing MailCraft keeps starts with a NUL.
const PIECED: &[u8] = b"\0MailCraft:pieces\0";
/// Files up to this size (as kept: compressed when that helps) stay whole: one page's worth. A
/// bigger value took the next power of two of pages, so it goes in pieces.
pub(super) const WHOLE_MAX: usize = PAGE - PAGE_SLACK;
/// A file kept compressed starts with this, then its deflate stream.
pub(super) const DEFLATED: &[u8] = b"\0MailCraft:deflate\0";
/// Files smaller than this aren't worth compressing.
const DEFLATE_MIN: usize = 1024;
/// The most a compressed file may come to (a crafted data file can't fill the memory).
pub(super) const INFLATE_MAX: u64 = 1 << 30;

/// A file as kept: compressed (fast deflate) when that saves a tenth or more.
///
/// miniz_oxide is called directly rather than through flate2: in the app, another crate turns
/// flate2's zlib-rs backend on for everyone, and its fastest level (static Huffman tables) kept
/// base64 attachments at 0.995 of their size, so they were stored raw; miniz's level 1 keeps
/// them at about 0.76, and is faster. The stream is the same raw deflate either way.
///
/// The stream is written straight after the prefix into a buffer that holds no more than would
/// be kept, so there's no second copy, and a file that won't compress stops as soon as it fills it.
pub(super) fn pack(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    use miniz_oxide::deflate::core::{CompressorOxide, TDEFLFlush, TDEFLStatus, compress, create_comp_flags_from_zip_params};
    if bytes.len() < DEFLATE_MIN || bytes.starts_with(DEFLATED) {
        return bytes.into();
    }
    // Kept compressed only when it comes to less than this.
    let limit = bytes.len() - bytes.len() / 10;
    let mut out = vec![0; limit];
    let Some(prefix) = out.get_mut(..DEFLATED.len()) else { return bytes.into() };
    prefix.copy_from_slice(DEFLATED);
    // Level 1, raw deflate (window bits 0), the default strategy: as `compress_to_vec(bytes, 1)`.
    let mut deflate = CompressorOxide::new(create_comp_flags_from_zip_params(1, 0, 0));
    let (mut read, mut at) = (0, DEFLATED.len());
    loop {
        let (Some(input), Some(room)) = (bytes.get(read..), out.get_mut(at..)) else { return bytes.into() };
        if room.is_empty() {
            return bytes.into();
        }
        let (status, took, wrote) = compress(&mut deflate, input, room, TDEFLFlush::Finish);
        read = read.saturating_add(took);
        at = at.saturating_add(wrote);
        match status {
            TDEFLStatus::Done if at < limit => break,
            TDEFLStatus::Okay if took > 0 || wrote > 0 => {}
            _ => return bytes.into(),
        }
    }
    out.truncate(at);
    out.into()
}

/// [`pack`] for a file the caller lets go of: what doesn't compress is kept as it is, not copied.
pub(crate) fn pack_owned(raw: Vec<u8>) -> Vec<u8> {
    let packed = match pack(&raw) {
        std::borrow::Cow::Owned(v) => Some(v),
        std::borrow::Cow::Borrowed(_) => None,
    };
    packed.unwrap_or(raw)
}

/// A file as kept, back as it was written.
pub(crate) fn unpack(kept: Vec<u8>) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let Some(stream) = kept.strip_prefix(DEFLATED) else { return Ok(kept) };
    let mut out = Vec::with_capacity(stream.len().saturating_mul(3).min(64 << 20));
    flate2::read::DeflateDecoder::new(stream).take(INFLATE_MAX + 1).read_to_end(&mut out)?;
    if out.len() as u64 > INFLATE_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a compressed file is too big"));
    }
    Ok(out)
}
/// The database's page, and what a page run needs besides the value (its header and the key).
pub(super) const PAGE: usize = 4096;
pub(super) const PAGE_SLACK: usize = 512;

/// The sizes a file of `len` bytes is cut into: pieces filling 16 pages, then the rest in pieces
/// filling 8, 4, 2 and 1 (the last one smaller).
pub(super) fn piece_sizes(len: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut left = len;
    while left > 0 {
        let fits = [16usize, 8, 4, 2, 1].into_iter().map(|pages| pages * PAGE - PAGE_SLACK).find(|cap| *cap <= left);
        let n = fits.unwrap_or(left).min(left);
        out.push(n);
        left -= n;
    }
    out
}

/// A file kept in pieces: its length.
pub(super) fn pieced_len(note: &[u8]) -> Option<u64> {
    let rest = note.strip_prefix(PIECED)?;
    Some(u64::from_le_bytes(rest.get(..8)?.try_into().ok()?))
}

/// An account's tables in a write transaction: files whole or in pieces.
pub(super) struct Tables<'t> {
    pub(super) files: redb::Table<'t, &'static str, &'static [u8]>,
    pub(super) pieces: redb::Table<'t, (&'static str, u32), &'static [u8]>,
}

impl Tables<'_> {
    pub(super) fn has(&self, key: &str) -> io::Result<bool> {
        Ok(self.files.get(key).map_err(other)?.is_some())
    }

    /// Keep `bytes` at `key` (replacing what was there), compressed when that helps.
    pub(super) fn put(&mut self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.put_kept(key, &pack(bytes))
    }

    /// Keep bytes already as kept (compressed or not), whole or in pieces. The pieces of what was
    /// there go first, looked for only when something was (a new key, most of an import, has
    /// none: the range scan for them was 4.5 s of a 13-minute import).
    pub(super) fn put_kept(&mut self, key: &str, bytes: &[u8]) -> io::Result<()> {
        if self.has(key)? {
            self.drop_pieces(key)?;
        }
        if bytes.len() <= WHOLE_MAX {
            self.files.insert(key, bytes).map_err(other)?;
            return Ok(());
        }
        let mut note = PIECED.to_vec();
        note.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.files.insert(key, note.as_slice()).map_err(other)?;
        let mut at = 0;
        for (i, n) in piece_sizes(bytes.len()).into_iter().enumerate() {
            let piece = bytes.get(at..at + n).ok_or_else(|| other("piece out of range"))?;
            self.pieces.insert((key, u32::try_from(i).map_err(other)?), piece).map_err(other)?;
            at += n;
        }
        Ok(())
    }

    /// Take the file at `key` out: its bytes as kept (still compressed), when it was there.
    pub(super) fn take_kept(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(v) = self.files.remove(key).map_err(other)?.map(|g| g.value().to_vec()) else { return Ok(None) };
        let Some(len) = pieced_len(&v) else { return Ok(Some(v)) };
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(1 << 30));
        for (_, piece) in self.pieces.extract_from_if((key, 0)..=(key, u32::MAX), |_, _| true).map_err(other)?.flatten() {
            out.extend_from_slice(piece.value());
        }
        Ok(Some(out))
    }

    fn drop_pieces(&mut self, key: &str) -> io::Result<()> {
        self.pieces.retain_in((key, 0)..=(key, u32::MAX), |_, _| false).map_err(other)
    }
}

/// A file from a read transaction: whole, or put together from its pieces, and uncompressed.
pub(super) fn read_file(
    tx: &redb::ReadTransaction,
    files: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> io::Result<Option<Vec<u8>>> {
    let Some(v) = files.get(key).map_err(other)?.map(|g| g.value().to_vec()) else { return Ok(None) };
    let Some(len) = pieced_len(&v) else { return unpack(v).map(Some) };
    let pieces = tx.open_table(PIECES).map_err(other)?;
    let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(1 << 30));
    for item in pieces.range((key, 0)..=(key, u32::MAX)).map_err(other)? {
        let (_, piece) = item.map_err(other)?;
        out.extend_from_slice(piece.value());
    }
    if out.len() as u64 != len {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{key}: pieces missing")));
    }
    unpack(out).map(Some)
}
