//! Dialogs: how each is framed and shown: its window over the app, its title, the part that
//! scrolls and its row of buttons.

use super::*;

/// A dialog's title: a subtitle of the type ramp, with room under it.
pub(crate) fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    let r = ui.label(RichText::new(tl!(s)).font(theme::semibold(crate::sizes::SUBTITLE)).color(t.text));
    // What the dialog is called, to a screen reader ([`show`] names the dialog after it).
    ui.ctx().data_mut(|d| d.insert_temp(title_id(), Some(r.id)));
    ui.add_space(6.0);
}

pub(super) fn title_id() -> egui::Id {
    egui::Id::unique("dialog-title")
}

/// A row of buttons' height.
const ROW_H: f32 = 30.0;
/// The space between the part of a dialog that scrolls and its buttons under it.
const FOOTER_GAP: f32 = 12.0;

/// Where a dialog's row of buttons goes, under the part of it that scrolls, so they're always in
/// sight: as [`show`] laid it out the frame before (`None` as it opens), and whether a row took
/// it this frame.
#[derive(Clone, Copy, Debug, Default)]
struct Footer {
    at: Option<Rect>,
    taken: bool,
}

fn footer_id() -> egui::Id {
    egui::Id::unique("dialog-footer")
}

/// Take the dialog's place for its buttons, if there is one and no row has it yet this frame:
/// `Some(None)` as the dialog opens, its place not known yet.
fn take_footer(ui: &Ui) -> Option<Option<Rect>> {
    let f = ui.data(|d| d.get_temp::<Footer>(footer_id())).filter(|f| !f.taken)?;
    ui.data_mut(|d| d.insert_temp(footer_id(), Footer { taken: true, ..f }));
    Some(f.at)
}

/// A dialog's bottom row, right-aligned and one row tall: a layout given all the room left made
/// every dialog as tall as the last one shown, with its buttons floating halfway down. In a dialog
/// shown by [`show`], under the part that scrolls, where it stays in sight (the whole dialog
/// scrolled, and a tall one's buttons were out of sight below the window).
///
/// `add` adds Cancel (or Close) first and the main button last: the row puts them in the system's
/// order, the main button first on Windows (right to left), last elsewhere (left to right, at the
/// row's end, as wide as it was the frame before).
pub(crate) fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    button_row_from(ui, |_| {}, add)
}

/// [`button_row`] with `start` at the row's other end (a form's Delete, apart from Save and
/// Cancel).
pub(crate) fn button_row_from<R>(ui: &mut Ui, start: impl FnOnce(&mut Ui), add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = match take_footer(ui) {
        Some(Some(rect)) => rect,
        _ => ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover()).0,
    };
    let mut first = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(crate::bidi::row_from_start(egui::Align::Center)));
    first.set_clip_rect(rect.expand(3.0));
    start(&mut first);
    if MAIN_FIRST {
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
        row.set_clip_rect(rect.expand(3.0));
        return add(&mut row);
    }
    let width_id = ui.scope_id().with("button-row-width");
    let width: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(0.0);
    let from = egui::Rect::from_min_max(egui::pos2((rect.right() - width).max(rect.left()), rect.top()), rect.max);
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(from).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.set_clip_rect(rect.expand(3.0));
    let out = add(&mut row);
    let wide = row.min_rect().width();
    if (wide - width).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(width_id, wide));
        ui.ctx().request_discard("a dialog's buttons were measured");
    }
    out
}

/// Whether a row of buttons drawn now goes under the part that scrolls (not after the rest).
fn in_footer(ui: &Ui) -> bool {
    ui.data(|d| d.get_temp::<Footer>(footer_id())).is_some_and(|f| !f.taken && f.at.is_some())
}

/// A Close button on its own row; whether the dialog stays open.
pub(super) fn close_row(ui: &mut Ui) -> bool {
    let t = Tokens::of(ui);
    button_row(ui, |ui| !widgets::secondary_button(ui, "Close", &t, true).clicked())
}

/// Whether a dialog's main button comes before Cancel, as Windows' own dialogs have it; a Mac's
/// and GNOME's have Cancel first, the main button at the end.
pub(crate) const MAIN_FIRST: bool = cfg!(target_os = "windows");

/// A dialog's main button `ok` and Cancel: whether each was clicked.
pub(super) fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    pair(ui, t, widgets::ButtonKind::Primary, ok, ok_enabled)
}

/// A dialog's button `ok` that deletes or removes for good (red, and not Enter's) and Cancel.
pub(super) fn danger_buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    pair(ui, t, widgets::ButtonKind::Danger, ok, ok_enabled)
}

fn pair(ui: &mut Ui, t: &Tokens, kind: widgets::ButtonKind, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    if !in_footer(ui) {
        ui.add_space(10.0);
    }
    button_row(ui, |ui| {
        res.1 = widgets::secondary_button(ui, "Cancel", t, true).clicked();
        res.0 = widgets::dialog_button(ui, kind, ok, t, ok_enabled).clicked();
    });
    // Enter: the main button of a question (nothing in it has the keys), never one that deletes.
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter && ok_enabled && kind == widgets::ButtonKind::Primary && ui.memory(|m| m.focused().is_none()) {
        res.0 = true;
    }
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
    // buttons off a 2160-pixel screen), 24 points clear of its top and bottom, its frame's
    // margins counted (they weren't, and a tall one touched both): what doesn't fit scrolls.
    let max = (ctx.content_rect().height() - 48.0 - frame.total_margin().sum().y).max(200.0);
    // A screen reader hears the dialog's title as it opens; a question that must be answered
    // (delete for good? save before closing?) is an alert, as wordcraft's save prompt is.
    let id = egui::Id::unique(("dialog", std::mem::discriminant(d)));
    let role = if d.is_alert() { egui::accesskit::Role::AlertDialog } else { egui::accesskit::Role::Dialog };
    // Its buttons' place, as laid out the frame before, for its row of buttons to take.
    let foot_id = id.with("footer");
    let foot_was: Option<Rect> = ctx.data(|data| data.get_temp(foot_id));
    ctx.data_mut(|data| data.insert_temp(footer_id(), Footer { at: foot_was, taken: false }));
    let modal = egui::Modal::new(id).frame(frame).show(ctx, |ui| {
        ui.scope_builder(egui::UiBuilder::new().accessibility_role(role), |ui| {
            // Its buttons, text boxes and drop-downs one height, as Windows' own dialogs' (they
            // were 26 points, its main button 30, a form's title 36).
            ui.spacing_mut().interact_size.y = crate::sizes::CONTROL_H;
            ui.spacing_mut().button_padding = egui::vec2(12.0, 4.0);
            // As tall as it needs, up to the window (its buttons under it): only then does it
            // scroll.
            // (egui puts its space between two things there before the gap is added).
            let between = ui.spacing().item_spacing.y;
            let room = (max - foot_was.map_or(0.0, |_| ROW_H + FOOTER_GAP)).max(100.0);
            let scrolled = egui::ScrollArea::vertical()
                .id_salt("dialog-body")
                .max_height(room)
                .min_scrolled_height(room)
                .show(ui, |ui| body(ui, d, s, pics, t, acts));
            // Its buttons' place, taken by its row: laid out again when it moves (as it opens,
            // or grows), so they're never drawn a frame out of place.
            let taken = ui.data(|data| data.get_temp::<Footer>(footer_id())).is_some_and(|f| f.taken);
            let foot = taken.then(|| {
                ui.add_space((FOOTER_GAP - between).max(0.0));
                ui.allocate_exact_size(vec2(scrolled.inner_rect.width(), ROW_H), Sense::hover()).0
            });
            if foot != foot_was {
                ui.data_mut(|data| {
                    if let Some(r) = foot {
                        data.insert_temp(foot_id, r);
                    } else {
                        data.remove::<Rect>(foot_id);
                    }
                });
                ui.ctx().request_discard("a dialog's buttons moved");
            }
            (scrolled.inner, ui.unique_id())
        })
        .inner
    });
    ctx.data_mut(|data| data.remove::<Footer>(footer_id()));
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
    let shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: egui::Color32::from_black_alpha(if t.dark { 120 } else { 45 }) };
    egui::Frame::new()
        .fill(t.surface)
        .corner_radius(crate::sizes::SURFACE_RADIUS)
        .inner_margin(Margin::same(crate::sizes::GAP_L as i8))
        .stroke(egui::Stroke::new(1.0, t.border))
        .shadow(shadow)
}
