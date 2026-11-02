//! Writing a generation: a flush's files made ready, then written in short transactions and
//! sealed by its meta; the files no generation uses removed; words made again from the texts.

use super::*;

impl Writer {
    /// Prepare and commit what changed: the new generation, or `None` when nothing had.
    pub fn flush(&mut self, store: &Store) -> Result<Option<u64>> {
        let f = self.prepare(store)?;
        self.commit(store, f)
    }

    /// Whether a flush has anything to write (a rebuild left to finish is the commit's).
    pub(super) fn has_changes(&self) -> bool {
        !self.dirty.is_empty() || !self.words.is_empty() || !self.texts.is_empty() || !self.drop_texts.is_empty() || self.marks_changed
    }

    pub(super) fn file(&self, name: Name, bytes: Vec<u8>) -> (PathBuf, Prepared) {
        (name.path(&self.base), Store::prepare(self.packs, bytes))
    }

    /// The next generation's files, encoded and compressed, nothing written: the text pages that
    /// lose a text or gain some (read back for it), the changed doc pages, a segment of the words
    /// added, and the meta naming them.
    pub fn prepare(&self, store: &Store) -> Result<Flush> {
        let mut meta = Meta { gone: Vec::new(), ..self.meta.clone() };
        if !self.has_changes() {
            return Ok(Flush { stamp: self.stamp, nothing: true, meta, files: Vec::new(), placed: Vec::new() });
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
        meta.marks = self.marks.iter().map(|(f, r)| (*f, *r)).collect();
        Ok(Flush { stamp: self.stamp, nothing: false, meta, files, placed })
    }

    pub(super) fn seg_info(&self, id: u32, docs: u32, seg: &SegFiles) -> Result<SegInfo> {
        let pages = |n: usize| u32::try_from(n).map_err(|_| damaged(&self.base, "segment too big"));
        Ok(SegInfo {
            id,
            docs,
            terms: seg.terms,
            lex_pages: pages(seg.lex_pages())?,
            post_pages: pages(seg.post_pages())?,
            bytes: seg.bytes(),
            lo: seg.lo,
            hi: seg.hi,
        })
    }

    /// A segment's files: its head, and the pages it still holds.
    pub(super) fn seg_files(&self, id: u32, seg: SegFiles, files: &mut Vec<(PathBuf, Prepared)>) {
        files.push(self.file(Name::SegHead(id), seg.head));
        for (n, page) in (seg.lex_from..).zip(seg.lex) {
            files.push(self.file(Name::SegLex(id, n), page));
        }
        for (n, page) in (seg.post_from..).zip(seg.post) {
            files.push(self.file(Name::SegPost(id, n), page));
        }
    }

    /// The text pages: those losing a text written again without it (or emptied), the new texts
    /// added to the last page while it has room and then to new pages. Where each new text went.
    pub(super) fn prepare_texts(&self, store: &Store, meta: &mut Meta, files: &mut Vec<(PathBuf, Prepared)>) -> Result<Vec<(u32, u32)>> {
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
    pub(super) fn read_text_page(&self, store: &Store, p: u32) -> Result<Vec<(u32, Vec<u8>)>> {
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
        self.marks_changed = false;
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
    /// for the disk (the meta's does, for them all), each after the read marks waiting
    /// ([`Store::give_way_to_marks`]), as the meta and a removal are.
    pub(super) fn write_batches(&self, store: &Store, files: Vec<(PathBuf, Prepared)>) -> Result<()> {
        let mut batch = Vec::new();
        let mut bytes = 0usize;
        for (p, prepared) in files {
            if !batch.is_empty() && bytes.saturating_add(prepared.kept_len()) > BACKGROUND_COMMIT {
                let b = std::mem::take(&mut batch);
                store.give_way_to_marks(&self.account);
                self.held.writing(|| store.files.write_prepared(b, false))?;
                bytes = 0;
            }
            bytes = bytes.saturating_add(prepared.kept_len());
            batch.push((p, prepared));
        }
        if !batch.is_empty() {
            store.give_way_to_marks(&self.account);
            self.held.writing(|| store.files.write_prepared(batch, false))?;
        }
        Ok(())
    }

    /// The meta, the commit point, and the files `gone` (those already gone are no matter): one
    /// transaction, durable and sealed, so a crash after it costs the data file no repair. A meta
    /// that wouldn't read back is never written: the index is to be made again.
    pub(super) fn write_meta(&self, store: &Store, meta: &Meta, gone: &[Name]) -> Result<()> {
        let path = self.base.join("meta.bin");
        let bytes = encode_meta(meta);
        if bytes.len() > MAX_META || format::decode_meta(&bytes).is_err() {
            return Err(damaged(&path, "a meta it couldn't read back"));
        }
        let gone: Vec<PathBuf> = gone.iter().map(|n| n.path(&self.base)).collect();
        store.give_way_to_marks(&self.account);
        self.held.writing(|| store.files.replace(&self.base, Some((&path, &bytes)), &gone, true))
    }

    /// Remove files (those already gone are no matter), in one transaction not waited for.
    pub(super) fn remove_files(&self, store: &Store, names: &[Name]) {
        if names.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = names.iter().map(|n| n.path(&self.base)).collect();
        store.give_way_to_marks(&self.account);
        if let Err(e) = self.held.writing(|| store.files.replace(&self.base, None, &paths, false)) {
            log::warn!("{}'s search index: files not removed: {e}", self.account);
        }
    }

    /// Remove the index's files the meta doesn't use: those of a commit that didn't finish, and
    /// those a finished one left to remove. Only names the index gives its files are touched.
    pub(super) fn clean(&mut self, store: &Store) {
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
    pub(super) fn rebuild(&mut self, store: &Store) -> Result<()> {
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
            if bytes >= self.in_flight / 2 {
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

    pub(super) fn write_segment(&self, store: &Store, meta: &mut Meta, words: BTreeMap<Vec<u8>, Vec<u32>>, docs: u32) -> Result<()> {
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
