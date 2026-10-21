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

/// Room for a picture: an invisible, wide character the picture is drawn over.
pub(super) fn picture_format() -> TextFormat {
    TextFormat { font_id: theme::regular(PICTURE_PX), color: Color32::TRANSPARENT, extra_letter_spacing: PICTURE_PX * 0.8, ..Default::default() }
}

/// The text laid out with its styles; `shift` moves a paragraph right (alignment).
pub(super) fn job(text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, shift: &[f32]) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
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
                let f = if plain {
                    plain_fmt.clone()
                } else if piece.starts_with(PICTURE) && st.style(i + run_start).picture.is_some() {
                    picture_format()
                } else {
                    format(&st.style(i + run_start), t)
                };
                job.append(piece, lead, f);
                lead = 0.0;
                run_start = k + 1;
            }
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
    if plain || st.paras.iter().enumerate().all(|(n, p)| align_of(n, p.align) == Align::Left) {
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
    ui.painter().layout_job(job(text, st, plain, t, wrap, &shift))
}
