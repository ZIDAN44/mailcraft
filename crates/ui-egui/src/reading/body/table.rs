//! The reading pane: a laid-out message's table rows, their cells side by side or stacked, and
//! the widths of its columns.

use super::*;

impl Body<'_> {
    /// A table row: its cells side by side, or one under the other when there's no room.
    /// `columns`: the widths the table's columns ask for (a width set in one row is the whole
    /// column's, as HTML sizes columns).
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
        use mime::layout::Width;
        // The room between the cells side by side comes out of the row's width.
        let between = room.spacing * row.len().saturating_sub(1) as f32;
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
        // Cells that ask for nothing extra (spacers) next to wider ones.
        let used: f32 = widths.iter().sum();
        if used < width
            && let Some(last) = widths.last_mut()
        {
            *last += width - used;
        }
        // Stacked when a column is squeezed too narrow to read; one as narrow as it asks (a
        // business card's photo, issue #11) stays beside the others.
        let squeezed = |w: f32, asked: Option<f32>| w < MIN_COLUMN && asked.is_none_or(|a| w + 0.5 < a);
        let stack = row.len() > 1 && widths.iter().zip(row).zip(&asked).any(|((w, c), a)| squeezed(*w, *a) && !c.blocks.is_empty());
        let cell = |this: &mut Self, st: &mut UiState, ui: &mut Ui, c: &mime::layout::Cell, w: f32| {
            let bg = theme::box_color(c.background, page, &st.tokens);
            let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
            let behind = c.background_image.as_deref().and_then(|src| this.background(st, ui.ctx(), src));
            // Its own padding, else its table's.
            let pad = c.padding.map_or([room.padding; 4], |p| p.map(f32::from));
            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let framed = egui::Frame::new().fill(fill).inner_margin(room_margin(pad)).show(ui, |ui| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let inner = (w - pad[1] - pad[3]).max(1.0);
                    ui.set_width(inner);
                    this.blocks(st, ui, &c.blocks, inner, bg.unwrap_or(page), c.align, depth + 1);
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
        };
        if stack {
            for c in row.iter().filter(|c| !c.blocks.is_empty()) {
                cell(self, st, ui, c, width);
            }
        } else {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = room.spacing;
                for (c, w) in row.iter().zip(&widths) {
                    cell(self, st, ui, c, *w);
                }
            });
        }
    }
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
