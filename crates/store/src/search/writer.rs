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

mod commit;
mod merge;

pub use merge::{RENUMBER_SLACK, TOMBSTONE_AGE};

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
    /// A smaller [`RENUMBER_SLACK`], to reach it sooner.
    #[cfg(test)]
    pub(crate) renumber_slack: u32,
    /// The most bytes the last merge held, as it counted them.
    #[cfg(test)]
    pub(crate) merge_peak: usize,
    _held: Held,
}

/// A new generation made ready by [`Writer::prepare`], to be [committed](Writer::commit).
#[derive(Debug)]
pub struct Flush {
    stamp: u64,
    /// Nothing changed.
    nothing: bool,
    meta: Meta,
    files: Vec<(PathBuf, Prepared)>,
    /// The text page each new text went in.
    placed: Vec<(u32, u32)>,
}

impl Flush {
    /// Nothing changed: committing it writes nothing.
    pub fn is_empty(&self) -> bool {
        self.nothing
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
pub(super) fn add_words(words: &mut BTreeMap<Vec<u8>, Vec<u32>>, doc: u32, text: &str, names: &[String]) -> usize {
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
        let by_key = by_key(&docs);
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
            #[cfg(test)]
            renumber_slack: RENUMBER_SLACK,
            #[cfg(test)]
            merge_peak: 0,
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
        let (folder, id) = self.make_room(key)?;
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

    /// The folder number and doc id a message added takes, the doc it replaces removed.
    fn make_room(&mut self, key: &MessageKey) -> Result<(u16, u32)> {
        let folder = self.folder(&key.folder)?;
        let id = u32::try_from(self.docs.len()).ok().filter(|id| *id < u32::MAX - 1).ok_or_else(|| damaged(&self.base, "no doc ids left"))?;
        if let Some(old) = self.holder(key) {
            self.remove(old);
        }
        Ok((folder, id))
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
}

/// Each live doc by its folder and uid.
fn by_key(docs: &[DocRec]) -> HashMap<(u16, u32), u32> {
    let mut by_key = HashMap::with_capacity(docs.len());
    for (i, d) in docs.iter().enumerate() {
        if d.live() {
            by_key.insert((d.folder, d.uid), u32::try_from(i).unwrap_or(u32::MAX));
        }
    }
    by_key
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
