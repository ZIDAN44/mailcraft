//! Search: the query language of the search box and an inverted index for fast offline search.
//!
//! The syntax follows the familiar mail-client form: bare words match anywhere (as word
//! prefixes, so `proj` finds "project"), `"quoted phrases"` match as written, and keywords narrow
//! the field: `from:`, `to:` (also `cc:`), `subject:`, `body:`, `attachment:` (the files'
//! names), `has:attachment` / `hasattachments:yes` (a file that isn't a picture shown in the
//! text), `is:unread` / `is:read`, `is:flagged`, `is:awaiting` (sent and not answered yet),
//! `larger:` / `smaller:` (a size: `100KB`, `2MB`, bytes alone), and dates: `received:` (also
//! `sent:`, `date:`) a day, month or year, `today`, `"last week"`, a range `2024-01-01..2024-03-31`
//! or `>=2024-03-01`; `after:X` is from X's start on and `before:X` before it ([`dates`]). A leading
//! `-` negates a term. `from:` and `to:` take a list, `from:ana,jun`, matching any of them (terms
//! otherwise must all match). Words are read [folded](fold): `resume` finds "Résumé".
//!
//! A query is checked against a message with [`Query::eval`], which answers yes, no, or (when the
//! message's text isn't known well enough) "can't tell" ([`Tri`]). [`Index`] maps every word of
//! every message to the messages containing it (a `BTreeMap`, so a prefix is one range scan): a
//! query asks it for candidates and then checks each, so it may over-approximate but never misses.

use crate::{EmailAddress, Summary};

mod advanced;
pub mod dates;
pub mod fold;
mod fold_table;
mod index;
mod matching;
mod parse;
#[cfg(test)]
mod tests;

pub use advanced::Advanced;
pub use dates::{DateBound, DatePoint, DateSpec, Edge, Relative};
pub use fold::{MAX_TOKEN_CHARS, TOKENIZER_VERSION, fold_text, fold_word, tokens};
pub use index::Index;
pub use matching::{Doc, SummaryText, TextField, TextSource, Tri, indexed};
use parse::split_tokens;
pub(crate) use parse::split_tokens_with;

/// Lower-cased words of a text (UAX #29 word boundaries), as typed: what a server is asked for.
/// Search itself reads them [folded](fold_word).
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    unicode_segmentation::UnicodeSegmentation::unicode_words(text).map(str::to_lowercase)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Any,
    From,
    To,
    Subject,
    Body,
    /// The files' names (`attachment:`, `attachments:`).
    Attachment,
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
    /// keywords), or any keyword that isn't a [system one](crate::organize::keywords::is_system_keyword).
    Categorized(Vec<String>),
    /// Bigger than this many bytes (`larger:`), or (`false`) smaller (`smaller:`).
    Size(bool, u64),
    /// Any one of these (`from:ana,jun`).
    AnyOf(Vec<Term>),
    /// Mail the user sent that nobody has answered yet (`is:awaiting`): the caller works out
    /// which, from every conversation ([`Doc::awaiting`]).
    Awaiting,
    /// Dated within a period (`received:2024`, `after:`, `before:`): named until
    /// [resolved](Query::resolve) against the clock, and matching nothing for sure before.
    Date(DateSpec),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    pub term: Term,
    pub negated: bool,
}

/// Something typed that couldn't be read, and was left out of the search (the search box says
/// so, rather than looking for it as words).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A date that isn't one (`received:2024-13`): the term as typed.
    Date(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub clauses: Vec<Clause>,
    /// What was left out, in the order typed.
    pub problems: Vec<Problem>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
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

/// Most characters of a message's text kept for search; a longer one is cut there
/// ([`IndexDoc::truncated`]).
pub const MAX_INDEX_CHARS: usize = 1_000_000;

/// What search keeps of a downloaded message: its text and its files' names, read without
/// decoding the files themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexDoc {
    /// The text the reader sees (HTML as text), with the subjects and text of attached messages;
    /// at most [`MAX_INDEX_CHARS`] characters.
    pub text: String,
    /// The files' names and the attached messages' (`<subject>.eml`); pictures the text shows
    /// aren't files and aren't named.
    pub names: Vec<String>,
    /// How many files the message carries (`has:attachment`).
    pub files: u32,
    /// Something was left out: text past the limit, parts nested too deep, too many parts.
    pub truncated: bool,
    /// The message has text of its own (or a `winmail.dat`'s), not only files.
    pub has_text: bool,
}

impl IndexDoc {
    /// The text and then the names, a line each: what search reads as one text.
    pub fn joined(&self) -> String {
        let mut out = String::with_capacity(self.text.len() + self.names.iter().map(|n| n.len() + 1).sum::<usize>());
        out.push_str(&self.text);
        for n in &self.names {
            out.push('\n');
            out.push_str(n);
        }
        out
    }
}
