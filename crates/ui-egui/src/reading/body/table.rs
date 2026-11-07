//! The reading pane: a laid-out message's table rows, their cells side by side or stacked, and
//! the widths of its columns.

use super::*;

impl Body<'_> {
    /// A table row: its cells side by side, or one under the other when there's no room.
    /// `columns`: the widths the table's columns ask for (a width set in one row is the whole
    /// column's, as HTML sizes columns). Side by side, the row is as tall as its tallest cell (or
    /// the height one asks for, issue #47): each cell's background fills it, and its content sits
    /// at the top, in the middle or at the bottom, as its `valign` says. Heights are those the
    /// cells' content took the frame before (egui lays out once), so a row settles a frame on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn row(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        row: &[mime::layout::Cell],
        columns: &[Option<mime::layout::Width>],
        width: f32,
        page: [u8; 3],
        room: Room,
        depth: usize,
    ) {
        let (widths, stack) = cell_widths(row, columns, width, room.spacing);
        let heights_id = ui.next_auto_id().with("cell-heights");
        let known: Vec<f32> = ui.ctx().data(|d| d.get_temp(heights_id)).unwrap_or_default();
        // Each cell's padding: its own, else its table's.
        let pads: Vec<[f32; 4]> = row.iter().map(|c| c.padding.map_or([room.padding; 4], |p| p.map(f32::from))).collect();
        let outer: Vec<f32> = pads.iter().enumerate().map(|(i, p)| known.get(i).copied().unwrap_or(0.0) + p[0] + p[2]).collect();
        let asked: Vec<f32> = row.iter().map(|c| c.height.map_or(0.0, f32::from)).collect();
        let valigns: Vec<mime::layout::VAlign> = row.iter().map(|c| c.valign).collect();
        let (row_h, offsets) = if stack { (0.0, vec![0.0; row.len()]) } else { row_heights(&outer, &asked, &valigns) };
        let cell = |this: &mut Self, st: &mut UiState, ui: &mut Ui, i: usize, w: f32| -> f32 {
            let (Some(c), Some(pad)) = (row.get(i), pads.get(i).copied()) else { return 0.0 };
            let bg = theme::box_color(c.background, page, &st.tokens);
            let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
            let behind = c.background_image.as_deref().and_then(|src| this.background(st, ui.ctx(), src));
            // As tall as the row (or as it asks, stacked), its content where it says.
            let tall = if stack { asked.get(i).copied().unwrap_or(0.0) } else { row_h };
            let down = offsets.get(i).copied().unwrap_or(0.0);
            let mut content = 0.0;
            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let framed = egui::Frame::new().fill(fill).inner_margin(room_margin(pad)).show(ui, |ui| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let inner = (w - pad[1] - pad[3]).max(1.0);
                    ui.set_width(inner);
                    ui.set_min_height((tall - pad[0] - pad[2]).max(0.0));
                    ui.add_space(down);
                    let top = ui.cursor().top();
                    this.blocks(st, ui, &c.blocks, inner, bg.unwrap_or(page), c.align, depth + 1);
                    content = (ui.cursor().top() - top).max(0.0);
                    slot
                });
                let rect = framed.response.rect;
                if let Some(tex) = &behind {
                    ui.painter().set(framed.inner, cover(tex, rect));
                }
                // Boxes placed over it; the cell grows to hold any that reach past its end.
                let mut bottom = rect.bottom();
                for o in &c.overlays {
                    bottom = bottom.max(this.overlay(st, ui, o, rect, bg.unwrap_or(page), depth));
                }
                if bottom > rect.bottom() {
                    ui.add_space(bottom - rect.bottom());
                }
            });
            content
        };
        let mut took = vec![0.0; row.len()];
        if stack {
            for (i, c) in row.iter().enumerate() {
                if !c.blocks.is_empty()
                    && let Some(t) = took.get_mut(i)
                {
                    *t = cell(self, st, ui, i, width);
                }
            }
        } else {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = room.spacing;
                for (i, w) in widths.iter().enumerate() {
                    let h = cell(self, st, ui, i, *w);
                    if let Some(t) = took.get_mut(i) {
                        *t = h;
                    }
                }
            });
        }
        // Heights that changed (the first frame, a picture come, a narrower window): the row is
        // laid out again with them.
        if !ui.is_sizing_pass() && (took.len() != known.len() || took.iter().zip(&known).any(|(a, b)| (a - b).abs() > 0.5)) {
            ui.ctx().data_mut(|d| d.insert_temp(heights_id, took));
            ui.ctx().request_repaint();
        }
    }
}

/// Each cell's width in a row `width` wide, `spacing` between the cells, `columns` the widths the
/// table's columns ask for; and whether the cells stack instead, one too narrow to read beside
/// the others. What cells ask for (pixels, or a share of the row as a browser takes it), the
/// rest shared out by those that ask for nothing; all of it shared in proportion when they ask
/// for more than there is.
pub(crate) fn cell_widths(row: &[mime::layout::Cell], columns: &[Option<mime::layout::Width>], width: f32, spacing: f32) -> (Vec<f32>, bool) {
    use mime::layout::Width;
    // The room between the cells side by side comes out of the row's width.
    let between = spacing * row.len().saturating_sub(1) as f32;
    let width = (width - between).max(1.0);
    let px = |w: Width| match w {
        Width::Px(px) => px as f32,
        Width::Percent(pc) => width * f32::from(pc) / 100.0,
    };
    // Widths: what cells (or their columns) ask for, the rest shared by the others.
    let mut col = 0usize;
    let asked: Vec<Option<f32>> = row
        .iter()
        .map(|c| {
            let span = usize::from(c.span.max(1));
            let from_columns = || {
                let parts = columns.get(col..col + span)?;
                (parts.len() == span && parts.iter().all(Option::is_some)).then(|| parts.iter().flatten().map(|w| px(*w)).sum::<f32>())
            };
            let w = c.width.map(px).or_else(from_columns);
            col += span;
            w
        })
        .collect();
    let fixed: f32 = asked.iter().flatten().sum();
    let free = asked.iter().filter(|a| a.is_none()).count();
    let mut widths: Vec<f32> = if fixed > width || (free > 0 && width - fixed < MIN_COLUMN * free as f32) {
        // Asked for more than there is: shared out in proportion.
        let total: f32 = asked.iter().map(|a| a.unwrap_or(MIN_COLUMN)).sum::<f32>().max(1.0);
        asked.iter().map(|a| a.unwrap_or(MIN_COLUMN) / total * width).collect()
    } else {
        let each = if free > 0 { (width - fixed) / free as f32 } else { 0.0 };
        asked.iter().map(|a| a.unwrap_or(each)).collect()
    };
    // Room left over: shared by the cells that ask for a share of the row, in proportion, as a
    // browser shares it (three cells of 33% fill it equally); else the last cell's (a spacer next
    // to a wider cell asks for nothing more).
    let used: f32 = widths.iter().sum();
    let shares: f32 = row.iter().zip(&widths).filter(|(c, _)| matches!(c.width, Some(Width::Percent(_)))).map(|(_, w)| *w).sum();
    if used < width && shares > 0.0 {
        let grow = (width - used) / shares;
        for (w, c) in widths.iter_mut().zip(row) {
            if matches!(c.width, Some(Width::Percent(_))) {
                *w += *w * grow;
            }
        }
    } else if used < width
        && let Some(last) = widths.last_mut()
    {
        *last += width - used;
    }
    // Stacked when a column is squeezed too narrow to read; one as narrow as it asks (a
    // business card's photo, issue #11) stays beside the others.
    let squeezed = |w: f32, asked: Option<f32>| w < MIN_COLUMN && asked.is_none_or(|a| w + 0.5 < a);
    let stack = row.len() > 1 && widths.iter().zip(row).zip(&asked).any(|((w, c), a)| squeezed(*w, *a) && !c.blocks.is_empty());
    (widths, stack)
}

/// A row's height and how far down each cell's content starts: `outer` each cell's height with
/// its padding as last laid out, `asked` the height each asks for at least. The row is as tall as
/// the tallest of either; content sits at the top, in the middle or at the bottom of what's left.
pub(crate) fn row_heights(outer: &[f32], asked: &[f32], valign: &[mime::layout::VAlign]) -> (f32, Vec<f32>) {
    use mime::layout::VAlign;
    let tallest = outer.iter().chain(asked).fold(0.0f32, |a, b| a.max(*b));
    let offsets = outer
        .iter()
        .zip(valign)
        .map(|(h, v)| match v {
            VAlign::Top => 0.0,
            VAlign::Middle => ((tallest - h) / 2.0).max(0.0),
            VAlign::Bottom => (tallest - h).max(0.0),
        })
        .collect();
    (tallest, offsets)
}

/// A table's room: between and around its cells, and inside those that set none of their own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Room {
    pub(super) spacing: f32,
    pub(super) padding: f32,
}

/// Room in pixels (top, right, bottom, left) as a frame's margin, held to what one takes.
pub(super) fn room_margin(p: [f32; 4]) -> Margin {
    let m = |v: f32| v.round().clamp(0.0, 120.0) as i8;
    Margin { top: m(p[0]), right: m(p[1]), bottom: m(p[2]), left: m(p[3]) }
}

/// The width each column of a table asks for: the first cell of one column that sets one, in any
/// row (HTML sizes a whole column so; only the first row set it in a report whose other rows then
/// lined up elsewhere).
pub(crate) fn column_widths(rows: &[Vec<mime::layout::Cell>]) -> Vec<Option<mime::layout::Width>> {
    const MAX_COLUMNS: usize = 64;
    let mut out: Vec<Option<mime::layout::Width>> = Vec::new();
    for row in rows {
        let mut col = 0usize;
        for c in row {
            let span = usize::from(c.span.max(1));
            if span == 1 && col < MAX_COLUMNS {
                if out.len() <= col {
                    out.resize(col + 1, None);
                }
                if let (Some(slot), Some(w)) = (out.get_mut(col), c.width)
                    && slot.is_none()
                {
                    *slot = Some(w);
                }
            }
            col = col.saturating_add(span);
        }
    }
    out
}

#[cfg(test)]
mod tests;
