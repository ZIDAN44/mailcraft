//! Long message text laid out by what is in view. A message of a hundred thousand characters
//! was one galley, laid out whole in the frame it opened (179 ms). Its text goes in chunks and
//! its formatted lines in blocks: those near the view are laid out, the others keep their place
//! by height (measured once laid out, guessed before), and a couple more are measured each frame
//! so the scroll bar settles. The parts a selection starts and ends in are always laid out (and
//! for a copy, those between); Ctrl+A then a copy takes the whole text ([`super::text_selection`]).

use std::ops::Range;

use super::*;

/// Plain text longer than this (bytes) is laid out by chunks.
pub(crate) const LONG: usize = 20_000;
/// Most bytes in a chunk: one ends at a line's end when there is one in reach.
pub(crate) const CHUNK: usize = 4096;
/// Formatted text of more lines than `MANY_LINES` is laid out in blocks of this many.
pub(crate) const BLOCK_LINES: usize = 64;
pub(crate) const MANY_LINES: usize = 2_000;
/// Parts not measured yet that are laid out in a frame besides those in view.
const MEASURED_PER_FRAME: usize = 2;
/// Texts whose parts are kept however long ago they were shown (the reading pane's, and those of
/// messages in their own windows); past these, those not shown for `FORGET` seconds go.
const KEPT: usize = 8;
const FORGET: f64 = 30.0;
/// Most texts whose parts are kept, however many are shown at once.
const MOST: usize = 64;
/// What the average letter's width is measured on.
const SAMPLE: &str = "The quick brown fox jumps over the lazy dog, 0123456789.";

/// One shown text's parts: their heights, and where they were laid out.
#[derive(Default)]
pub(crate) struct LongText {
    /// What it was made for: the text's length and its number of parts.
    of: (usize, usize),
    width_bits: u32,
    /// Heights guessed at that width, and those measured.
    guessed: Vec<f32>,
    measured: Vec<Option<f32>>,
    /// How many parts the last frame laid out, and where they were.
    pub(crate) laid: usize,
    pub(crate) shown: Vec<(usize, Rect)>,
    /// When it was last shown (the input's time).
    shown_at: f64,
    /// A plain text's chunks and whether it has right-to-left letters, found once.
    plain: Option<Plain>,
}

/// What a plain text is cut into, and how it reads, for the text that is this ([`stamp`]).
struct Plain {
    stamp: u64,
    ranges: Vec<Range<usize>>,
    rtl: bool,
}

/// What tells a text from every other, one as long too: all of it, hashed (four strands side by
/// side, eight bytes at a time each: a text of megabytes is hashed every frame it's shown). By
/// its length alone, another text as long shown in its place (a message read again in another
/// encoding) kept the first one's cuts, some of them inside a letter, and its direction.
fn stamp(text: &str) -> u64 {
    const BY: u64 = 0x51_7c_c1_b7_27_22_0a_95;
    let mix = |h: u64, word: u64| (h.rotate_left(5) ^ word).wrapping_mul(BY);
    let mut strands = [text.len() as u64, 1, 2, 3];
    let (blocks, rest) = text.as_bytes().as_chunks::<32>();
    for block in blocks {
        let (words, _) = block.as_chunks::<8>();
        for (strand, word) in strands.iter_mut().zip(words) {
            *strand = mix(*strand, u64::from_le_bytes(*word));
        }
    }
    let all = strands.iter().fold(0, |h, strand| mix(h, *strand));
    rest.iter().fold(all, |h, byte| mix(h, u64::from(*byte)))
}

impl LongText {
    /// Fresh for a text of `len` bytes in `n` parts, unless it already is that one's.
    fn fit(&mut self, len: usize, n: usize) {
        if self.of != (len, n) {
            *self = LongText { of: (len, n), measured: vec![None; n], ..Default::default() };
        }
    }

    /// Each part's guessed height, and its measured one once laid out.
    #[cfg(test)]
    pub(crate) fn heights(&self) -> impl Iterator<Item = (f32, Option<f32>)> + '_ {
        self.guessed.iter().copied().zip(self.measured.iter().copied())
    }
}

/// `text` cut in pieces of at most `max` bytes: at a line's end, the line break in neither piece,
/// or for a longer line at a character's edge. No piece is empty.
pub(crate) fn chunks(text: &str, max: usize) -> Vec<Range<usize>> {
    let max = max.max(4);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let cap = start.saturating_add(max);
        if cap >= text.len() {
            out.push(start..text.len());
            break;
        }
        // A line break is one byte that is never part of another character.
        let from = start.saturating_add(1);
        if let Some(at) = bytes.get(from..cap).and_then(|w| w.iter().rposition(|b| *b == b'\n')) {
            let nl = from.saturating_add(at);
            out.push(start..nl);
            start = nl.saturating_add(1);
            continue;
        }
        let mut cut = cap;
        while cut > start && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == start {
            cut = start.saturating_add(1);
            while cut < text.len() && !text.is_char_boundary(cut) {
                cut = cut.saturating_add(1);
            }
        }
        out.push(start..cut);
        start = cut;
    }
    out
}

/// How many rows `text` wraps to at `per_row` letters a row.
fn rows(text: &str, per_row: usize) -> usize {
    let per_row = per_row.max(1);
    text.split('\n').map(|l| l.chars().count().div_ceil(per_row).max(1)).fold(0usize, usize::saturating_add)
}

/// The height of a row of `font`, and the width of one of its letters (on average). The row is
/// measured on laid-out lines: they're rounded to whole pixels, a font's row height isn't (at
/// 14.5 points, 17 against 16.66: a guess a third of a pixel short a row, 20 pixels a chunk).
fn metrics(ui: &Ui, font: &egui::FontId) -> (f32, f32) {
    const ROWS: usize = 8;
    ui.fonts_mut(|f| {
        let lines = [SAMPLE; ROWS].join("\n");
        let size = f.layout_no_wrap(lines, font.clone(), Color32::WHITE).size();
        let row = if size.y > 0.0 { size.y / ROWS as f32 } else { f.row_height(font) };
        (row, (size.x / SAMPLE.len() as f32).max(1.0))
    })
}

/// The state of the text shown under `id`, taken out of `st` while it's drawn.
fn take(st: &mut UiState, id: egui::Id) -> LongText {
    st.long_text.remove(&id).unwrap_or_default()
}

/// Keep the state of the text shown under `id` at `now`. Past `KEPT`, those not shown for a while
/// go, the latest first staying; never more than `MOST`. (Past eight, all went: with nine shown,
/// each was measured anew every frame and its scroll bar never settled.)
fn put_back(st: &mut UiState, id: egui::Id, mut v: LongText, now: f64) {
    v.shown_at = now;
    st.long_text.insert(id, v);
    if st.long_text.len() <= KEPT {
        return;
    }
    let mut latest: Vec<(egui::Id, f64)> = st.long_text.iter().map(|(id, v)| (*id, v.shown_at)).collect();
    latest.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, (id, at)) in latest.into_iter().enumerate() {
        if i >= MOST || (i >= KEPT && now - at >= FORGET) {
            st.long_text.remove(&id);
        }
    }
}

/// A long plain text (`LONG`), in chunks; right-to-left text keeps its own labels.
pub(super) fn plain(st: &mut UiState, ui: &mut Ui, id: egui::Id, text: &str) {
    let t = st.tokens;
    let font = theme::regular(14.5);
    let mut v = take(st, id);
    // Cut and looked through once, not on every frame (4 MB took 3.4 ms a frame).
    let stamp = stamp(text);
    let (ranges, rtl) = match v.plain.take().filter(|p| p.stamp == stamp) {
        Some(p) => (p.ranges, p.rtl),
        None => (chunks(text, CHUNK), crate::bidi::has_rtl(text)),
    };
    v.fit(text.len(), ranges.len());
    let pins = text_selection::parts(st, ranges.len());
    let (row, letter) = metrics(ui, &font);
    let guess = |i: usize, width: f32| {
        let s = ranges.get(i).and_then(|r| text.get(r.clone())).unwrap_or_default();
        rows(s, (width / letter) as usize) as f32 * row
    };
    ui.scope(|ui| {
        // Chunks one under the other as the lines of one text (and copied so, a line break apart).
        ui.spacing_mut().item_spacing.y = 0.0;
        parts(ui, &mut v, pins, guess, |ui, i, lay| {
            let Some(s) = ranges.get(i).and_then(|r| text.get(r.clone())).filter(|_| lay) else { return };
            text_selection::part(st, i);
            // A chunk after a line break (cut there, the break in neither) starts a line.
            let start = ranges.get(i).map_or(0, |r| r.start);
            if start > 0 && text.as_bytes().get(start - 1) == Some(&b'\n') {
                text_selection::line(st);
            }
            if rtl {
                let job = egui::text::LayoutJob::simple(s.to_string(), font.clone(), t.text, ui.available_width());
                let r = text_selection::rtl_run(st, ui, job);
                rtl_copy_menu(&r, None, text);
            } else {
                text_selection::run(st, ui, RichText::new(s).font(font.clone()).color(t.text), None);
            }
        });
    });
    v.plain = Some(Plain { stamp, ranges, rtl });
    put_back(st, id, v, ui.input(|i| i.time));
}

/// A part of formatted text: some of its lines, or a table of its layout.
enum Unit<'a> {
    Lines(&'a [Vec<mime::Span>]),
    Table(&'a mime::layout::Block),
}

fn units(p: &mime::Parsed) -> Vec<Unit<'_>> {
    if p.layout.is_empty() {
        return p.rich.chunks(BLOCK_LINES).map(Unit::Lines).collect();
    }
    let mut out = Vec::new();
    for b in &p.layout {
        match b {
            mime::layout::Block::Text(lines) => out.extend(lines.chunks(BLOCK_LINES).map(Unit::Lines)),
            mime::layout::Block::Table(_) => out.push(Unit::Table(b)),
        }
    }
    out
}

/// Whether the formatted text has lines enough (`MANY_LINES`) to be laid out by blocks.
pub(super) fn many_lines(p: &mime::Parsed) -> bool {
    let n = if p.layout.is_empty() {
        p.rich.len()
    } else {
        p.layout
            .iter()
            .map(|b| match b {
                mime::layout::Block::Text(lines) => lines.len(),
                mime::layout::Block::Table(_) => 1,
            })
            .fold(0usize, usize::saturating_add)
    };
    n > MANY_LINES
}

/// Formatted text of many lines, `width` wide on `page`, by blocks of lines (and its layout's
/// tables one by one). Pictures in the blocks passed over count as shown in place.
pub(super) fn rich(body: &mut Body, st: &mut UiState, ui: &mut Ui, id: egui::Id, width: f32, page: [u8; 3]) {
    let p = body.p;
    let parts_of = units(p);
    let mut v = take(st, id);
    v.fit(p.text.len(), parts_of.len());
    let (row, letter) = metrics(ui, &theme::regular(14.5));
    let gap = ui.spacing().item_spacing.y;
    let pins = text_selection::parts(st, parts_of.len());
    let guess = |i: usize, width: f32| match parts_of.get(i) {
        Some(Unit::Lines(lines)) => lines
            .iter()
            .map(|line| {
                let text: usize = line.iter().map(|sp| sp.text.chars().count()).fold(0, usize::saturating_add);
                let pictures = if line.iter().any(|sp| sp.image.is_some()) { 120.0 } else { 0.0 };
                if line.is_empty() {
                    super::body::BLANK
                } else {
                    text.div_ceil(((width / letter) as usize).max(1)).max(1) as f32 * row + gap + pictures
                }
            })
            .sum(),
        Some(Unit::Table(mime::layout::Block::Table(t))) => t.rows.len().min(1_000) as f32 * 3.0 * row,
        _ => 0.0,
    };
    parts(ui, &mut v, pins, guess, |ui, i, lay| match (parts_of.get(i), lay) {
        (Some(Unit::Lines(lines)), true) => {
            text_selection::part(st, i);
            body.lines(st, ui, lines, width, page, Align::Min);
        }
        (Some(Unit::Table(b)), true) => {
            text_selection::part(st, i);
            text_selection::line(st);
            body.blocks(st, ui, std::slice::from_ref(*b), width, page, mime::layout::Align::Left, 0);
        }
        (Some(Unit::Lines(lines)), false) => pictures_of_lines(lines, body.shown),
        (Some(Unit::Table(b)), false) => pictures_of_blocks(std::slice::from_ref(*b), body.shown, 0),
        (None, _) => {}
    });
    put_back(st, id, v, ui.input(|i| i.time));
}

fn pictures_of_lines(lines: &[Vec<mime::Span>], out: &mut Vec<String>) {
    out.extend(lines.iter().flatten().filter_map(|sp| sp.image.clone()));
}

/// The pictures laid-out parts show (as [`Body::blocks`] goes, as deep).
fn pictures_of_blocks(blocks: &[mime::layout::Block], out: &mut Vec<String>, depth: usize) {
    for b in blocks {
        match b {
            mime::layout::Block::Text(lines) => pictures_of_lines(lines, out),
            mime::layout::Block::Table(t) if depth < super::body::MAX_NESTED => {
                out.extend(t.background_image.clone());
                for c in t.rows.iter().flatten() {
                    out.extend(c.background_image.clone());
                    pictures_of_blocks(&c.blocks, out, depth + 1);
                    for o in &c.overlays {
                        pictures_of_blocks(&o.blocks, out, depth + 1);
                    }
                }
            }
            mime::layout::Block::Table(_) => {}
        }
    }
}

/// Lay out `v`'s parts one under the other: those near the view, those a selection starts or ends
/// in (and their neighbours, for Shift with the arrows), all a copied selection spans, and a few
/// not measured yet. `draw(ui, i, true)` lays part `i` out; `draw(ui, i, false)` says it's passed
/// over, its room kept by its height; `guess(i, width)` is that before it's measured. `pins`: the
/// parts a selection's ends are in, those it spans, and whether it's all of the text.
fn parts(
    ui: &mut Ui,
    v: &mut LongText,
    pins: (Vec<usize>, Range<usize>, bool),
    guess: impl Fn(usize, f32) -> f32,
    mut draw: impl FnMut(&mut Ui, usize, bool),
) {
    let n = v.measured.len();
    let width = ui.available_width();
    if v.width_bits != width.to_bits() || v.guessed.len() != n {
        v.width_bits = width.to_bits();
        v.guessed = (0..n).map(|i| guess(i, width)).collect();
        v.measured.iter_mut().for_each(|h| *h = None);
    }
    let copying = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)));
    // Laid out whatever the view: the ends of a selection, and for a copy, everything between
    // them (all of it is the whole text, copied as it is).
    let (ends, span, all) = pins;
    let span = if copying && !all { span } else { 0..0 };
    let clip = ui.clip_rect();
    let near = clip.expand2(vec2(0.0, clip.height()));
    let left = ui.cursor().left();
    // None measured in the frame the text first shows, which has enough to lay out in view.
    let mut budget = if v.measured.iter().any(Option::is_some) { MEASURED_PER_FRAME } else { 0 };
    v.laid = 0;
    v.shown.clear();
    for i in 0..n {
        let top = ui.cursor().top();
        let known = v.measured.get(i).copied().flatten();
        let h = known.or_else(|| v.guessed.get(i).copied()).unwrap_or(0.0);
        let in_view = top <= near.bottom() && top + h >= near.top();
        let pinned = ends.iter().any(|e| e.abs_diff(i) <= 1) || span.contains(&i);
        let measure = known.is_none() && !in_view && !pinned && budget > 0;
        if measure {
            budget -= 1;
        }
        let lay = in_view || pinned || measure;
        let r = ui.push_id(i, |ui| {
            draw(ui, i, lay);
            if !lay {
                ui.allocate_space(vec2(width, h));
            }
        });
        if !lay {
            continue;
        }
        let rect = Rect::from_x_y_ranges(left..=left + width, r.response.rect.y_range());
        if let Some(m) = v.measured.get_mut(i) {
            *m = Some(r.response.rect.height());
        }
        v.laid += 1;
        v.shown.push((i, rect));
    }
    if budget < MEASURED_PER_FRAME && v.measured.iter().any(Option::is_none) {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests;
