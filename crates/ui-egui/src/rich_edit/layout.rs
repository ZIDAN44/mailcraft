//! The message editor's text laid out: each run in its style, pictures, lists' indents, alignment.

use super::*;

/// A look as text formatting (the person's colours adjusted to stay readable on the page).
pub(super) fn format(s: &Style, t: &Tokens) -> TextFormat {
    let px = f32::from(s.size()) * PX;
    let font = if s.mono {
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

/// Room for a picture: an invisible, wide character the picture is drawn over, and
/// [`PICTURE_GAP`] after it.
pub(super) fn picture_format() -> TextFormat {
    TextFormat { font_id: theme::regular(PICTURE_PX), color: Color32::TRANSPARENT, ..Default::default() }
}

/// Room a picture takes after its character, put before what follows it. (As letter spacing it
/// went before the character, and not at all at the start of a line: the picture sat some 60
/// points right of its room, or squeezed into the character's 36 at a line's start.)
const PICTURE_GAP: f32 = PICTURE_PX * 0.8;

/// The text laid out with its styles; `shift` moves a paragraph right (alignment).
pub(super) fn job(text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, shift: &[f32]) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    // Where a paragraph goes is decided here (`layout_aligned`), against the editor's width;
    // egui would put a right-to-left one against the widest line instead.
    job.halign_min_is_start = false;
    let plain_fmt = TextFormat { font_id: theme::regular(14.5), color: t.text, ..Default::default() };
    let mut i = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i.saturating_sub(1)), t) };
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
            if k + 1 == chars.len() || cur != next || pic {
                let from = chars.get(run_start).map_or(line.len(), |(b, _)| *b);
                let to = chars.get(k + 1).map_or(line.len(), |(b, _)| *b);
                let piece = line.get(from..to).unwrap_or("");
                let picture = !plain && piece.starts_with(PICTURE) && st.style(i + run_start).picture.is_some();
                let f = if plain {
                    plain_fmt.clone()
                } else if picture {
                    picture_format()
                } else {
                    format(&st.style(i + run_start), t)
                };
                job.append(piece, lead, f);
                lead = if picture { PICTURE_GAP } else { 0.0 };
                run_start = k + 1;
            }
        }
        // A picture last in its line: its room after it still.
        if !chars.is_empty() && lead > 0.0 {
            job.append("", lead, picture_format());
        }
        if chars.is_empty() && lead > 0.0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i), t) };
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
pub(super) fn layout(ui: &Ui, text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32) -> Arc<Galley> {
    indent_empty_rows(layout_aligned(ui, text, st, plain, t, wrap), st, plain)
}

pub(super) fn layout_aligned(ui: &Ui, text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32) -> Arc<Galley> {
    let first = ui.painter().layout_job(job(text, st, plain, t, wrap, &[]));
    // A right-to-left paragraph left as it was sits against the right edge.
    let lines: Vec<&str> = text.split('\n').collect();
    let align_of = |n: usize, a: Align| {
        if a == Align::Left && lines.get(n).is_some_and(|l| crate::bidi::has_rtl(l) && crate::bidi::paragraph_rtl(l)) { Align::Right } else { a }
    };
    let rtl_of = |n: usize| lines.get(n).is_some_and(|l| crate::bidi::has_rtl(l) && crate::bidi::paragraph_rtl(l));
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
    let shifted = ui.painter().layout_job(job(text, st, plain, t, wrap, &shift));
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
