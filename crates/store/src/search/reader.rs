//! Reading an account's index: which doc a message is, and which docs have a word starting so.
//!
//! The doc table and each segment's head are held in memory; lexicon, postings and text pages are
//! read as they're needed, a read transaction a page (none outlives a call), and the latest kept
//! in a small cache keyed by the index's epoch. A word is looked for a few pages at a time
//! ([`Reader::scan_prefix`]), so a short prefix of a big index is spread over as many steps as it
//! takes. [`Reader::refresh`] follows the writer: it reads only the doc pages and segments a newer
//! generation changed.

use std::collections::HashMap;

use mailcraft_model::search::TOKENIZER_VERSION;

use super::format::{
    Blob, LexEntry, Meta, Postings, SegInfo, decode_blob, decode_head, decode_lex, decode_post_page, decode_postings, decode_text_page,
};
use super::*;

/// What the reader caches of the pages it reads.
const CACHE_BYTES: usize = 8 << 20;
/// Times a reader opens again when commits keep removing what it was reading.
const RETRIES: usize = 16;

/// What the index knows of a doc besides its words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocInfo {
    /// Its files (`has:attachment`).
    pub files: u16,
    /// It has text of its own.
    pub has_text: bool,
    /// Something of it was left out: a word not found may still be in it.
    pub truncated: bool,
    /// Some of its parts went unread: a file may be among them.
    pub parts_cut: bool,
    /// Its source isn't kept here: only its summary can be searched (no word is in it, and it's
    /// not waiting to be indexed either).
    pub no_source: bool,
    pub msgid_hash: u64,
    pub size: u32,
}

impl DocInfo {
    /// Whether the message carries files, as far as the index knows: `None` when its source
    /// isn't kept or it couldn't be read (its summary says, then). `has:attachment` and the
    /// summaries put right by a pass ([`Pass::take_files`]) read it alike.
    pub fn files_known(&self) -> Option<bool> {
        let bits = if self.has_text { HAS_TEXT } else { 0 }
            | if self.truncated { TRUNCATED } else { 0 }
            | if self.no_source { NO_SOURCE } else { 0 }
            | if self.parts_cut { 0 } else { WHOLE };
        files_known(self.files, bits)
    }
}

/// Where a [`Reader::scan_prefix`] got to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanCursor {
    seg: usize,
    page: Option<u32>,
    entry: usize,
}

/// Pages read lately, the least recently used let go first.
#[derive(Debug, Default)]
struct Cache {
    pages: HashMap<String, (u64, Arc<Vec<u8>>)>,
    bytes: usize,
    tick: u64,
    /// Pages read from the store (not found here).
    reads: u64,
}

impl Cache {
    fn get(&mut self, store: &Store, epoch: u64, path: &Path) -> Result<Arc<Vec<u8>>> {
        self.tick = self.tick.wrapping_add(1);
        let key = format!("{epoch:016x}/{}", path.display());
        if let Some((t, page)) = self.pages.get_mut(&key) {
            *t = self.tick;
            return Ok(page.clone());
        }
        self.reads = self.reads.wrapping_add(1);
        let page = Arc::new(need_file(store, path)?);
        if page.len() <= CACHE_BYTES / 4 {
            while self.bytes + page.len() > CACHE_BYTES {
                let Some(oldest) = self.pages.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) else { break };
                if let Some((_, p)) = self.pages.remove(&oldest) {
                    self.bytes = self.bytes.saturating_sub(p.len());
                }
            }
            self.bytes += page.len();
            self.pages.insert(key, (self.tick, page.clone()));
        }
        Ok(page)
    }
}

/// An account's index as of one generation, read.
#[derive(Debug)]
pub struct Reader {
    account: String,
    base: PathBuf,
    meta: Meta,
    docs: Vec<DocRec>,
    folder_ix: HashMap<String, u16>,
    by_key: HashMap<(u16, u32), u32>,
    /// Each segment's lexicon pages' first words, in the meta's order.
    heads: Vec<(SegInfo, Vec<Vec<u8>>)>,
    cache: Cache,
}

impl Reader {
    /// `account`'s index, read; `None` when it has none yet, or none to use: words read with
    /// another tokenizer (the writer makes them again), or an index that doesn't read back (set
    /// aside, to be made again). While its writer is held that's the writer's to do: the
    /// damage is the error ([`is_damaged`]). Commits that keep removing what it reads, faster
    /// than it reads it, are an error that says so ([`is_gone`]): try again later.
    pub fn open(store: &Store, account: &str) -> Result<Option<Reader>> {
        let mut tries = 0;
        loop {
            let meta = match load_meta(store, account) {
                Ok(Some(m)) => m,
                Ok(None) => return Ok(None),
                Err(e) if is_damaged(&e) => return damage(store, account, e),
                Err(e) => return Err(e),
            };
            let at = (meta.epoch, meta.generation);
            match Reader::read(store, account, meta) {
                // A commit removed a page between the meta and it: read the new one. A page the
                // same meta still names and isn't there is damage.
                Err(e) if is_gone(&e) => {
                    let now = load_meta(store, account).ok().flatten().map(|m| (m.epoch, m.generation));
                    if now == Some(at) {
                        return damage(store, account, missing(e));
                    }
                    tries += 1;
                    if tries >= RETRIES {
                        return Err(e);
                    }
                }
                Err(e) if is_damaged(&e) => return damage(store, account, e),
                other => return other,
            }
        }
    }

    fn read(store: &Store, account: &str, meta: Meta) -> Result<Option<Reader>> {
        let base = dir(store, account);
        if meta.tokenizer != TOKENIZER_VERSION {
            return Ok(None);
        }
        let docs = load_docs(store, &base, &meta)?;
        let mut r = Reader {
            account: account.to_string(),
            base,
            meta: Meta::default(),
            docs,
            folder_ix: HashMap::new(),
            by_key: HashMap::new(),
            heads: Vec::new(),
            cache: Cache::default(),
        };
        for (i, d) in r.docs.iter().enumerate() {
            if d.live() {
                r.by_key.insert((d.folder, d.uid), u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        r.heads = r.load_heads(store, &meta)?;
        r.set_meta(meta);
        Ok(Some(r))
    }

    fn set_meta(&mut self, meta: Meta) {
        self.folder_ix = meta.folders.iter().enumerate().filter_map(|(i, f)| Some((f.clone(), u16::try_from(i).ok()?))).collect();
        self.meta = meta;
    }

    /// The heads of `meta`'s segments: those held kept, the others read.
    fn load_heads(&self, store: &Store, meta: &Meta) -> Result<Vec<(SegInfo, Vec<Vec<u8>>)>> {
        let held: HashMap<u32, &(SegInfo, Vec<Vec<u8>>)> = self.heads.iter().map(|h| (h.0.id, h)).collect();
        let mut out = Vec::with_capacity(meta.segments.len());
        for s in &meta.segments {
            match held.get(&s.id).filter(|(old, _)| old == s) {
                Some(h) => out.push((*h).clone()),
                None => {
                    let path = Name::SegHead(s.id).path(&self.base);
                    let bytes = need_file(store, &path)?;
                    out.push((*s, decode_head(&bytes, s).map_err(|what| damaged(&path, what))?));
                }
            }
        }
        Ok(out)
    }

    /// Follow the writer: the doc pages and segments a newer generation changed are read, the
    /// rest kept. Whether anything changed. An index set aside or made anew since is read whole
    /// again; one that is gone, or can't be used, is an error ([`is_gone`], [`is_damaged`]).
    pub fn refresh(&mut self, store: &Store) -> Result<bool> {
        let meta = match load_meta(store, &self.account) {
            Ok(Some(m)) => m,
            Ok(None) => return Err(io_err(&self.base)(io::Error::from(io::ErrorKind::NotFound))),
            Err(e) => return Err(e),
        };
        if meta.generation == self.meta.generation && meta.epoch == self.meta.epoch {
            return Ok(false);
        }
        if meta.epoch != self.meta.epoch || meta.tokenizer != self.meta.tokenizer || meta.next_doc < self.meta.next_doc {
            let Some(r) = Reader::open(store, &self.account)? else {
                return Err(io_err(&self.base)(io::Error::from(io::ErrorKind::NotFound)));
            };
            *self = r;
            return Ok(true);
        }
        // All read and checked before anything changes: a refresh that fails leaves the reader
        // as it was, at the generation before.
        let mut pages = Vec::new();
        let mut live = self.docs.iter().filter(|d| d.live()).count();
        let mut len = self.docs.len();
        for p in 0..meta.doc_pages.len() {
            if self.meta.doc_pages.get(p) == meta.doc_pages.get(p) {
                continue;
            }
            let recs = load_doc_page(store, &self.base, &meta, p)?;
            let start = p.saturating_mul(DOCS_PER_PAGE as usize);
            let had = self.docs.get(start..).unwrap_or_default().iter().take(recs.len()).filter(|d| d.live()).count();
            live = live.saturating_sub(had).saturating_add(recs.iter().filter(|d| d.live()).count());
            len = len.max(start.saturating_add(recs.len()));
            pages.push((start, recs));
        }
        if len != meta.next_doc as usize || live != meta.live as usize {
            return Err(damaged(&self.base.join("meta.bin"), "counts not the doc table's"));
        }
        let heads = self.load_heads(store, &meta)?;
        for (start, recs) in pages {
            for (i, d) in recs.into_iter().enumerate() {
                let doc = u32::try_from(start + i).unwrap_or(u32::MAX);
                if let Some(old) = self.docs.get(start + i).copied()
                    && old.live()
                    && self.by_key.get(&(old.folder, old.uid)) == Some(&doc)
                {
                    self.by_key.remove(&(old.folder, old.uid));
                }
                if d.live() {
                    self.by_key.insert((d.folder, d.uid), doc);
                }
                match self.docs.get_mut(start + i) {
                    Some(slot) => *slot = d,
                    None => self.docs.push(d),
                }
            }
        }
        self.heads = heads;
        self.set_meta(meta);
        Ok(true)
    }

    pub fn generation(&self) -> u64 {
        self.meta.generation
    }

    /// The index's epoch: another one is another index (made again), whatever its generation.
    pub fn epoch(&self) -> u64 {
        self.meta.epoch
    }

    /// One past the highest doc id: the size of a [`Bits`] for this index.
    pub fn len(&self) -> u32 {
        self.meta.next_doc
    }

    pub fn is_empty(&self) -> bool {
        self.meta.next_doc == 0
    }

    /// Docs in the index (those removed not counted).
    pub fn live(&self) -> u32 {
        self.meta.live
    }

    /// Lexicon, postings and text pages read from the store so far (those its cache held not
    /// counted): what a search's budget counts.
    pub fn pages_read(&self) -> u64 {
        self.cache.reads
    }

    /// The doc a message is, when it's indexed.
    pub fn doc(&self, key: &MessageKey) -> Option<u32> {
        self.doc_at(self.folder_number(&key.folder)?, key.validity, key.uid)
    }

    /// What the index numbers a folder by, for looking up its messages one after another
    /// ([`Reader::doc_at`]) without the folder's name each time.
    pub fn folder_number(&self, folder: &str) -> Option<u16> {
        self.folder_ix.get(folder).copied()
    }

    /// The doc a message of the folder numbered `folder` is, when it's indexed.
    pub fn doc_at(&self, folder: u16, validity: u32, uid: u32) -> Option<u32> {
        let doc = *self.by_key.get(&(folder, uid))?;
        self.docs.get(doc as usize).filter(|d| d.live() && d.validity == validity).map(|_| doc)
    }

    /// The message a live doc is.
    pub fn key(&self, doc: u32) -> Option<MessageKey> {
        let d = self.docs.get(doc as usize).filter(|d| d.live())?;
        let folder = self.meta.folders.get(usize::from(d.folder))?;
        Some(MessageKey { account: self.account.clone(), folder: folder.clone(), validity: d.validity, uid: d.uid })
    }

    pub fn info(&self, doc: u32) -> Option<DocInfo> {
        let d = self.docs.get(doc as usize).filter(|d| d.live())?;
        Some(DocInfo {
            files: d.files,
            has_text: d.bits & HAS_TEXT != 0,
            truncated: d.bits & TRUNCATED != 0,
            // Docs from before [`WHOLE`] lack it: one cut is taken as cut anywhere.
            parts_cut: d.bits & TRUNCATED != 0 && d.bits & WHOLE == 0,
            no_source: d.bits & NO_SOURCE != 0,
            msgid_hash: d.msgid_hash,
            size: d.size,
        })
    }

    /// Set in `out` the live docs with a word of `field` starting with `prefix` (folded as
    /// [`tokens`](mailcraft_model::search::tokens) gives words; an [address](address_words)
    /// lower-cased), reading at most about `max_pages` pages (at least one word's worth): `true`
    /// once done, `false` to be called again with the same cursor.
    pub fn scan_prefix(&mut self, store: &Store, field: Field, prefix: &str, cur: &mut ScanCursor, out: &mut Bits, max_pages: usize) -> Result<bool> {
        let target = key(field, prefix);
        let mut read = 0usize;
        let mut moved = false;
        while let Some((info, firsts)) = self.heads.get(cur.seg) {
            let info = *info;
            let page = match cur.page {
                Some(p) => p,
                None => {
                    // The last page starting at or before the prefix: its words may run into it.
                    let at = firsts.partition_point(|f| f.as_slice() <= target.as_slice()).saturating_sub(1);
                    cur.entry = 0;
                    u32::try_from(at).unwrap_or(u32::MAX)
                }
            };
            cur.page = Some(page);
            if page >= info.lex_pages {
                (cur.seg, cur.page) = (cur.seg + 1, None);
                continue;
            }
            if moved && read >= max_pages {
                return Ok(false);
            }
            let entries = self.lex_page(store, &info, page)?;
            read += 1;
            let mut past = false;
            while let Some(e) = entries.get(cur.entry) {
                if e.key.as_slice() < target.as_slice() {
                    cur.entry += 1;
                    continue;
                }
                if !e.key.starts_with(&target) {
                    past = true;
                    break;
                }
                if matches!(e.at, Postings::At { .. }) && moved && read >= max_pages {
                    return Ok(false);
                }
                read += self.postings(store, &info, e, out)?;
                cur.entry += 1;
                moved = true;
            }
            moved = true;
            if past {
                (cur.seg, cur.page) = (cur.seg + 1, None);
            } else {
                (cur.page, cur.entry) = (Some(page + 1), 0);
            }
        }
        Ok(true)
    }

    /// The live docs with a word of `field` starting with `prefix`, all of them at once.
    pub fn word(&mut self, store: &Store, field: Field, prefix: &str) -> Result<Bits> {
        let mut out = Bits::new(self.len());
        let mut cur = ScanCursor::default();
        while !self.scan_prefix(store, field, prefix, &mut cur, &mut out, usize::MAX)? {}
        Ok(out)
    }

    fn lex_page(&mut self, store: &Store, info: &SegInfo, page: u32) -> Result<Vec<LexEntry>> {
        let path = Name::SegLex(info.id, page).path(&self.base);
        let bytes = self.cache.get(store, self.meta.epoch, &path)?;
        decode_lex(&bytes, self.meta.next_doc, info.post_pages).map_err(|what| damaged(&path, what))
    }

    /// Set a word's live docs in `out`; the postings pages read.
    fn postings(&mut self, store: &Store, info: &SegInfo, e: &LexEntry, out: &mut Bits) -> Result<usize> {
        let (docs, read) = match &e.at {
            Postings::Inline(docs) => (docs.clone(), 0),
            &Postings::At { page, offset, len } => {
                let mut bytes: Vec<u8> = Vec::with_capacity((len as usize).min(16 << 20));
                let (mut p, mut from, mut read) = (page, offset as usize, 0usize);
                while bytes.len() < len as usize {
                    let path = Name::SegPost(info.id, p).path(&self.base);
                    if p >= info.post_pages {
                        return Err(damaged(&path, "postings past the segment"));
                    }
                    let raw = self.cache.get(store, self.meta.epoch, &path)?;
                    let data = decode_post_page(&raw).map_err(|what| damaged(&path, what))?;
                    let rest = data.get(from..).ok_or_else(|| damaged(&path, "postings past the page"))?;
                    let want = (len as usize - bytes.len()).min(rest.len());
                    bytes.extend_from_slice(rest.get(..want).unwrap_or_default());
                    if want == 0 {
                        return Err(damaged(&path, "postings past the page"));
                    }
                    (p, from, read) = (p.saturating_add(1), 0, read + 1);
                }
                let path = Name::SegPost(info.id, page).path(&self.base);
                (decode_postings(&bytes, e.df, self.meta.next_doc).map_err(|what| damaged(&path, what))?, read)
            }
        };
        for d in docs {
            if self.docs.get(d as usize).is_some_and(DocRec::live) {
                out.set(d);
            }
        }
        Ok(read)
    }

    /// A live doc's text and files' names as kept; `None` when none is kept.
    pub fn text(&mut self, store: &Store, doc: u32) -> Result<Option<Blob>> {
        let Some(d) = self.docs.get(doc as usize).filter(|d| d.live() && d.text != NO_TEXT) else { return Ok(None) };
        let Some(t) = self.meta.text_pages.get(d.text as usize).filter(|t| t.generation != 0) else { return Ok(None) };
        let path = Name::Text { generation: t.generation, page: d.text }.path(&self.base);
        let bytes = self.cache.get(store, self.meta.epoch, &path)?;
        let recs = decode_text_page(&bytes, self.meta.next_doc).map_err(|what| damaged(&path, what))?;
        let Ok(i) = recs.binary_search_by_key(&doc, |(d, _)| *d) else { return Ok(None) };
        let Some((_, blob)) = recs.get(i) else { return Ok(None) };
        decode_blob(blob).map(Some).map_err(|what| damaged(&path, what))
    }
}

/// The index doesn't read back: set aside to be made again, `None`. While its writer is held,
/// the error instead, for the writer's holder to set it aside (a writer would put back the meta
/// renamed under it).
fn damage(store: &Store, account: &str, e: StoreError) -> Result<Option<Reader>> {
    // Held while it's set aside, so no writer opens the index meanwhile (and writes it back).
    let Ok(held) = Held::take(store, account) else { return Err(e) };
    set_aside(store, account)?;
    drop(held);
    Ok(None)
}

/// A page the meta names that stays missing: damage.
fn missing(e: StoreError) -> StoreError {
    match e {
        StoreError::Io { path, .. } => StoreError::Damaged { path, what: "a page the meta names is missing".into() },
        e => e,
    }
}
