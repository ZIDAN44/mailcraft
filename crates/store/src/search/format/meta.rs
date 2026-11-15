//! The meta, the index's commit point: what the index is now (its doc and text pages, its
//! folders and segments) and the files it uses.

use super::*;

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
    /// Its places pages: none in a segment written before they were kept.
    pub(crate) pos_pages: u32,
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
        let pos = (0..self.pos_pages).map(|n| Name::SegPos(self.id, n));
        std::iter::once(Name::SegHead(self.id)).chain(lex).chain(post).chain(pos)
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
        Name::SegPos(id, p) => (5, u64::from(id), u64::from(p)),
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
        5 => Name::SegPos(id()?, b),
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
    // Each segment's places pages, after what a meta from before them ends with.
    put_len(&mut out, m.segments.len());
    for s in &m.segments {
        put_var(&mut out, u64::from(s.pos_pages));
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
            pos_pages: 0,
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
    // A meta from before places were kept ends here: its segments have none.
    if r.left() > 0 {
        if r.count(MAX_SEGMENTS, 1)? != m.segments.len() {
            return Err("places pages of other segments");
        }
        for s in &mut m.segments {
            s.pos_pages = r.var32()?;
            if s.pos_pages as usize > MAX_PAGES {
                return Err("segment out of range");
            }
        }
    }
    r.end()?;
    Ok(m)
}
