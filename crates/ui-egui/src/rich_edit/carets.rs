//! Where the cursor goes before each character of the laid-out text, and the selection drawn
//! from that while the text doesn't have the keyboard.

use egui::text::CharIndex;

use super::*;

/// The most characters of a selection drawn behind the text.
const MOST: usize = 20_000;

/// Where the cursor is drawn before characters asked for in order, as
/// [`Galley::pos_from_cursor`] has it, without looking for each one's row from the first row
/// (that made drawing a long selection rows × characters a frame).
pub(super) struct Carets<'a> {
    g: &'a Galley,
    /// The row of the last character asked for, and the character it starts at.
    row: usize,
    start: usize,
    /// How far right the rows reach (an empty right-to-left row's cursor goes there).
    right: Option<f32>,
}

impl<'a> Carets<'a> {
    pub(super) fn new(g: &'a Galley) -> Carets<'a> {
        Carets { g, row: 0, start: 0, right: None }
    }

    /// The cursor before character `i` (a rectangle no wider than a line, as tall as its row).
    /// Quickest asked for in order: an earlier character looks from the first row again.
    pub(super) fn at(&mut self, i: usize) -> Rect {
        if i < self.start {
            (self.row, self.start) = (0, 0);
        }
        // Its row: the first that ends at it or after it.
        while let Some(r) = self.g.rows.get(self.row) {
            if i <= self.start.saturating_add(r.char_count_excluding_newline().0) {
                let x = self.x(self.row, i.saturating_sub(self.start));
                return Rect::from_min_max(egui::pos2(x, r.min_y()), egui::pos2(x, r.max_y()));
            }
            self.start = self.start.saturating_add(r.char_count_including_newline().0);
            self.row += 1;
        }
        // Past the end: after the last row's last character.
        let last = self.g.rows.len().saturating_sub(1);
        match self.g.rows.get(last) {
            Some(r) => {
                let x = self.x(last, r.char_count_including_newline().0);
                Rect::from_min_max(egui::pos2(x, r.min_y()), egui::pos2(x, r.max_y()))
            }
            None => Rect::ZERO,
        }
    }

    /// The cursor's x in row `row` before its character `column`, sticking to the character
    /// before it (as a `CCursor::new` does).
    fn x(&mut self, row: usize, column: usize) -> f32 {
        let Some(r) = self.g.rows.get(row) else { return 0.0 };
        if r.glyphs.is_empty() && r.rtl_paragraph && self.g.job.halign == egui::Align::Min && self.g.job.halign_min_is_start {
            let rows = &self.g.rows;
            return *self.right.get_or_insert_with(|| rows.iter().map(|row| row.pos.x + row.size.x).fold(0.0, f32::max));
        }
        r.pos.x + r.x_offset_with_affinity(CharIndex(column), false)
    }
}

/// The rectangles a selection covers, one per row (galley coordinates). Of a long selection,
/// the first [`MOST`] characters are drawn.
pub(super) fn selection_rects(g: &Galley, sel: Range<usize>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    let mut carets = Carets::new(g);
    let mut a = carets.at(sel.start);
    for i in sel.start..sel.end.min(sel.start + MOST) {
        let b = carets.at(i + 1);
        // A line's end: the selection runs a little past the text, as text boxes show it.
        let right = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + 5.0 };
        let r = Rect::from_min_max(egui::pos2(a.min.x, a.min.y), egui::pos2(right.max(a.min.x), a.max.y));
        match out.last_mut() {
            Some(last) if (last.min.y - r.min.y).abs() < 0.5 => *last = last.union(r),
            _ => out.push(r),
        }
        a = b;
    }
    out
}
