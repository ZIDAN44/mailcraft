//! The search index kept with each account's mail, so a search never reads a message to answer.
//!
//! It is a set of files under the account (`<account>/search/v1/`), so it lives wherever the
//! account does: in its data file, on disk for the folder layout, in memory for an online-only
//! account and the browser. Nothing else in the store changes for it: a data file from before
//! simply has no index yet, and a build from before never looks at these names.
//!
//! ```text
//! meta.bin            what the index is now: written last, the commit point of every change
//! docs/<g>-<p>.bin    the doc table, 4,096 docs a page of 32 bytes each, a page rewritten
//!                     under the generation that changed it (copy on write)
//! t/<g>-<p>.bin       the messages' text and files' names (the phrase checker's source, and
//!                     what the postings are made again from), in pages of up to 256 KiB
//! s/<id>-h.bin        a segment's head: each lexicon page's first word
//! s/<id>-l<n>.bin     a lexicon page: words, front-coded, with their postings or where they are
//! s/<id>-p<n>.bin     a postings page: sorted doc ids as varint deltas
//! ```
//!
//! A word is kept as its field's letter and the word [folded](mailcraft_model::search::fold):
//! `b` the text, `n` the files' names, `a` whole addresses and their domains (so
//! `ana@studio.example` needn't be checked against the text). A change ([`Writer::flush`]) writes
//! its files under names the current meta doesn't use, in short transactions, then the meta: a
//! crash before the meta leaves the generation before it, and what was written for the new one
//! is found by its names and removed when the writer next opens. Files a generation no longer
//! uses are named in its meta and removed after it. Every file is read as hostile input
//! (`format.rs`): an index that doesn't read back is set aside (`meta.bin.bad`) and made again.

use std::collections::BTreeSet;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

mod format;
mod reader;
mod writer;

#[cfg(test)]
mod tests;

pub use format::Blob;
pub use reader::{DocInfo, Reader, ScanCursor};
pub use writer::{Flush, Writer};

/// The doc table's page size, in docs: a doc's id is its page × this + its slot.
pub const DOCS_PER_PAGE: u32 = 4096;
/// A text page is closed once this full (a longer text has a page of its own).
pub const TEXT_PAGE: usize = 256 << 10;
/// Most bytes in a lexicon page.
pub const LEX_PAGE: usize = 64 << 10;
/// Most bytes in a postings page (a longer list goes on into the next ones).
pub const POST_PAGE: usize = 256 << 10;
/// Longest word kept, its field's letter included: 40 characters of up to 4 bytes.
pub const MAX_KEY: usize = 1 + 160;
/// What a writer holds in memory before it must write (its words, postings and texts): it
/// flushes at half, so a flush being written and the next one filling stay under it.
pub const IN_FLIGHT_BYTES: usize = 64 << 20;
/// Most folders an index can name (the doc table keeps a folder as a 16-bit number); past it,
/// the index is made again.
pub const MAX_FOLDERS: usize = 0xFFFE;
/// Most address words (addresses and their domains) kept for one message.
const MAX_ADDRESS_WORDS: usize = 256;

/// Where in a message a word is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// The text the reader sees.
    Body,
    /// The files' names.
    Names,
    /// Whole addresses and domains in the text and names, as typed (lower-case).
    Address,
}

impl Field {
    /// The letter a word of this field is kept under.
    pub fn letter(self) -> u8 {
        match self {
            Field::Body => b'b',
            Field::Names => b'n',
            Field::Address => b'a',
        }
    }
}

impl From<mailcraft_model::search::TextField> for Field {
    fn from(f: mailcraft_model::search::TextField) -> Field {
        match f {
            mailcraft_model::search::TextField::Body => Field::Body,
            mailcraft_model::search::TextField::Names => Field::Names,
        }
    }
}

/// A word as the index keeps it: its field's letter, then the word.
pub fn key(field: Field, word: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(1 + word.len());
    k.push(field.letter());
    k.extend_from_slice(word.as_bytes());
    k
}

/// The addresses in a text (`ana@studio.example`), lower-cased, with their domains and the
/// domains those are in (`studio.example`, but not a lone `example`): what [`Field::Address`]
/// holds. Added to `out`, at most [`MAX_ADDRESS_WORDS`] of them all told.
pub fn address_words(text: &str, out: &mut BTreeSet<String>) {
    let local = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-' | '\'');
    let host = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-');
    for (at, _) in text.match_indices('@') {
        if out.len() >= MAX_ADDRESS_WORDS {
            return;
        }
        let (Some(before), Some(after)) = (text.get(..at), text.get(at + 1..)) else { continue };
        let start = before.char_indices().rev().take_while(|(_, c)| local(*c)).last().map_or(before.len(), |(i, _)| i);
        let end = after.char_indices().find(|(_, c)| !host(*c)).map_or(after.len(), |(i, _)| i);
        let (Some(name), Some(domain)) = (before.get(start..), after.get(..end)) else { continue };
        let name = name.trim_matches('.');
        let domain = domain.trim_matches(|c| c == '.' || c == '-').to_ascii_lowercase();
        if name.is_empty() || !domain.contains('.') {
            continue;
        }
        let whole = format!("{}@{domain}", name.to_ascii_lowercase());
        if whole.len() < MAX_KEY {
            out.insert(whole);
        }
        // The domain, and each it is part of while it still has a dot.
        let mut d = domain.as_str();
        while d.contains('.') && d.len() < MAX_KEY && out.len() < MAX_ADDRESS_WORDS {
            out.insert(d.to_string());
            match d.split_once('.') {
                Some((_, rest)) => d = rest,
                None => break,
            }
        }
    }
}

/// One bit per doc: the docs a word was found in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bits {
    words: Vec<u64>,
    len: u32,
}

impl Bits {
    /// Room for docs `0..len`, none set.
    pub fn new(len: u32) -> Bits {
        Bits { words: vec![0; (len as usize).div_ceil(64)], len }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Set doc `i` (one past the room is left out).
    pub fn set(&mut self, i: u32) {
        if let Some(w) = self.words.get_mut((i / 64) as usize)
            && i < self.len
        {
            *w |= 1 << (i % 64);
        }
    }

    pub fn get(&self, i: u32) -> bool {
        self.words.get((i / 64) as usize).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    /// How many are set.
    pub fn count(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).fold(0u32, u32::saturating_add)
    }

    /// Keep only those `other` has set too.
    pub fn and(&mut self, other: &Bits) {
        for (i, w) in self.words.iter_mut().enumerate() {
            *w &= other.words.get(i).copied().unwrap_or(0);
        }
    }

    /// The docs set, in order.
    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &w)| {
            let base = u32::try_from(i).unwrap_or(u32::MAX).saturating_mul(64);
            (0..64u32).filter(move |b| w & (1 << b) != 0).map(move |b| base.saturating_add(b))
        })
    }
}

/// A doc as the doc table keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocRec {
    /// Its folder's place in the meta's list; [`DEAD`] once removed.
    pub(crate) folder: u16,
    pub(crate) validity: u32,
    pub(crate) uid: u32,
    /// The text page its text is in ([`NO_TEXT`]: none kept yet).
    pub(crate) text: u32,
    /// Its files (`has:attachment`).
    pub(crate) files: u16,
    /// [`HAS_TEXT`], [`TRUNCATED`].
    pub(crate) bits: u8,
    /// Its Message-ID's hash and its size: a message moved is known by them.
    pub(crate) msgid_hash: u64,
    pub(crate) size: u32,
}

/// The folder number of a removed doc.
pub(crate) const DEAD: u16 = 0xFFFF;
/// The text page of a doc whose text isn't kept.
pub(crate) const NO_TEXT: u32 = u32::MAX;
/// The message has text of its own, not only files.
pub(crate) const HAS_TEXT: u8 = 1;
/// Something of it was left out (too long, nested too deep).
pub(crate) const TRUNCATED: u8 = 2;

impl DocRec {
    pub(crate) fn live(&self) -> bool {
        self.folder != DEAD
    }
}

/// Where an account's index is.
pub(crate) fn dir(store: &Store, account: &str) -> PathBuf {
    store.account_dir(account).join("search").join("v1")
}

pub(crate) fn meta_path(store: &Store, account: &str) -> PathBuf {
    dir(store, account).join("meta.bin")
}

/// A file of the index, by what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Name {
    /// A doc page as of a generation.
    Docs {
        generation: u64,
        page: u32,
    },
    /// A text page as of a generation.
    Text {
        generation: u64,
        page: u32,
    },
    SegHead(u32),
    SegLex(u32, u32),
    SegPost(u32, u32),
}

impl Name {
    /// Its path under the index's folder.
    pub(crate) fn rel(self) -> String {
        match self {
            Name::Docs { generation, page } => format!("docs/{generation}-{page}.bin"),
            Name::Text { generation, page } => format!("t/{generation}-{page}.bin"),
            Name::SegHead(id) => format!("s/{id}-h.bin"),
            Name::SegLex(id, n) => format!("s/{id}-l{n}.bin"),
            Name::SegPost(id, n) => format!("s/{id}-p{n}.bin"),
        }
    }

    pub(crate) fn path(self, base: &Path) -> PathBuf {
        let rel = self.rel();
        rel.split('/').fold(base.to_path_buf(), |p, part| p.join(part))
    }

    /// The file named `name` in the index's folder `sub` (`docs`, `t`, `s`), when it's one of
    /// the index's (what isn't is left alone).
    pub(crate) fn parse(sub: &str, name: &str) -> Option<Name> {
        let stem = name.strip_suffix(".bin")?;
        let (a, b) = stem.split_once('-')?;
        match sub {
            "docs" => Some(Name::Docs { generation: a.parse().ok()?, page: b.parse().ok()? }),
            "t" => Some(Name::Text { generation: a.parse().ok()?, page: b.parse().ok()? }),
            "s" => {
                let id = a.parse().ok()?;
                match (b, b.get(1..)) {
                    ("h", _) => Some(Name::SegHead(id)),
                    (_, Some(n)) if b.starts_with('l') => Some(Name::SegLex(id, n.parse().ok()?)),
                    (_, Some(n)) if b.starts_with('p') => Some(Name::SegPost(id, n.parse().ok()?)),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// The folders under the index's folder that hold its files.
pub(crate) const SUBDIRS: [&str; 3] = ["docs", "t", "s"];

/// Set an account's index aside (its meta renamed `meta.bin.bad`) to be made again: it didn't
/// read back. The new one starts past its generation and segment numbers (`Writer::open`).
pub fn set_aside(store: &Store, account: &str) -> Result<()> {
    let meta = meta_path(store, account);
    let bad = meta.with_extension("bin.bad");
    log::warn!("{account}'s search index doesn't read back: set aside, to be made again");
    match store.files.remove_file(&bad) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&bad)(e)),
        _ => {}
    }
    match store.files.rename(&meta, &bad) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&meta)(e)),
        _ => Ok(()),
    }
}

/// A new index's epoch: a number no index before it here had, so nothing cached of an index
/// set aside is taken for the new one's (whose files may have the same names).
pub(crate) fn new_epoch(not: u64) -> u64 {
    static MADE: AtomicU64 = AtomicU64::new(0);
    let n = MADE.fetch_add(1, Ordering::Relaxed);
    let e = std::collections::hash_map::RandomState::new().hash_one((n, &MADE as *const AtomicU64 as usize));
    if e == not || e == 0 { e.wrapping_add(1).max(1) } else { e }
}

/// An account's index held for writing: let go when dropped.
#[derive(Debug)]
pub(crate) struct Held {
    writers: Arc<Mutex<HashSet<String>>>,
    account: String,
}

impl Held {
    /// Hold `account`'s index, or say it's held already.
    pub(crate) fn take(store: &Store, account: &str) -> Result<Held> {
        let mut g = store.search_writers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !g.insert(account.to_string()) {
            return Err(StoreError::Busy { path: format!("{account}/search") });
        }
        Ok(Held { writers: store.search_writers.clone(), account: account.to_string() })
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.writers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&self.account);
    }
}

/// A damaged index file: `what` was wrong with it.
pub(crate) fn damaged(path: &Path, what: &str) -> StoreError {
    StoreError::Damaged { path: path.display().to_string(), what: what.to_string() }
}

/// An index file's bytes; `None` when it isn't there.
pub(crate) fn read_file(store: &Store, path: &Path) -> Result<Option<Vec<u8>>> {
    match store.files.read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(damaged(path, "unreadable")),
        Err(e) => Err(io_err(path)(e)),
    }
}

/// An index file that must be there.
pub(crate) fn need_file(store: &Store, path: &Path) -> Result<Vec<u8>> {
    read_file(store, path)?.ok_or_else(|| StoreError::Io { path: path.display().to_string(), source: io::Error::from(io::ErrorKind::NotFound) })
}

/// Whether an error is a file of the index that wasn't there (a change removed it after the
/// meta that named it was read: read the meta again).
pub fn is_gone(e: &StoreError) -> bool {
    matches!(e, StoreError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound)
}

/// Whether an error is a damaged index ([`set_aside`] it).
pub fn is_damaged(e: &StoreError) -> bool {
    matches!(e, StoreError::Damaged { .. })
}

/// An account's index's meta; `None` when it has none.
pub(crate) fn load_meta(store: &Store, account: &str) -> Result<Option<format::Meta>> {
    let path = meta_path(store, account);
    let Some(bytes) = read_file(store, &path)? else { return Ok(None) };
    format::decode_meta(&bytes).map(Some).map_err(|what| damaged(&path, what))
}

/// Doc page `p` of `meta`'s table.
pub(crate) fn load_doc_page(store: &Store, base: &Path, meta: &format::Meta, p: usize) -> Result<Vec<DocRec>> {
    let generation = meta.doc_pages.get(p).copied().unwrap_or(0);
    let path = Name::Docs { generation, page: u32::try_from(p).unwrap_or(u32::MAX) }.path(base);
    let bytes = need_file(store, &path)?;
    format::decode_docs(&bytes, format::page_docs(meta.next_doc, p), meta.folders.len(), meta.text_pages.len()).map_err(|what| damaged(&path, what))
}

/// `meta`'s whole doc table, its counts checked against the meta's.
pub(crate) fn load_docs(store: &Store, base: &Path, meta: &format::Meta) -> Result<Vec<DocRec>> {
    let mut docs = Vec::with_capacity(meta.doc_pages.len().min(64).saturating_mul(DOCS_PER_PAGE as usize));
    for p in 0..meta.doc_pages.len() {
        docs.extend(load_doc_page(store, base, meta, p)?);
    }
    let live = docs.iter().filter(|d| d.live()).count();
    if live != meta.live as usize || docs.len() - live != meta.dead as usize {
        return Err(damaged(&base.join("meta.bin"), "counts not the doc table's"));
    }
    Ok(docs)
}
