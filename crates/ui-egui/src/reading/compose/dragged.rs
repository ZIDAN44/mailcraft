//! Messages dragged from the list onto a message being written (issue #270): attached to it, each
//! an `.eml`, as Outlook attaches a message dropped on one. On a message written in the main
//! window egui sees the drop; on one in a window of its own it doesn't (the main window has the
//! mouse until the button is let go), so where the drag ends is found on the screen.

use egui::{Pos2, ViewportId};

use crate::panes::list::DragKeys;

use super::*;

/// Whether messages from the list are being dragged.
pub(crate) fn dragging(ctx: &egui::Context) -> bool {
    egui::DragAndDrop::has_payload_of_type::<DragKeys>(ctx)
}

/// Where the last pass a drag was over a message being written is kept ([`over_message`]).
fn over_id() -> egui::Id {
    egui::Id::unique("messages-over-a-message")
}

/// A drag from the list is over a message being written this pass: what it carries is attached
/// when it's let go, and the drag says so.
pub(crate) fn note_over(ctx: &egui::Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(over_id(), pass));
}

/// Whether the drag from the list was over a message being written, this pass or the one before
/// (the list draws its drag before the message is drawn).
pub(crate) fn over_message(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(over_id())).is_some_and(|at| at.saturating_add(1) >= pass)
}

/// The message being written in the main window: messages let go over it are attached, and its
/// edge says it takes them while they're over it.
pub(super) fn onto_message(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    if !dragging(ui.ctx()) {
        return;
    }
    let rect = ui.max_rect();
    let resp = ui.interact(rect, ui.scope_id().with("messages-dropped"), Sense::hover());
    if resp.contains_pointer() {
        note_over(ui.ctx());
        ui.painter().rect_stroke(rect.shrink(2.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    if resp.dnd_release_payload::<DragKeys>().is_some() {
        acts.push(Action::Run("compose.attachMessages", json!({})));
    }
}

/// Where the pointer is on the screen, in egui's points: the system's cursor where it's known
/// (`os`, in physical pixels, Windows), else egui's last place for it in the main window plus
/// where that window is (the main window keeps getting the pointer while a drag holds the
/// button, on every system). `None` where windows aren't placed on a screen (Wayland).
pub(crate) fn pointer_on_screen(ctx: &egui::Context, os: Option<[i32; 2]>) -> Option<Pos2> {
    if let Some([x, y]) = os {
        let ppp = ctx.pixels_per_point();
        return (ppp > 0.0).then(|| egui::pos2(x as f32 / ppp, y as f32 / ppp));
    }
    let local = ctx.input(|i| i.pointer.latest_pos())?;
    let window = ctx.input(|i| i.viewport().inner_rect)?;
    Some(window.min + local.to_vec2())
}

/// Where the drag was last seen on the screen: `now` when it's known (kept), else the place kept.
/// Let go outside the main window, the pointer leaves it in the same frame (X11's grab ends
/// with the button), and where it was let go would be lost.
pub(crate) fn last_place(ctx: &egui::Context, now: Option<Pos2>) -> Option<Pos2> {
    let id = egui::Id::unique("messages-dragged-on-screen");
    match now {
        Some(at) => {
            ctx.data_mut(|d| d.insert_temp(id, at));
            Some(at)
        }
        None => ctx.data(|d| d.get_temp::<Pos2>(id)),
    }
}

/// Of the messages being written in windows of their own (each its id and its window's), the
/// one whose window is under `at` on the screen. Where the main window is, it's the main
/// window's: a drag from its list brought it to the front, over any message window it overlaps
/// (one let go over a folder there went into a message behind it).
pub(crate) fn window_under(ctx: &egui::Context, windows: &[(u64, ViewportId)], at: Pos2) -> Option<(u64, ViewportId)> {
    let rect_of = |v: ViewportId| ctx.input_for(v, |i| i.viewport().outer_rect.or(i.viewport().inner_rect));
    if rect_of(ViewportId::ROOT).is_some_and(|r| r.contains(at)) {
        return None;
    }
    windows.iter().copied().find(|(_, v)| rect_of(*v).is_some_and(|r| r.contains(at)))
}
