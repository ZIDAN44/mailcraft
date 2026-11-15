//! The index's files as bytes. Each starts with four letters naming what it is and a format
//! version; numbers are little-endian, or LEB128 varints where they vary. Everything read back is
//! checked (lengths against what's left, counts against caps, doc ids against the meta's), so a
//! damaged or hostile file is an error, never a panic or a huge allocation.

use std::ops::Range;
use std::sync::Arc;

use mailcraft_model::search::IndexDoc;

use super::*;

mod meta;
mod pos;
mod seg;
mod texts;

pub(crate) use meta::{Meta, SegInfo, TextPage, decode_meta, encode_meta};
pub(crate) use pos::{PlacesStream, PosAt, decode_places, decode_pos_page, kept_places, long_key, needs_places, put_places, split_places};
pub(crate) use seg::{LexEntry, LexStream, Postings, SegBuilder, SegFiles, SegPage, decode_head, decode_lex, decode_post_page, decode_postings};
pub use texts::Blob;
pub(crate) use texts::{TEXT_REC_HEAD, decode_blob, decode_text_page, encode_blob, encode_text_page, kept_names, kept_text, text_page_index};

/// What was wrong, for the error.
pub(crate) type R<T> = std::result::Result<T, &'static str>;

const META: &[u8; 4] = b"MCSM";
const DOCS: &[u8; 4] = b"MCSD";
const TEXT: &[u8; 4] = b"MCST";
const BLOB: &[u8; 4] = b"MCSB";
const HEAD: &[u8; 4] = b"MCSH";
const LEX: &[u8; 4] = b"MCSL";
const POST: &[u8; 4] = b"MCSP";
const POS: &[u8; 4] = b"MCSX";
/// The files' format. A newer one goes under `search/v2/`; a newer version in `v1` is damage.
pub(crate) const VERSION: u8 = 1;
/// A doc record's own version (the 2 bytes after its fields are kept for a later one).
const REC_VERSION: u8 = 1;
/// A doc record's bytes.
pub(crate) const REC: usize = 32;
pub(crate) const MAX_META: usize = 32 << 20;
/// Most bytes a text page read back may have (the writer's are at most [`TEXT_PAGE`], or one text
/// alone with its names, under [`BACKGROUND_COMMIT`]).
pub(crate) const MAX_TEXT_PAGE: usize = 8 << 20;
/// Most bytes of a message's text kept ([`encode_blob`] cuts one past this): with its names, a
/// page alone still goes in one background transaction and the reader's cache. A whole
/// [`MAX_INDEX_CHARS`](mailcraft_model::search::MAX_INDEX_CHARS) of plain text fits.
pub(crate) const MAX_BLOB_TEXT: usize = BACKGROUND_COMMIT - MAX_NAMES_BYTES - (64 << 10);
/// Most bytes of a message's files' names kept.
const MAX_NAMES_BYTES: usize = 64 << 10;
/// Longest folder or file name kept (the writer takes no longer folder name).
pub(crate) const MAX_NAME_BYTES: usize = 4096;
/// Most segments a meta names (the writer makes its words again before it gets there).
pub(crate) const MAX_SEGMENTS: usize = 4096;
const MAX_PAGES: usize = 1 << 20;
/// Bytes read from the front, every read checked.
pub(crate) struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Rd<'a> {
        Rd(b)
    }

    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let (a, b) = self.0.split_at_checked(n).ok_or("cut short")?;
        self.0 = b;
        Ok(a)
    }

    fn array<const N: usize>(&mut self) -> R<[u8; N]> {
        <[u8; N]>::try_from(self.take(N)?).map_err(|_| "cut short")
    }

    fn u8(&mut self) -> R<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    fn u16(&mut self) -> R<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> R<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// A varint of at most 10 bytes.
    fn var(&mut self) -> R<u64> {
        let mut v: u64 = 0;
        for i in 0..10u32 {
            let b = self.u8()?;
            let bits = u64::from(b & 0x7f);
            if i == 9 && b > 1 {
                return Err("number too big");
            }
            v |= bits << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("number too long")
    }

    fn var32(&mut self) -> R<u32> {
        u32::try_from(self.var()?).map_err(|_| "number too big")
    }

    /// A count of things each at least `each` bytes long, at most `max`.
    fn count(&mut self, max: usize, each: usize) -> R<usize> {
        let n = usize::try_from(self.var()?).map_err(|_| "count too big")?;
        if n > max || n.saturating_mul(each) > self.0.len() {
            return Err("count past what's there");
        }
        Ok(n)
    }

    fn header(&mut self, magic: &[u8; 4]) -> R<()> {
        if self.take(4)? != magic {
            return Err("not this kind of file");
        }
        match self.u8()? {
            VERSION => Ok(()),
            _ => Err("another format"),
        }
    }

    fn string(&mut self, len: usize) -> R<String> {
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| "text not UTF-8")
    }

    fn left(&self) -> usize {
        self.0.len()
    }

    fn end(&self) -> R<()> {
        if self.0.is_empty() { Ok(()) } else { Err("bytes past the end") }
    }
}

pub(crate) fn put_var(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_len(out: &mut Vec<u8>, n: usize) {
    put_var(out, n as u64);
}

/// Numbers kept rising, the first as it is and each after as the step from the one before (a
/// varint each): a word's docs, below the index's next doc, or a doc's places of a word.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Rising {
    /// Docs below this one.
    Docs(u32),
    Places,
}

impl Rising {
    fn past(self) -> &'static str {
        match self {
            Rising::Docs(_) => "doc past the ids",
            Rising::Places => "place past the end",
        }
    }

    fn out_of_order(self) -> &'static str {
        match self {
            Rising::Docs(_) => "docs out of order",
            Rising::Places => "places out of order",
        }
    }

    /// `n` of them read from `r`: each step more than nothing, none past the end.
    fn read(self, r: &mut Rd<'_>, n: usize) -> R<Vec<u32>> {
        // Each is at least a byte.
        let mut out = Vec::with_capacity(n.min(r.left()));
        let mut last: Option<u32> = None;
        for _ in 0..n {
            let d = r.var32()?;
            let v = match last {
                None => d,
                Some(l) if d > 0 => l.checked_add(d).ok_or(self.past())?,
                Some(_) => return Err(self.out_of_order()),
            };
            if let Rising::Docs(below) = self
                && v >= below
            {
                return Err(self.past());
            }
            out.push(v);
            last = Some(v);
        }
        Ok(out)
    }
}

/// Numbers rising, none twice, added to `out` as [`Rising`] reads them.
fn put_rising(out: &mut Vec<u8>, values: &[u32]) {
    let mut last: Option<u32> = None;
    for &v in values {
        put_var(out, u64::from(last.map_or(v, |l| v.wrapping_sub(l))));
        last = Some(v);
    }
}

/// Most bytes set aside for a run before it is read (its length is the file's word).
const RUN_RESERVE: usize = 16 << 20;

/// Which of a segment's pages a run of bytes is in: a word's docs ([`Postings::At`]) or its
/// places ([`PosAt`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Run {
    Postings,
    Places,
}

impl Run {
    /// Page `p` of segment `seg`.
    fn name(self, seg: u32, p: u32) -> Name {
        match self {
            Run::Postings => Name::SegPost(seg, p),
            Run::Places => Name::SegPos(seg, p),
        }
    }

    /// The segment's pages of this kind.
    fn pages(self, info: &SegInfo) -> u32 {
        match self {
            Run::Postings => info.post_pages,
            Run::Places => info.pos_pages,
        }
    }

    /// A page's bytes past its header.
    fn data(self, page: &[u8]) -> R<&[u8]> {
        match self {
            Run::Postings => decode_post_page(page),
            Run::Places => decode_pos_page(page),
        }
    }

    fn past_segment(self) -> &'static str {
        match self {
            Run::Postings => "postings past the segment",
            Run::Places => "places past the segment",
        }
    }

    fn past_page(self) -> &'static str {
        match self {
            Run::Postings => "postings past the page",
            Run::Places => "places past the page",
        }
    }

    /// `len` bytes of segment `info`'s pages of this kind, from `offset` in page `page` and on
    /// into the next pages from their starts: each page one the segment has, and holding some of
    /// what's left. `fetch` reads page `p`'s file (named `path` under `base`).
    pub(crate) fn read(
        self,
        base: &Path,
        info: &SegInfo,
        (page, offset, len): (u32, usize, usize),
        mut fetch: impl FnMut(u32, &Path) -> Result<Arc<Vec<u8>>>,
    ) -> Result<Vec<u8>> {
        let mut bytes: Vec<u8> = Vec::with_capacity(len.min(RUN_RESERVE));
        let (mut p, mut from) = (page, offset);
        while bytes.len() < len {
            let path = self.name(info.id, p).path(base);
            if p >= self.pages(info) {
                return Err(damaged(&path, self.past_segment()));
            }
            let raw = fetch(p, &path)?;
            let data = self.data(&raw).map_err(|what| damaged(&path, what))?;
            let rest = data.get(from..).ok_or_else(|| damaged(&path, self.past_page()))?;
            let want = (len - bytes.len()).min(rest.len());
            if want == 0 {
                return Err(damaged(&path, self.past_page()));
            }
            bytes.extend_from_slice(rest.get(..want).unwrap_or_default());
            (p, from) = (p.saturating_add(1), 0);
        }
        Ok(bytes)
    }
}

/// How many docs page `p` of a table of `next_doc` holds.
pub(crate) fn page_docs(next_doc: u32, p: usize) -> usize {
    let start = (p as u64).saturating_mul(u64::from(DOCS_PER_PAGE));
    u64::from(next_doc).saturating_sub(start).min(u64::from(DOCS_PER_PAGE)) as usize
}

pub(crate) fn encode_docs(recs: &[DocRec]) -> Vec<u8> {
    let mut out = Vec::with_capacity(7 + recs.len() * REC);
    out.extend_from_slice(DOCS);
    out.push(VERSION);
    out.extend_from_slice(&u16::try_from(recs.len()).unwrap_or(u16::MAX).to_le_bytes());
    for d in recs {
        out.push(REC_VERSION);
        out.extend_from_slice(&d.folder.to_le_bytes());
        out.extend_from_slice(&d.validity.to_le_bytes());
        out.extend_from_slice(&d.uid.to_le_bytes());
        out.extend_from_slice(&d.text.to_le_bytes());
        out.extend_from_slice(&d.files.to_le_bytes());
        out.push(d.bits);
        out.extend_from_slice(&d.msgid_hash.to_le_bytes());
        out.extend_from_slice(&d.size.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
    }
    out
}

/// A doc page that should hold `expect` docs, of an index with `folders` folders and
/// `text_pages` text pages.
pub(crate) fn decode_docs(b: &[u8], expect: usize, folders: usize, text_pages: usize) -> R<Vec<DocRec>> {
    let mut r = Rd::new(b);
    r.header(DOCS)?;
    let n = usize::from(r.u16()?);
    if n != expect || r.left() != n.saturating_mul(REC) {
        return Err("doc page of another size");
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        // A later record version keeps these fields where they are.
        if r.u8()? < REC_VERSION {
            return Err("doc record of no version");
        }
        let d = DocRec {
            folder: r.u16()?,
            validity: r.u32()?,
            uid: r.u32()?,
            text: r.u32()?,
            files: r.u16()?,
            bits: r.u8()?,
            msgid_hash: r.u64()?,
            size: r.u32()?,
        };
        r.u16()?;
        if (d.live() && usize::from(d.folder) >= folders) || (d.text != NO_TEXT && d.text as usize >= text_pages) {
            return Err("doc names what isn't there");
        }
        out.push(d);
    }
    r.end()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
        [magic.as_slice(), &[VERSION], data].concat()
    }

    proptest::proptest! {
        /// Rising numbers read back as they went in, as docs and as places; read as docs, one
        /// at or past the doc given is damage.
        #[test]
        fn rising_numbers_read_back(v in proptest::collection::btree_set(proptest::prelude::any::<u32>(), 0..40)) {
            let v: Vec<u32> = v.into_iter().collect();
            let mut b = Vec::new();
            put_rising(&mut b, &v);
            for kind in [Rising::Places, Rising::Docs(u32::MAX)] {
                let mut r = Rd::new(&b);
                let got = kind.read(&mut r, v.len());
                proptest::prop_assert!(r.end().is_ok() || got.is_err());
                match kind {
                    Rising::Docs(_) if v.last() == Some(&u32::MAX) => proptest::prop_assert_eq!(got, Err("doc past the ids")),
                    _ => proptest::prop_assert_eq!(got, Ok(v.clone())),
                }
            }
            if let Some(&last) = v.last() {
                proptest::prop_assert_eq!(Rising::Docs(last).read(&mut Rd::new(&b), v.len()), Err("doc past the ids"));
            }
        }
    }

    /// Each way rising numbers can be damaged, the words said for docs and for places.
    #[test]
    fn rising_numbers_damaged() {
        let read = |kind: Rising, b: &[u8], n| kind.read(&mut Rd::new(b), n);
        assert_eq!(read(Rising::Docs(10), &[3, 0], 2), Err("docs out of order"), "a step of nothing");
        assert_eq!(read(Rising::Places, &[3, 0], 2), Err("places out of order"));
        let over = [0xff, 0xff, 0xff, 0xff, 0x0f, 1];
        assert_eq!(read(Rising::Docs(u32::MAX), &over, 2), Err("doc past the ids"), "past u32");
        assert_eq!(read(Rising::Places, &over, 2), Err("place past the end"));
        assert_eq!(read(Rising::Docs(5), &[2, 3], 2), Err("doc past the ids"), "at the doc given");
        assert_eq!(read(Rising::Docs(5), &[5], 1), Err("doc past the ids"), "the first too");
        assert_eq!(read(Rising::Places, &[2], 2), Err("cut short"));
        assert_eq!(read(Rising::Places, &[0x80, 0x80, 0x80, 0x80, 0x10], 1), Err("number too big"));
        assert_eq!(read(Rising::Docs(5), &[2, 2], 2), Ok(vec![2, 4]));
    }

    /// A run read across pages; and each way one can be wrong, damage of the page it is in.
    #[test]
    fn a_run_reads_across_pages_or_is_damage() {
        let base = Path::new("idx");
        let info = SegInfo { id: 3, lex_pages: 1, post_pages: 2, pos_pages: 2, ..SegInfo::default() };
        let files = [
            (Name::SegPost(3, 0), page(POST, b"abcdef")),
            (Name::SegPost(3, 1), page(POST, b"ghij")),
            (Name::SegPos(3, 0), page(POS, b"klm")),
            (Name::SegPos(3, 1), page(POST, b"nop")),
        ];
        let read = |kind: Run, at| {
            let mut pages = Vec::new();
            let fetch = |p, path: &Path| {
                pages.push(p);
                let file = files.iter().find(|(n, _)| n.path(base) == path).map(|(_, b)| Arc::new(b.clone()));
                file.ok_or_else(|| damaged(path, "no such file"))
            };
            let got = kind.read(base, &info, at, fetch).map_err(|e| e.to_string());
            (got, pages)
        };
        let at = |p: &str| Path::new("idx").join("s").join(p).display().to_string();
        let damage = |p: &str, what: &str| Err(format!("{}: damaged ({what})", at(p)));
        assert_eq!(read(Run::Postings, (0, 4, 4)), (Ok(b"efgh".to_vec()), vec![0, 1]));
        assert_eq!(read(Run::Postings, (1, 9, 0)), (Ok(Vec::new()), vec![]), "nothing to read");
        assert_eq!(read(Run::Postings, (1, 2, 4)).0, damage("3-p2.bin", "postings past the segment"));
        assert_eq!(read(Run::Postings, (0, 7, 1)).0, damage("3-p0.bin", "postings past the page"));
        assert_eq!(read(Run::Postings, (0, 6, 1)).0, damage("3-p0.bin", "postings past the page"), "nothing left in it");
        assert_eq!(read(Run::Places, (0, 1, 2)), (Ok(b"lm".to_vec()), vec![0]));
        assert_eq!(read(Run::Places, (0, 1, 4)).0, damage("3-x1.bin", "not this kind of file"));
        assert_eq!(read(Run::Places, (0, 3, 1)).0, damage("3-x0.bin", "places past the page"));
        assert_eq!(read(Run::Places, (2, 0, 1)).0, damage("3-x2.bin", "places past the segment"));
    }
}
