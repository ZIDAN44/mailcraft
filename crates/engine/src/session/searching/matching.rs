//! Search: what's known of a message's text, and the verdict on a message.
//!
//! The index answers a word from the docs it found the word in ([`IndexText`]); what it can't
//! settle (a phrase, an address written oddly, a message whose text was cut short) is checked
//! against the message's text as the index keeps it ([`Verifying`]), a slice at a time; waiting
//! (the CLI, tests), a message the index doesn't have yet is read from its source instead
//! ([`from_source`]). Either way what a message's text said is kept as [`Answers`], so a flag
//! changed on it later is weighed without reading anything again: flags are never text.

use std::cell::RefCell;
use std::sync::Arc;

use mailcraft_model::search::indexed;
use mailcraft_store::search::{Blob, DocInfo, Field, Reader};

use super::folded::{Folded, TextOf};
use super::look_up::Terms;
use super::*;

/// Most bytes of a message's text checked in one unit of work.
pub(crate) const VERIFY_BYTES: usize = 256 << 10;
/// How far back each slice after the first starts, so a phrase across two slices is found.
const OVERLAP: usize = 4 << 10;

/// Something a query asks of a message's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Atom {
    /// A word starting so (folded).
    Word(TextField, String),
    /// Folded words one after another, and the phrase as typed.
    Phrase(TextField, Vec<String>, String),
}

impl Atom {
    fn field(&self) -> TextField {
        match self {
            Atom::Word(f, _) | Atom::Phrase(f, ..) => *f,
        }
    }

    /// Whether `text` holds it.
    fn in_text(&self, text: &str) -> bool {
        match self {
            Atom::Word(f, w) => text.word(*f, w) == Tri::Yes,
            Atom::Phrase(f, words, raw) => text.phrase(*f, words, raw) == Tri::Yes,
        }
    }
}

/// What a query asks of a message's text, noted as it's asked (it can't tell anything).
#[derive(Default)]
pub(crate) struct Recorder(RefCell<Vec<Atom>>);

impl Recorder {
    /// What `query` asks of `m`'s text, given what its summary (its text `st`) already settles.
    pub(crate) fn atoms(query: &Query, m: &Summary, st: &SummaryText, awaiting: bool) -> Vec<Atom> {
        let rec = Recorder::default();
        query.eval(&Doc { summary: m, summary_text: st, awaiting, files: None, text: &rec });
        rec.0.into_inner()
    }

    fn note(&self, a: Atom) {
        let mut v = self.0.borrow_mut();
        if !v.contains(&a) {
            v.push(a);
        }
    }
}

impl TextSource for Recorder {
    fn word(&self, field: TextField, prefix: &str) -> Tri {
        self.note(Atom::Word(field, prefix.to_string()));
        Tri::Unknown
    }

    fn phrase(&self, field: TextField, phrase: &[String], raw: &str) -> Tri {
        self.note(Atom::Phrase(field, phrase.to_vec(), raw.to_string()));
        Tri::Unknown
    }
}

/// What a message's text was found to say, for each thing asked of it; and whether it has
/// files, as its text was read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Answers {
    atoms: Vec<(Atom, Tri)>,
    pub(crate) files: Option<bool>,
    /// The text was cut short: what it doesn't settle may be past the cut ([`Verdict::CutShort`]).
    truncated: bool,
}

impl Answers {
    /// Whether it says anything (a text that couldn't be read says nothing, and isn't kept).
    pub(crate) fn said(&self) -> bool {
        !self.atoms.is_empty() || self.files.is_some()
    }
}

impl TextSource for Answers {
    fn word(&self, field: TextField, prefix: &str) -> Tri {
        let hit = |a: &Atom| matches!(a, Atom::Word(f, w) if *f == field && w == prefix);
        self.atoms.iter().find(|(a, _)| hit(a)).map_or(Tri::Unknown, |(_, t)| *t)
    }

    fn phrase(&self, field: TextField, phrase: &[String], raw: &str) -> Tri {
        let hit = |a: &Atom| matches!(a, Atom::Phrase(f, p, r) if *f == field && p == phrase && r == raw);
        self.atoms.iter().find(|(a, _)| hit(a)).map_or(Tri::Unknown, |(_, t)| *t)
    }
}

/// Nothing to read: no word is there (a message whose source isn't kept here is searched by its
/// summary alone).
pub(crate) struct NoText;

impl TextSource for NoText {
    fn word(&self, _: TextField, _: &str) -> Tri {
        Tri::No
    }

    fn phrase(&self, _: TextField, _: &[String], _: &str) -> Tri {
        Tri::No
    }
}

/// Not read yet: can't tell.
pub(crate) struct Unindexed;

impl TextSource for Unindexed {
    fn word(&self, _: TextField, _: &str) -> Tri {
        Tri::Unknown
    }

    fn phrase(&self, _: TextField, _: &[String], _: &str) -> Tri {
        Tri::Unknown
    }
}

/// A doc's words as the index found them. A word is there or not; a phrase is not there when
/// one of its words isn't, and to be checked against the text when all are; an address written
/// so the index can't look it up (`@studio.example`) is checked too.
struct IndexText<'a> {
    terms: &'a Terms,
    doc: u32,
    info: DocInfo,
}

impl IndexText<'_> {
    fn not_found(&self, field: TextField) -> Tri {
        indexed(false, field, self.info.truncated, self.info.has_text)
    }
}

impl TextSource for IndexText<'_> {
    fn word(&self, field: TextField, prefix: &str) -> Tri {
        if prefix.is_empty() {
            return Tri::Unknown;
        }
        let Some(found) = self.terms.has(Field::from(field), prefix, self.doc) else { return Tri::Unknown };
        let address = prefix.contains(['@', '.']);
        if found || (address && self.terms.has(Field::Address, prefix, self.doc) == Some(true)) {
            return Tri::Yes;
        }
        // Found inside a word as written: the index keeps whole addresses and domains, from
        // their start.
        if address && !prefix.starts_with(char::is_alphanumeric) {
            return Tri::Unknown;
        }
        self.not_found(field)
    }

    fn phrase(&self, field: TextField, phrase: &[String], _raw: &str) -> Tri {
        if phrase.is_empty() {
            return Tri::Unknown;
        }
        for w in phrase {
            match self.terms.has(Field::from(field), w, self.doc) {
                Some(true) => {}
                Some(false) => return self.not_found(field),
                None => return Tri::Unknown,
            }
        }
        Tri::Unknown
    }
}

/// A message to check further, as it was when found.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) m: Summary,
    /// Its doc in its account's index, when it's there.
    pub(crate) doc: Option<u32>,
    pub(crate) awaiting: bool,
    /// Its place in its folder's list when it was found: its summary's text is kept there
    /// ([`FolderTexts::kept`](super::folder_text::FolderTexts::kept)), not made again.
    pub(crate) at: Option<usize>,
}

/// What a search decided about a message, or what it must do to decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Yes,
    No,
    /// Indexed, but only its text can settle it: checked against the text kept for this doc.
    Verify(u32),
    /// Not indexed yet: it may match.
    Missing,
    /// Indexed, but its text was cut short (too long): what's past the cut may match, and
    /// nothing kept can tell (its kept text is what its words were indexed from).
    CutShort,
}

/// The index an account has, and the words a search looked up in it (none: the query asks
/// none, only whether there are files).
#[derive(Clone, Copy)]
pub(crate) struct Index<'a> {
    pub(crate) reader: &'a Reader,
    pub(crate) terms: Option<&'a Terms>,
    /// The folder whose messages are weighed, as the index numbers it (`None`: none of its
    /// messages is indexed).
    folder: Option<u16>,
}

impl<'a> Index<'a> {
    /// The index as it weighs the messages of `folder`: the folder looked up once, not for each.
    pub(crate) fn of(reader: &'a Reader, terms: Option<&'a Terms>, folder: &str) -> Index<'a> {
        Index { reader, terms, folder: reader.folder_number(folder) }
    }

    /// `m`'s doc and what's known of it, when it's indexed (`m` is of the folder).
    fn doc(&self, m: &Summary) -> Option<(u32, DocInfo)> {
        let d = self.reader.doc_at(self.folder?, m.key.validity, m.key.uid)?;
        Some((d, self.reader.info(d)?))
    }
}

/// The verdict on `m` (its summary's text `st`): from its summary and the index, or from what
/// its text was found to say before (`known`).
pub(crate) fn classify(query: &Query, m: &Summary, st: &SummaryText, awaiting: bool, index: Option<Index<'_>>, known: Option<&Answers>) -> Verdict {
    let doc = index.and_then(|ix| ix.doc(m));
    // Files as the index read them: pictures the text shows aren't counted (`has:attachment`).
    // One it couldn't read is the summary's to say, as for the Has files filter.
    let files = known.and_then(|k| k.files).or_else(|| doc.and_then(|(_, i)| i.files_known()));
    let eval = |text: &dyn TextSource| query.eval(&Doc { summary: m, summary_text: st, awaiting, files, text });
    let settled = |t: Tri| match t {
        Tri::Yes => Verdict::Yes,
        Tri::No => Verdict::No,
        Tri::Unknown => Verdict::Missing,
    };
    if let Some(k) = known {
        return match eval(k) {
            // Not found in what was kept of a long message: past its cut it may be.
            Tri::Unknown if k.truncated => Verdict::CutShort,
            t => settled(t),
        };
    }
    match (index.and_then(|ix| ix.terms), doc) {
        (_, Some((_, info))) if info.no_source => settled(eval(&NoText)),
        (Some(terms), Some((d, info))) => match eval(&IndexText { terms, doc: d, info }) {
            Tri::Yes => Verdict::Yes,
            Tri::No => Verdict::No,
            // Unknown only for the cut: its words are in the index as they are in the text kept,
            // so checking that text says nothing more (it read the text of every long message
            // for each word searched: 163 to 341 ms a search of the 41 GB file).
            Tri::Unknown if info.truncated && eval(&IndexText { terms, doc: d, info: DocInfo { truncated: false, ..info } }) != Tri::Unknown => {
                Verdict::CutShort
            }
            Tri::Unknown => Verdict::Verify(d),
        },
        _ => settled(eval(&Unindexed)),
    }
}

/// What `atoms` find in a message's source, read the way the index reads it.
pub(crate) fn from_source(raw: &[u8], atoms: &[Atom]) -> Answers {
    let doc = mime::index_doc(raw);
    let atoms = atoms
        .iter()
        .map(|a| {
            let t = match a {
                Atom::Word(f, w) => doc.word(*f, w),
                Atom::Phrase(f, words, r) => doc.phrase(*f, words, r),
            };
            (a.clone(), t)
        })
        .collect();
    Answers { atoms, files: Some(doc.files > 0), truncated: false }
}

/// A message's text being checked against what a query asks of it, a slice a unit.
pub(crate) struct Verifying {
    pub(crate) c: Candidate,
    /// Its summary's text, which weighs what its text says.
    pub(crate) st: SummaryText,
    doc: u32,
    /// The index's epoch `doc` is of.
    epoch: u64,
    info: DocInfo,
    atoms: Vec<Atom>,
    found: Vec<bool>,
    /// The text kept for the doc, once read (`None` inside: none is kept).
    blob: Option<Option<Blob>>,
    /// Where in the text the next slice starts.
    at: usize,
    /// Its text folded before ([`FoldedTexts`](super::folded::FoldedTexts)), checked in its
    /// place: nothing read, nothing folded.
    kept: Option<Arc<Folded>>,
    /// The next of its slices checked.
    slice: usize,
    /// Its text folded as it's checked, to be kept (`None`: what's asked can't be told from
    /// the folded text, only from the text as written).
    folding: Option<Folded>,
}

/// Most bytes of a text folded before checked in one unit: looking through them is several
/// times quicker than folding them.
const KEPT_BYTES: usize = 4 * VERIFY_BYTES;

impl Verifying {
    /// `of`: its doc; `kept`: its text folded before, if it was.
    pub(crate) fn new(query: &Query, c: Candidate, st: SummaryText, of: TextOf, info: DocInfo, kept: Option<Arc<Folded>>) -> Verifying {
        let atoms = Recorder::atoms(query, &c.m, &st, c.awaiting);
        let found = vec![false; atoms.len()];
        // A phrase of words is told from the folded text alone; a word (or a phrase of
        // punctuation: `"@"`) is looked for as written too.
        let foldable = atoms.iter().all(|a| a.field() != TextField::Body || matches!(a, Atom::Phrase(_, words, _) if !words.is_empty()));
        let kept = kept.filter(|_| foldable);
        let folding = (foldable && kept.is_none()).then(Folded::default);
        Verifying { c, st, doc: of.doc, epoch: of.epoch, info, atoms, found, blob: None, at: 0, kept, slice: 0, folding }
    }

    /// The doc its text is read from.
    pub(crate) fn text_of(&self) -> TextOf {
        TextOf { epoch: self.epoch, doc: self.doc }
    }

    /// Its text folded as it was checked, when it was checked in full: to be kept.
    pub(crate) fn take_folded(&mut self) -> Option<Folded> {
        self.folding.take()
    }

    /// One unit: the text read, or one slice of it checked. What it said once it's all
    /// checked; a text that can't be read says nothing for sure.
    pub(crate) fn step(&mut self, reader: &mut Reader, store: &Store) -> Option<Answers> {
        if let Some(kept) = self.kept.clone() {
            return self.step_kept(&kept);
        }
        let Some(blob) = &self.blob else {
            let blob = match reader.text(store, self.doc) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("{}: a message's text couldn't be read for search: {e}", self.c.m.key.account);
                    self.folding = None;
                    return Some(Answers::default());
                }
            };
            // The files' names are short: checked with the reading.
            if let Some(b) = &blob {
                self.check_names(&b.names);
                if let Some(f) = self.folding.as_mut() {
                    f.names = b.names.clone();
                }
            }
            self.blob = Some(blob);
            return None;
        };
        let text = blob.as_ref().map_or("", |b| b.text.as_str());
        if self.at < text.len() {
            let (start, end) = slice_at(text, self.at);
            let chunk = text.get(start..end).unwrap_or_default();
            match self.folding.as_mut() {
                // Folded once for every phrase asked, and kept.
                Some(folding) => {
                    let folded = mailcraft_model::search::fold_all(chunk);
                    for (a, f) in self.atoms.iter().zip(self.found.iter_mut()) {
                        if let Atom::Phrase(TextField::Body, words, _) = a
                            && !*f
                            && mailcraft_model::search::folded_has_phrase(&folded, words)
                        {
                            *f = true;
                        }
                    }
                    folding.slices.push(folded);
                }
                None => {
                    for (a, f) in self.atoms.iter().zip(self.found.iter_mut()) {
                        if !*f && a.field() == TextField::Body && a.in_text(chunk) {
                            *f = true;
                        }
                    }
                }
            }
            self.at = end.max(self.at.saturating_add(1));
            if self.at < text.len() {
                return None;
            }
        }
        Some(self.answers())
    }

    /// One unit of checking the text folded before: its names, and its slices up to
    /// [`KEPT_BYTES`].
    fn step_kept(&mut self, kept: &Folded) -> Option<Answers> {
        if self.slice == 0 {
            self.check_names(&kept.names);
        }
        let mut looked = 0usize;
        while let Some(folded) = kept.slices.get(self.slice) {
            if looked >= KEPT_BYTES {
                return None;
            }
            for (a, f) in self.atoms.iter().zip(self.found.iter_mut()) {
                if let Atom::Phrase(TextField::Body, words, _) = a
                    && !*f
                    && mailcraft_model::search::folded_has_phrase(folded, words)
                {
                    *f = true;
                }
            }
            looked = looked.saturating_add(folded.len());
            self.slice = self.slice.saturating_add(1);
        }
        Some(self.answers())
    }

    /// The files' names checked.
    fn check_names(&mut self, names: &[String]) {
        for (a, f) in self.atoms.iter().zip(self.found.iter_mut()) {
            if a.field() == TextField::Names && names.iter().any(|n| a.in_text(n)) {
                *f = true;
            }
        }
    }

    /// What the text said, all of it checked.
    fn answers(&self) -> Answers {
        let (truncated, has_text) = (self.info.truncated, self.info.has_text);
        let atoms = self.atoms.iter().zip(&self.found).map(|(a, f)| (a.clone(), indexed(*f, a.field(), truncated, has_text))).collect();
        Answers { atoms, files: self.info.files_known(), truncated }
    }
}

/// The slice of `text` checked from `at`: [`VERIFY_BYTES`] on to the next word's end (a word
/// isn't cut), started [`OVERLAP`] before at a word's start, so a phrase across slices is in one
/// of them. Either end looks only so far for a word's edge (a text with no spaces: CJK, a long
/// run of base64): a slice is never more than `VERIFY_BYTES + 2 * OVERLAP + VERIFY_BYTES / 4`.
fn slice_at(text: &str, at: usize) -> (usize, usize) {
    // Between words: no letter or digit just before `i`.
    let edge = |i: usize| text.is_char_boundary(i) && text.get(..i).and_then(|t| t.chars().next_back()).is_none_or(|c| !c.is_alphanumeric());
    let mut start = 0;
    if at > 0 {
        let from = at.saturating_sub(OVERLAP);
        let floor = at.saturating_sub(OVERLAP.saturating_mul(2));
        start = from;
        while start > floor && !edge(start) {
            start -= 1;
        }
        // No word's start near: from the overlap, on a character.
        if !edge(start) {
            start = from;
            while !text.is_char_boundary(start) {
                start = start.saturating_sub(1);
            }
        }
    }
    let mut end = at.saturating_add(VERIFY_BYTES).min(text.len());
    let cap = end.saturating_add(VERIFY_BYTES / 4).min(text.len());
    while end < cap && !edge(end) {
        end += 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    (start, end)
}

#[cfg(test)]
mod tests;
