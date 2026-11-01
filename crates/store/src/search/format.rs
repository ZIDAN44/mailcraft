//! The index's files as bytes. Each starts with four letters naming what it is and a format
//! version; numbers are little-endian, or LEB128 varints where they vary. Everything read back is
//! checked (lengths against what's left, counts against caps, doc ids against the meta's), so a
//! damaged or hostile file is an error, never a panic or a huge allocation.

use mailcraft_model::search::IndexDoc;

use super::*;

mod seg;

pub(crate) use seg::{LexEntry, LexStream, Postings, SegBuilder, SegFiles, SegPage, decode_head, decode_lex, decode_post_page, decode_postings};

/// What was wrong, for the error.
pub(crate) type R<T> = std::result::Result<T, &'static str>;

const META: &[u8; 4] = b"MCSM";
const DOCS: &[u8; 4] = b"MCSD";
const TEXT: &[u8; 4] = b"MCST";
const BLOB: &[u8; 4] = b"MCSB";
const HEAD: &[u8; 4] = b"MCSH";
const LEX: &[u8; 4] = b"MCSL";
const POST: &[u8; 4] = b"MCSP";
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

/// A text page as of the generation that wrote it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextPage {
    /// 0: emptied (every text in it removed), no file.
    pub(crate) generation: u64,
    /// Its bytes (a page under [`TEXT_PAGE`] is filled on).
    pub(crate) bytes: u32,
}

/// A segment as the meta lists it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SegInfo {
    pub(crate) id: u32,
    pub(crate) docs: u32,
    pub(crate) terms: u32,
    pub(crate) lex_pages: u32,
    pub(crate) post_pages: u32,
    pub(crate) bytes: u64,
    /// The lowest and highest doc its words name: a doc outside them isn't in it.
    pub(crate) lo: u32,
    pub(crate) hi: u32,
}

impl SegInfo {
    /// Its files.
    pub(crate) fn names(&self) -> impl Iterator<Item = Name> + '_ {
        let lex = (0..self.lex_pages).map(|n| Name::SegLex(self.id, n));
        let post = (0..self.post_pages).map(|n| Name::SegPost(self.id, n));
        std::iter::once(Name::SegHead(self.id)).chain(lex).chain(post)
    }
}

/// What the index is: the commit point, written last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Meta {
    /// The tokenizer its words were read with ([`TOKENIZER_VERSION`](mailcraft_model::search::TOKENIZER_VERSION)).
    pub(crate) tokenizer: u32,
    /// Counts the commits.
    pub(crate) generation: u64,
    /// Made once per index ([`new_epoch`]).
    pub(crate) epoch: u64,
    /// The next doc's id: every doc below it is in the table.
    pub(crate) next_doc: u32,
    pub(crate) next_seg: u32,
    pub(crate) live: u32,
    pub(crate) dead: u32,
    /// Each doc page's generation (its file's name).
    pub(crate) doc_pages: Vec<u64>,
    pub(crate) text_pages: Vec<TextPage>,
    pub(crate) folders: Vec<String>,
    pub(crate) segments: Vec<SegInfo>,
    /// Files this generation no longer uses: removed after it's written (and again later, when
    /// that didn't happen).
    pub(crate) gone: Vec<Name>,
    /// Since when (Unix seconds) a doc removed has had its words still in a segment; 0: none has.
    pub(crate) dead_since: u64,
    /// Each folder reconciled, by its place in `folders`, and the revision of its list and
    /// sources it was reconciled at.
    pub(crate) marks: Vec<(u16, u64)>,
}

impl Meta {
    /// The files it uses.
    pub(crate) fn uses(&self) -> BTreeSet<Name> {
        let mut out = BTreeSet::new();
        for (p, &generation) in self.doc_pages.iter().enumerate() {
            out.insert(Name::Docs { generation, page: u32::try_from(p).unwrap_or(u32::MAX) });
        }
        for (p, t) in self.text_pages.iter().enumerate() {
            if t.generation != 0 {
                out.insert(Name::Text { generation: t.generation, page: u32::try_from(p).unwrap_or(u32::MAX) });
            }
        }
        for s in &self.segments {
            out.extend(s.names());
        }
        out
    }
}

fn put_name(out: &mut Vec<u8>, n: &Name) {
    let (tag, a, b) = match *n {
        Name::Docs { generation, page } => (0, generation, u64::from(page)),
        Name::Text { generation, page } => (1, generation, u64::from(page)),
        Name::SegHead(id) => (2, u64::from(id), 0),
        Name::SegLex(id, p) => (3, u64::from(id), u64::from(p)),
        Name::SegPost(id, p) => (4, u64::from(id), u64::from(p)),
    };
    out.push(tag);
    put_var(out, a);
    put_var(out, b);
}

fn get_name(r: &mut Rd<'_>) -> R<Name> {
    let tag = r.u8()?;
    let a = r.var()?;
    let b = r.var32()?;
    let id = || u32::try_from(a).map_err(|_| "number too big");
    Ok(match tag {
        0 => Name::Docs { generation: a, page: b },
        1 => Name::Text { generation: a, page: b },
        2 => Name::SegHead(id()?),
        3 => Name::SegLex(id()?, b),
        4 => Name::SegPost(id()?, b),
        _ => return Err("unknown file kind"),
    })
}

pub(crate) fn encode_meta(m: &Meta) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + m.doc_pages.len() * 4 + m.text_pages.len() * 8 + m.folders.iter().map(|f| f.len() + 2).sum::<usize>());
    out.extend_from_slice(META);
    out.push(VERSION);
    out.extend_from_slice(&m.tokenizer.to_le_bytes());
    out.extend_from_slice(&m.generation.to_le_bytes());
    out.extend_from_slice(&m.epoch.to_le_bytes());
    for n in [m.next_doc, m.next_seg, m.live, m.dead] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    put_len(&mut out, m.doc_pages.len());
    for g in &m.doc_pages {
        put_var(&mut out, *g);
    }
    put_len(&mut out, m.text_pages.len());
    for t in &m.text_pages {
        put_var(&mut out, t.generation);
        put_var(&mut out, u64::from(t.bytes));
    }
    put_len(&mut out, m.folders.len());
    for f in &m.folders {
        put_len(&mut out, f.len());
        out.extend_from_slice(f.as_bytes());
    }
    put_len(&mut out, m.segments.len());
    for s in &m.segments {
        for n in [s.id, s.docs, s.terms, s.lex_pages, s.post_pages] {
            put_var(&mut out, u64::from(n));
        }
        put_var(&mut out, s.bytes);
        put_var(&mut out, u64::from(s.lo));
        put_var(&mut out, u64::from(s.hi));
    }
    put_len(&mut out, m.gone.len());
    for n in &m.gone {
        put_name(&mut out, n);
    }
    put_var(&mut out, m.dead_since);
    put_len(&mut out, m.marks.len());
    for (f, rev) in &m.marks {
        put_var(&mut out, u64::from(*f));
        out.extend_from_slice(&rev.to_le_bytes());
    }
    out
}

pub(crate) fn decode_meta(b: &[u8]) -> R<Meta> {
    if b.len() > MAX_META {
        return Err("too big");
    }
    let mut r = Rd::new(b);
    r.header(META)?;
    let mut m = Meta { tokenizer: r.u32()?, generation: r.u64()?, epoch: r.u64()?, ..Meta::default() };
    m.next_doc = r.u32()?;
    m.next_seg = r.u32()?;
    m.live = r.u32()?;
    m.dead = r.u32()?;
    if u64::from(m.live) + u64::from(m.dead) > u64::from(m.next_doc) {
        return Err("more docs than ids");
    }
    let n = r.count(MAX_PAGES, 1)?;
    if n != m.next_doc.div_ceil(DOCS_PER_PAGE) as usize {
        return Err("doc pages don't match the docs");
    }
    for _ in 0..n {
        let g = r.var()?;
        if g == 0 || g > m.generation {
            return Err("doc page from no generation");
        }
        m.doc_pages.push(g);
    }
    for _ in 0..r.count(MAX_PAGES, 2)? {
        let t = TextPage { generation: r.var()?, bytes: r.var32()? };
        if t.generation > m.generation || t.bytes as usize > MAX_TEXT_PAGE {
            return Err("text page out of range");
        }
        m.text_pages.push(t);
    }
    for _ in 0..r.count(MAX_FOLDERS, 1)? {
        let len = r.count(MAX_NAME_BYTES, 1)?;
        m.folders.push(r.string(len)?);
    }
    let mut ids = BTreeSet::new();
    for _ in 0..r.count(MAX_SEGMENTS, 8)? {
        let s = SegInfo {
            id: r.var32()?,
            docs: r.var32()?,
            terms: r.var32()?,
            lex_pages: r.var32()?,
            post_pages: r.var32()?,
            bytes: r.var()?,
            lo: r.var32()?,
            hi: r.var32()?,
        };
        if s.id >= m.next_seg
            || !ids.insert(s.id)
            || s.lex_pages == 0
            || s.lex_pages as usize > MAX_PAGES
            || s.post_pages as usize > MAX_PAGES
            || s.lo > s.hi
            || s.hi >= m.next_doc
        {
            return Err("segment out of range");
        }
        m.segments.push(s);
    }
    for _ in 0..r.count(MAX_PAGES, 3)? {
        m.gone.push(get_name(&mut r)?);
    }
    m.dead_since = r.var()?;
    let mut marked = BTreeSet::new();
    for _ in 0..r.count(MAX_FOLDERS, 9)? {
        let f = u16::try_from(r.var()?).map_err(|_| "number too big")?;
        if usize::from(f) >= m.folders.len() || !marked.insert(f) {
            return Err("mark of no folder");
        }
        m.marks.push((f, r.u64()?));
    }
    r.end()?;
    Ok(m)
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

/// What the index keeps of a message's text: its files' names and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blob {
    pub names: Vec<String>,
    pub text: String,
}

/// What is kept of a message's text: at most [`MAX_BLOB_TEXT`] bytes, cut where a character
/// starts.
pub(crate) fn kept_text(text: &str) -> &str {
    let mut end = text.len().min(MAX_BLOB_TEXT);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}

/// A text kept for a message (at most [`MAX_NAMES_BYTES`] of names and [`MAX_BLOB_TEXT`] of
/// text), and whether some of it was left out.
pub(crate) fn encode_blob(doc: &IndexDoc) -> (Vec<u8>, bool) {
    let mut names: Vec<&str> = Vec::new();
    let mut room = MAX_NAMES_BYTES;
    for n in &doc.names {
        let mut end = n.len().min(MAX_NAME_BYTES).min(room);
        while !n.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let Some(n) = n.get(..end) else { continue };
        if n.is_empty() || names.len() >= usize::from(u16::MAX) {
            break;
        }
        room -= n.len();
        names.push(n);
    }
    let text = kept_text(&doc.text);
    let cut = text.len() < doc.text.len() || names.len() < doc.names.len();
    let mut out = Vec::with_capacity(16 + room + text.len());
    out.extend_from_slice(BLOB);
    out.push(VERSION);
    out.extend_from_slice(&u16::try_from(names.len()).unwrap_or(u16::MAX).to_le_bytes());
    for n in names {
        out.extend_from_slice(&u16::try_from(n.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(n.as_bytes());
    }
    out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    (out, cut)
}

pub(crate) fn decode_blob(b: &[u8]) -> R<Blob> {
    let mut r = Rd::new(b);
    r.header(BLOB)?;
    let n = usize::from(r.u16()?);
    if n.saturating_mul(2) > r.left() {
        return Err("names past what's there");
    }
    let mut blob = Blob { names: Vec::with_capacity(n), text: String::new() };
    for _ in 0..n {
        let len = usize::from(r.u16()?);
        blob.names.push(r.string(len)?);
    }
    let len = r.u32()? as usize;
    blob.text = r.string(len)?;
    r.end()?;
    Ok(blob)
}

/// A text page: each doc's text, by doc (rising).
pub(crate) fn encode_text_page<'a>(recs: impl Iterator<Item = (u32, &'a [u8])>) -> Vec<u8> {
    let recs: Vec<(u32, &[u8])> = recs.collect();
    let mut out = Vec::with_capacity(7 + recs.iter().map(|(_, b)| b.len() + 8).sum::<usize>());
    out.extend_from_slice(TEXT);
    out.push(VERSION);
    out.extend_from_slice(&u16::try_from(recs.len()).unwrap_or(u16::MAX).to_le_bytes());
    for (doc, blob) in recs {
        out.extend_from_slice(&doc.to_le_bytes());
        out.extend_from_slice(&u32::try_from(blob.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(blob);
    }
    out
}

/// A text page's texts (not yet read), their docs rising and below `next_doc`.
pub(crate) fn decode_text_page(b: &[u8], next_doc: u32) -> R<Vec<(u32, &[u8])>> {
    if b.len() > MAX_TEXT_PAGE {
        return Err("too big");
    }
    let mut r = Rd::new(b);
    r.header(TEXT)?;
    let n = usize::from(r.u16()?);
    if n.saturating_mul(8) > r.left() {
        return Err("texts past what's there");
    }
    let mut out: Vec<(u32, &[u8])> = Vec::with_capacity(n);
    for _ in 0..n {
        let doc = r.u32()?;
        let len = r.u32()? as usize;
        if doc >= next_doc || out.last().is_some_and(|(d, _)| *d >= doc) {
            return Err("text of a doc out of order");
        }
        out.push((doc, r.take(len)?));
    }
    r.end()?;
    Ok(out)
}
