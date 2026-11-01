//! Checking a query against a message: yes, no, or can't tell yet ([`Tri`]).
//!
//! What the summary holds (flags, sizes, dates, the senders, the subject, the preview) is always
//! known. A message's text and its files' names come from a [`TextSource`]: the search index,
//! or the text itself. When that can't settle a word (the message isn't indexed yet, or its text
//! was cut short), the answer is [`Tri::Unknown`], and the search says some results may be
//! missing rather than leaving them out quietly. Clauses are checked the summary's first, and a
//! no ends it, so the text is only asked about for a message that passes everything else.

use std::borrow::Cow;

use super::fold::{all_words, has_phrase, has_prefix, join};
use super::*;

/// Kleene's three values: a message matches, doesn't, or can't be told yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    Yes,
    No,
    Unknown,
}

impl std::ops::Not for Tri {
    type Output = Tri;

    fn not(self) -> Tri {
        match self {
            Tri::Yes => Tri::No,
            Tri::No => Tri::Yes,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

impl Tri {
    /// Both: no when either is no, yes when both are yes.
    pub fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::No, _) | (_, Tri::No) => Tri::No,
            (Tri::Yes, Tri::Yes) => Tri::Yes,
            _ => Tri::Unknown,
        }
    }

    /// Either: yes when either is yes, no when both are no.
    pub fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::Yes, _) | (_, Tri::Yes) => Tri::Yes,
            (Tri::No, Tri::No) => Tri::No,
            _ => Tri::Unknown,
        }
    }
}

impl From<bool> for Tri {
    fn from(b: bool) -> Tri {
        if b { Tri::Yes } else { Tri::No }
    }
}

/// Where in a message's text a word is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextField {
    /// The text the reader sees.
    Body,
    /// The files' names.
    Names,
}

/// A message's text, as far as it's known.
pub trait TextSource {
    /// Does any word of `field` start with `prefix` (already folded)?
    fn word(&self, field: TextField, prefix: &str) -> Tri;
    /// Are these folded words in `field`, one after another? `raw` is the phrase as typed
    /// (lower-cased), looked for as written when it has no words (`@`). Unknown: it must be
    /// checked against the text itself.
    fn phrase(&self, field: TextField, phrase: &[String], raw: &str) -> Tri;
}

/// What an index answers for a word it looked up (`found`, or not). Not there is a no, unless
/// what it was made from was cut short (too long, too many parts) or had no text to read (only
/// files), when it can't tell.
pub fn indexed(found: bool, field: TextField, truncated: bool, has_text: bool) -> Tri {
    match (found, field) {
        (true, _) => Tri::Yes,
        (false, TextField::Body) if truncated || !has_text => Tri::Unknown,
        (false, TextField::Names) if truncated => Tri::Unknown,
        (false, _) => Tri::No,
    }
}

/// A whole text known: each answer is yes or no. Both fields are this text (a message's text
/// and names [joined](IndexDoc::joined)).
impl TextSource for &str {
    fn word(&self, _: TextField, prefix: &str) -> Tri {
        // An address is also found inside a word (`ana@studio.example` in `<ana@studio.example>`).
        Tri::from(all_words(self).any(|w| w.starts_with(prefix)) || (prefix.contains(['@', '.']) && self.to_lowercase().contains(prefix)))
    }

    fn phrase(&self, _: TextField, phrase: &[String], raw: &str) -> Tri {
        if phrase.is_empty() {
            return Tri::from(!raw.is_empty() && self.to_lowercase().contains(raw));
        }
        let mut text = String::with_capacity(self.len());
        join(all_words(self), &mut text);
        Tri::from(has_phrase(&text, &phrase.join(" ")))
    }
}

/// A message's text read for search: its text and its names, each known, unless it was cut
/// short or had no text ([`indexed`]).
impl TextSource for IndexDoc {
    fn word(&self, field: TextField, prefix: &str) -> Tri {
        let found = match field {
            TextField::Body => self.text.as_str().word(field, prefix) == Tri::Yes,
            TextField::Names => self.names.iter().any(|n| n.as_str().word(field, prefix) == Tri::Yes),
        };
        indexed(found, field, self.truncated, self.has_text)
    }

    fn phrase(&self, field: TextField, phrase: &[String], raw: &str) -> Tri {
        let found = match field {
            TextField::Body => self.text.as_str().phrase(field, phrase, raw) == Tri::Yes,
            TextField::Names => self.names.iter().any(|n| n.as_str().phrase(field, phrase, raw) == Tri::Yes),
        };
        indexed(found, field, self.truncated, self.has_text)
    }
}

/// The parts of a summary's text, in [`SummaryText`].
#[derive(Clone, Copy)]
enum Part {
    Subject = 0,
    From = 1,
    /// To and Cc.
    To = 2,
    Preview = 3,
}

const PARTS: [Part; 4] = [Part::Subject, Part::From, Part::To, Part::Preview];

/// A summary's text as search reads it: each part's words folded and joined by spaces, and the
/// subject and addresses lower-cased as written (for addresses and punctuation, found inside
/// words). Made once per message and kept by the caller: checking a word is then a scan of a
/// short string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SummaryText {
    folded: String,
    /// Where each part ends in `folded` (each starts where the one before ends).
    ends: [usize; 4],
    raw: String,
    /// Where the subject, From and To end in `raw`.
    raw_ends: [usize; 3],
}

impl SummaryText {
    pub fn of(m: &Summary) -> SummaryText {
        SummaryText::with_parts(m, [true; 4])
    }

    /// Only the parts `q` looks in, the others left empty: what a search not keeping it reads
    /// (folding the preview and every recipient for `from:` made such a search several times
    /// slower). Nothing at all for a query of flags, sizes and dates.
    pub fn for_query(m: &Summary, q: &Query) -> SummaryText {
        fn mark(t: &Term, want: &mut [bool; 4]) {
            match t {
                Term::Words(f, _) | Term::Phrase(f, _) => {
                    for p in places(f).0 {
                        if let Some(w) = want.get_mut(*p as usize) {
                            *w = true;
                        }
                    }
                }
                Term::AnyOf(any) => any.iter().for_each(|t| mark(t, want)),
                _ => {}
            }
        }
        let mut want = [false; 4];
        q.clauses.iter().for_each(|c| mark(&c.term, &mut want));
        if want == [false; 4] {
            return SummaryText::default();
        }
        SummaryText::with_parts(m, want)
    }

    fn with_parts(m: &Summary, want: [bool; 4]) -> SummaryText {
        let mut st = SummaryText::default();
        let wanted = |p: Part| want.get(p as usize).copied().unwrap_or(false);
        let from = if wanted(Part::From) { addr_text(std::slice::from_ref(&m.from)) } else { String::new() };
        let mut to = String::new();
        if wanted(Part::To) {
            to = addr_text(&m.to);
            to.push_str(&addr_text(&m.cc));
        }
        let subject = if wanted(Part::Subject) { m.subject.as_str() } else { "" };
        let preview = if wanted(Part::Preview) { m.preview.as_str() } else { "" };
        for (i, text) in [subject, &from, &to, preview].into_iter().enumerate() {
            let mut part = String::new();
            join(all_words(text), &mut part);
            st.folded.push_str(&part);
            if let Some(end) = st.ends.get_mut(i) {
                *end = st.folded.len();
            }
            if let Some(end) = st.raw_ends.get_mut(i) {
                st.raw.push_str(&text.to_lowercase());
                *end = st.raw.len();
            }
        }
        st
    }

    /// Bytes held, for a cache's budget.
    pub fn bytes(&self) -> usize {
        self.folded.len().saturating_add(self.raw.len()).saturating_add(std::mem::size_of::<SummaryText>())
    }

    fn part(&self, p: Part) -> &str {
        let i = p as usize;
        let start = i.checked_sub(1).and_then(|j| self.ends.get(j)).copied().unwrap_or(0);
        let end = self.ends.get(i).copied().unwrap_or(0);
        self.folded.get(start..end).unwrap_or("")
    }

    /// The part as written, lower-cased; `None` for the preview (not kept twice).
    fn raw_part(&self, p: Part) -> Option<&str> {
        let i = p as usize;
        let start = i.checked_sub(1).and_then(|j| self.raw_ends.get(j)).copied().unwrap_or(0);
        let end = self.raw_ends.get(i).copied()?;
        self.raw.get(start..end)
    }
}

/// What a query is checked against.
pub struct Doc<'a> {
    pub summary: &'a Summary,
    pub summary_text: &'a SummaryText,
    /// The user sent it a while ago and no answer has come ([`Term::Awaiting`]).
    pub awaiting: bool,
    /// Whether it has files, as its text was read (the index's, which leaves out pictures the
    /// text shows): what `has:attachment` asks, before the summary's own mark.
    pub files: Option<bool>,
    pub text: &'a dyn TextSource,
}

impl Query {
    /// Whether a message matches: yes, no, or can't tell yet (its text isn't known well enough).
    pub fn eval(&self, doc: &Doc<'_>) -> Tri {
        let mut out = Tri::Yes;
        // The summary's clauses first: a no there and the text isn't asked.
        for text in [false, true] {
            for c in self.clauses.iter().filter(|c| term_needs_text(&c.term) == text) {
                let t = clause(&c.term, doc);
                out = out.and(if c.negated { !t } else { t });
                if out == Tri::No {
                    return Tri::No;
                }
            }
        }
        out
    }

    /// Whether a message surely matches.
    pub fn matches(&self, doc: &Doc<'_>) -> bool {
        self.eval(doc) == Tri::Yes
    }

    /// Whether the query asks about a message's text or names (words anywhere, in the body, in
    /// the names): the summaries alone can't answer it.
    pub fn needs_text(&self) -> bool {
        self.clauses.iter().any(|c| term_needs_text(&c.term))
    }

    /// The folded words the text is asked for: where, the word, and whether it must be missing.
    /// Words anywhere are asked of the body and of the names.
    pub fn text_words(&self) -> Vec<(TextField, String, bool)> {
        fn term(t: &Term, negated: bool, out: &mut Vec<(TextField, String, bool)>) {
            let fields: &[TextField] = match t {
                Term::Words(f, _) | Term::Phrase(f, _) => match f {
                    Field::Any => &[TextField::Body, TextField::Names],
                    Field::Body => &[TextField::Body],
                    Field::Attachment => &[TextField::Names],
                    Field::From | Field::To | Field::Subject => &[],
                },
                Term::AnyOf(any) => {
                    any.iter().for_each(|t| term(t, negated, out));
                    &[]
                }
                _ => &[],
            };
            let words: Vec<Cow<'_, str>> = match t {
                Term::Words(_, ws) => ws.iter().map(|w| fold_word(w)).collect(),
                Term::Phrase(_, p) => tokens(p).collect(),
                _ => Vec::new(),
            };
            for f in fields {
                for w in &words {
                    let item = (*f, w.to_string(), negated);
                    if !out.contains(&item) {
                        out.push(item);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for c in &self.clauses {
            term(&c.term, c.negated, &mut out);
        }
        out
    }

    /// The folded words an index can narrow by: the words of positive text clauses.
    pub fn index_words(&self) -> Vec<String> {
        let mut v = Vec::new();
        for c in self.clauses.iter().filter(|c| !c.negated) {
            match &c.term {
                // An empty word would be a prefix of every one: the index would offer everything.
                Term::Words(_, w) => v.extend(w.iter().map(|w| fold_word(w).into_owned()).filter(|w| !w.is_empty())),
                Term::Phrase(_, p) => v.extend(tokens(p).map(Cow::into_owned)),
                _ => {}
            }
        }
        v
    }
}

fn term_needs_text(t: &Term) -> bool {
    match t {
        Term::Words(f, _) | Term::Phrase(f, _) => matches!(f, Field::Any | Field::Body | Field::Attachment),
        Term::AnyOf(any) => any.iter().any(term_needs_text),
        _ => false,
    }
}

fn clause(term: &Term, doc: &Doc<'_>) -> Tri {
    let m = doc.summary;
    match term {
        Term::HasAttachment(b) => Tri::from(doc.files.unwrap_or(m.has_attachments) == *b),
        Term::Unread(b) => Tri::from(!m.flags.seen == *b),
        Term::Flagged(b) => Tri::from(m.flags.flagged == *b),
        Term::Keyword(k) => Tri::from(m.flags.has(k) || m.flags.has(&crate::keyword_for(k))),
        Term::Important => Tri::from(m.importance > 0),
        Term::Awaiting => Tri::from(doc.awaiting),
        // A keyword not in the list shows as a category too (issue #28): any but a system one.
        Term::Categorized(ks) => {
            Tri::from(ks.iter().any(|k| m.flags.has(k)) || m.flags.keywords.iter().any(|k| !crate::organize::keywords::is_system_keyword(k)))
        }
        Term::Size(true, b) => Tri::from(u64::from(m.size) > *b),
        Term::Size(false, b) => Tri::from(u64::from(m.size) < *b),
        // `received:` (`after:`, `before:`) reads when it came, `sent:` and `date:` its Date:.
        Term::Date(spec) => spec.holds(if spec.sent { m.date } else { m.when() }).map_or(Tri::Unknown, Tri::from),
        Term::AnyOf(ts) => ts.iter().fold(Tri::No, |acc, t| acc.or(clause(t, doc))),
        Term::Words(f, ws) => ws.iter().fold(Tri::Yes, |acc, w| if acc == Tri::No { acc } else { acc.and(word(f, w, doc)) }),
        Term::Phrase(f, p) => phrase(f, p, doc),
    }
}

/// The summary's parts a field looks in, and the text's.
fn places(f: &Field) -> (&'static [Part], &'static [TextField]) {
    match f {
        Field::From => (&[Part::From], &[]),
        Field::To => (&[Part::To], &[]),
        Field::Subject => (&[Part::Subject], &[]),
        Field::Body => (&[Part::Preview], &[TextField::Body]),
        Field::Attachment => (&[], &[TextField::Names]),
        Field::Any => (&PARTS, &[TextField::Body, TextField::Names]),
    }
}

/// A word (as typed, lower-cased) in a field: a word there starting with it, or, for an address
/// or a domain, the text as written holding it.
fn word(f: &Field, w: &str, doc: &Doc<'_>) -> Tri {
    let folded = fold_word(w);
    let address = w.contains(['@', '.']);
    let (parts, fields) = places(f);
    let st = doc.summary_text;
    // The preview isn't kept as written: an address is looked for in it as `phrase` does.
    let raw_has = |p: Part| match st.raw_part(p) {
        Some(raw) => raw.contains(w),
        None => doc.summary.preview.to_lowercase().contains(w),
    };
    let in_summary = parts.iter().any(|p| has_prefix(st.part(*p), &folded) || (address && raw_has(*p)));
    if in_summary {
        return Tri::Yes;
    }
    fields.iter().fold(Tri::No, |acc, tf| if acc == Tri::Yes { acc } else { acc.or(doc.text.word(*tf, &folded)) })
}

/// A phrase (as typed, lower-cased) in a field: its folded words one after another, or, when it
/// has none (`@`), the text as written holding it.
fn phrase(f: &Field, p: &str, doc: &Doc<'_>) -> Tri {
    let words: Vec<String> = all_words(p).map(Cow::into_owned).collect();
    let joined = words.join(" ");
    let (parts, fields) = places(f);
    let st = doc.summary_text;
    let in_part = |part: Part| {
        if !words.is_empty() {
            return has_phrase(st.part(part), &joined);
        }
        match st.raw_part(part) {
            Some(raw) => raw.contains(p),
            None => doc.summary.preview.to_lowercase().contains(p),
        }
    };
    if parts.iter().any(|part| in_part(*part)) {
        return Tri::Yes;
    }
    fields.iter().fold(Tri::No, |acc, tf| if acc == Tri::Yes { acc } else { acc.or(doc.text.phrase(*tf, &words, p)) })
}
