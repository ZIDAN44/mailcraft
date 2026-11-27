//! Dialogs: how each is framed and shown: its window over the app, its title, the part that
//! scrolls and its row of buttons.

use super::*;

pub(super) fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    let r = ui.label(RichText::new(tl!(s)).font(theme::semibold(18.0)).color(t.text));
    // What the dialog is called, to a screen reader ([`show`] names the dialog after it).
    ui.ctx().data_mut(|d| d.insert_temp(title_id(), Some(r.id)));
    ui.add_space(6.0);
}

pub(super) fn title_id() -> egui::Id {
    egui::Id::unique("dialog-title")
}

/// A dialog's bottom row, right-aligned and one row tall: a layout given all the room left made
/// every dialog as tall as the last one shown, with its buttons floating halfway down.
pub(super) fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), add).inner
}

/// A Close button on its own row; whether the dialog stays open.
pub(super) fn close_row(ui: &mut Ui) -> bool {
    button_row(ui, |ui| !ui.button(tl!("Close")).clicked())
}

pub(super) fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(10.0);
    button_row(ui, |ui| {
        if widgets::primary_button(ui, ok, t, ok_enabled).clicked() {
            res.0 = true;
        }
        if ui.button(tl!("Cancel")).clicked() {
            res.1 = true;
        }
    });
    res
}

/// Show the dialog; returns whether it stays open. `pics` draws a picture it previews.
pub(crate) fn show(
    ctx: &egui::Context,
    d: &mut Dialog,
    s: &Session,
    pics: &mut crate::reading::Pictures,
    t: &Tokens,
    acts: &mut Vec<Action>,
) -> bool {
    let frame = dialog_frame(t);
    // Each kind of dialog keeps its own size: one id for all let a small one open at the size
    // of the last big one.
    // Never taller than the window (issue #1: an invitation's notes put the dialog's title and
    // buttons off a 2160-pixel screen): what doesn't fit scrolls.
    let max = (ctx.content_rect().height() - 48.0).max(200.0);
    // A screen reader hears the dialog's title as it opens; a question that must be answered
    // (delete for good? save before closing?) is an alert, as wordcraft's save prompt is.
    let id = egui::Id::unique(("dialog", std::mem::discriminant(d)));
    let role = if d.is_alert() { egui::accesskit::Role::AlertDialog } else { egui::accesskit::Role::Dialog };
    let modal = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.scope_builder(egui::UiBuilder::new().accessibility_role(role), |ui| {
            // As tall as it needs, up to the window: only then does it scroll.
            let keep = egui::ScrollArea::vertical()
                .id_salt("dialog-body")
                .max_height(max)
                .min_scrolled_height(max)
                .show(ui, |ui| body(ui, d, s, pics, t, acts))
                .inner;
            (keep, ui.unique_id())
        })
        .inner
    });
    // Named by its title.
    let (keep, node_id) = modal.inner;
    let named = ctx.data_mut(|data| data.remove_temp::<Option<egui::Id>>(title_id()).flatten());
    ctx.accesskit_node_builder(node_id, |node| {
        node.set_modal();
        if let Some(title) = named {
            node.push_labelled_by(title.accesskit_id());
        }
    });
    // The part of a compaction that has a data file to itself can't be stopped, and nothing of
    // its account can be done meanwhile: Escape or a click beside it doesn't put it away.
    let stays = matches!(d, Dialog::Compacting { account, .. } if s.compaction.exclusive(account));
    // Escape (when nothing in it, a list it opened, has it first) and its own buttons close it; a
    // click beside it only one to look at (a click there put away a rule or an automatic reply
    // half written, as Windows' dialogs never do).
    let escape = modal.is_top_modal && !modal.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let beside = modal.backdrop_response.clicked() && d.closes_on_a_click_beside();
    keep && (stays || !(escape || beside || modal.response.should_close()))
}

/// How every dialog looks (the main window's, and those over a message's own window).
pub(crate) fn dialog_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.surface).corner_radius(12).inner_margin(Margin::same(22)).stroke(egui::Stroke::new(1.0, t.border))
}
