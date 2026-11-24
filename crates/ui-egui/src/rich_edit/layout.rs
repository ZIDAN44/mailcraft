//! The message editor's text laid out: each run in its style, pictures, lists' indents, alignment.

use super::*;

/// A look as text formatting (the person's colours adjusted to stay readable on the page). A
/// typeface of the computer's is drawn in it where the computer's fonts are offered (issue
/// #166: it was sent with the message, and shown in the editor's own font): the look's own, else
/// the message's (`base`, the font chosen for new messages); so is its size (`size`, issue #267).
pub(super) fn format(s: &Style, base: Option<&str>, size: u8, t: &Tokens) -> TextFormat {
    let px = f32::from(s.size.unwrap_or(size)) * PX;
    let typeface = s.font.as_deref().or(base).filter(|_| !s.mono).and_then(|f| crate::mail_fonts::family(f, s.bold));
    let font = if let Some(family) = typeface {
        FontId::new(px, family)
    } else if s.mono {
        FontId::monospace(px * 0.92)
    } else if s.bold {
        theme::bold(px)
    } else {
        theme::regular(px)
    };
    let (mut color, background) = theme::mail_colors(s.color, s.highlight, t, t.text);
    if s.link.is_some() && s.color.is_none() {
        color = t.accent;
    }
    let line = Stroke::new((px / 14.0).max(1.0), color);
    TextFormat {
        font_id: font,
        color,
        background: background.unwrap_or(Color32::TRANSPARENT),
        italics: s.italic,
        underline: if s.underline || s.link.is_some() { line } else { Stroke::NONE },
        strikethrough: if s.strike { line } else { Stroke::NONE },
        ..Default::default()
    }
}

/// Room for a picture `height` tall: an invisible character the picture is drawn over, as tall
/// as its row needs; its width is room put after the character, before what follows it, or,
/// last in its line, before the character (egui sizes and wraps a row by its letters, so room
/// after the last one wasn't the row's: a picture that didn't fit stayed on the row, squeezed).
/// (As letter spacing it went before the character, and not at all at the start of a line.)
pub(super) fn picture_format(height: f32) -> TextFormat {
    TextFormat { font_id: theme::regular(14.5), line_height: Some(height + 4.0), color: Color32::TRANSPARENT, ..Default::default() }
}

/// A part of the text with something drawn in its place (a signature's HTML code, laid out as
/// it's sent, issue #90). The text stays in the message as it is, so what's sent and every
/// place in the text are what they were: its letters are laid out too small to see, and its
/// last line is as tall as what's drawn, with that much room before it (the cursor after it is
/// that tall, at its right, as after a picture).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    /// What it's drawn for: the signature's code, or a table pasted in (issue #268).
    pub what: CardFor,
    /// The characters it stands for.
    pub range: std::ops::Range<usize>,
    /// What's one thing with it (a signature's `-- ` line and its text): the cursor doesn't
    /// stop in it, and Backspace after it or Delete before it takes all of it.
    pub whole: std::ops::Range<usize>,
    /// How big what's drawn is.
    pub size: egui::Vec2,
}

impl Card {
    /// Where the cursor is before it: the end of the line above, else its start.
    pub(super) fn before(&self, text: &str) -> usize {
        let above = self.whole.start.checked_sub(1).filter(|at| text.chars().nth(*at) == Some('\n'));
        above.unwrap_or(self.whole.start)
    }

    /// Where it is in `new`, the text `old` once changed: moved along with what was typed or
    /// taken out before it; none when the change is in it (its text is the writer's then).
    pub(super) fn moved(&self, old: &str, new: &str) -> Option<Card> {
        let same_start = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
        if same_start < self.whole.end {
            // A change before it, all of it before it: the same text after the change.
            let (old_len, new_len) = (old.chars().count(), new.chars().count());
            let same_end = old.chars().rev().zip(new.chars().rev()).take_while(|(a, b)| a == b).count();
            if old_len.saturating_sub(same_end) > self.whole.start {
                return None;
            }
            let shift = |at: usize| (at + new_len).checked_sub(old_len);
            let range = shift(self.range.start)?..shift(self.range.end)?;
            let whole = shift(self.whole.start)?..shift(self.whole.end)?;
            return Some(Card { what: self.what, range, whole, size: self.size });
        }
        Some(self.clone())
    }

    /// The character its last line starts at: the line that keeps the room.
    pub(super) fn last_line(&self, text: &str) -> usize {
        let breaks = text.chars().enumerate().take(self.range.end).skip(self.range.start);
        breaks.filter(|(_, ch)| *ch == '\n').map(|(i, _)| i + 1).last().unwrap_or(self.range.start)
    }
}

/// What a [`Card`] is drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CardFor {
    /// The signature's HTML code (issue #90).
    Signature,
    /// The table pasted in at this place in [`mailcraft_engine::Compose::tables`] (issue #268).
    Table(usize),
}

/// Letters that take (next to) no room and aren't seen: a [`Card`]'s text. `height`: its line's.
fn hidden_format(height: f32) -> TextFormat {
    TextFormat { font_id: theme::regular(1.0), line_height: Some(height), color: Color32::TRANSPARENT, ..Default::default() }
}

/// What the message's text is laid out with: its styles, whether it's plain text, the colours,
/// the width it wraps at, its pictures' own sizes and a part of it drawn as something else.
#[derive(Clone, Copy)]
pub(super) struct Lay<'a> {
    pub(super) st: &'a Styles,
    pub(super) plain: bool,
    pub(super) t: &'a Tokens,
    pub(super) wrap: f32,
    pub(super) own: &'a HashMap<String, egui::Vec2>,
    /// What's drawn over some of the text, in its place ([`Card`]).
    pub(super) cards: &'a [Card],
    /// The message's own typeface, for text with none set on it.
    pub(super) font: Option<&'a str>,
    /// The message's own size in points, for text with none set on it (issue #267).
    pub(super) size: u8,
}

/// The text laid out as `lay` says; `shift` moves a paragraph right (alignment).
pub(super) fn job(text: &str, lay: Lay<'_>, shift: &[f32]) -> LayoutJob {
    let Lay { st, plain, t, wrap, own, cards, font, size: text_size } = lay;
    let cards: Vec<(&Card, usize)> = if plain { Vec::new() } else { cards.iter().map(|c| (c, c.last_line(text))).collect() };
    // A card's letter at `at`: how tall its line is, and the room before it (its last line's start).
    let hidden = |at: usize| {
        cards.iter().find(|(c, _)| c.range.contains(&at)).map(|(c, last)| {
            let room = if wrap.is_finite() { c.size.x.min((wrap - 40.0).max(0.0)) } else { c.size.x };
            (if at >= *last { c.size.y + 4.0 } else { 0.5 }, if at == *last { room } else { 0.0 })
        })
    };
    let edge = |at: usize| cards.iter().any(|(c, _)| at == c.range.start || at == c.range.end);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    // Where a paragraph goes is decided here (`layout_aligned`), against the editor's width;
    // egui would put a right-to-left one against the widest line instead.
    job.halign_min_is_start = false;
    let plain_fmt = TextFormat { font_id: theme::regular(14.5), color: t.text, ..Default::default() };
    let mut i = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            let f = match hidden(i) {
                Some((height, _)) => hidden_format(height),
                None if plain => plain_fmt.clone(),
                None => format(&st.style(i.saturating_sub(1)), font, text_size, t),
            };
            job.append("\n", 0.0, f);
            i += 1;
        }
        let p = st.para(n);
        let mut lead = if plain { 0.0 } else { f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { INDENT } };
        lead += shift.get(n).copied().unwrap_or(0.0);
        let mut run_start = 0usize;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for k in 0..chars.len() {
            let cur = st.chars.get(i + k).copied();
            let next = st.chars.get(i + k + 1).copied();
            let pic = chars.get(k).is_some_and(|(_, ch)| *ch == PICTURE) || chars.get(k + 1).is_some_and(|(_, ch)| *ch == PICTURE);
            if k + 1 == chars.len() || cur != next || pic || edge(i + k + 1) {
                let from = chars.get(run_start).map_or(line.len(), |(b, _)| *b);
                let to = chars.get(k + 1).map_or(line.len(), |(b, _)| *b);
                let piece = line.get(from..to).unwrap_or("");
                let style = st.style(i + run_start);
                // A card's letters: unseen, the room for what's drawn before its last line.
                if let Some((height, room)) = hidden(i + run_start) {
                    job.append(piece, lead + room, hidden_format(height));
                    lead = 0.0;
                    run_start = k + 1;
                    continue;
                }
                let size = (!plain && piece.starts_with(PICTURE))
                    .then(|| style.picture.as_ref().map(|cid| super::pictures::shown(own.get(cid).copied(), style.picture_width, wrap)))
                    .flatten();
                let f = match size {
                    _ if plain => plain_fmt.clone(),
                    Some(size) => picture_format(size.y),
                    None => format(&style, font, text_size, t),
                };
                // Last in its line, a picture's room goes before it.
                let last = to == line.len();
                match size {
                    Some(size) if last => {
                        job.append(piece, lead + size.x, f);
                        lead = 0.0;
                    }
                    _ => {
                        job.append(piece, lead, f);
                        lead = size.map_or(0.0, |s| s.x);
                    }
                }
                run_start = k + 1;
            }
        }
        if chars.is_empty() && lead > 0.0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i), font, text_size, t) };
            job.append("", lead, f);
        }
        i += chars.len();
    }
    job
}

/// egui puts an empty line at the left edge whatever its indent, so the caret on an empty
/// bulleted or indented line sat before its bullet: such rows are moved to their indent.
pub(super) fn indent_empty_rows(g: Arc<Galley>, st: &Styles, plain: bool) -> Arc<Galley> {
    if plain {
        return g;
    }
    let lead = |n: usize| {
        let p = st.para(n);
        f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { INDENT }
    };
    let mut para = 0usize;
    let mut moves = Vec::new();
    for (i, r) in g.rows.iter().enumerate() {
        if r.row.glyphs.is_empty() && lead(para) > 0.0 {
            moves.push((i, lead(para)));
        }
        if r.ends_with_newline {
            para += 1;
        }
    }
    if moves.is_empty() {
        return g;
    }
    let mut out = (*g).clone();
    for (i, x) in moves {
        if let Some(r) = out.rows.get_mut(i) {
            r.pos.x = x;
        }
    }
    Arc::new(out)
}

/// Lay the text out; centred and right-aligned paragraphs that fit on one line are moved over.
/// (Afresh, for the tests: the editor keeps what it laid out with [`layout_kept`].)
#[cfg(test)]
pub(super) fn layout(ui: &Ui, text: &str, lay: Lay<'_>) -> Arc<Galley> {
    let first = ui.painter().layout_job(job(text, lay, &[]));
    indent_empty_rows(aligned(ui, first, text, lay), lay.st, lay.plain)
}

/// The text laid out last, given again while it would be laid out the same: egui gives back the
/// same first layout (it keeps a layout while it's used, and lays out afresh when the fonts
/// change), with the same paragraphs and width. Aligned paragraphs (laid out twice), right-to-left
/// ones and empty list items were moved on every frame.
#[derive(Default)]
pub(super) struct Laid {
    first: Option<Arc<Galley>>,
    paras: Vec<rich::Para>,
    plain: bool,
    wrap: f32,
    out: Option<Arc<Galley>>,
}

/// The text laid out, its centred and right-aligned paragraphs that fit on one line moved over;
/// given again from `laid` when it's the same.
pub(super) fn layout_kept(laid: &mut Laid, ui: &Ui, text: &str, lay: Lay<'_>) -> Arc<Galley> {
    let Lay { st, plain, wrap, .. } = lay;
    let first = ui.painter().layout_job(job(text, lay, &[]));
    if let (Some(was), Some(out)) = (&laid.first, &laid.out)
        && Arc::ptr_eq(was, &first)
        && laid.paras == st.paras
        && laid.plain == plain
        && laid.wrap.to_bits() == wrap.to_bits()
    {
        return out.clone();
    }
    let out = indent_empty_rows(aligned(ui, first.clone(), text, lay), st, plain);
    *laid = Laid { first: Some(first), paras: st.paras.clone(), plain, wrap, out: Some(out.clone()) };
    out
}

/// `first` (the text laid out) with its centred and right-aligned paragraphs moved over.
fn aligned(ui: &Ui, first: Arc<Galley>, text: &str, lay: Lay<'_>) -> Arc<Galley> {
    let Lay { st, plain, wrap, .. } = lay;
    // A right-to-left paragraph left as it was sits against the right edge. (Each paragraph is
    // looked at once: it was again for each of its rows, and twice to align it.)
    let rtl: Vec<bool> = text.split('\n').map(|l| crate::bidi::has_rtl(l) && crate::bidi::paragraph_rtl(l)).collect();
    let rtl_of = |n: usize| rtl.get(n).copied().unwrap_or(false);
    let align_of = |n: usize, a: Align| if a == Align::Left && rtl_of(n) { Align::Right } else { a };
    if plain {
        // Plain text has no alignment of its own: right-to-left paragraphs still start on the right.
        return rtl_rows_right(first, rtl_of, wrap, 1);
    }
    if st.paras.iter().enumerate().all(|(n, p)| align_of(n, p.align) == Align::Left) {
        return first;
    }
    let mut shift = vec![0.0; st.paras.len()];
    let mut row = 0;
    for (n, p) in st.paras.iter().enumerate() {
        // The rows this paragraph takes.
        let start = row;
        while let Some(r) = first.rows.get(row) {
            row += 1;
            if r.ends_with_newline {
                break;
            }
        }
        let align = align_of(n, p.align);
        if align != Align::Left
            && row == start + 1
            && let Some(r) = first.rows.get(start)
        {
            let room = (wrap - r.rect().max.x).max(0.0);
            if let Some(s) = shift.get_mut(n) {
                *s = if align == Align::Center { room / 2.0 } else { room };
            }
        }
    }
    let shifted = ui.painter().layout_job(job(text, lay, &shift));
    // A right-to-left paragraph left as it was and wrapped: each of its rows against the right edge.
    rtl_rows_right(shifted, |n| st.para(n).align == Align::Left && rtl_of(n), wrap, 2)
}

/// The rows of the paragraphs `rtl` names that take at least `min_rows` rows, moved against the
/// right edge (`wrap`), where right-to-left text starts.
fn rtl_rows_right(g: Arc<Galley>, rtl: impl Fn(usize) -> bool, wrap: f32, min_rows: usize) -> Arc<Galley> {
    if !wrap.is_finite() {
        return g;
    }
    // Each row's paragraph, and how many rows each paragraph takes.
    let mut para_of = Vec::with_capacity(g.rows.len());
    let mut counts: Vec<usize> = vec![0];
    for r in &g.rows {
        para_of.push(counts.len() - 1);
        if let Some(c) = counts.last_mut() {
            *c += 1;
        }
        if r.ends_with_newline {
            counts.push(0);
        }
    }
    let moves: Vec<(usize, f32)> = g
        .rows
        .iter()
        .enumerate()
        .filter(|(i, _)| para_of.get(*i).is_some_and(|&n| rtl(n) && counts.get(n).is_some_and(|&c| c >= min_rows)))
        .map(|(i, r)| (i, (wrap - r.rect().width()).max(0.0)))
        .collect();
    if moves.is_empty() {
        return g;
    }
    let mut out = (*g).clone();
    for (i, x) in moves {
        if let Some(r) = out.rows.get_mut(i) {
            r.pos.x = x;
        }
    }
    Arc::new(out)
}
