//! A segment's files: the head, lexicon pages, postings pages and places pages, made from words
//! in rising order and read back checked.

use super::*;

/// A lexicon entry's postings kept in it, up to this many docs.
const INLINE: usize = 8;
/// A lexicon entry's kind: its postings in postings pages, not in it.
const AT: u8 = 1;
/// A lexicon entry's kind: where its places are follows.
const PLACED: u8 = 2;

/// A segment written: its head, and its lexicon, postings and places pages not yet
/// [taken](SegBuilder::take).
#[derive(Debug, Default)]
pub(crate) struct SegFiles {
    pub(crate) head: Vec<u8>,
    pub(crate) lex: Vec<Vec<u8>>,
    pub(crate) post: Vec<Vec<u8>>,
    pub(crate) pos: Vec<Vec<u8>>,
    /// The numbers of the first pages in `lex`, `post` and `pos` (those before were taken).
    pub(crate) lex_from: u32,
    pub(crate) post_from: u32,
    pub(crate) pos_from: u32,
    pub(crate) terms: u32,
    /// The bytes of the pages taken.
    taken: u64,
    /// The lowest and highest doc its words name.
    pub(crate) lo: u32,
    pub(crate) hi: u32,
}

impl SegFiles {
    /// Its bytes, the pages taken counted.
    pub(crate) fn bytes(&self) -> u64 {
        let all = self.lex.iter().chain(&self.post).chain(&self.pos).map(Vec::len).fold(self.head.len(), usize::saturating_add);
        self.taken.saturating_add(all as u64)
    }

    pub(crate) fn lex_pages(&self) -> usize {
        (self.lex_from as usize).saturating_add(self.lex.len())
    }

    pub(crate) fn post_pages(&self) -> usize {
        (self.post_from as usize).saturating_add(self.post.len())
    }

    pub(crate) fn pos_pages(&self) -> usize {
        (self.pos_from as usize).saturating_add(self.pos.len())
    }

    /// The pages held, and the pages held as they're being made: what the builder keeps in memory.
    fn held(&self) -> usize {
        self.lex.iter().chain(&self.post).chain(&self.pos).map(Vec::len).fold(0, usize::saturating_add)
    }
}

/// A page of a segment, finished.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SegPage {
    Lex(u32, Vec<u8>),
    Post(u32, Vec<u8>),
    Pos(u32, Vec<u8>),
}

/// Makes a segment from words in rising order.
#[derive(Default)]
pub(crate) struct SegBuilder {
    done: SegFiles,
    /// Each lexicon page's first word and how many it has.
    firsts: Vec<(Vec<u8>, u32)>,
    page: Vec<u8>,
    count: u16,
    prev: Vec<u8>,
    post: Vec<u8>,
    pos: Vec<u8>,
}

/// `bytes` added to the pages being made of `magic`, the page under way (`page`) going on into
/// new ones as each fills to `size`.
fn spill(page: &mut Vec<u8>, done: &mut Vec<Vec<u8>>, magic: &[u8; 4], size: usize, bytes: &[u8]) {
    let mut rest = bytes;
    while !rest.is_empty() {
        let room = (size + 5).saturating_sub(page.len()).max(1);
        let (now, later) = rest.split_at(room.min(rest.len()));
        page.extend_from_slice(now);
        rest = later;
        if page.len() >= size + 5 {
            done.push(std::mem::replace(page, page_start(magic)));
        }
    }
}

fn page_start(magic: &[u8; 4]) -> Vec<u8> {
    let mut p = Vec::with_capacity(64 << 10);
    p.extend_from_slice(magic);
    p.push(VERSION);
    p
}

impl SegBuilder {
    /// A word (field letter first, at most [`MAX_KEY`] bytes), its docs, rising, and their places
    /// ([`put_places`], a block a doc, in their order; none for a word that has none:
    /// [`needs_places`]). A word that should have places and has none isn't kept.
    pub(crate) fn push(&mut self, key: &[u8], docs: &[u32], places: &[u8]) {
        let (Some(&first), Some(&last)) = (docs.first(), docs.last()) else { return };
        // A block a doc is at least 2 bytes (its count, a place).
        let unplaced = if needs_places(key) { places.len() < docs.len().saturating_mul(2) } else { !places.is_empty() };
        if key.is_empty() || key.len() > MAX_KEY || unplaced {
            return;
        }
        if self.done.terms == 0 {
            (self.done.lo, self.done.hi) = (first, last);
        }
        (self.done.lo, self.done.hi) = (self.done.lo.min(first), self.done.hi.max(last));
        let mut list = Vec::with_capacity(docs.len() * 2);
        let mut last = None;
        for &d in docs {
            put_var(&mut list, u64::from(last.map_or(d, |l: u32| d.wrapping_sub(l))));
            last = Some(d);
        }
        let mut payload = Vec::with_capacity(16);
        let placed = if places.is_empty() { 0 } else { PLACED };
        if docs.len() <= INLINE {
            payload.push(placed);
            payload.extend_from_slice(&list);
        } else {
            if self.post.is_empty() {
                self.post = page_start(POST);
            }
            payload.push(AT | placed);
            put_len(&mut payload, self.done.post_pages());
            put_len(&mut payload, self.post.len() - 5);
            put_len(&mut payload, list.len());
            spill(&mut self.post, &mut self.done.post, POST, POST_PAGE, &list);
        }
        // Where its places start among the segment's: a page's first word's is in the page's
        // head, and each word's places follow the one's before.
        let at = (self.done.pos_pages() as u64).saturating_mul(POS_PAGE as u64).saturating_add(self.pos.len().saturating_sub(5) as u64);
        if !places.is_empty() {
            if self.pos.is_empty() {
                self.pos = page_start(POS);
            }
            put_len(&mut payload, places.len());
            spill(&mut self.pos, &mut self.done.pos, POS, POS_PAGE, places);
        }
        let entry = |prev: &[u8]| {
            let shared = prev.iter().zip(key).take_while(|(a, b)| a == b).count();
            let mut e = Vec::with_capacity(key.len() + payload.len() + 8);
            put_len(&mut e, shared);
            put_len(&mut e, key.len() - shared);
            e.extend_from_slice(key.get(shared..).unwrap_or_default());
            put_len(&mut e, docs.len());
            e.extend_from_slice(&payload);
            e
        };
        let mut e = entry(&self.prev);
        if self.count > 0 && (self.page.len() + e.len() > LEX_PAGE || self.count == u16::MAX) {
            self.close_page();
            e = entry(&[]);
        }
        if self.count == 0 {
            self.page = page_start(LEX);
            self.page.extend_from_slice(&[0, 0]);
            put_var(&mut self.page, at);
            self.firsts.push((key.to_vec(), 0));
        }
        self.page.extend_from_slice(&e);
        self.count += 1;
        if let Some(f) = self.firsts.last_mut() {
            f.1 += 1;
        }
        self.prev = key.to_vec();
        self.done.terms = self.done.terms.saturating_add(1);
    }

    fn close_page(&mut self) {
        if self.count == 0 {
            return;
        }
        let count = self.count.to_le_bytes();
        if let Some(c) = self.page.get_mut(5..7) {
            c.copy_from_slice(&count);
        }
        self.done.lex.push(std::mem::take(&mut self.page));
        self.count = 0;
        self.prev.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.done.terms == 0
    }

    /// What it holds in memory, in bytes: the pages finished and not taken, and those being made.
    pub(crate) fn held(&self) -> usize {
        self.done.held().saturating_add(self.page.len()).saturating_add(self.post.len()).saturating_add(self.pos.len())
    }

    /// The pages finished so far, to be written before the segment is: they're no longer held.
    pub(crate) fn take(&mut self) -> Vec<SegPage> {
        let d = &mut self.done;
        let mut out = Vec::with_capacity(d.lex.len() + d.post.len() + d.pos.len());
        for page in d.lex.drain(..) {
            d.taken = d.taken.saturating_add(page.len() as u64);
            out.push(SegPage::Lex(d.lex_from, page));
            d.lex_from = d.lex_from.saturating_add(1);
        }
        for page in d.post.drain(..) {
            d.taken = d.taken.saturating_add(page.len() as u64);
            out.push(SegPage::Post(d.post_from, page));
            d.post_from = d.post_from.saturating_add(1);
        }
        for page in d.pos.drain(..) {
            d.taken = d.taken.saturating_add(page.len() as u64);
            out.push(SegPage::Pos(d.pos_from, page));
            d.pos_from = d.pos_from.saturating_add(1);
        }
        out
    }

    pub(crate) fn finish(mut self) -> SegFiles {
        self.close_page();
        if self.post.len() > 5 {
            self.done.post.push(std::mem::take(&mut self.post));
        }
        if self.pos.len() > 5 {
            self.done.pos.push(std::mem::take(&mut self.pos));
        }
        let mut head = page_start(HEAD);
        put_len(&mut head, self.done.lex_pages());
        put_len(&mut head, self.done.post_pages());
        put_var(&mut head, u64::from(self.done.terms));
        for (k, n) in &self.firsts {
            head.push(u8::try_from(k.len()).unwrap_or(u8::MAX));
            head.extend_from_slice(k);
            put_var(&mut head, u64::from(*n));
        }
        self.done.head = head;
        self.done
    }
}

/// A segment's head: each lexicon page's first word.
pub(crate) fn decode_head(b: &[u8], s: &SegInfo) -> R<Vec<Vec<u8>>> {
    let mut r = Rd::new(b);
    r.header(HEAD)?;
    let (lex, post) = (r.var32()?, r.var32()?);
    let _terms = r.var32()?;
    if lex != s.lex_pages || post != s.post_pages {
        return Err("head of another segment");
    }
    let mut out: Vec<Vec<u8>> = Vec::with_capacity((lex as usize).min(r.left()));
    for _ in 0..lex {
        let len = usize::from(r.u8()?);
        let k = r.take(len)?;
        if k.is_empty() || len > MAX_KEY || out.last().is_some_and(|p| p.as_slice() >= k) {
            return Err("head's words out of order");
        }
        out.push(k.to_vec());
        r.var32()?;
    }
    r.end()?;
    Ok(out)
}

/// Where a word's docs are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Postings {
    Inline(Vec<u32>),
    /// From `offset` in postings page `page`, `len` bytes, going on into the next pages.
    At {
        page: u32,
        offset: u32,
        len: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LexEntry {
    pub(crate) key: Vec<u8>,
    pub(crate) df: u32,
    pub(crate) at: Postings,
    /// Where its places are: none for an address, nor in a segment from before they were kept.
    pub(crate) pos: Option<PosAt>,
}

/// A lexicon page of a segment with `post_pages` postings pages and `pos_pages` places pages, in
/// an index of `next_doc` docs.
pub(crate) fn decode_lex(b: &[u8], next_doc: u32, post_pages: u32, pos_pages: u32) -> R<Vec<LexEntry>> {
    let mut page = LexStream::new(b.to_vec(), next_doc, post_pages, pos_pages)?;
    let mut out: Vec<LexEntry> = Vec::with_capacity(page.left);
    while let Some(e) = page.next_entry()? {
        out.push(e);
    }
    Ok(out)
}

/// A lexicon page read a word at a time: its bytes held, and the word before (a merge holds one
/// page of each segment it merges, not each page's words).
pub(crate) struct LexStream {
    raw: Vec<u8>,
    pos: usize,
    /// Words not yet read.
    left: usize,
    prev: Vec<u8>,
    next_doc: u32,
    post_pages: u32,
    pos_pages: u32,
    /// Where the next word's places start among the segment's.
    places_at: u64,
}

impl LexStream {
    /// A lexicon page of a segment with `post_pages` postings pages and `pos_pages` places pages,
    /// in an index of `next_doc` docs: its head checked.
    pub(crate) fn new(raw: Vec<u8>, next_doc: u32, post_pages: u32, pos_pages: u32) -> R<LexStream> {
        if raw.len() > LEX_PAGE + MAX_KEY + 64 {
            return Err("too big");
        }
        let mut r = Rd::new(&raw);
        r.header(LEX)?;
        let n = usize::from(r.u16()?);
        if n == 0 || n.saturating_mul(4) > r.left() {
            return Err("words past what's there");
        }
        let places_at = r.var()?;
        if places_at > u64::from(pos_pages).saturating_mul(POS_PAGE as u64) {
            return Err("places out of range");
        }
        let pos = raw.len() - r.left();
        Ok(LexStream { raw, pos, left: n, prev: Vec::new(), next_doc, post_pages, pos_pages, places_at })
    }

    /// What it holds, in bytes.
    pub(crate) fn held(&self) -> usize {
        self.raw.len().saturating_add(self.prev.len())
    }

    /// The next word; `None` past the last (the page checked to end there).
    pub(crate) fn next_entry(&mut self) -> R<Option<LexEntry>> {
        let mut r = Rd::new(self.raw.get(self.pos..).unwrap_or_default());
        if self.left == 0 {
            r.end()?;
            return Ok(None);
        }
        // No word is empty: none before means this is the first.
        let first = self.prev.is_empty();
        let (next_doc, post_pages, prev) = (self.next_doc, self.post_pages, &self.prev);
        let shared = r.count(MAX_KEY, 0)?;
        let suffix = r.count(MAX_KEY, 1)?;
        if shared > prev.len() || shared + suffix > MAX_KEY || shared + suffix == 0 {
            return Err("word out of range");
        }
        let mut key = prev.get(..shared).unwrap_or_default().to_vec();
        key.extend_from_slice(r.take(suffix)?);
        if !first && key <= *prev {
            return Err("words out of order");
        }
        let df = r.var32()?;
        if df == 0 || df > next_doc {
            return Err("word in no docs");
        }
        let kind = r.u8()?;
        if kind & !(AT | PLACED) != 0 {
            return Err("unknown postings kind");
        }
        let at = match kind & AT {
            0 => {
                if df as usize > INLINE {
                    return Err("too many docs inline");
                }
                let mut docs = Vec::with_capacity(df as usize);
                let mut last: Option<u32> = None;
                for _ in 0..df {
                    let d = r.var32()?;
                    let doc = match last {
                        None => d,
                        Some(l) if d > 0 => l.checked_add(d).ok_or("doc past the ids")?,
                        Some(_) => return Err("docs out of order"),
                    };
                    if doc >= next_doc {
                        return Err("doc past the ids");
                    }
                    docs.push(doc);
                    last = Some(doc);
                }
                Postings::Inline(docs)
            }
            _ => {
                let at = Postings::At { page: r.var32()?, offset: r.var32()?, len: r.var32()? };
                if let Postings::At { page, offset, len } = at
                    && (page >= post_pages || offset as usize > POST_PAGE || (len as usize) < df as usize || u64::from(len) > u64::from(df) * 5)
                {
                    return Err("postings out of range");
                }
                at
            }
        };
        let pos = match kind & PLACED {
            0 => None,
            _ => {
                let at = PosAt::starting(self.places_at, r.var()?, df, self.pos_pages)?;
                self.places_at = self.places_at.saturating_add(at.len);
                Some(at)
            }
        };
        self.pos = self.raw.len() - r.left();
        self.left -= 1;
        self.prev.clone_from(&key);
        Ok(Some(LexEntry { key, df, at, pos }))
    }
}

/// A postings page's own bytes.
pub(crate) fn decode_post_page(b: &[u8]) -> R<&[u8]> {
    if b.len() > POST_PAGE + 5 {
        return Err("too big");
    }
    let mut r = Rd::new(b);
    r.header(POST)?;
    r.take(r.left())
}

/// `df` docs from their varint deltas, rising and below `next_doc`.
pub(crate) fn decode_postings(b: &[u8], df: u32, next_doc: u32) -> R<Vec<u32>> {
    let mut r = Rd::new(b);
    let mut out = Vec::with_capacity((df as usize).min(b.len()));
    let mut last: Option<u32> = None;
    for _ in 0..df {
        let d = r.var32()?;
        let doc = match last {
            None => d,
            Some(l) if d > 0 => l.checked_add(d).ok_or("doc past the ids")?,
            Some(_) => return Err("docs out of order"),
        };
        if doc >= next_doc {
            return Err("doc past the ids");
        }
        out.push(doc);
        last = Some(doc);
    }
    r.end()?;
    Ok(out)
}
