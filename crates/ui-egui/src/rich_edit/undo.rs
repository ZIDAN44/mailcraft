//! Undo and Redo in the message editor: the text and its look as they were. The newest step is
//! kept whole, each older one as what differs from the step after it (each step was a whole
//! copy: a 300 KB message took some 180 MB at 200 steps), and the oldest go when the steps take
//! more room than [`UNDO_BYTES`].

use std::collections::VecDeque;

use super::*;

/// Most undo steps kept.
pub(super) const UNDO_MAX: usize = 200;
/// Most room the steps take (bytes; the newest is kept whatever its size).
pub(super) const UNDO_BYTES: usize = 32 << 20;

/// The text and its look as they were, and where the cursor was.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Snap {
    pub text: String,
    pub styles: Styles,
    pub cursor: usize,
}

impl Snap {
    pub(super) fn of(c: &Compose, cursor: usize) -> Snap {
        Snap { text: c.body.clone(), styles: c.styles.clone(), cursor }
    }

    fn bytes(&self) -> usize {
        self.text.len()
            + std::mem::size_of_val(self.styles.chars.as_slice())
            + palette_bytes(&self.styles.palette)
            + std::mem::size_of_val(self.styles.paras.as_slice())
    }
}

/// Steps to go back (or forward) to, the newest last.
#[derive(Default)]
pub(super) struct History {
    steps: VecDeque<Kept>,
    bytes: usize,
}

/// A step: whole (the newest), or what differs in it from the step after it.
enum Kept {
    Whole(Snap),
    Diff(Diff),
}

impl Kept {
    fn bytes(&self) -> usize {
        match self {
            Kept::Whole(s) => s.bytes(),
            Kept::Diff(d) => d.bytes(),
        }
    }
}

impl History {
    pub(super) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.steps.len()
    }

    /// The room the steps take (bytes).
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn clear(&mut self) {
        self.steps.clear();
        self.bytes = 0;
    }

    /// Keep `s` as the newest step; the one that was is kept as what differs from it. The
    /// oldest go past [`UNDO_MAX`] steps or [`UNDO_BYTES`].
    pub(super) fn push(&mut self, s: Snap) {
        if let Some(top) = self.steps.pop_back() {
            self.bytes = self.bytes.saturating_sub(top.bytes());
            let kept = match top {
                Kept::Whole(top) => Kept::Diff(Diff::between(&s, &top)),
                diff => diff,
            };
            self.bytes = self.bytes.saturating_add(kept.bytes());
            self.steps.push_back(kept);
        }
        self.bytes = self.bytes.saturating_add(s.bytes());
        self.steps.push_back(Kept::Whole(s));
        while self.steps.len() > UNDO_MAX || (self.bytes > UNDO_BYTES && self.steps.len() > 1) {
            if let Some(old) = self.steps.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.bytes());
            }
        }
    }

    /// The newest step, taken off; the one before it is whole again.
    pub(super) fn pop(&mut self) -> Option<Snap> {
        let Some(Kept::Whole(top)) = self.steps.pop_back() else {
            // (Never so: the newest is kept whole.)
            self.clear();
            return None;
        };
        self.bytes = self.bytes.saturating_sub(top.bytes());
        if let Some(Kept::Diff(d)) = self.steps.pop_back() {
            self.bytes = self.bytes.saturating_sub(d.bytes());
            match d.apply(&top) {
                Some(before) => {
                    self.bytes = self.bytes.saturating_add(before.bytes());
                    self.steps.push_back(Kept::Whole(before));
                }
                // (Never so: a step's difference is from the one after it.) What's older is
                // forgotten rather than put back wrong.
                None => self.clear(),
            }
        }
        Some(top)
    }
}

/// What differs in a step from the step after it: in each part, what's put in place of a run of
/// the newer step's.
struct Diff {
    text: Splice<String>,
    chars: Splice<Vec<u16>>,
    palette: Splice<Vec<Style>>,
    paras: Splice<Vec<rich::Para>>,
    cursor: usize,
}

impl Diff {
    /// `old` as what differs in it from `new`.
    fn between(new: &Snap, old: &Snap) -> Diff {
        Diff {
            text: Splice::text(&new.text, &old.text),
            chars: Splice::list(&new.styles.chars, &old.styles.chars),
            palette: Splice::list(&new.styles.palette, &old.styles.palette),
            paras: Splice::list(&new.styles.paras, &old.styles.paras),
            cursor: old.cursor,
        }
    }

    /// The step this was, from the one after it (`None` if it isn't that one's).
    fn apply(&self, new: &Snap) -> Option<Snap> {
        let text = self.text.onto_text(&new.text)?;
        let chars = self.chars.onto_list(&new.styles.chars)?;
        let palette = self.palette.onto_list(&new.styles.palette)?;
        let paras = self.paras.onto_list(&new.styles.paras)?;
        Some(Snap { text, styles: Styles { chars, palette, paras }, cursor: self.cursor })
    }

    fn bytes(&self) -> usize {
        self.text.with.len()
            + std::mem::size_of_val(self.chars.with.as_slice())
            + palette_bytes(&self.palette.with)
            + std::mem::size_of_val(self.paras.with.as_slice())
    }
}

/// `with` in place of `at..at + len` of a newer one.
struct Splice<W> {
    at: usize,
    len: usize,
    with: W,
}

/// How long the same start and end of `a` and `b` are, not overlapping (compared a block at a
/// time first: a step is taken on every pause in typing, and a long message's text compared one
/// letter at a time took a millisecond).
fn same_ends<T: PartialEq>(a: &[T], b: &[T]) -> (usize, usize) {
    const BLOCK: usize = 256;
    let (la, lb) = (a.len(), b.len());
    let shorter = la.min(lb);
    let mut prefix = 0;
    while prefix + BLOCK <= shorter && a.get(prefix..prefix + BLOCK) == b.get(prefix..prefix + BLOCK) {
        prefix += BLOCK;
    }
    prefix += a.iter().skip(prefix).zip(b.iter().skip(prefix)).take_while(|(x, y)| x == y).count();
    let room = shorter - prefix;
    let mut suffix = 0;
    while suffix + BLOCK <= room && a.get(la - suffix - BLOCK..la - suffix) == b.get(lb - suffix - BLOCK..lb - suffix) {
        suffix += BLOCK;
    }
    suffix += a.iter().rev().skip(suffix).zip(b.iter().rev().skip(suffix)).take(room - suffix).take_while(|(x, y)| x == y).count();
    (prefix, suffix)
}

impl<T: Clone + PartialEq> Splice<Vec<T>> {
    /// `old` as what's put in place of the run that differs in `new`.
    fn list(new: &[T], old: &[T]) -> Self {
        let (prefix, suffix) = same_ends(new, old);
        let with = old.get(prefix..old.len().saturating_sub(suffix)).unwrap_or_default().to_vec();
        Splice { at: prefix, len: new.len().saturating_sub(suffix).saturating_sub(prefix), with }
    }

    /// The older list, from `new`.
    fn onto_list(&self, new: &[T]) -> Option<Vec<T>> {
        let (head, tail) = (new.get(..self.at)?, new.get(self.at.checked_add(self.len)?..)?);
        let mut out = Vec::with_capacity(head.len() + self.with.len() + tail.len());
        out.extend_from_slice(head);
        out.extend_from_slice(&self.with);
        out.extend_from_slice(tail);
        Some(out)
    }
}

impl Splice<String> {
    /// As [`Splice::list`] on the texts' bytes, the run starting and ending between characters.
    fn text(new: &str, old: &str) -> Self {
        let (mut prefix, mut suffix) = same_ends(new.as_bytes(), old.as_bytes());
        while !(new.is_char_boundary(prefix) && old.is_char_boundary(prefix)) {
            prefix = prefix.saturating_sub(1);
        }
        let ends = |suffix: usize| (new.len().saturating_sub(suffix), old.len().saturating_sub(suffix));
        while suffix > 0 && !(new.is_char_boundary(ends(suffix).0) && old.is_char_boundary(ends(suffix).1)) {
            suffix -= 1;
        }
        let (new_end, old_end) = ends(suffix);
        let with = old.get(prefix..old_end.max(prefix)).unwrap_or_default().to_string();
        Splice { at: prefix, len: new_end.saturating_sub(prefix), with }
    }

    /// The older text, from `new`.
    fn onto_text(&self, new: &str) -> Option<String> {
        let (head, tail) = (new.get(..self.at)?, new.get(self.at.checked_add(self.len)?..)?);
        let mut out = String::with_capacity(head.len() + self.with.len() + tail.len());
        out.push_str(head);
        out.push_str(&self.with);
        out.push_str(tail);
        Some(out)
    }
}

/// The room a palette takes (bytes): its looks, and the names in them.
fn palette_bytes(p: &[Style]) -> usize {
    let names = |s: &Style| [&s.font, &s.link, &s.picture].iter().map(|x| x.as_ref().map_or(0, String::len)).sum::<usize>();
    std::mem::size_of_val(p) + p.iter().map(names).sum::<usize>()
}
