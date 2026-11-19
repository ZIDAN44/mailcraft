//! Search: the query language of the search box, and the words a message is indexed by.
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
//! message's text isn't known well enough) "can't tell" ([`Tri`]). The words a message is indexed
//! by are its [`placed_tokens`]; the index itself, on disk, is the store's (`mailcraft_store::search`).

use crate::Summary;

mod advanced;
pub mod dates;
pub mod fold;
mod fold_table;
mod hits;
mod matching;
mod parse;
#[cfg(test)]
mod tests;

pub use advanced::Advanced;
pub use dates::{DateBound, DatePoint, DateSpec, Edge, Relative};
pub use fold::{MAX_TOKEN_CHARS, Placed, TOKENIZER_VERSION, fold_all, fold_word, folded_has_phrase, parts, placed_tokens, tokens};
pub use hits::{MAX_HITS, hits};
pub use matching::{Doc, DocText, SummaryText, TextField, TextSource, Tri, WholeText, indexed};
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
    /// Its follow-up marked complete and not flagged again since (`is:complete`:
    /// [`Flags::follow_up_complete`](crate::Flags::follow_up_complete)).
    Complete,
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

impl Term {
    /// Whether it looks in the files' names (`attachment:`), alone or among others. A server can
    /// only ask the whole text for those, which finds more: asked negated, it would find too
    /// little, so a server is asked the rest and what it finds is checked here.
    pub fn asks_names(&self) -> bool {
        match self {
            Term::Words(f, _) | Term::Phrase(f, _) => *f == Field::Attachment,
            Term::AnyOf(any) => any.iter().any(Term::asks_names),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    pub term: Term,
    pub negated: bool,
}

/// Something typed that couldn't be read (the search box says so): left out of the search, or,
/// when it wasn't meant as one (`before: launch`), looked for as the words typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A date that isn't one (`received:2024-13`, `after:party`): the term as typed.
    Date(String),
    /// A size, or a yes or no, that isn't one (`larger:abc`, `isread:x`): the term as typed.
    Value(String),
    /// More terms than a search takes ([`MAX_TERMS`]), or more names in a list than it takes
    /// ([`MAX_LIST`]): the rest were left out.
    TooLong,
}

/// The most terms a search takes; those past them are left out ([`Problem::TooLong`]).
pub const MAX_TERMS: usize = 64;
/// The most names a list takes (`from:ana,jun,…`).
pub const MAX_LIST: usize = 50;

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
    /// Some of its parts weren't read (nested too deep, too many, too big): `files` may miss
    /// some. Only text cut at the limit leaves `truncated` without this, and the count whole.
    pub parts_cut: bool,
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
