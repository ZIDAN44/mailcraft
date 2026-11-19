//! What a screen reader gets of a message's text (issue #175): one document it can go through by
//! line, word and letter, as Outlook's reading pane is. The text is drawn in runs, each laid out
//! as an egui label. With a screen reader there, the text is a document that takes the keyboard
//! (its one stop for Tab, and F6's in the reading pane), the runs' labels are put under it
//! ([`crate::a11y`]) with their text as AccessKit's text runs, and a caret the arrows move
//! (Shift selects, as with the pointer) is told to the reader as the document's selection: what
//! a screen reader reads a line, a word or a letter of as it moves.

use egui::accesskit::{TextPosition, TextSelection};
use egui::emath::TSTransform;
use egui::epaint::text::PlacedRow;

use super::*;

/// The text's document (the text `id`'s), over where the text was last frame: under its runs,
/// taking no pointer.
pub(super) fn document(ui: &mut Ui, id: egui::Id, rect: Rect) -> Response {
    let r = ui.interact(rect, id.with("text-document"), Sense::focusable_noninteractive());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Document, true, tl!("Message text")));
    r
}

/// The document's part of the frame, once the runs are laid out: their text runs, the runs put
/// under it (egui hangs each from the part of the window it's drawn in), and the caret or the
/// selection. With the keyboard, its arrows are its own (egui's
/// would take the keyboard to the next control), and the caret is drawn.
pub(super) fn tell(d: &Doc, ui: &Ui, doc: &Response) {
    let ctx = ui.ctx();
    let mut labels = Vec::with_capacity(d.runs.len());
    for r in &d.runs {
        let global = ctx.layer_transform_to_global(r.painter.layer_id()).unwrap_or_default() * TSTransform::from_translation(r.pos.to_vec2());
        egui::text_selection::accesskit_text::update_accesskit_for_text_widget(ctx, r.response.id, None, global, &r.galley);
        labels.push(r.response.id.accesskit_id());
    }
    crate::a11y::put_under(ctx, doc.id.accesskit_id(), labels);
    let at = |m: Mark| {
        let (i, c) = d.resolve(m)?;
        d.runs.get(i).map(|r| (position(r, c), r.caret(c)))
    };
    let (Some((anchor, _)), Some((focus, caret))) = (d.anchor.and_then(at), d.head.and_then(at)) else { return };
    ctx.accesskit_node_builder(doc.id, |n| n.set_text_selection(TextSelection { anchor, focus }));
    if doc.has_focus() {
        let arrows = egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() };
        ctx.memory_mut(|m| m.set_focus_lock_filter(doc.id, arrows));
        if d.selection().is_none() {
            ui.painter().line_segment([caret.center_top(), caret.center_bottom()], Stroke::new(1.5, ui.visuals().text_color()));
        }
    }
}

/// Where `c` is in run `r`, as AccessKit names it: the text run egui made on its row that holds
/// it, and the letter in that.
fn position(r: &Run, c: CCursor) -> TextPosition {
    let at = r.galley.layout_from_cursor(c);
    let column = at.column.0;
    let runs = r.galley.rows.get(at.row).map(row_runs).unwrap_or_default();
    let k = runs
        .iter()
        .position(|run| if c.prefer_next_row { column < run.end } else { column <= run.end })
        .unwrap_or_else(|| runs.len().saturating_sub(1));
    let start = runs.get(k).map_or(0, |run| run.start);
    TextPosition { node: r.response.id.with(at.row).with(k).accesskit_id(), character_index: column.saturating_sub(start) }
}

/// The most letters in one of egui's text runs (AccessKit's word starts are a byte each).
const RUN_MAX: usize = 255;

/// A row's text runs as egui makes them for AccessKit (its own function for it isn't public): a
/// stretch of one reading direction, at most [`RUN_MAX`] letters, the row's line break in its
/// last.
fn row_runs(row: &PlacedRow) -> Vec<Range<usize>> {
    let glyphs = &row.glyphs;
    let total = glyphs.len() + usize::from(row.ends_with_newline);
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..glyphs.len() {
        let level = |j: usize| glyphs.get(j).map(|g| g.bidi_level);
        if level(i) != level(i - 1) || i - start == RUN_MAX {
            runs.push(start..i);
            start = i;
        }
    }
    if total - start > RUN_MAX {
        runs.push(start..glyphs.len());
        start = glyphs.len();
    }
    runs.push(start..total);
    runs
}
