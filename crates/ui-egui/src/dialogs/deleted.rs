//! Dialogs: what a list in a dialog just lost (a rule, a category), named over the list with
//! Undo, so deleting one needs no question first and a slip isn't for good.

use super::*;

/// The last thing deleted from the list `list` while its dialog is open: what Undo puts back.
/// Forgotten once the dialog closes (it isn't drawn for a frame), so it isn't offered the next
/// time the dialog opens.
pub(super) fn last<T: Clone + Send + Sync + 'static>(ui: &Ui, list: &str) -> Option<T> {
    let (kept, seen) = (egui::Id::unique(("deleted", list)), egui::Id::unique(("deleted-seen", list)));
    let now = ui.ctx().cumulative_pass_nr();
    let before: Option<u64> = ui.data(|d| d.get_temp(seen));
    ui.data_mut(|d| d.insert_temp(seen, now));
    if before.is_none_or(|last| last.saturating_add(2) < now) {
        ui.data_mut(|d| d.remove::<T>(kept));
        return None;
    }
    ui.data(|d| d.get_temp(kept))
}

/// Remember `what` was just deleted from `list` (taking the place of what was before).
pub(super) fn remember<T: Clone + Send + Sync + 'static>(ui: &Ui, list: &str, what: T) {
    ui.data_mut(|d| d.insert_temp(egui::Id::unique(("deleted", list)), what));
}

/// Forget what was deleted from `list` (put back, or something else changed).
pub(super) fn forget<T: Clone + Send + Sync + 'static>(ui: &Ui, list: &str) {
    ui.data_mut(|d| d.remove::<T>(egui::Id::unique(("deleted", list))));
}

/// "“Name” deleted. Undo": whether Undo was clicked.
pub(super) fn undo_line(ui: &mut Ui, t: &Tokens, name: &str) -> bool {
    let mut undo = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tf!("“{name}” deleted.", name = name)).color(t.dim));
        undo = ui.add(egui::Link::new(RichText::new(tl!("Undo")).color(t.accent))).clicked();
    });
    ui.add_space(4.0);
    undo
}
