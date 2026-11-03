//! A segment's places pages: where each word of the text and the names is in each doc it's in,
//! so a phrase is told from its words' places, never from the text.
//!
//! A word's places follow its postings, a block a doc in the same order: the count of its places,
//! then the places rising, the first as it is and each after as the step from the one before (no
//! length: a block passed over is read to its end a byte at a time, cheaper than a byte more a
//! doc a word, a third of most blocks). A place is the word's place among the doc's words (as a phrase reads them,
//! [`placed_tokens`](mailcraft_model::search::placed_tokens)) doubled, plus 1 for one of a
//! word's parts. The names' words go on from one name to the next with a place left between, so
//! no phrase runs from one name into the next. A field's words too long to index have their
//! places under [`long_key`]. Everything read back is checked, as the rest of the index is.

use std::ops::Range;

use super::*;

/// A place that is one of a word's parts, not a word.
pub(crate) const PART: u32 = 1;
/// Most places one doc can have for a word: one a word of its text and names kept, a gap after
/// each name, doubled.
const MAX_PLACES: u64 = 2 * (MAX_BLOB_TEXT as u64 + MAX_NAMES_BYTES as u64 + (1 << 17));
/// Most bytes of a place.
const MAX_PLACE_BYTES: usize = 5;

/// Whether a word's postings carry places: the text's and the names' words, and their long
/// words; addresses don't.
pub(crate) fn needs_places(key: &[u8]) -> bool {
    matches!(key.first(), Some(b'b' | b'n' | b'~'))
}

/// The key a field's words too long to index keep their places under: no word's key starts with
/// `~`, so no word looked up ever reads it.
pub fn long_key(field: Field) -> Vec<u8> {
    vec![b'~', field.letter()]
}

/// A doc's places of a word (rising, none twice), added to `out` as its block.
pub(crate) fn put_places(out: &mut Vec<u8>, places: &[u32]) {
    put_len(out, places.len());
    let mut last: Option<u32> = None;
    for &p in places {
        put_var(out, u64::from(last.map_or(p, |l| p.wrapping_sub(l))));
        last = Some(p);
    }
}

/// The block at the start of `b`: its count of places and where their bytes are; `None` when `b`
/// ends before it does.
fn block_at(b: &[u8]) -> R<Option<(usize, Range<usize>)>> {
    let mut r = Rd::new(b);
    let count = match r.var() {
        Ok(c) => c,
        // A count is at most 10 bytes: the rest of it is still to come.
        Err(_) if b.len() < 10 => return Ok(None),
        Err(e) => return Err(e),
    };
    if count == 0 || count > MAX_PLACES {
        return Err("places out of range");
    }
    let count = usize::try_from(count).map_err(|_| "places out of range")?;
    let start = b.len() - r.left();
    let (mut at, mut seen, mut run) = (start, 0usize, 0usize);
    while seen < count {
        let Some(&byte) = b.get(at) else { return Ok(None) };
        at += 1;
        run += 1;
        if run > MAX_PLACE_BYTES {
            return Err("place too long");
        }
        if byte & 0x80 == 0 {
            (seen, run) = (seen + 1, 0);
        }
    }
    Ok(Some((count, start..at)))
}

/// `count` places from a block's bytes: rising, each a step on from the one before.
pub(crate) fn decode_places(b: &[u8], count: usize) -> R<Vec<u32>> {
    if count > b.len() {
        return Err("more places than bytes");
    }
    let mut r = Rd::new(b);
    let mut out = Vec::with_capacity(count);
    let mut last: Option<u32> = None;
    for _ in 0..count {
        let d = r.var32()?;
        let p = match last {
            None => d,
            Some(l) if d > 0 => l.checked_add(d).ok_or("place past the end")?,
            Some(_) => return Err("places out of order"),
        };
        out.push(p);
        last = Some(p);
    }
    r.end()?;
    Ok(out)
}

/// A word's places whole (`df` docs'), split into each doc's block: its count of places and where
/// its bytes are in `b`.
pub(crate) fn split_places(b: &[u8], df: usize) -> R<Vec<(usize, std::ops::Range<usize>)>> {
    let mut out = Vec::with_capacity(df.min(b.len() / 2));
    let mut from = 0usize;
    for _ in 0..df {
        let (count, at) = block_at(b.get(from..).unwrap_or_default())?.ok_or("cut short")?;
        let at = from + at.start..from + at.end;
        from = at.end;
        out.push((count, at));
    }
    if from != b.len() {
        return Err("bytes past the end");
    }
    Ok(out)
}

/// A places page's own bytes.
pub(crate) fn decode_pos_page(b: &[u8]) -> R<&[u8]> {
    if b.len() > POS_PAGE + 5 {
        return Err("too big");
    }
    let mut r = Rd::new(b);
    r.header(POS)?;
    r.take(r.left())
}

/// Where a word's places are: from `offset` in places page `page`, `len` bytes, going on into
/// the next pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PosAt {
    pub(crate) page: u32,
    pub(crate) offset: u32,
    pub(crate) len: u64,
}

impl PosAt {
    /// A word in `df` docs whose places are `len` bytes from `start` among those of a segment of
    /// `pos_pages` places pages (each full page holds [`POS_PAGE`] of them).
    pub(super) fn starting(start: u64, len: u64, df: u32, pos_pages: u32) -> R<PosAt> {
        let page = u32::try_from(start / POS_PAGE as u64).map_err(|_| "places out of range")?;
        let at = PosAt { page, offset: (start % POS_PAGE as u64) as u32, len };
        let room = u64::from(pos_pages.saturating_sub(at.page)).saturating_mul(POS_PAGE as u64);
        if at.page >= pos_pages || at.offset as usize > POS_PAGE || at.len < u64::from(df).saturating_mul(2) || at.len > room {
            return Err("places out of range");
        }
        Ok(at)
    }
}

/// A word's places read as their pages come: each doc's block in turn, a page's bytes held at
/// most beside what's left of the block before.
#[derive(Debug)]
pub(crate) struct PlacesStream {
    buf: Vec<u8>,
    at: usize,
    /// Bytes still to come from the pages.
    left: u64,
    /// Blocks read so far, of `df`.
    read: u32,
    df: u32,
}

impl PlacesStream {
    pub(crate) fn new(at: PosAt, df: u32) -> PlacesStream {
        PlacesStream { buf: Vec::new(), at: 0, left: at.len, read: 0, df }
    }

    /// The place in the postings of the doc whose block is next.
    pub(crate) fn next_index(&self) -> usize {
        self.read as usize
    }

    /// Bytes still to come.
    pub(crate) fn wants(&self) -> u64 {
        self.left
    }

    /// More of its bytes, at most [`wants`](PlacesStream::wants).
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if self.at > 0 && self.at >= self.buf.len() / 2 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        let n = bytes.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        self.buf.extend_from_slice(bytes.get(..n).unwrap_or_default());
        self.left = self.left.saturating_sub(n as u64);
    }

    /// The next doc's block: its place in the postings, its count of places, and its bytes;
    /// `None` when more bytes are wanted first, or at the end (checked to end there).
    pub(crate) fn next_block(&mut self) -> R<Option<(usize, usize, &[u8])>> {
        let rest = self.buf.get(self.at..).unwrap_or_default();
        if self.read >= self.df {
            return if rest.is_empty() { Ok(None) } else { Err("places past the docs") };
        }
        let (count, at) = match block_at(rest)? {
            Some(b) => b,
            None if self.left > 0 => return Ok(None),
            None => return Err("cut short"),
        };
        let (start, end) = (self.at.saturating_add(at.start), self.at.saturating_add(at.end));
        let doc = self.read as usize;
        self.at = end;
        self.read += 1;
        let block = self.buf.get(start..end).ok_or("cut short")?;
        Ok(Some((doc, count, block)))
    }

    /// Every block read, and nothing past them.
    pub(crate) fn finish(&self) -> R<()> {
        if self.left == 0 && self.read == self.df && self.at == self.buf.len() { Ok(()) } else { Err("places not the postings'") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blocks go in and come back whole, however the bytes are split into pages; damaged ones are
    /// errors.
    #[test]
    fn places_read_back_however_they_come() {
        let docs: Vec<Vec<u32>> = vec![vec![0], vec![2, 3, 10, 400_000], (0..3000).map(|i| i * 2).collect(), vec![u32::MAX - 1]];
        let mut stream = Vec::new();
        for d in &docs {
            put_places(&mut stream, d);
        }
        for split in [1, 2, 7, 100, stream.len()] {
            let at = PosAt { page: 0, offset: 0, len: stream.len() as u64 };
            let mut s = PlacesStream::new(at, docs.len() as u32);
            let mut got = Vec::new();
            for chunk in stream.chunks(split) {
                s.feed(chunk);
                while let Some((i, count, b)) = s.next_block().unwrap() {
                    assert_eq!(i, got.len());
                    got.push(decode_places(b, count).unwrap());
                }
            }
            s.finish().unwrap();
            assert_eq!(got, docs, "split {split}");
        }
        assert!(decode_places(&[5, 0], 2).is_err(), "a step of nothing");
        assert!(decode_places(&[5], 2).is_err(), "more places than bytes");
        let mut s = PlacesStream::new(PosAt { page: 0, offset: 0, len: 2 }, 1);
        s.feed(&[0, 1]);
        assert!(s.next_block().is_err(), "a doc with no places");
        let mut s = PlacesStream::new(PosAt { page: 0, offset: 0, len: 3 }, 1);
        s.feed(&[2, 9, 0x80]);
        assert!(s.next_block().is_err(), "a block past the stream");
        let mut s = PlacesStream::new(PosAt { page: 0, offset: 0, len: 8 }, 1);
        s.feed(&[1, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 1]);
        assert!(s.next_block().is_err(), "a place too long");
        assert!(split_places(&[1, 4, 1, 5, 7], 2).is_err(), "bytes past the blocks");
        assert_eq!(split_places(&[1, 4, 2, 5, 7], 2).unwrap(), [(1, 1..2), (2, 3..5)]);
    }
}
