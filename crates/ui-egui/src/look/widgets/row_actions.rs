//! A list row's buttons (Edit, Delete…), seen while the row is pointed at or one of them has the
//! keyboard: always there to click or reach with Tab, drawn only then (on every row at once, the
//! list was a column of pencils and bins).

use super::*;

/// Whether a row's buttons are seen now: the pointer on `row`, or the keyboard on something in
/// it (where it was last laid out).
pub(crate) fn row_actions_shown(ui: &Ui, row: egui::Rect) -> bool {
    let focused = ui.ctx().memory(|m| m.focused()).and_then(|id| ui.ctx().read_response(id));
    ui.rect_contains_pointer(row) || focused.is_some_and(|r| row.contains_rect(r.rect))
}

/// `add`, a row's buttons, seen only when [`row_actions_shown`].
pub(crate) fn row_actions<R>(ui: &mut Ui, row: egui::Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    let shown = row_actions_shown(ui, row);
    ui.scope(|ui| {
        if !shown {
            ui.multiply_opacity(0.0);
        }
        add(ui)
    })
    .inner
}
