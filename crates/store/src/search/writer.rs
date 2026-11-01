//! Writing an account's index: docs added, removed and moved in memory, then written as a new
//! generation ([`Writer::flush`]).
//!
//! A flush is made in two steps. [`Writer::prepare`] encodes and compresses everything the new
//! generation needs without writing anything (it may run anywhere the writer can be read).
//! [`Writer::commit`] then writes those files in transactions of at most
//! [`BACKGROUND_COMMIT`](crate::BACKGROUND_COMMIT) bytes, under names the current meta doesn't use,
//! and the meta last, sealed. Until the meta is in, the index is the generation before: a batch
//! that fails, or a crash, costs nothing but the files written, which the next commit or the next
//! [`Writer::open`] removes.

use std::collections::{BTreeMap, HashMap};

use mailcraft_model::search::{IndexDoc, TOKENIZER_VERSION, tokens};

use super::format::{
    MAX_META, MAX_NAME_BYTES, MAX_SEGMENTS, Meta, SegBuilder, SegFiles, SegInfo, TextPage, encode_blob, encode_docs, encode_meta, encode_text_page,
};
use super::*;

/// One account's index being written. One at a time per account: another
/// [`open`](Writer::open) while it's held is [`StoreError::Busy`].
#[derive(Debug)]
pub struct Writer {
    account: String,
    base: PathBuf,
    packs: bool,
    /// As last committed.
    meta: Meta,
    /// As of now, the changes since included.
    docs: Vec<DocRec>,
    folders: Vec<String>,
    folder_ix: HashMap<String, u16>,
    /// The folders' names' bytes, all told.
    folder_bytes: usize,
    /// Each live doc by (folder, uid).
    by_key: HashMap<(u16, u32), u32>,
    /// Doc pages changed since the meta.
    dirty: BTreeSet<usize>,
    /// The words added since, and their docs.
    words: BTreeMap<Vec<u8>, Vec<u32>>,
    word_bytes: usize,
    word_docs: u32,
    /// Texts of the docs added since, by doc.
    texts: Vec<(u32, Vec<u8>)>,
    text_bytes: usize,
    /// Texts to take out of their pages, by page.
    drop_texts: BTreeMap<u32, BTreeSet<u32>>,
    /// The bytes of those pages, each read and written again by the next flush.
    drop_bytes: usize,
    live: u32,
    dead: u32,
    /// Changes with every change: a flush prepared before one is out of date.
    stamp: u64,
    /// A commit failed: files it wrote may be left to remove.
    orphans: bool,
    /// Stop a commit after its files, before its meta (a crash there).
    #[cfg(test)]
    pub(crate) stop_before_meta: bool,
    /// Fewer segments than a meta names, to reach the limit sooner.
    #[cfg(test)]
    pub(crate) max_segments: usize,
    /// A smaller [`IN_FLIGHT_BYTES`], to reach it sooner.
    #[cfg(test)]
    pub(crate) in_flight: usize,
    _held: Held,
}

/// A new generation made ready by [`Writer::prepare`], to be [committed](Writer::commit).
#[derive(Debug)]
pub struct Flush {
    stamp: u64,
    meta: Meta,
    files: Vec<(PathBuf, Prepared)>,
    /// The text page each new text went in.
    placed: Vec<(u32, u32)>,
}

impl Flush {
    /// Nothing changed: committing it writes nothing.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.meta.gone.is_empty()
    }

    /// What its files keep, in bytes.
    pub fn bytes(&self) -> usize {
        self.files.iter().map(|(_, p)| p.kept_len()).fold(0, usize::saturating_add)
    }
}

fn other(what: &str) -> io::Error {
    io::Error::other(what.to_string())
}

/// A doc's words, added to the words to write: the text's, the names', and both's addresses.
/// Returns the bytes the words took.
fn add_words(words: &mut BTreeMap<Vec<u8>, Vec<u32>>, doc: u32, text: &str, names: &[String]) -> usize {
    let mut added = 0usize;
    let mut add = |k: Vec<u8>| {
        if k.len() > MAX_KEY {
            return;
        }
        let len = k.len();
        let list = words.entry(k).or_insert_with(|| {
            added += len + 48;
            Vec::new()
        });
        if list.last() != Some(&doc) {
            list.push(doc);
            added += 4;
        }
    };
    for w in tokens(text) {
        add(key(Field::Body, &w));
    }
    for n in names {
        for w in tokens(n) {
            add(key(Field::Names, &w));
        }
    }
    let mut addresses = BTreeSet::new();
    address_words(text, &mut addresses);
    for n in names {
        address_words(n, &mut addresses);
    }
    for a in addresses {
        add(key(Field::Address, &a));
    }
    added
}

impl Writer {
    /// Hold `account`'s index and read it: a new one when it has none, or when what it has doesn't
    /// read back (set aside first). Files of a commit that didn't finish are removed; words read
    /// with another tokenizer are made again from the texts kept (no message is read).
    pub fn open(store: &Store, account: &str) -> Result<Writer> {
        let held = Held::take(store, account)?;
        let base = dir(store, account);
        let loaded = match load(store, account, &base) {
            Ok(l) => l,
            Err(e) if is_damaged(&e) => {
                set_aside(store, account)?;
                None
            }
            Err(e) => return Err(e),
        };
        let (meta, docs) = match loaded {
            Some(l) => l,
            None => (fresh(store, account), Vec::new()),
        };
        let mut w = Writer::with(store, account, base, meta, docs, held);
        w.clean(store);
        if w.meta.tokenizer != TOKENIZER_VERSION {
            match w.rebuild(store) {
                Ok(()) => {}
                Err(e) if is_damaged(&e) => {
                    set_aside(store, account)?;
                    let base = w.base.clone();
                    w = Writer::with(store, account, base, fresh(store, account), Vec::new(), w._held);
                    w.clean(store);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(w)
    }

    fn with(store: &Store, account: &str, base: PathBuf, meta: Meta, docs: Vec<DocRec>, held: Held) -> Writer {
        let folders = meta.folders.clone();
        let folder_bytes = folders.iter().map(String::len).fold(0, usize::saturating_add);
        let folder_ix = folders.iter().enumerate().filter_map(|(i, f)| Some((f.clone(), u16::try_from(i).ok()?))).collect();
        let mut by_key = HashMap::with_capacity(docs.len());
        for (i, d) in docs.iter().enumerate() {
            if d.live() {
                by_key.insert((d.folder, d.uid), u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        Writer {
            account: account.to_string(),
            base,
            packs: store.packs(account),
            live: meta.live,
            dead: meta.dead,
            meta,
            docs,
            folders,
            folder_ix,
            folder_bytes,
            by_key,
            dirty: BTreeSet::new(),
            words: BTreeMap::new(),
            word_bytes: 0,
            word_docs: 0,
            texts: Vec::new(),
            text_bytes: 0,
            drop_texts: BTreeMap::new(),
            drop_bytes: 0,
            stamp: 0,
            orphans: false,
            #[cfg(test)]
            stop_before_meta: false,
            #[cfg(test)]
            max_segments: MAX_SEGMENTS,
            #[cfg(test)]
            in_flight: IN_FLIGHT_BYTES,
            _held: held,
        }
    }

    /// The generation last committed.
    pub fn generation(&self) -> u64 {
        self.meta.generation
    }

    /// Docs in the index now (those removed not counted).
    pub fn live(&self) -> u32 {
        self.live
    }

    /// The doc a message is, when it's in.
    pub fn find(&self, key: &MessageKey) -> Option<u32> {
        self.holder(key).filter(|&doc| self.docs.get(doc as usize).is_some_and(|d| d.validity == key.validity))
    }

    /// The live doc at a message's folder and uid, whatever its folder's validity: under another
    /// validity it's another message, which a message added there replaces.
    fn holder(&self, key: &MessageKey) -> Option<u32> {
        let f = self.folder_ix.get(&key.folder)?;
        let doc = *self.by_key.get(&(*f, key.uid))?;
        self.docs.get(doc as usize).filter(|d| d.live()).map(|_| doc)
    }

    /// The message `doc` is, with its Message-ID's hash and size (how a move is recognised).
    pub fn doc(&self, doc: u32) -> Option<(MessageKey, u64, u32)> {
        let d = self.docs.get(doc as usize).filter(|d| d.live())?;
        let folder = self.folders.get(usize::from(d.folder))?;
        let key = MessageKey { account: self.account.clone(), folder: folder.clone(), validity: d.validity, uid: d.uid };
        Some((key, d.msgid_hash, d.size))
    }

    /// Bytes held for the next flush, or that it will hold: the text pages losing a text are
    /// read and written again whole.
    pub fn pending_bytes(&self) -> usize {
        let docs = self.dirty.len().saturating_mul(DOCS_PER_PAGE as usize * format::REC);
        self.word_bytes.saturating_add(self.text_bytes).saturating_add(self.drop_bytes).saturating_add(docs)
    }

    /// Whether enough is held to flush now ([`IN_FLIGHT_BYTES`]: its half).
    pub fn should_flush(&self) -> bool {
        #[cfg(not(test))]
        let in_flight = IN_FLIGHT_BYTES;
        #[cfg(test)]
        let in_flight = self.in_flight;
        self.pending_bytes() >= in_flight / 2
    }

    /// A folder's number, a new folder given the next. A name longer than the meta keeps is
    /// refused (the message isn't indexed); past the folders, or the bytes of their names, the
    /// meta can hold, the index is to be made again (the folders no message is in left out).
    fn folder(&mut self, name: &str) -> Result<u16> {
        if let Some(f) = self.folder_ix.get(name) {
            return Ok(*f);
        }
        if name.len() > MAX_NAME_BYTES {
            return Err(io_err(&self.base)(io::Error::new(io::ErrorKind::InvalidInput, "a folder name too long to index")));
        }
        let f = u16::try_from(self.folders.len()).ok().filter(|f| usize::from(*f) < MAX_FOLDERS);
        let Some(f) = f else { return Err(damaged(&self.base, "more folders than it can name")) };
        let bytes = self.folder_bytes.saturating_add(name.len());
        if bytes > MAX_FOLDER_BYTES {
            return Err(damaged(&self.base, "more folder names than it can hold"));
        }
        self.folder_bytes = bytes;
        self.folders.push(name.to_string());
        self.folder_ix.insert(name.to_string(), f);
        Ok(f)
    }

    fn touch(&mut self, doc: u32) {
        self.dirty.insert((doc / DOCS_PER_PAGE) as usize);
        self.stamp = self.stamp.wrapping_add(1);
    }

    /// Add a message: `key`, its Message-ID's hash and size, and what search reads of it. Its
    /// doc's id; an index already holding the message (or another at its folder and uid, under
    /// an older validity) drops the doc it had. A text longer than a text page keeps is cut
    /// (the doc marked truncated). An error when the index has no room left (ids, folders): it
    /// must be made again; or when the folder's name is too long to keep (not added).
    pub fn add(&mut self, key: &MessageKey, msgid_hash: u64, size: u32, doc: &IndexDoc) -> Result<u32> {
        let folder = self.folder(&key.folder)?;
        let id = u32::try_from(self.docs.len()).ok().filter(|id| *id < u32::MAX - 1).ok_or_else(|| damaged(&self.base, "no doc ids left"))?;
        if let Some(old) = self.holder(key) {
            self.remove(old);
        }
        let (blob, cut) = encode_blob(doc);
        let bits = if doc.has_text { HAS_TEXT } else { 0 } | if doc.truncated || cut { TRUNCATED } else { 0 };
        let files = u16::try_from(doc.files).unwrap_or(u16::MAX);
        self.docs.push(DocRec { folder, validity: key.validity, uid: key.uid, text: NO_TEXT, files, bits, msgid_hash, size });
        self.by_key.insert((folder, key.uid), id);
        self.word_bytes = self.word_bytes.saturating_add(add_words(&mut self.words, id, format::kept_text(&doc.text), &doc.names));
        self.word_docs = self.word_docs.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(blob.len());
        self.texts.push((id, blob));
        self.live = self.live.saturating_add(1);
        self.touch(id);
        Ok(id)
    }

    /// Take a doc out: it no longer matches, and its text leaves the index with the next flush.
    /// Its words go when its segment is next merged.
    pub fn remove(&mut self, doc: u32) {
        let Some(d) = self.docs.get_mut(doc as usize).filter(|d| d.live()) else { return };
        let (folder, uid, text) = (d.folder, d.uid, d.text);
        d.folder = DEAD;
        d.text = NO_TEXT;
        if self.by_key.get(&(folder, uid)) == Some(&doc) {
            self.by_key.remove(&(folder, uid));
        }
        if text == NO_TEXT {
            if let Some(i) = self.texts.iter().position(|(t, _)| *t == doc) {
                let (_, blob) = self.texts.remove(i);
                self.text_bytes = self.text_bytes.saturating_sub(blob.len());
            }
        } else {
            let page = self.meta.text_pages.get(text as usize).map_or(0, |t| t.bytes as usize);
            let drop = self.drop_texts.entry(text).or_default();
            if drop.is_empty() {
                self.drop_bytes = self.drop_bytes.saturating_add(page);
            }
            drop.insert(doc);
        }
        self.live = self.live.saturating_sub(1);
        self.dead = self.dead.saturating_add(1);
        self.touch(doc);
    }

    /// A doc's message moved to `key` (its text and words are the same).
    pub fn retarget(&mut self, doc: u32, key: &MessageKey) -> Result<()> {
        if self.docs.get(doc as usize).is_none_or(|d| !d.live()) {
            return Ok(());
        }
        let folder = self.folder(&key.folder)?;
        if let Some(other) = self.holder(key).filter(|o| *o != doc) {
            self.remove(other);
        }
        let Some(d) = self.docs.get_mut(doc as usize) else { return Ok(()) };
        let old = (d.folder, d.uid);
        (d.folder, d.validity, d.uid) = (folder, key.validity, key.uid);
        if self.by_key.get(&old) == Some(&doc) {
            self.by_key.remove(&old);
        }
        self.by_key.insert((folder, key.uid), doc);
        self.touch(doc);
        Ok(())
    }

    /// Prepare and commit what changed: the new generation, or `None` when nothing had.
    pub fn flush(&mut self, store: &Store) -> Result<Option<u64>> {
        let f = self.prepare(store)?;
        self.commit(store, f)
    }

    /// Whether a flush has anything to write (a rebuild left to finish is the commit's).
    fn has_changes(&self) -> bool {
        !self.dirty.is_empty() || !self.words.is_empty() || !self.texts.is_empty() || !self.drop_texts.is_empty()
    }

    fn file(&self, name: Name, bytes: Vec<u8>) -> (PathBuf, Prepared) {
        (name.path(&self.base), Store::prepare(self.packs, bytes))
    }

    /// The next generation's files, encoded and compressed, nothing written: the text pages that
    /// lose a text or gain some (read back for it), the changed doc pages, a segment of the words
    /// added, and the meta naming them.
    pub fn prepare(&self, store: &Store) -> Result<Flush> {
        let mut meta = Meta { gone: Vec::new(), ..self.meta.clone() };
        if !self.has_changes() {
            return Ok(Flush { stamp: self.stamp, meta, files: Vec::new(), placed: Vec::new() });
        }
        meta.generation = meta.generation.checked_add(1).ok_or_else(|| damaged(&self.base, "no generations left"))?;
        let mut files = Vec::new();
        let placed = self.prepare_texts(store, &mut meta, &mut files)?;
        let text_of: HashMap<u32, u32> = placed.iter().copied().collect();
        // The doc pages that changed.
        meta.next_doc = u32::try_from(self.docs.len()).map_err(|_| damaged(&self.base, "no doc ids left"))?;
        for &p in &self.dirty {
            let start = p.saturating_mul(DOCS_PER_PAGE as usize);
            let Some(page) = self.docs.get(start..(start + DOCS_PER_PAGE as usize).min(self.docs.len())) else { continue };
            let recs: Vec<DocRec> = page
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let doc = u32::try_from(start + i).unwrap_or(u32::MAX);
                    DocRec { text: text_of.get(&doc).copied().unwrap_or(d.text), ..*d }
                })
                .collect();
            let page_no = u32::try_from(p).unwrap_or(u32::MAX);
            if let Some(&g) = self.meta.doc_pages.get(p) {
                meta.gone.push(Name::Docs { generation: g, page: page_no });
            }
            while meta.doc_pages.len() <= p {
                meta.doc_pages.push(meta.generation);
            }
            if let Some(g) = meta.doc_pages.get_mut(p) {
                *g = meta.generation;
            }
            files.push(self.file(Name::Docs { generation: meta.generation, page: page_no }, encode_docs(&recs)));
        }
        // The words added: a segment. With as many as a meta names, none: the words are made
        // again from the texts as the meta is in (marked to be, should that not finish).
        #[cfg(not(test))]
        let max_segments = MAX_SEGMENTS;
        #[cfg(test)]
        let max_segments = self.max_segments.min(MAX_SEGMENTS);
        if self.meta.segments.len() >= max_segments {
            meta.tokenizer = 0;
        } else if !self.words.is_empty() {
            let mut b = SegBuilder::default();
            for (k, docs) in &self.words {
                b.push(k, docs);
            }
            if !b.is_empty() {
                let id = meta.next_seg;
                meta.next_seg = id.checked_add(1).ok_or_else(|| damaged(&self.base, "no segment ids left"))?;
                let seg = b.finish();
                meta.segments.push(self.seg_info(id, self.word_docs, &seg)?);
                self.seg_files(id, seg, &mut files);
            }
        }
        meta.live = self.live;
        meta.dead = self.dead;
        meta.folders.clone_from(&self.folders);
        Ok(Flush { stamp: self.stamp, meta, files, placed })
    }

    fn seg_info(&self, id: u32, docs: u32, seg: &SegFiles) -> Result<SegInfo> {
        let pages = |n: usize| u32::try_from(n).map_err(|_| damaged(&self.base, "segment too big"));
        Ok(SegInfo { id, docs, terms: seg.terms, lex_pages: pages(seg.lex.len())?, post_pages: pages(seg.post.len())?, bytes: seg.bytes() })
    }

    fn seg_files(&self, id: u32, seg: SegFiles, files: &mut Vec<(PathBuf, Prepared)>) {
        files.push(self.file(Name::SegHead(id), seg.head));
        for (n, page) in seg.lex.into_iter().enumerate() {
            files.push(self.file(Name::SegLex(id, u32::try_from(n).unwrap_or(u32::MAX)), page));
        }
        for (n, page) in seg.post.into_iter().enumerate() {
            files.push(self.file(Name::SegPost(id, u32::try_from(n).unwrap_or(u32::MAX)), page));
        }
    }

    /// The text pages: those losing a text written again without it (or emptied), the new texts
    /// added to the last page while it has room and then to new pages. Where each new text went.
    fn prepare_texts(&self, store: &Store, meta: &mut Meta, files: &mut Vec<(PathBuf, Prepared)>) -> Result<Vec<(u32, u32)>> {
        let mut pages: BTreeMap<u32, Vec<(u32, Vec<u8>)>> = BTreeMap::new();
        for (&p, drop) in &self.drop_texts {
            let mut recs = self.read_text_page(store, p)?;
            recs.retain(|(d, _)| !drop.contains(d));
            pages.insert(p, recs);
        }
        let mut placed = Vec::with_capacity(self.texts.len());
        if !self.texts.is_empty() {
            // The last page, while it has room.
            let last = meta.text_pages.len().checked_sub(1).and_then(|p| u32::try_from(p).ok());
            let mut at = match last {
                Some(p) if meta.text_pages.get(p as usize).is_some_and(|t| t.generation != 0 && (t.bytes as usize) < TEXT_PAGE) => {
                    if let std::collections::btree_map::Entry::Vacant(v) = pages.entry(p) {
                        v.insert(self.read_text_page(store, p)?);
                    }
                    p
                }
                _ => u32::try_from(meta.text_pages.len()).map_err(|_| damaged(&self.base, "too many text pages"))?,
            };
            let mut size = pages.get(&at).map_or(0, |r| r.iter().map(|(_, b)| b.len() + 8).sum::<usize>());
            let mut count = pages.get(&at).map_or(0, Vec::len);
            for (doc, blob) in &self.texts {
                // A page this text would take past [`TEXT_PAGE`] is ended: a longer text has a
                // page of its own.
                let over = size > 0 && size.saturating_add(blob.len() + 8) > TEXT_PAGE;
                if over || count >= usize::from(u16::MAX) {
                    at = at.checked_add(1).ok_or_else(|| damaged(&self.base, "too many text pages"))?;
                    (size, count) = (0, 0);
                }
                size += blob.len() + 8;
                count += 1;
                pages.entry(at).or_default().push((*doc, blob.clone()));
                placed.push((*doc, at));
            }
        }
        for (p, recs) in pages {
            let i = p as usize;
            if let Some(old) = meta.text_pages.get(i).filter(|t| t.generation != 0) {
                meta.gone.push(Name::Text { generation: old.generation, page: p });
            }
            while meta.text_pages.len() <= i {
                meta.text_pages.push(TextPage::default());
            }
            let Some(slot) = meta.text_pages.get_mut(i) else { continue };
            if recs.is_empty() {
                *slot = TextPage::default();
                continue;
            }
            let bytes = encode_text_page(recs.iter().map(|(d, b)| (*d, b.as_slice())));
            *slot = TextPage { generation: meta.generation, bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX) };
            files.push(self.file(Name::Text { generation: meta.generation, page: p }, bytes));
        }
        Ok(placed)
    }

    /// A committed text page's texts.
    fn read_text_page(&self, store: &Store, p: u32) -> Result<Vec<(u32, Vec<u8>)>> {
        let Some(t) = self.meta.text_pages.get(p as usize).filter(|t| t.generation != 0) else { return Ok(Vec::new()) };
        let path = Name::Text { generation: t.generation, page: p }.path(&self.base);
        // The writer is held: a page the meta names and isn't there is damage.
        let bytes = need_file(store, &path).map_err(|e| if is_gone(&e) { damaged(&path, "a text page is missing") } else { e })?;
        let recs = format::decode_text_page(&bytes, self.meta.next_doc).map_err(|what| damaged(&path, what))?;
        Ok(recs.into_iter().map(|(d, b)| (d, b.to_vec())).collect())
    }

    /// Write a prepared flush: its files in short transactions, then its meta with the files it
    /// no longer uses removed, in one sealed transaction. The new generation, or `None` when the
    /// flush had nothing. A flush prepared before a later change is refused (prepare it again);
    /// one that fails before its meta leaves the generation before it, its files removed by the
    /// next commit. Past [`MAX_SEGMENTS`] the words are then made again from the texts: an error
    /// there that isn't damage is left for the next flush to finish (`Ok`, the generation in).
    pub fn commit(&mut self, store: &Store, flush: Flush) -> Result<Option<u64>> {
        if flush.stamp != self.stamp {
            return Err(io_err(&self.base)(other("the index changed after this flush was made")));
        }
        if flush.is_empty() {
            // Words a commit before began to make again and didn't finish.
            if self.meta.tokenizer != TOKENIZER_VERSION {
                if self.orphans {
                    self.clean(store);
                }
                self.rebuild(store)?;
                return Ok(Some(self.meta.generation));
            }
            return Ok(None);
        }
        if self.orphans {
            self.clean(store);
        }
        self.orphans = true;
        self.write_batches(store, flush.files)?;
        #[cfg(test)]
        if self.stop_before_meta {
            return Err(io_err(&self.base)(other("stopped before the meta")));
        }
        // What the meta now in no longer used, should it not all have gone with it, too.
        let gone: Vec<Name> = self.meta.gone.iter().chain(&flush.meta.gone).copied().collect();
        self.write_meta(store, &flush.meta, &gone)?;
        self.orphans = false;
        let generation = flush.meta.generation;
        for (doc, page) in flush.placed {
            if let Some(d) = self.docs.get_mut(doc as usize) {
                d.text = page;
            }
        }
        self.meta = flush.meta;
        self.words.clear();
        (self.word_bytes, self.word_docs) = (0, 0);
        self.texts.clear();
        self.text_bytes = 0;
        self.drop_texts.clear();
        self.drop_bytes = 0;
        self.dirty.clear();
        self.stamp = self.stamp.wrapping_add(1);
        // Segments up to what a meta names: their words made into few again. The generation is
        // in whatever comes of it: one that doesn't finish is tried again by the next flush (the
        // reader has no index meanwhile), and an index it finds damaged is the error.
        if self.meta.tokenizer != TOKENIZER_VERSION
            && let Err(e) = self.rebuild(store)
        {
            if is_damaged(&e) {
                return Err(e);
            }
            log::warn!("{}'s search index: its words not made again yet: {e}", self.account);
        }
        Ok(Some(generation))
    }

    /// Files written in transactions of at most [`BACKGROUND_COMMIT`] kept bytes, none waiting
    /// for the disk (the meta's does, for them all).
    fn write_batches(&self, store: &Store, files: Vec<(PathBuf, Prepared)>) -> Result<()> {
        let mut batch = Vec::new();
        let mut bytes = 0usize;
        for (p, prepared) in files {
            if !batch.is_empty() && bytes.saturating_add(prepared.kept_len()) > BACKGROUND_COMMIT {
                store.files.write_prepared(std::mem::take(&mut batch), false)?;
                bytes = 0;
            }
            bytes = bytes.saturating_add(prepared.kept_len());
            batch.push((p, prepared));
        }
        if !batch.is_empty() {
            store.files.write_prepared(batch, false)?;
        }
        Ok(())
    }

    /// The meta, the commit point, and the files `gone` (those already gone are no matter): one
    /// transaction, durable and sealed, so a crash after it costs the data file no repair. A meta
    /// that wouldn't read back is never written: the index is to be made again.
    fn write_meta(&self, store: &Store, meta: &Meta, gone: &[Name]) -> Result<()> {
        let path = self.base.join("meta.bin");
        let bytes = encode_meta(meta);
        if bytes.len() > MAX_META || format::decode_meta(&bytes).is_err() {
            return Err(damaged(&path, "a meta it couldn't read back"));
        }
        let gone: Vec<PathBuf> = gone.iter().map(|n| n.path(&self.base)).collect();
        store.files.replace(&self.base, Some((&path, &bytes)), &gone, true)
    }

    /// Remove files (those already gone are no matter), in one transaction not waited for.
    fn remove_files(&self, store: &Store, names: &[Name]) {
        if names.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = names.iter().map(|n| n.path(&self.base)).collect();
        if let Err(e) = store.files.replace(&self.base, None, &paths, false) {
            log::warn!("{}'s search index: files not removed: {e}", self.account);
        }
    }

    /// Remove the index's files the meta doesn't use: those of a commit that didn't finish, and
    /// those a finished one left to remove. Only names the index gives its files are touched.
    fn clean(&mut self, store: &Store) {
        let uses = self.meta.uses();
        let mut extra = Vec::new();
        for sub in SUBDIRS {
            for (name, _) in store.files.list(&self.base.join(sub)) {
                if let Some(n) = Name::parse(sub, &name).filter(|n| !uses.contains(n)) {
                    extra.push(n);
                }
            }
        }
        self.remove_files(store, &extra);
        self.orphans = false;
    }

    /// The words made again from the texts kept: they were read with another tokenizer. Written
    /// as new segments in short batches as they fill, and switched to with one meta.
    fn rebuild(&mut self, store: &Store) -> Result<()> {
        log::info!("{}'s search index: reading its words again (tokenizer {} → {TOKENIZER_VERSION})", self.account, self.meta.tokenizer);
        let mut meta = Meta { tokenizer: TOKENIZER_VERSION, segments: Vec::new(), gone: Vec::new(), ..self.meta.clone() };
        meta.generation = meta.generation.checked_add(1).ok_or_else(|| damaged(&self.base, "no generations left"))?;
        for s in &self.meta.segments {
            meta.gone.extend(s.names());
        }
        self.orphans = true;
        let mut words = BTreeMap::new();
        let (mut bytes, mut docs) = (0usize, 0u32);
        for p in 0..self.meta.text_pages.len() {
            let p = u32::try_from(p).unwrap_or(u32::MAX);
            for (doc, blob) in self.read_text_page(store, p)? {
                if self.docs.get(doc as usize).is_none_or(|d| !d.live() || d.text != p) {
                    continue;
                }
                let path = Name::Text { generation: self.meta.text_pages.get(p as usize).map_or(0, |t| t.generation), page: p }.path(&self.base);
                let blob = format::decode_blob(&blob).map_err(|what| damaged(&path, what))?;
                bytes = bytes.saturating_add(add_words(&mut words, doc, &blob.text, &blob.names));
                docs = docs.saturating_add(1);
            }
            if bytes >= IN_FLIGHT_BYTES / 2 {
                self.write_segment(store, &mut meta, std::mem::take(&mut words), docs)?;
                (bytes, docs) = (0, 0);
            }
        }
        self.write_segment(store, &mut meta, words, docs)?;
        let gone: Vec<Name> = self.meta.gone.iter().chain(&meta.gone).copied().collect();
        self.write_meta(store, &meta, &gone)?;
        self.orphans = false;
        self.meta = meta;
        Ok(())
    }

    fn write_segment(&self, store: &Store, meta: &mut Meta, words: BTreeMap<Vec<u8>, Vec<u32>>, docs: u32) -> Result<()> {
        let mut b = SegBuilder::default();
        for (k, list) in &words {
            b.push(k, list);
        }
        if b.is_empty() {
            return Ok(());
        }
        if meta.segments.len() >= MAX_SEGMENTS {
            return Err(damaged(&self.base, "more words than its segments hold"));
        }
        let id = meta.next_seg;
        meta.next_seg = id.checked_add(1).ok_or_else(|| damaged(&self.base, "no segment ids left"))?;
        let seg = b.finish();
        meta.segments.push(self.seg_info(id, docs, &seg)?);
        let mut files = Vec::new();
        self.seg_files(id, seg, &mut files);
        self.write_batches(store, files)
    }
}

/// An account's index as committed: its meta and doc table; `None` when it has none.
fn load(store: &Store, account: &str, base: &Path) -> Result<Option<(Meta, Vec<DocRec>)>> {
    let Some(meta) = load_meta(store, account)? else { return Ok(None) };
    let docs = match load_docs(store, base, &meta) {
        Ok(d) => d,
        // The writer is held: a page the meta names and isn't there is damage.
        Err(e) if is_gone(&e) => return Err(damaged(base, "a doc page is missing")),
        Err(e) => return Err(e),
    };
    Ok(Some((meta, docs)))
}

/// A new index's meta: past the generation and segments of one set aside (its meta's, when it
/// reads, and its files' names still there), so nothing of that one is taken for this one's.
/// Numbers too near the end to go on from start again (the new epoch keeps them apart).
fn fresh(store: &Store, account: &str) -> Meta {
    let base = dir(store, account);
    let bad = meta_path(store, account).with_extension("bin.bad");
    let old = read_file(store, &bad).ok().flatten().and_then(|b| format::decode_meta(&b).ok()).unwrap_or_default();
    let (mut generation, mut next_seg) = (old.generation, old.next_seg);
    for sub in SUBDIRS {
        for (name, _) in store.files.list(&base.join(sub)) {
            match Name::parse(sub, &name) {
                Some(Name::Docs { generation: g, .. } | Name::Text { generation: g, .. }) => generation = generation.max(g),
                Some(Name::SegHead(id) | Name::SegLex(id, _) | Name::SegPost(id, _)) => next_seg = next_seg.max(id.saturating_add(1)),
                None => {}
            }
        }
    }
    if generation > u64::MAX / 2 {
        generation = 0;
    }
    if next_seg > u32::MAX / 2 {
        next_seg = 0;
    }
    Meta { tokenizer: TOKENIZER_VERSION, generation, next_seg, epoch: new_epoch(old.epoch), ..Meta::default() }
}
