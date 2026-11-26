//! `OR` and brackets in a query: `(from:ana is:unread) OR -(subject:x body:y)`.
//!
//! `OR` (in capitals, as Gmail and Outlook write it) joins the terms on each side into
//! [`Term::AnyOf`] and binds closer than the terms around it: `a b OR c` is a and (b or c). A
//! bracketed group is [`Term::All`], negated with `-(`. What doesn't pair up is read as before
//! brackets meant anything: a `)` with no group open stays part of its word, a group left open
//! ends with the query, an `OR` with nothing before it is the word "or", one with nothing after
//! it is dropped. Groups nest at most [`MAX_DEPTH`] deep; a bracket past that is part of its word.

use super::*;

/// The deepest groups nest (each one a level of recursion when a message is checked).
pub const MAX_DEPTH: usize = 8;

/// A token, or the brackets and `OR`s around the terms.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Piece {
    /// `(`, or `-(` (`negated`).
    Open {
        negated: bool,
    },
    Close,
    Or,
    Term(String),
}

/// The tokens with brackets peeled off them: `(from:ana` is an open bracket and `from:ana`,
/// `x))` is `x` and two closing ones while two groups are open. Brackets inside quotes stay.
pub(super) fn pieces(tokens: impl IntoIterator<Item = String>) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for tok in tokens {
        if tok == "OR" {
            out.push(Piece::Or);
            continue;
        }
        let mut rest = tok.as_str();
        loop {
            if depth >= MAX_DEPTH {
                break;
            }
            if let Some(r) = rest.strip_prefix('(') {
                out.push(Piece::Open { negated: false });
                rest = r;
            } else if let Some(r) = rest.strip_prefix("-(") {
                out.push(Piece::Open { negated: true });
                rest = r;
            } else {
                break;
            }
            depth += 1;
        }
        // Closing brackets at its end close the groups open, no more; one after an open quote
        // (`"smile :)`) is inside it.
        let mut closes = 0usize;
        while closes < depth && rest.ends_with(')') && rest.matches('"').count() % 2 == 0 {
            rest = rest.get(..rest.len() - 1).unwrap_or_default();
            closes += 1;
        }
        if !rest.is_empty() {
            out.push(Piece::Term(rest.to_string()));
        }
        for _ in 0..closes {
            out.push(Piece::Close);
        }
        depth -= closes;
    }
    out
}

/// A group being read: its clauses so far, and whether an `OR` waits for the next one.
struct Frame {
    negated: bool,
    clauses: Vec<Clause>,
    or_waiting: bool,
}

impl Frame {
    fn new(negated: bool) -> Frame {
        Frame { negated, clauses: Vec::new(), or_waiting: false }
    }

    /// A clause in: joined to the one before when an `OR` came between them.
    fn push(&mut self, c: Clause) {
        let waiting = std::mem::take(&mut self.or_waiting);
        match self.clauses.last_mut() {
            Some(prev) if waiting => either(prev, c),
            _ => self.clauses.push(c),
        }
    }

    /// The group as one clause; none when it's empty, the one inside when it holds one.
    fn into_clause(self) -> Option<Clause> {
        let Frame { negated, clauses, .. } = self;
        let mut clauses = flattened(clauses);
        match clauses.len() {
            0 => None,
            1 => clauses.pop().map(|c| Clause { negated: c.negated != negated, term: c.term }),
            _ => Some(Clause { term: Term::All(clauses), negated }),
        }
    }
}

/// A group's clauses with the groups in them that are plain "and" spread out: `a (b c)` is
/// `a b c`, which a server can be asked (it isn't asked groups). Done once the group is read,
/// when no `OR` can join one of them any more.
fn flattened(clauses: Vec<Clause>) -> Vec<Clause> {
    let mut out = Vec::with_capacity(clauses.len());
    for c in clauses {
        match c {
            Clause { term: Term::All(inner), negated: false } => out.extend(inner),
            c => out.push(c),
        }
    }
    out
}

/// `prev OR c`, in `prev`'s place: an `AnyOf` grows (OR doesn't care how its terms are
/// grouped), anything else becomes one.
fn either(prev: &mut Clause, c: Clause) {
    let item = |c: Clause| if c.negated { Term::All(vec![c]) } else { c.term };
    if let (false, Term::AnyOf(any)) = (prev.negated, &mut prev.term) {
        any.push(item(c));
        return;
    }
    let first = std::mem::replace(prev, Clause { term: Term::AnyOf(Vec::new()), negated: false });
    prev.term = Term::AnyOf(vec![item(first), item(c)]);
}

/// The query's clauses from its pieces; `term` reads a token (none when it's left out).
pub(super) fn build(pieces: Vec<Piece>, term: &mut dyn FnMut(String) -> Option<Clause>) -> Vec<Clause> {
    let mut stack = vec![Frame::new(false)];
    for p in pieces {
        match p {
            Piece::Term(tok) => {
                if let (Some(c), Some(top)) = (term(tok), stack.last_mut()) {
                    top.push(c);
                }
            }
            Piece::Or => {
                if let Some(top) = stack.last_mut() {
                    if top.clauses.is_empty() {
                        // Nothing to join it to: the word it was before.
                        if let Some(c) = term("OR".to_string()) {
                            top.push(c);
                        }
                    } else {
                        top.or_waiting = true;
                    }
                }
            }
            Piece::Open { negated } => stack.push(Frame::new(negated)),
            Piece::Close => close(&mut stack),
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().map(|f| flattened(f.clauses)).unwrap_or_default()
}

/// The innermost group ends: it goes into the one around it.
fn close(stack: &mut Vec<Frame>) {
    if stack.len() < 2 {
        return;
    }
    // An empty group leaves an `OR` before it waiting for what follows.
    if let Some(group) = stack.pop().and_then(Frame::into_clause)
        && let Some(top) = stack.last_mut()
    {
        top.push(group);
    }
}

#[cfg(test)]
mod tests;
