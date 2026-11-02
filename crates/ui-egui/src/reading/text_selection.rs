//! A message's text selected as one text, as in Outlook's reading pane. The text is drawn in runs
//! (a formatted line's spans and links, a table's cells, a long text's chunks, a right-to-left
//! line, a plain message whole), which were egui labels, each with a selection of its own: Ctrl+A
//! took the paragraph under the selection, Shift with the arrows stopped at its edges, and egui
//! let go of a selection whose ends it no longer drew, so a drag couldn't scroll the message.
//! The runs are now laid out as labels are and drawn here, and the selection is two marks in the
//! text (a run, and a letter in it): a press in the text starts one, a drag moves its end (held
//! at or past the view's top or bottom, it scrolls, faster the farther out), Shift+click,
//! Shift with the arrows, Home and End (Ctrl too) move it across runs, keeping it in view, and
//! Ctrl+A takes the whole text. A copy joins what's selected in reading order, a line break
//! between lines. The keys are the text's while it has the keyboard: from a press in it until a
//! press elsewhere (the message list keeps Ctrl+A and the arrows) or a box taking the keys; a
//! message's own window gives them to its text from the start. Another message shown in its
//! place lets go of a text's selection and keys. On a touch screen a finger dragged over the
//! text scrolls it, as over egui's labels: a press there only places the selection's start.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::text::{CCursor, CharIndex, Galley, LayoutJob};
use egui::text_selection::CCursorRange;
use egui::text_selection::text_cursor_state::{is_word_char, slice_char_range};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::UiState;

/// Within this distance of the view's top or bottom (its last pixels), or past it, a selection
/// drag scrolls; not on the text before it, which would run away under a still pointer.
const EDGE: f32 = 2.0;
/// How fast: points a second for each point the pointer is into that band, or past the view.
const EDGE_SPEED: f32 = 20.0;
/// The widest gap between two runs side by side on a row whose shading is joined (pixel
/// rounding's; a table's next cell is farther).
const MAX_SEAM: f32 = 3.0;
/// Most line breaks copied between two runs (a paragraph's gap).
const MAX_BREAKS: u8 = 2;
/// Texts whose selections are kept (the reading pane's, and those of messages in their own windows).
const KEPT: usize = 8;

/// One end of a selection: a run (by the part of a long text it's in, and its place there) and
/// a letter in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mark {
    part: u32,
    run: u32,
    at: CCursor,
}

impl Mark {
    /// Before and after every run: what Ctrl+A, Ctrl+Shift+Home and Ctrl+Shift+End select to,
    /// whether or not the runs at the text's ends are laid out.
    const START: Mark = Mark { part: 0, run: 0, at: CCursor { index: CharIndex(0), prefer_next_row: false } };
    const END: Mark = Mark { part: u32::MAX, run: u32::MAX, at: CCursor { index: CharIndex(usize::MAX), prefer_next_row: false } };

    fn run_key(self) -> (u32, u32) {
        (self.part, self.run)
    }

    fn key(self) -> (u32, u32, usize) {
        (self.part, self.run, self.at.index.0)
    }
}

/// A run of text laid out this frame.
struct Run {
    part: u32,
    n: u32,
    /// Line breaks before it: 0 on the line of the run before, 1 on the next, 2 after a gap.
    breaks: u8,
    galley: Arc<Galley>,
    pos: Pos2,
    color: Color32,
    underline: Stroke,
    painter: egui::Painter,
    slot: egui::layers::ShapeIdx,
    response: Response,
    link: bool,
}

impl Run {
    fn len(&self) -> usize {
        self.galley.end().index.0
    }

    /// Its rows on the screen.
    fn rows(&self) -> impl Iterator<Item = Rect> + '_ {
        self.galley.rows.iter().map(|r| r.rect_without_leading_space().translate(self.pos.to_vec2()))
    }

    /// Where `c` is, on the screen.
    fn caret(&self, c: CCursor) -> Rect {
        self.galley.pos_from_cursor(c).translate(self.pos.to_vec2())
    }
}

/// One text's selection, and its runs this frame.
#[derive(Default)]
pub(crate) struct Doc {
    anchor: Option<Mark>,
    head: Option<Mark>,
    /// A drag from a press in the text is under way.
    dragging: bool,
    /// It has the keyboard.
    active: bool,
    /// Where on the screen Shift+Up and Shift+Down keep to.
    h_x: Option<f32>,
    /// Where the text was last frame (a press there starts a selection), the part of it in view,
    /// and the pointer.
    rect: Option<Rect>,
    view: Option<Rect>,
    pointer: Option<Pos2>,
    /// It's in a message's own window.
    window: bool,
    /// The pass it was last drawn in, and on which layer (a message's own window, or the main one).
    pass: u64,
    layer: Option<egui::LayerId>,
    part: u32,
    n: u32,
    breaks: u8,
    top: f32,
    runs: Vec<Run>,
    /// What's under the runs, between and beside them.
    under: Option<Response>,
}

/// The selections of the texts shown, and the one being drawn.
#[derive(Default)]
pub(crate) struct TextSelection {
    docs: HashMap<egui::Id, Doc>,
    cur: Option<(egui::Id, Doc)>,
    /// The main window's message text had the keyboard last frame: its keys aren't the list's.
    pub(crate) keys: bool,
}

impl TextSelection {
    /// Whether a text shown is selected whole (Ctrl+A).
    #[cfg(test)]
    pub(crate) fn all(&self) -> bool {
        self.docs.values().any(Doc::all)
    }

    /// Where the texts drawn last are, and on what order of layer (tests).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Vec<(Rect, egui::Order)> {
        let last = self.docs.values().map(|d| d.pass).max().unwrap_or(0);
        self.docs.values().filter(|d| d.pass == last).filter_map(|d| Some((d.rect?, d.layer?.order))).collect()
    }

    /// The part in view of the text drawn last on a layer of `order` (tests).
    #[cfg(test)]
    pub(crate) fn view(&self, order: egui::Order) -> Option<Rect> {
        let last = self.docs.values().map(|d| d.pass).max().unwrap_or(0);
        self.docs.values().filter(|d| d.pass == last && d.layer.is_some_and(|l| l.order == order)).find_map(|d| d.view)
    }
}

impl Doc {
    /// Its two ends, in reading order.
    fn selection(&self) -> Option<(Mark, Mark)> {
        let (a, h) = (self.anchor?, self.head?);
        (a != h).then(|| if a.key() <= h.key() { (a, h) } else { (h, a) })
    }

    fn all(&self) -> bool {
        self.selection().is_some_and(|(lo, hi)| lo == Mark::START && hi == Mark::END)
    }

    fn mark(&self, i: usize, at: CCursor) -> Option<Mark> {
        self.runs.get(i).map(|r| Mark { part: r.part, run: r.n, at })
    }

    /// The letters of run `i` selected.
    fn range_of(&self, i: usize) -> Option<Range<usize>> {
        let (lo, hi) = self.selection()?;
        let r = self.runs.get(i)?;
        let k = (r.part, r.n);
        if k < lo.run_key() || k > hi.run_key() {
            return None;
        }
        let len = r.len();
        let end = if k == hi.run_key() { hi.at.index.0.min(len) } else { len };
        let start = if k == lo.run_key() { lo.at.index.0.min(end) } else { 0 };
        Some(start..end)
    }

    /// What's selected, in reading order; `whole` when all of it is (a long text, not all laid out).
    fn text(&self, whole: Option<&str>) -> String {
        if let Some(w) = whole.filter(|_| self.all()) {
            return w.to_string();
        }
        let mut out = String::new();
        let mut first = true;
        for (i, r) in self.runs.iter().enumerate() {
            let Some(range) = self.range_of(i) else { continue };
            if !first {
                (0..r.breaks).for_each(|_| out.push('\n'));
            }
            first = false;
            out.push_str(slice_char_range(r.galley.text(), CharIndex(range.start)..CharIndex(range.end)));
        }
        out
    }

    /// The run, and the letter in it, nearest `p` among `among`, of the rows `keep` keeps: the
    /// row nearest up or down, then across.
    fn nearest(&self, among: Range<usize>, p: Pos2, keep: impl Fn(Rect) -> bool) -> Option<(usize, CCursor)> {
        let gap = |v: f32, r: egui::Rangef| if v < r.min { r.min - v } else { (v - r.max).max(0.0) };
        let mut best: Option<((f32, f32), usize, Rect)> = None;
        for i in among {
            let Some(r) = self.runs.get(i) else { continue };
            for row in r.rows().filter(|row| keep(*row)) {
                let score = (gap(p.y, row.y_range()), gap(p.x, row.x_range()));
                if best.is_none_or(|(b, ..)| score < b) {
                    best = Some((score, i, row));
                }
            }
        }
        let (_, i, row) = best?;
        let r = self.runs.get(i)?;
        let at = pos2(p.x.clamp(row.left() - 1.0, row.right() + 1.0), row.center().y);
        Some((i, r.galley.cursor_from_pos(at - r.pos)))
    }

    /// Where `m` is among the runs laid out: its own, or the first after it (a mark past them all
    /// at the end of the last).
    fn resolve(&self, m: Mark) -> Option<(usize, CCursor)> {
        let last = self.runs.len().checked_sub(1)?;
        for (i, r) in self.runs.iter().enumerate() {
            let k = (r.part, r.n);
            if k == m.run_key() {
                let at = if m.at.index.0 > r.len() { r.galley.end() } else { m.at };
                return Some((i, at));
            }
            if k > m.run_key() {
                return Some((i, CCursor::default()));
            }
        }
        self.runs.get(last).map(|r| (last, r.galley.end()))
    }

    /// The runs on the line of run `i` (line breaks apart).
    fn line_of(&self, i: usize) -> Range<usize> {
        let starts = |j: usize| self.runs.get(j).is_none_or(|r| r.breaks > 0 || j == 0);
        let mut s = i;
        while !starts(s) {
            s -= 1;
        }
        let mut e = i.saturating_add(1);
        while e < self.runs.len() && !starts(e) {
            e += 1;
        }
        s..e
    }

    /// Run `i` selected (`range`) to its end, and the next beside it on the row selected from its
    /// start: where, in the last row of run `i` (its own coordinates), its shading ends to meet
    /// the next run's. Each run shades its own letters, and drawn at their places rounded to
    /// pixels (as text is) two runs' shadings overlap a little or leave a hairline between them:
    /// a darker or a lighter line down the selection, the shading being translucent.
    fn seam(&self, i: usize, range: &Range<usize>, pixels_per_point: f32) -> Option<f32> {
        let r = self.runs.get(i)?;
        let j = i.checked_add(1)?;
        let next = self.runs.get(j)?;
        if range.end < r.len() || next.breaks > 0 || self.range_of(j).is_none_or(|n| n.is_empty() || n.start > 0) {
            return None;
        }
        let last = r.galley.rows.last()?;
        // Side by side on a row: not a table's next cell, nor a run wrapped onto the next row.
        let (a, b) = (r.rows().last()?, next.rows().next()?);
        if last.row.rtl_paragraph || a.y_range().intersection(b.y_range()).span() <= 0.0 || (b.left() - a.right()).abs() > MAX_SEAM {
            return None;
        }
        let drawn = |r: &Run| r.pos.round_to_pixels(pixels_per_point);
        let starts = drawn(next).x + next.galley.pos_from_cursor(CCursor::default()).min.x;
        Some(starts - drawn(r).x - last.pos.x)
    }

    /// A double-click: the word at `at` in run `i`; a triple-click: its line (a paragraph).
    fn select_unit(&mut self, i: usize, at: CCursor, line: bool) {
        let Some(r) = self.runs.get(i) else { return };
        let chars: Vec<char> = r.galley.text().chars().collect();
        let at = at.index.0.min(chars.len());
        let (from, to) = if line {
            let in_run = |c: &char| *c == '\n';
            let s = chars.get(..at).and_then(|b| b.iter().rposition(in_run)).map(|p| (i, p + 1));
            let e = chars.get(at..).and_then(|b| b.iter().position(in_run)).map(|p| (i, at + p));
            let l = self.line_of(i);
            let first = l.start;
            let last = l.end.saturating_sub(1);
            (s.unwrap_or((first, 0)), e.unwrap_or_else(|| (last, self.runs.get(last).map_or(0, Run::len))))
        } else {
            let word = |c: &char| is_word_char(*c);
            let here = chars.get(at).filter(|c| word(c)).or_else(|| at.checked_sub(1).and_then(|p| chars.get(p)).filter(|c| word(c)));
            if here.is_none() {
                return;
            }
            let s = chars.get(..at).and_then(|b| b.iter().rposition(|c| !word(c))).map_or(0, |p| p + 1);
            let e = chars.get(at..).and_then(|b| b.iter().position(|c| !word(c))).map_or(chars.len(), |p| at + p);
            ((i, s), (i, e))
        };
        self.anchor = self.mark(from.0, CCursor::new(from.1));
        self.head = self.mark(to.0, CCursor::new(to.1));
    }

    /// Shift with an arrow, Home or End (Ctrl too): the selection's moving end goes on, across
    /// runs. Where it went, to keep in view.
    fn step(&mut self, os: egui::os::OperatingSystem, key: Key, m: Modifiers, text: Rect) -> Option<Rect> {
        let last = self.runs.len().checked_sub(1)?;
        if self.head.is_none() {
            self.anchor = self.mark(0, CCursor::default());
            self.head = self.anchor;
        }
        if m.command && matches!(key, Key::Home | Key::End) {
            self.h_x = None;
            let home = key == Key::Home;
            self.head = Some(if home { Mark::START } else { Mark::END });
            return Some(if home {
                Rect::from_min_size(text.min, vec2(1.0, 20.0))
            } else {
                Rect::from_min_size(text.left_bottom() - vec2(0.0, 20.0), vec2(1.0, 20.0))
            });
        }
        let (mut i, c) = self.resolve(self.head?)?;
        let run = self.runs.get(i)?;
        let caret = run.caret(c);
        let mut to;
        match key {
            Key::ArrowUp | Key::ArrowDown => {
                // By where it is on the screen: rows of this run and of others alike.
                let x = *self.h_x.get_or_insert(caret.center().x);
                let found = if key == Key::ArrowDown {
                    self.nearest(0..self.runs.len(), pos2(x, caret.bottom()), |row| row.center().y > caret.bottom())
                } else {
                    self.nearest(0..self.runs.len(), pos2(x, caret.top()), |row| row.center().y < caret.top())
                };
                (i, to) = found.unwrap_or_else(|| {
                    if key == Key::ArrowDown {
                        (last, self.runs.get(last).map_or_else(CCursor::default, |r| r.galley.end()))
                    } else {
                        (0, CCursor::default())
                    }
                });
            }
            // To the start or end of the line on the screen, whatever runs it's made of (a bold
            // word is one of its own).
            Key::Home | Key::End => {
                self.h_x = None;
                (i, to) = self.across(os, i, c, key, m).unwrap_or((i, c));
            }
            _ => {
                self.h_x = None;
                let mut r = CCursorRange::one(c);
                r.on_key_press(os, &run.galley, &m, key);
                to = r.primary;
                if to.index == c.index {
                    (i, to) = self.across(os, i, c, key, m).unwrap_or((i, c));
                }
            }
        }
        self.head = self.mark(i, to);
        self.runs.get(i).map(|r| r.caret(to))
    }

    /// A key that didn't move within run `i` (at its edge) goes on into the next run or the one
    /// before, or along the line for Home and End.
    fn across(&self, os: egui::os::OperatingSystem, i: usize, c: CCursor, key: Key, m: Modifiers) -> Option<(usize, CCursor)> {
        let again = |j: usize, at: CCursor| {
            let r = self.runs.get(j)?;
            let mut range = CCursorRange::one(at);
            range.on_key_press(os, &r.galley, &m, key);
            Some((j, range.primary))
        };
        // Left and Right go on the screen: in a right-to-left paragraph, Left goes on in the text.
        let rtl = self.runs.get(i)?.galley.rows.first().is_some_and(|r| r.row.rtl_paragraph);
        match key {
            Key::ArrowLeft | Key::ArrowRight if (key == Key::ArrowLeft) != rtl => {
                let j = i.checked_sub(1)?;
                let end = self.runs.get(j)?.galley.end();
                // On the same line, the step is a letter (or word) of that run; else the line break.
                if self.runs.get(i)?.breaks == 0 { again(j, end) } else { Some((j, end)) }
            }
            Key::ArrowLeft | Key::ArrowRight => {
                let j = i.checked_add(1).filter(|j| *j < self.runs.len())?;
                if self.runs.get(j)?.breaks == 0 { again(j, CCursor::default()) } else { Some((j, CCursor::default())) }
            }
            Key::Home | Key::End => {
                let y = self.runs.get(i)?.caret(c).center().y;
                let line = self.line_of(i);
                let on_row = |row: Rect| row.y_range().contains(y);
                let x = if key == Key::Home { f32::NEG_INFINITY } else { f32::INFINITY };
                let rows = line.clone().flat_map(|j| self.runs.get(j).into_iter().flat_map(|r| r.rows()));
                let edge = rows.filter(|r| on_row(*r)).map(|r| if key == Key::Home { r.left() } else { r.right() });
                let edge = if key == Key::Home { edge.fold(f32::INFINITY, f32::min) } else { edge.fold(f32::NEG_INFINITY, f32::max) };
                let p = pos2(if edge.is_finite() { edge } else { x }, y);
                self.nearest(line, p, on_row)
            }
            _ => None,
        }
    }
}

/// How far a selection drag scrolls this frame with the pointer at `y`, at or past the top or
/// bottom of `view`; faster the farther it goes. A positive delta moves the content down.
/// (PdfCraft's, for its pages, with its band of the view's edge narrowed to the last pixels.)
pub(crate) fn edge_delta(y: f32, view: Rect, dt: f32) -> f32 {
    if !y.is_finite() || !dt.is_finite() || view.height() < 4.0 * EDGE {
        return 0.0;
    }
    let distance = if y > view.bottom() - EDGE {
        y - (view.bottom() - EDGE)
    } else if y < view.top() + EDGE {
        y - (view.top() + EDGE)
    } else {
        return 0.0;
    };
    -distance.clamp(-1_000_000.0, 1_000_000.0) * EDGE_SPEED * dt.clamp(0.0, 0.05)
}

/// The text shown under `id` starts here; `window`: in a message's own window.
pub(crate) fn begin(st: &mut UiState, ui: &mut Ui, id: egui::Id, window: bool) {
    let ts = &mut st.text_selection;
    if let Some((old, d)) = ts.cur.take() {
        ts.docs.insert(old, d);
    }
    let kept = ts.docs.remove(&id);
    let seen = kept.is_some();
    let mut d = kept.unwrap_or_default();
    // Not shown last pass (another message was, or none): it's shown again unselected and
    // without the keyboard, as Outlook does when the list goes back to it; a message's own
    // window has nothing else to give the keys to.
    if !seen || d.pass.saturating_add(1) < ui.ctx().cumulative_pass_nr() {
        (d.anchor, d.head, d.dragging, d.h_x) = (None, None, false, None);
        d.active = window;
    }
    d.window = window;
    d.runs.clear();
    (d.part, d.n, d.breaks) = (0, 0, 0);
    d.top = ui.cursor().top();
    // Under the text (its runs are on top): a press or drag between lines is the text's, not a
    // drag of the scroll area. Not on a touch screen, where a finger's drag scrolls (as over
    // egui's labels there).
    let touch = ui.input(|i| i.has_touch_screen());
    d.under = d.rect.filter(|_| !touch).map(|r| ui.interact(r, id.with("text-under"), Sense::drag()));
    ts.cur = Some((id, d));
}

/// The runs that follow are part `i` of a long text.
pub(crate) fn part(st: &mut UiState, i: usize) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.part = u32::try_from(i).unwrap_or(u32::MAX - 1);
        d.n = 0;
    }
}

/// The next run starts a line.
pub(crate) fn line(st: &mut UiState) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.breaks = d.breaks.max(1);
    }
}

/// An empty line: the next run starts a paragraph.
pub(crate) fn blank(st: &mut UiState) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.breaks = d.breaks.max(1).saturating_add(1).min(MAX_BREAKS);
    }
}

/// The parts of a long text of `n` parts the selection's ends are in, and those it spans.
pub(crate) fn parts(st: &UiState, n: usize) -> (Vec<usize>, Range<usize>, bool) {
    let Some((_, d)) = &st.text_selection.cur else { return (Vec::new(), 0..0, false) };
    let last = n.saturating_sub(1);
    let part = |m: Mark| usize::try_from(m.part).map_or(last, |p| p.min(last));
    let ends: Vec<usize> = [d.anchor, d.head].into_iter().flatten().map(part).collect();
    let span = d.selection().map_or(0..0, |(lo, hi)| part(lo)..part(hi).saturating_add(1));
    (ends, span, d.all())
}

/// A run of a message's text: laid out as a label, drawn with the selection. `link`: the colour
/// of a link's underline, shown on hover (a click on it is the caller's).
pub(crate) fn run(st: &mut UiState, ui: &mut Ui, text: impl Into<egui::WidgetText>, link: Option<Color32>) -> Response {
    let mut label = egui::Label::new(text).selectable(true);
    if link.is_some() {
        label = label.sense(Sense::click());
    }
    let (pos, galley, response) = label.layout_in_ui(ui);
    let role = if link.is_some() { egui::Role::Link } else { egui::Role::Label };
    response.widget_info(|| egui::WidgetInfo::labeled(role, ui.is_enabled(), galley.text()));
    let hovered = response.hovered();
    let underline = link.filter(|_| hovered).map_or(Stroke::NONE, |c| Stroke::new(1.0, c));
    if link.is_some() && hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = ui.visuals().text_color();
    // Not a message's text (or laid out unseen to be measured): drawn as a label.
    let Some((_, d)) = st.text_selection.cur.as_mut().filter(|_| ui.is_visible()) else {
        if ui.is_visible() && ui.is_rect_visible(response.rect) {
            ui.painter().add(egui::epaint::TextShape::new(pos, galley, color).with_underline(underline));
        }
        return response;
    };
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    let breaks = std::mem::take(&mut d.breaks);
    d.runs.push(Run { part: d.part, n: d.n, breaks, galley, pos, color, underline, painter, slot, response: response.clone(), link: link.is_some() });
    d.n = d.n.saturating_add(1);
    response
}

/// A run that may read right to left: wrapped at the width there is, a right-to-left paragraph
/// against the right edge, where it starts (as [`crate::bidi::add_label`]).
pub(crate) fn rtl_run(st: &mut UiState, ui: &mut Ui, mut job: LayoutJob) -> Response {
    job.wrap.max_width = ui.available_width();
    if crate::bidi::has_rtl(&job.text) && crate::bidi::paragraph_rtl(&job.text) {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| run(st, ui, job, None)).inner
    } else {
        run(st, ui, job, None)
    }
}

/// The text started with [`begin`] ends here: the pointer and keys, the copy, and the runs drawn
/// with what's selected. `whole` is what a copy of all of it takes, when not all of it is laid out.
pub(crate) fn finish(st: &mut UiState, ui: &mut Ui, whole: Option<&str>) {
    let Some((id, mut d)) = st.text_selection.cur.take() else { return };
    let ctx = ui.ctx().clone();
    let left = ui.max_rect().left();
    let rect = Rect::from_min_max(pos2(left, d.top), pos2(ui.max_rect().right().max(left), ui.min_rect().bottom().max(d.top)));
    d.rect = Some(rect);
    let clip = ui.clip_rect();
    d.view = Some(rect.intersect(clip));
    let layer = ui.layer_id();
    d.layer = Some(layer);
    d.pass = ctx.cumulative_pass_nr();
    let (touch, pressed, down, shift, latest, origin, dt, double, triple) = ctx.input(|i| {
        let p = &i.pointer;
        let b = egui::PointerButton::Primary;
        (
            i.has_touch_screen(),
            p.primary_pressed(),
            p.primary_down(),
            i.modifiers.shift,
            p.latest_pos(),
            p.press_origin(),
            i.stable_dt,
            p.button_double_clicked(b),
            p.button_triple_clicked(b),
        )
    });
    if latest.is_some() {
        d.pointer = latest;
    }
    let over = |p: Pos2| rect.intersect(clip).contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
    // A press in the text (on a run or between them): a selection starts there (or, with Shift,
    // goes on to there). On the scroll bar (in the margin beside the text, or over its edge) or a
    // picture in it, the selection stays as it is.
    let ours = d.runs.iter().map(|r| &r.response).chain(&d.under).any(Response::is_pointer_button_down_on);
    let beside = |p: Pos2| clip.contains(p) && !rect.x_range().contains(p.x);
    // Where it was pressed, not where the pointer went on to in the same frame (a quick drag).
    if pressed && let Some(p) = origin.or(d.pointer) {
        if over(p) && ours {
            d.active = true;
            d.h_x = None;
            if let Some((i, c)) = d.nearest(0..d.runs.len(), p, |_| true) {
                let m = d.mark(i, c);
                if !(shift && d.anchor.is_some()) {
                    d.anchor = m;
                }
                d.head = m;
                // A finger's drag scrolls the text instead (the scroll area has it).
                d.dragging = !touch;
            }
        } else if !over(p) && !beside(p) && ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background) {
            // Elsewhere in the window (the message list, the folders): the keyboard is theirs.
            d.active = false;
            d.dragging = false;
            d.anchor = None;
            d.head = None;
        }
    }
    if d.dragging {
        if !down {
            d.dragging = false;
        } else if let Some(p) = d.pointer {
            // At or past the view's top or bottom, it scrolls; the end follows the text that
            // comes under the pointer, kept in the view.
            let dy = edge_delta(p.y, clip, dt);
            if dy != 0.0 {
                ui.scroll_with_delta_animation(vec2(0.0, dy), egui::style::ScrollAnimation::none());
                ctx.request_repaint();
            }
            let view = rect.intersect(clip);
            let q = if view.is_positive() { view.clamp(p) } else { p };
            if let Some((i, c)) = d.nearest(0..d.runs.len(), q, |_| true) {
                d.head = d.mark(i, c);
            }
        }
    }
    if (double || triple)
        && let Some(p) = d.pointer.filter(|p| over(*p))
        && let Some((i, c)) = d.nearest(0..d.runs.len(), p, |_| true)
    {
        d.select_unit(i, c, triple);
    }
    // A message's own window (a viewport of its own, its keys its own): the text has them,
    // clicked into or not, the toolbar pressed or not.
    if d.window && ctx.viewport_id() != egui::ViewportId::ROOT {
        d.active = true;
    }
    if ctx.text_edit_focused() {
        d.active = false;
    }
    if d.active {
        keys(&mut d, ui, rect, whole);
        if ctx.viewport_id() == egui::ViewportId::ROOT {
            st.text_selection.keys = true;
        }
    }
    if d.dragging || d.runs.iter().any(|r| !r.link && r.response.hovered()) {
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    paint(&d, ui);
    let ts = &mut st.text_selection;
    if ts.docs.len() >= KEPT {
        ts.docs.retain(|_, v| v.active);
    }
    d.runs.clear();
    ts.docs.insert(id, d);
}

/// The text's keys: Ctrl+A, Shift with the arrows, Home and End, Escape, and a copy.
fn keys(d: &mut Doc, ui: &mut Ui, rect: Rect, whole: Option<&str>) {
    let os = ui.ctx().os();
    let events = ui.input(|i| i.events.clone());
    let mut used = false;
    let mut show = None;
    for e in &events {
        match e {
            egui::Event::Key { key: Key::A, pressed: true, modifiers, .. } if modifiers.command && !modifiers.shift => {
                d.anchor = Some(Mark::START);
                d.head = Some(Mark::END);
                used = true;
            }
            egui::Event::Key { key, pressed: true, modifiers, .. }
                if modifiers.shift && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End) =>
            {
                show = d.step(os, *key, *modifiers, rect).or(show);
                used = true;
            }
            egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                d.anchor = None;
                d.head = None;
            }
            egui::Event::Copy | egui::Event::Cut if d.selection().is_some() => {
                ui.ctx().copy_text(d.text(whole));
                used = true;
            }
            _ => {}
        }
    }
    if used {
        ui.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key: Key::A, modifiers, .. } => !modifiers.command,
                egui::Event::Key { key, modifiers, .. } => {
                    !(modifiers.shift && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End))
                }
                egui::Event::Copy | egui::Event::Cut => false,
                _ => true,
            })
        });
    }
    if let Some(r) = show {
        ui.scroll_to_rect(r.expand(8.0), None);
    }
}

/// The runs, with what's selected of them shaded, a run's shading meeting the next's on a row.
fn paint(d: &Doc, ui: &Ui) {
    let bg = ui.visuals().selection.bg_fill;
    for (i, r) in d.runs.iter().enumerate() {
        if !r.painter.clip_rect().intersects(r.response.rect) {
            continue;
        }
        let mut galley = r.galley.clone();
        if let Some(range) = d.range_of(i).filter(|g| !g.is_empty()) {
            let sel = CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end));
            egui::text_selection::visuals::paint_text_selection(&mut galley, ui.visuals(), &sel, None);
            if let Some(x) = d.seam(i, &range, ui.ctx().pixels_per_point()) {
                meet(&mut galley, x, bg);
            }
        }
        r.painter.set(r.slot, egui::epaint::TextShape::new(r.pos, galley, r.color).with_underline(r.underline));
    }
}

/// The shading of `galley`'s last row (its rectangle's right edge) moved to end at `x`.
fn meet(galley: &mut Arc<Galley>, x: f32, bg: Color32) {
    let Some(placed) = Arc::make_mut(galley).rows.last_mut() else { return };
    let visuals = &mut Arc::make_mut(&mut placed.row).visuals;
    let shaded = |v: &&mut egui::epaint::Vertex| v.color == bg;
    let Some(right) = visuals.mesh.vertices.iter_mut().filter(shaded).map(|v| v.pos.x).reduce(f32::max) else { return };
    if (x - right).abs() > MAX_SEAM {
        return;
    }
    for v in visuals.mesh.vertices.iter_mut().filter(shaded).filter(|v| v.pos.x >= right) {
        v.pos.x = x;
    }
    visuals.mesh_bounds = visuals.mesh.calc_bounds();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_scrolls_toward_the_edge_it_is_at_faster_the_farther_out() {
        let view = Rect::from_min_max(pos2(0.0, 100.0), pos2(400.0, 500.0));
        assert_eq!(edge_delta(300.0, view, 0.016), 0.0, "nothing away from the edges");
        // On the text by the edges (its last line in view), it stays: it ran away under the pointer.
        assert_eq!(edge_delta(495.0, view, 0.016), 0.0, "nothing on the last line in view");
        assert_eq!(edge_delta(105.0, view, 0.016), 0.0, "or the first");
        assert!(edge_delta(499.5, view, 0.016) < 0.0, "the bottom edge moves the content up");
        assert!(edge_delta(100.5, view, 0.016) > 0.0, "the top edge moves it down");
        assert!(edge_delta(600.0, view, 0.016) < edge_delta(510.0, view, 0.016), "faster the farther past the view");
        assert!(edge_delta(0.0, view, 0.016) > edge_delta(90.0, view, 0.016), "and above it");
        for junk in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(edge_delta(junk, view, 0.016), 0.0);
            assert_eq!(edge_delta(600.0, view, junk), 0.0);
        }
        assert!(edge_delta(f32::MAX, view, 0.016).is_finite());
        assert!(edge_delta(600.0, view, 100.0).abs() <= edge_delta(600.0, view, 0.05).abs(), "a stalled frame doesn't jump");
        let small = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 6.0));
        assert_eq!(edge_delta(5.5, small, 0.016), 0.0, "a view too short to have edges");
    }
}
