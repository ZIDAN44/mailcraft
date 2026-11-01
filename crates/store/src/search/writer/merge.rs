//! Keeping an account's index small as it changes ([`Writer::merge_step`]): segments merged into
//! fewer, the words of removed docs dropped as they are, and doc ids given again once most of
//! them are removed docs'.
//!
//! A merge reads its segments a lexicon page at a time, in word order, and writes the merged
//! segment's pages as they fill, in transactions nothing reads yet; one meta then switches to it
//! and removes the segments it replaces. It holds a lexicon page and a postings page of each
//! segment it merges, the word being merged and the pages not yet written: k × ([`LEX_PAGE`] +
//! [`POST_PAGE`]) and the step's budget, and the longest list of docs. Giving the
//! ids again makes the index anew from the texts it keeps (no message is read), under names its
//! meta doesn't use, and switches to it the same way.
//!
//! A merge is due, in this order: for ids, past twice the live docs and [`RENUMBER_SLACK`]; for the
//! segments holding words of a doc removed over [`TOMBSTONE_AGE`] ago (a removed message's words
//! leave the file within a day of it); for [`TIER_MERGE`] segments of a size (by powers of 4
//! docs); and past [`MAX_LIVE_SEGMENTS`] segments.

use mailcraft_model::search::TOKENIZER_VERSION;

use super::super::format::{LexEntry, LexStream, Postings, SegPage, decode_blob, decode_post_page, decode_postings};
use super::*;

/// How long (seconds) the words of a removed doc may stay in a segment.
pub const TOMBSTONE_AGE: u64 = 24 * 60 * 60;
/// Doc ids are given again once there are more than twice the live docs and this many.
pub const RENUMBER_SLACK: u32 = 4096;
/// Segments of one size merged into one once there are this many.
const TIER_MERGE: usize = 8;
/// Past this many segments the smallest are merged, whatever their sizes.
const MAX_LIVE_SEGMENTS: usize = 24;
/// Most segments merged at once.
const MAX_MERGE: usize = 16;

/// What's due.
#[derive(Debug, PartialEq, Eq)]
enum Due {
    Renumber,
    /// These segments (places in the meta's list) merged into one.
    Merge(Vec<usize>),
}

/// A segment's size, by powers of 4 docs.
fn tier(docs: u32) -> u32 {
    (31 - docs.max(1).leading_zeros()) / 2
}

/// The `n` smallest of these segments, in the meta's order.
fn smallest(segs: &[SegInfo], mut of: Vec<usize>, n: usize) -> Vec<usize> {
    of.sort_by_key(|&i| segs.get(i).map(|s| (s.bytes, s.id)));
    of.truncate(n);
    of.sort_unstable();
    of
}

/// The docs the segments' words may name, as spans rising and apart.
fn spans(segs: &[SegInfo]) -> Vec<(u32, u32)> {
    let mut all: Vec<(u32, u32)> = segs.iter().map(|s| (s.lo, s.hi)).collect();
    all.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(all.len());
    for (lo, hi) in all {
        match out.last_mut() {
            Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

fn within(spans: &[(u32, u32)], doc: u32) -> bool {
    let i = spans.partition_point(|s| s.1 < doc);
    spans.get(i).is_some_and(|s| s.0 <= doc)
}

/// Two lists of docs, rising, as one.
fn union(a: Vec<u32>, b: Vec<u32>) -> Vec<u32> {
    if a.is_empty() {
        return b;
    }
    let mut out = Vec::with_capacity(a.len().saturating_add(b.len()));
    let (mut i, mut j) = (a.into_iter().peekable(), b.into_iter().peekable());
    loop {
        let next = match (i.peek(), j.peek()) {
            (Some(x), Some(y)) if x < y => i.next(),
            (Some(x), Some(y)) if x > y => j.next(),
            (Some(_), Some(_)) => {
                j.next();
                i.next()
            }
            (Some(_), None) => i.next(),
            (None, Some(_)) => j.next(),
            (None, None) => break,
        };
        out.extend(next);
    }
    out
}

/// A segment being merged, read a word at a time.
struct Input {
    info: SegInfo,
    /// The next lexicon page to read.
    next_page: u32,
    page: Option<LexStream>,
    /// Its word now.
    word: Option<LexEntry>,
    /// The postings page last read.
    post: Option<(u32, Vec<u8>)>,
}

impl Input {
    fn new(info: SegInfo) -> Input {
        Input { info, next_page: 0, page: None, word: None, post: None }
    }

    /// What it holds in memory.
    fn held(&self) -> usize {
        let word = self.word.as_ref().map_or(0, |e| e.key.len() + if let Postings::Inline(d) = &e.at { d.len() * 4 } else { 0 });
        let page = self.page.as_ref().map_or(0, LexStream::held);
        page.saturating_add(word).saturating_add(self.post.as_ref().map_or(0, |(_, p)| p.len()))
    }

    /// Its word now, the next lexicon page read when the one held is done; `None` at its end.
    fn current(&mut self, store: &Store, base: &Path, next_doc: u32) -> Result<Option<&LexEntry>> {
        while self.word.is_none() {
            if let Some(page) = self.page.as_mut() {
                let path = Name::SegLex(self.info.id, self.next_page.saturating_sub(1)).path(base);
                self.word = page.next_entry().map_err(|what| damaged(&path, what))?;
                if self.word.is_none() {
                    self.page = None;
                }
                continue;
            }
            if self.next_page >= self.info.lex_pages {
                self.post = None;
                break;
            }
            let path = Name::SegLex(self.info.id, self.next_page).path(base);
            let bytes = need_file(store, &path).map_err(|e| missing(&path, e))?;
            self.page = Some(LexStream::new(bytes, next_doc, self.info.post_pages).map_err(|what| damaged(&path, what))?);
            self.next_page += 1;
        }
        Ok(self.word.as_ref())
    }

    /// Its word's docs.
    fn docs(&mut self, store: &Store, base: &Path, e: &LexEntry, next_doc: u32) -> Result<Vec<u32>> {
        let (page, offset, len) = match &e.at {
            Postings::Inline(docs) => return Ok(docs.clone()),
            &Postings::At { page, offset, len } => (page, offset as usize, len as usize),
        };
        let mut bytes: Vec<u8> = Vec::with_capacity(len.min(16 << 20));
        let (mut p, mut from) = (page, offset);
        let first = Name::SegPost(self.info.id, page).path(base);
        while bytes.len() < len {
            let path = Name::SegPost(self.info.id, p).path(base);
            if p >= self.info.post_pages {
                return Err(damaged(&path, "postings past the segment"));
            }
            if self.post.as_ref().map(|(n, _)| *n) != Some(p) {
                self.post = None;
                self.post = Some((p, need_file(store, &path).map_err(|e| missing(&path, e))?));
            }
            let raw = self.post.as_ref().map(|(_, r)| r.as_slice()).unwrap_or_default();
            let data = decode_post_page(raw).map_err(|what| damaged(&path, what))?;
            let rest = data.get(from..).ok_or_else(|| damaged(&path, "postings past the page"))?;
            let want = (len - bytes.len()).min(rest.len());
            if want == 0 {
                return Err(damaged(&path, "postings past the page"));
            }
            bytes.extend_from_slice(rest.get(..want).unwrap_or_default());
            (p, from) = (p.saturating_add(1), 0);
        }
        decode_postings(&bytes, e.df, next_doc).map_err(|what| damaged(&first, what))
    }
}

/// The writer is held: a file the meta names and isn't there is damage.
fn missing(path: &Path, e: StoreError) -> StoreError {
    if is_gone(&e) { damaged(path, "a file the meta names is missing") } else { e }
}

impl Writer {
    /// Merge once, when a merge is due (the changes held are flushed first). `budget`: the most
    /// bytes of pages written in a transaction, and held before they are; `now`: Unix seconds,
    /// for how long a removed doc's words have stayed. Whether another merge is due: call it
    /// again (each is one step: a call can be stopped between them).
    pub fn merge_step(&mut self, store: &Store, budget: usize, now: u64) -> Result<bool> {
        self.flush(store)?;
        if self.meta.tokenizer != TOKENIZER_VERSION {
            // The words aren't all made again yet: the next flush does it.
            return Ok(false);
        }
        let budget = budget.clamp(LEX_PAGE, BACKGROUND_COMMIT);
        match self.due(now) {
            Some(Due::Renumber) => self.renumber(store, budget)?,
            Some(Due::Merge(segs)) => self.merge(store, &segs, budget, now)?,
            None => {
                let since = self.dead_since(&self.meta.segments, &self.tombstones(), now);
                if since != self.meta.dead_since {
                    self.write_since(store, since)?;
                }
                return Ok(false);
            }
        }
        Ok(self.due(now).is_some())
    }

    #[cfg(not(test))]
    fn renumber_slack(&self) -> u32 {
        RENUMBER_SLACK
    }

    #[cfg(test)]
    fn renumber_slack(&self) -> u32 {
        self.renumber_slack
    }

    /// The removed docs whose words may still be in a segment, rising.
    fn tombstones(&self) -> Vec<u32> {
        let dead = self.docs.iter().enumerate().filter(|(_, d)| !d.live() && d.bits & PURGED == 0);
        dead.map(|(i, _)| u32::try_from(i).unwrap_or(u32::MAX)).collect()
    }

    /// Since when removed docs' words have been in these segments: 0 when none are.
    fn dead_since(&self, segs: &[SegInfo], dead: &[u32], now: u64) -> u64 {
        let spans = spans(segs);
        if !dead.iter().any(|&d| within(&spans, d)) {
            0
        } else if self.meta.dead_since != 0 {
            self.meta.dead_since
        } else {
            now.max(1)
        }
    }

    fn due(&self, now: u64) -> Option<Due> {
        let (next, live) = (self.docs.len() as u64, u64::from(self.live));
        if next > live.saturating_mul(2).saturating_add(u64::from(self.renumber_slack())) {
            return Some(Due::Renumber);
        }
        let segs = &self.meta.segments;
        let dead = self.tombstones();
        let since = self.meta.dead_since;
        if since != 0 && now.saturating_sub(since) >= TOMBSTONE_AGE {
            let holding: Vec<usize> = segs.iter().enumerate().filter(|(_, s)| holds(&dead, s)).map(|(i, _)| i).collect();
            if !holding.is_empty() {
                return Some(Due::Merge(smallest(segs, holding, MAX_MERGE)));
            }
        }
        let mut tiers: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, s) in segs.iter().enumerate() {
            tiers.entry(tier(s.docs)).or_default().push(i);
        }
        if let Some(full) = tiers.into_values().find(|t| t.len() >= TIER_MERGE) {
            return Some(Due::Merge(smallest(segs, full, MAX_MERGE)));
        }
        if segs.len() > MAX_LIVE_SEGMENTS {
            return Some(Due::Merge(smallest(segs, (0..segs.len()).collect(), TIER_MERGE)));
        }
        None
    }

    /// The meta again, a removed doc's words first seen in a segment (or none left) since `since`.
    fn write_since(&mut self, store: &Store, since: u64) -> Result<()> {
        let mut meta = Meta { gone: Vec::new(), dead_since: since, ..self.meta.clone() };
        meta.generation = meta.generation.checked_add(1).ok_or_else(|| damaged(&self.base, "no generations left"))?;
        self.write_meta(store, &meta, &self.meta.gone)?;
        self.meta = meta;
        self.stamp = self.stamp.wrapping_add(1);
        Ok(())
    }

    /// Merge these segments into one, the words of removed docs left out.
    fn merge(&mut self, store: &Store, segs: &[usize], budget: usize, now: u64) -> Result<()> {
        let next_doc = self.meta.next_doc;
        let base = self.base.clone();
        let mut inputs: Vec<Input> = segs.iter().filter_map(|&i| self.meta.segments.get(i).copied()).map(Input::new).collect();
        let id = self.meta.next_seg;
        let mut meta = Meta { gone: Vec::new(), ..self.meta.clone() };
        meta.generation = meta.generation.checked_add(1).ok_or_else(|| damaged(&base, "no generations left"))?;
        meta.next_seg = id.checked_add(1).ok_or_else(|| damaged(&base, "no segment ids left"))?;
        self.orphans = true;
        let mut b = SegBuilder::default();
        let mut seen = Bits::new(next_doc);
        let mut last: Option<Vec<u8>> = None;
        let mut peak = 0usize;
        loop {
            // The lowest word now, and its docs in every segment that has it.
            let mut key: Option<Vec<u8>> = None;
            for inp in &mut inputs {
                if let Some(e) = inp.current(store, &base, next_doc)?
                    && key.as_ref().is_none_or(|k| e.key < *k)
                {
                    key = Some(e.key.clone());
                }
            }
            let Some(key) = key else { break };
            if last.as_ref().is_some_and(|l| key <= *l) {
                return Err(damaged(&base, "a segment's words out of order"));
            }
            let mut docs = Vec::new();
            for inp in &mut inputs {
                let Some(e) = inp.word.take_if(|e| e.key == key) else { continue };
                docs = union(docs, inp.docs(store, &base, &e, next_doc)?);
            }
            docs.retain(|&d| self.docs.get(d as usize).is_some_and(DocRec::live));
            let held = inputs.iter().map(Input::held).fold(b.held(), usize::saturating_add);
            peak = peak.max(held.saturating_add(docs.len() * 4));
            if !docs.is_empty() {
                for &d in &docs {
                    seen.set(d);
                }
                b.push(&key, &docs);
            }
            last = Some(key);
            if b.held() >= budget {
                let pages = b.take();
                self.write_pages(store, id, pages)?;
            }
        }
        for inp in &inputs {
            meta.gone.extend(inp.info.names());
        }
        let merged: BTreeSet<u32> = inputs.iter().map(|i| i.info.id).collect();
        meta.segments.retain(|s| !merged.contains(&s.id));
        // The merged segment has no removed doc's words: those no other segment can hold are gone.
        let left = spans(&meta.segments);
        if !b.is_empty() {
            let seg = b.finish();
            meta.segments.push(self.seg_info(id, seen.count(), &seg)?);
            let mut files = Vec::new();
            self.seg_files(id, seg, &mut files);
            self.write_batches(store, files)?;
        }
        let dead = self.tombstones();
        let (purged, kept): (Vec<u32>, Vec<u32>) = dead.into_iter().partition(|&d| !within(&left, d));
        let mut files = Vec::new();
        let mut pages: BTreeSet<usize> = purged.iter().map(|&d| (d / DOCS_PER_PAGE) as usize).collect();
        pages.retain(|&p| p < meta.doc_pages.len());
        for &p in &pages {
            let start = p.saturating_mul(DOCS_PER_PAGE as usize);
            let end = start.saturating_add(DOCS_PER_PAGE as usize).min(self.docs.len());
            let mut recs = self.docs.get(start..end).unwrap_or_default().to_vec();
            // Those purged only: a removed doc kept still has words in a segment not merged.
            let first = purged.partition_point(|&d| (d as usize) < start);
            for &d in purged.get(first..).unwrap_or_default().iter().take_while(|&&d| (d as usize) < end) {
                if let Some(r) = recs.get_mut((d as usize).saturating_sub(start)) {
                    r.bits |= PURGED;
                }
            }
            let page = u32::try_from(p).unwrap_or(u32::MAX);
            if let Some(g) = meta.doc_pages.get_mut(p) {
                meta.gone.push(Name::Docs { generation: *g, page });
                *g = meta.generation;
            }
            files.push(self.file(Name::Docs { generation: meta.generation, page }, encode_docs(&recs)));
        }
        self.write_batches(store, files)?;
        meta.dead_since = self.dead_since(&meta.segments, &kept, now);
        #[cfg(test)]
        if self.stop_before_meta {
            return Err(io_err(&self.base)(other("stopped before the meta")));
        }
        let gone: Vec<Name> = self.meta.gone.iter().chain(&meta.gone).copied().collect();
        self.write_meta(store, &meta, &gone)?;
        for d in purged {
            if let Some(r) = self.docs.get_mut(d as usize) {
                r.bits |= PURGED;
            }
        }
        self.meta = meta;
        self.orphans = false;
        self.stamp = self.stamp.wrapping_add(1);
        #[cfg(test)]
        {
            self.merge_peak = peak;
        }
        #[cfg(not(test))]
        let _ = peak;
        Ok(())
    }

    fn write_pages(&self, store: &Store, id: u32, pages: Vec<SegPage>) -> Result<()> {
        let files = pages
            .into_iter()
            .map(|p| match p {
                SegPage::Lex(n, bytes) => self.file(Name::SegLex(id, n), bytes),
                SegPage::Post(n, bytes) => self.file(Name::SegPost(id, n), bytes),
            })
            .collect();
        self.write_batches(store, files)
    }

    /// The index made anew with ids for the live docs only, from its texts: new doc and text
    /// pages, new segments, a new epoch (a reader reads it whole again).
    fn renumber(&mut self, store: &Store, budget: usize) -> Result<()> {
        log::info!("{}'s search index: {} docs, {} live: giving ids again", self.account, self.docs.len(), self.live);
        let mut meta = Meta {
            epoch: new_epoch(self.meta.epoch),
            doc_pages: Vec::new(),
            text_pages: Vec::new(),
            segments: Vec::new(),
            gone: Vec::new(),
            dead: 0,
            dead_since: 0,
            ..self.meta.clone()
        };
        meta.generation = meta.generation.checked_add(1).ok_or_else(|| damaged(&self.base, "no generations left"))?;
        let generation = meta.generation;
        self.orphans = true;
        let mut docs: Vec<DocRec> = Vec::with_capacity(self.live as usize);
        let mut files: Vec<(PathBuf, Prepared)> = Vec::new();
        let mut page: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut page_bytes = 0usize;
        let mut read: Option<(u32, Vec<(u32, Vec<u8>)>)> = None;
        let mut words = BTreeMap::new();
        let (mut word_bytes, mut word_docs) = (0usize, 0u32);
        for (old, d) in self.docs.iter().enumerate().filter(|(_, d)| d.live()) {
            let new = u32::try_from(docs.len()).map_err(|_| damaged(&self.base, "no doc ids left"))?;
            let mut rec = DocRec { bits: d.bits & !PURGED, ..*d };
            if d.text != NO_TEXT {
                if read.as_ref().map(|(p, _)| *p) != Some(d.text) {
                    read.take();
                    read = Some((d.text, self.read_text_page(store, d.text)?));
                }
                let path =
                    Name::Text { generation: self.meta.text_pages.get(d.text as usize).map_or(0, |t| t.generation), page: d.text }.path(&self.base);
                let recs = read.as_ref().map(|(_, r)| r.as_slice()).unwrap_or_default();
                let old = u32::try_from(old).unwrap_or(u32::MAX);
                let blob = recs.binary_search_by_key(&old, |(doc, _)| *doc).ok().and_then(|i| recs.get(i)).map(|(_, b)| b);
                let blob = blob.ok_or_else(|| damaged(&path, "a doc's text isn't in its page"))?;
                let text = decode_blob(blob).map_err(|what| damaged(&path, what))?;
                word_bytes = word_bytes.saturating_add(add_words(&mut words, new, &text.text, &text.names));
                word_docs = word_docs.saturating_add(1);
                if !page.is_empty() && (page_bytes.saturating_add(blob.len() + 8) > TEXT_PAGE || page.len() >= usize::from(u16::MAX)) {
                    self.close_text_page(&mut meta, std::mem::take(&mut page), &mut files);
                    page_bytes = 0;
                }
                rec.text = u32::try_from(meta.text_pages.len()).map_err(|_| damaged(&self.base, "too many text pages"))?;
                page_bytes = page_bytes.saturating_add(blob.len() + 8);
                page.push((new, blob.clone()));
            }
            docs.push(rec);
            if word_bytes >= self.in_flight / 2 {
                self.write_segment(store, &mut meta, std::mem::take(&mut words), word_docs)?;
                (word_bytes, word_docs) = (0, 0);
            }
            if files.iter().map(|(_, p)| p.kept_len()).fold(0, usize::saturating_add) >= budget {
                self.write_batches(store, std::mem::take(&mut files))?;
            }
        }
        if !page.is_empty() {
            self.close_text_page(&mut meta, page, &mut files);
        }
        self.write_segment(store, &mut meta, words, word_docs)?;
        for (p, recs) in docs.chunks(DOCS_PER_PAGE as usize).enumerate() {
            let p = u32::try_from(p).map_err(|_| damaged(&self.base, "too many doc pages"))?;
            files.push(self.file(Name::Docs { generation, page: p }, encode_docs(recs)));
            meta.doc_pages.push(generation);
        }
        self.write_batches(store, files)?;
        let n = u32::try_from(docs.len()).map_err(|_| damaged(&self.base, "no doc ids left"))?;
        (meta.next_doc, meta.live) = (n, n);
        #[cfg(test)]
        if self.stop_before_meta {
            return Err(io_err(&self.base)(other("stopped before the meta")));
        }
        let gone: Vec<Name> = self.meta.uses().into_iter().chain(self.meta.gone.iter().copied()).collect();
        self.write_meta(store, &meta, &gone)?;
        self.by_key = by_key(&docs);
        self.docs = docs;
        (self.live, self.dead) = (n, 0);
        self.meta = meta;
        self.orphans = false;
        self.stamp = self.stamp.wrapping_add(1);
        Ok(())
    }

    fn close_text_page(&self, meta: &mut Meta, recs: Vec<(u32, Vec<u8>)>, files: &mut Vec<(PathBuf, Prepared)>) {
        let page = u32::try_from(meta.text_pages.len()).unwrap_or(u32::MAX);
        let bytes = encode_text_page(recs.iter().map(|(d, b)| (*d, b.as_slice())));
        meta.text_pages.push(TextPage { generation: meta.generation, bytes: u32::try_from(bytes.len()).unwrap_or(u32::MAX) });
        files.push(self.file(Name::Text { generation: meta.generation, page }, bytes));
    }
}

/// Whether a segment's span holds one of these removed docs (rising).
fn holds(dead: &[u32], s: &SegInfo) -> bool {
    let i = dead.partition_point(|&d| d < s.lo);
    dead.get(i).is_some_and(|&d| d <= s.hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_and_spans() {
        assert_eq!([0, 1, 3, 4, 15, 16, 4096, u32::MAX].map(tier), [0, 0, 0, 1, 1, 2, 6, 15]);
        let seg = |lo, hi| SegInfo { lo, hi, ..SegInfo::default() };
        let s = spans(&[seg(10, 20), seg(0, 3), seg(21, 30), seg(50, 60)]);
        assert_eq!(s, [(0, 3), (10, 30), (50, 60)]);
        assert!(within(&s, 25) && within(&s, 0) && !within(&s, 4) && !within(&s, 61));
        assert!(holds(&[2, 15], &seg(10, 20)) && !holds(&[2, 25], &seg(10, 20)));
        assert_eq!(union(vec![1, 3, 5], vec![2, 3, 6]), [1, 2, 3, 5, 6]);
    }
}
