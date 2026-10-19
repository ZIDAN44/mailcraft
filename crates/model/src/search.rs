//! Search: the query language of the search box and an inverted index for fast offline search.
//!
//! The syntax follows the familiar mail-client form: bare words match anywhere (as word
//! prefixes, so `proj` finds "project"), `"quoted phrases"` match as written, and keywords narrow
//! the field: `from:`, `to:` (also `cc:`), `subject:`, `body:`, `has:attachment` /
//! `hasattachments:yes`, `is:unread` / `is:read`, `is:flagged`. A leading `-` negates a term.
//!
//! [`Index`] maps every word of every message to the messages containing it (a `BTreeMap`, so a
//! prefix is one range scan). A query asks the index for candidates and then checks each
//! candidate with [`Query::matches`], so the index may over-approximate but never misses.

use std::collections::{BTreeMap, BTreeSet};

use unicode_segmentation::UnicodeSegmentation;

use crate::{EmailAddress, Summary};

/// Lower-cased words of a text (UAX #29 word boundaries).
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.unicode_words().map(str::to_lowercase)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Any,
    From,
    To,
    Subject,
    Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    /// Words that must all occur as word prefixes in the field.
    Words(Field, Vec<String>),
    /// A phrase that must occur as written (case-insensitive).
    Phrase(Field, String),
    HasAttachment(bool),
    Unread(bool),
    Flagged(bool),
    /// Carries a keyword (`keyword:x`; `category:` names are mapped to keywords by the caller,
    /// and match the keyword spelled as the name too). `is:pinned` is the `$Pinned` keyword.
    Keyword(String),
    /// Marked high importance by its sender (`is:important`).
    Important,
    /// Carries one of these keywords (`is:categorized`: the caller fills in its categories'
    /// keywords; left empty, any keyword that isn't a system one, `$…`, counts).
    Categorized(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    pub term: Term,
    pub negated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub clauses: Vec<Clause>,
}

/// What a query is checked against: the summary and, when cached, the body text.
pub struct Doc<'a> {
    pub summary: &'a Summary,
    pub body: Option<&'a str>,
    /// It came from an [`Index`] search for the query's words: words looked for anywhere are known
    /// to be there, without its body (reading a big mailbox's bodies again for every search was
    /// most of its time).
    pub indexed: bool,
}

fn split_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                // Whitespace after a bare `field:` is skipped, joining the value to the field.
                if !cur.is_empty() && !field_waiting(&cur) {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Field prefixes the parser recognises.
const FIELDS: &[&str] = &[
    "from",
    "to",
    "cc",
    "subject",
    "body",
    "has",
    "hasattachment",
    "hasattachments",
    "is",
    "isread",
    "isflagged",
    "keyword",
    "category",
    "label",
    "tag",
];

/// True for a bare `field:` (optionally negated) still missing its value.
fn field_waiting(tok: &str) -> bool {
    let name = tok.strip_prefix('-').unwrap_or(tok);
    name.strip_suffix(':').is_some_and(|k| FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)))
}

fn yes_no(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn text_term(field: Field, value: &str) -> Option<Term> {
    if let Some(inner) = value.strip_prefix('"') {
        let phrase = inner.strip_suffix('"').unwrap_or(inner).trim().to_lowercase();
        return (!phrase.is_empty()).then_some(Term::Phrase(field, phrase));
    }
    let w: Vec<String> = words(value).collect();
    if w.is_empty() {
        // Punctuation-only input (`@`, `.`) is still a substring to look for.
        let v = value.trim().to_lowercase();
        return (!v.is_empty()).then_some(Term::Phrase(field, v));
    }
    Some(Term::Words(field, w))
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut clauses = Vec::new();
        for tok in split_tokens(input).into_iter().take(64) {
            let (negated, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let term = match tok.split_once(':') {
                Some((k, v)) if !v.is_empty() && !k.starts_with('"') => match k.to_ascii_lowercase().as_str() {
                    "from" => text_term(Field::From, v),
                    "to" | "cc" => text_term(Field::To, v),
                    "subject" => text_term(Field::Subject, v),
                    "body" => text_term(Field::Body, v),
                    "has" if v.eq_ignore_ascii_case("attachment") || v.eq_ignore_ascii_case("attachments") => Some(Term::HasAttachment(true)),
                    "hasattachment" | "hasattachments" => yes_no(v).map(Term::HasAttachment),
                    "is" => match v.to_ascii_lowercase().as_str() {
                        "unread" => Some(Term::Unread(true)),
                        "read" => Some(Term::Unread(false)),
                        "flagged" => Some(Term::Flagged(true)),
                        "unflagged" => Some(Term::Flagged(false)),
                        "pinned" => Some(Term::Keyword(crate::PINNED.to_string())),
                        "important" => Some(Term::Important),
                        "categorized" | "categorised" => Some(Term::Categorized(Vec::new())),
                        _ => text_term(Field::Any, &tok),
                    },
                    "isread" => yes_no(v).map(|b| Term::Unread(!b)),
                    "isflagged" => yes_no(v).map(Term::Flagged),
                    "keyword" | "category" | "label" | "tag" => Some(Term::Keyword(v.trim_matches('"').to_string())),
                    _ => text_term(Field::Any, &tok),
                },
                _ => text_term(Field::Any, &tok),
            };
            if let Some(term) = term {
                clauses.push(Clause { term, negated });
            }
        }
        Query { clauses }
    }

    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    /// The words the index can narrow by: words of positive text clauses.
    pub fn index_words(&self) -> Vec<String> {
        let mut v = Vec::new();
        for c in self.clauses.iter().filter(|c| !c.negated) {
            match &c.term {
                Term::Words(_, w) => v.extend(w.iter().cloned()),
                Term::Phrase(_, p) => v.extend(words(p)),
                _ => {}
            }
        }
        v
    }

    pub fn matches(&self, doc: &Doc<'_>) -> bool {
        self.clauses.iter().all(|c| clause_matches(&c.term, doc) != c.negated)
    }

    /// Whether checking a message found through the index needs its body: a phrase, words in the
    /// body alone, words that must be missing, or addresses (matched inside words). Plain words
    /// looked for anywhere are settled by the index.
    pub fn needs_body(&self) -> bool {
        self.clauses.iter().any(|c| match &c.term {
            Term::Phrase(Field::Any | Field::Body, _) | Term::Words(Field::Body, _) => true,
            Term::Words(Field::Any, ws) => c.negated || ws.iter().any(|w| w.contains(['@', '.'])),
            _ => false,
        })
    }
}

fn addr_text(list: &[EmailAddress]) -> String {
    let mut s = String::new();
    for a in list {
        s.push_str(&a.name);
        s.push(' ');
        s.push_str(&a.email);
        s.push(' ');
    }
    s
}

fn field_text(field: &Field, doc: &Doc<'_>) -> String {
    let m = doc.summary;
    match field {
        Field::From => addr_text(std::slice::from_ref(&m.from)),
        Field::To => {
            let mut s = addr_text(&m.to);
            s.push_str(&addr_text(&m.cc));
            s
        }
        Field::Subject => m.subject.clone(),
        Field::Body => doc.body.unwrap_or(&m.preview).to_string(),
        Field::Any => {
            let mut s = String::with_capacity(256);
            s.push_str(&m.subject);
            s.push(' ');
            s.push_str(&addr_text(std::slice::from_ref(&m.from)));
            s.push_str(&addr_text(&m.to));
            s.push_str(&addr_text(&m.cc));
            s.push_str(doc.body.unwrap_or(&m.preview));
            s
        }
    }
}

fn clause_matches(term: &Term, doc: &Doc<'_>) -> bool {
    let m = doc.summary;
    match term {
        Term::HasAttachment(b) => m.has_attachments == *b,
        Term::Unread(b) => !m.flags.seen == *b,
        Term::Flagged(b) => m.flags.flagged == *b,
        Term::Keyword(k) => m.flags.has(k) || m.flags.has(&crate::keyword_for(k)),
        Term::Important => m.importance > 0,
        Term::Categorized(ks) if ks.is_empty() => m.flags.keywords.iter().any(|k| !k.starts_with('$')),
        Term::Categorized(ks) => ks.iter().any(|k| m.flags.has(k)),
        Term::Phrase(f, p) => field_text(f, doc).to_lowercase().contains(p.as_str()),
        // Found through the index for these words: they're there (summary or body).
        Term::Words(Field::Any, ws) if doc.indexed && doc.body.is_none() && ws.iter().all(|w| !w.contains(['@', '.'])) => true,
        Term::Words(f, ws) => {
            let text = field_text(f, doc);
            let have: Vec<String> = words(&text).collect();
            let lower = text.to_lowercase();
            ws.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str())) || (w.contains(['@', '.']) && lower.contains(w.as_str())))
        }
    }
}

/// Words → documents. Document ids are the caller's (positions in its message table).
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each word's documents, sorted (an array: a set per word took several times the memory,
    /// gigabytes for a big mailbox).
    postings: BTreeMap<String, Vec<u32>>,
    docs: BTreeSet<u32>,
}

/// The text a summary contributes to the index.
pub fn summary_text(m: &Summary) -> String {
    let mut s = String::with_capacity(128 + m.preview.len());
    s.push_str(&m.subject);
    s.push(' ');
    s.push_str(&addr_text(std::slice::from_ref(&m.from)));
    s.push_str(&addr_text(&m.to));
    s.push_str(&addr_text(&m.cc));
    s.push_str(&m.preview);
    s
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Add a document's text (call again with more text, e.g. the body once fetched).
    pub fn add(&mut self, doc: u32, text: &str) {
        self.docs.insert(doc);
        for w in words(text) {
            // Very long "words" (base64 debris) only bloat the index.
            if w.chars().count() <= 40 {
                let list = self.postings.entry(w).or_default();
                // Documents mostly come in order: appended; one added later goes in its place.
                match list.last() {
                    Some(&last) if last == doc => {}
                    Some(&last) if last > doc => {
                        if let Err(at) = list.binary_search(&doc) {
                            list.insert(at, doc);
                        }
                    }
                    _ => list.push(doc),
                }
            }
        }
    }

    /// Documents containing a word starting with `prefix`.
    pub fn prefix(&self, prefix: &str) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        for (_, docs) in self.postings.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)) {
            out.extend(docs.iter().copied());
        }
        out
    }

    /// Candidates for a query: documents with every index word as a prefix of some word, or all
    /// documents when the query has no words (flag-only queries).
    pub fn candidates(&self, q: &Query) -> BTreeSet<u32> {
        let ws = q.index_words();
        if ws.is_empty() {
            return self.docs.clone();
        }
        let mut acc: Option<BTreeSet<u32>> = None;
        for w in ws {
            let hits = self.prefix(&w);
            acc = Some(match acc {
                None => hits,
                Some(a) => a.intersection(&hits).copied().collect(),
            });
            if acc.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        acc.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmailAddress, Flags};

    fn sample() -> Summary {
        Summary {
            subject: "Project kickoff: storyboards".into(),
            from: EmailAddress::new("Zoe Park", "zoe@studio.example"),
            to: vec![EmailAddress::new("Ann", "ann@studio.example")],
            preview: "Bring the sketches for the opening scene".into(),
            has_attachments: true,
            flags: Flags { seen: false, flagged: true, ..Default::default() },
            ..Default::default()
        }
    }

    fn q(s: &str, m: &Summary) -> bool {
        Query::parse(s).matches(&Doc { summary: m, body: None, indexed: false })
    }

    /// Whitespace between `field:` and its value is tolerated.
    #[test]
    fn whitespace_between_field_and_value() {
        let m = sample();
        assert_eq!(Query::parse("from: zoe").clauses, Query::parse("from:zoe").clauses);
        assert_eq!(Query::parse("FROM:   \"zoe park\"  -is:  read").clauses, Query::parse("from:\"zoe park\" -is:read").clauses);
        assert!(q("from: zoe", &m) && !q("from: ann", &m) && q("to: ann", &m) && q("-is: read", &m));
        // Unknown prefixes stay plain words, and so does a trailing `from:`.
        assert_eq!(Query::parse("note: zoe").clauses.len(), 2);
        assert_eq!(Query::parse("zoe from:").clauses.len(), 2);
        assert!(Query::parse("from:\t\n").clauses.len() == 1);
    }

    #[test]
    fn parsing() {
        let p = Query::parse("from:zoe \"opening scene\" -is:read has:attachment");
        assert_eq!(p.clauses.len(), 4);
        assert_eq!(p.clauses[0].term, Term::Words(Field::From, vec!["zoe".into()]));
        assert_eq!(p.clauses[1].term, Term::Phrase(Field::Any, "opening scene".into()));
        assert!(p.clauses[2].negated);
        assert!(Query::parse("   ").is_empty());
        assert_eq!(Query::parse("subject:\"a b\"").clauses[0].term, Term::Phrase(Field::Subject, "a b".into()));
    }

    #[test]
    fn matching() {
        let m = sample();
        assert!(q("proj", &m));
        assert!(q("from:zoe", &m));
        assert!(!q("from:ann", &m));
        assert!(q("to:ann", &m));
        assert!(q("subject:kickoff is:unread is:flagged", &m));
        assert!(q("hasattachments:yes", &m));
        assert!(!q("is:read", &m));
        assert!(q("\"opening scene\"", &m));
        assert!(!q("\"scene opening\"", &m));
        assert!(q("-from:ann", &m));
        assert!(q("zoe@studio.example", &m));
        assert!(q("body:sketches", &m));
        assert!(!q("is:important", &m) && !q("is:categorized", &m));
        let mut m2 = m.clone();
        m2.importance = 1;
        m2.flags.keywords = vec!["$Pinned".into(), "work".into()];
        assert!(q("is:important is:categorized", &m2));
        let label = Query { clauses: vec![Clause { term: Term::Categorized(vec!["$label1".into()]), negated: false }] };
        assert!(!label.matches(&Doc { summary: &m2, body: None, indexed: false }));
        assert!(Query::parse("").matches(&Doc { summary: &m, body: None, indexed: false }));
    }

    #[test]
    fn index_narrows_without_missing() {
        let a = sample();
        let mut b = sample();
        b.subject = "Invoice October".into();
        b.preview = "Payment due".into();
        let mut ix = Index::new();
        ix.add(0, &summary_text(&a));
        ix.add(1, &summary_text(&b));
        assert_eq!(ix.candidates(&Query::parse("invo")), BTreeSet::from([1]));
        assert_eq!(ix.candidates(&Query::parse("zoe proj")), BTreeSet::from([0]));
        assert_eq!(ix.candidates(&Query::parse("is:unread")).len(), 2);
        assert!(ix.candidates(&Query::parse("nothing-here")).is_empty());
        ix.add(1, "late body text");
        assert_eq!(ix.candidates(&Query::parse("late")), BTreeSet::from([1]));
    }

    /// The index keeps each word's documents sorted and once, whatever order they come in; plain
    /// words found through it need no body, while phrases, missing words and addresses do.
    #[test]
    fn the_index_settles_plain_words() {
        let mut ix = Index::new();
        ix.add(5, "storyboard review");
        ix.add(2, "storyboard storyboard");
        ix.add(9, "budget");
        ix.add(5, "storyboard again");
        assert_eq!(ix.prefix("story").into_iter().collect::<Vec<_>>(), [2, 5]);
        let q = Query::parse("storyboard review");
        assert_eq!(ix.candidates(&q).into_iter().collect::<Vec<_>>(), [5]);
        assert!(!q.needs_body());
        let m = Summary { subject: "Weekly".into(), ..Default::default() };
        assert!(q.matches(&Doc { summary: &m, body: None, indexed: true }), "the index found both words");
        assert!(!q.matches(&Doc { summary: &m, body: None, indexed: false }), "without it, the words aren't in the summary");
        for needs in ["\"storyboard review\"", "-budget storyboard", "body:storyboard", "zoe@studio.example"] {
            assert!(Query::parse(needs).needs_body(), "{needs}");
        }
        for not in ["from:zoe storyboard", "is:unread budget", "subject:weekly"] {
            assert!(!Query::parse(not).needs_body(), "{not}");
        }
    }
}
