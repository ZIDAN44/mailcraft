//! A recipient box's people (To, Cc, Bcc), each a chip (issue #269): its name, or its address
//! when it has none, all of it on hover. Its × takes it out; dragged onto another box it moves
//! there; its right-click menu moves, copies, edits or takes it out; a double-click puts it back
//! into the text to edit. One that isn't an address is outlined in red. The box's text is what's
//! kept and sent: the chips are its entries ([`addr::split_entries`]), the text box after them
//! what's still being typed.

use mailcraft_engine::model::addr;

use crate::dialogs::Recipients;

use super::*;

/// What a dragged chip carries: the box it's from, and its entry as written there.
#[derive(Clone)]
pub(crate) struct DragRecipient {
    pub(crate) from: Recipients,
    pub(crate) entry: String,
}

/// What was done to a box's chips this frame: done by [`apply`], as a move takes two boxes.
pub(crate) enum ChipDone {
    /// Its `n`th entry taken out.
    Remove(usize),
    /// Its `n`th entry put back into the text being typed.
    Edit(usize),
    /// Its `n`th entry moved to another box.
    Move(usize, Recipients),
    /// A chip of another box dropped on it.
    Dropped(DragRecipient),
}

/// The widest a chip is drawn; a longer name is cut short (all of it on hover).
const CHIP_MAX: f32 = 260.0;

/// The text box after the chips is at least this wide, else it goes to a line of its own.
const TYPING_MIN: f32 = 140.0;

/// A box's entries: each as written, and what's being typed after them.
pub(super) fn entries(value: &str) -> (Vec<String>, String) {
    let (ranges, at) = addr::split_entries(value);
    let entries = ranges.into_iter().filter_map(|r| value.get(r)).map(str::to_string).collect();
    (entries, value.get(at..).unwrap_or_default().to_string())
}

/// `field`'s chips, a row wrapped as they need, then `typing` (the text box, drawn by `edit`).
/// A chip of another box dropped anywhere on it is taken. What was done, for [`apply`].
pub(super) fn chips_and_typing<R>(ui: &mut Ui, t: &Tokens, field: Recipients, value: &str, edit: impl FnOnce(&mut Ui) -> R) -> (R, Vec<ChipDone>) {
    let (list, _) = entries(value);
    let mut done = Vec::new();
    let width = ui.available_width();
    // From where the interface's text starts (the right in Arabic and Hebrew), as the rows are.
    let inner = ui.allocate_ui_with_layout(vec2(width, ROW_H - 4.0), crate::bidi::row_from_start(Align::Center).with_main_wrap(true), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, entry) in list.iter().enumerate() {
            chip(ui, t, field, i, entry, &mut done);
        }
        // The text box in what's left of the row (a wrapping row's available width is all of
        // it: the box asked for that, and went to a line of its own), or on a line of its own
        // when that's too little.
        if !list.is_empty() {
            ui.add_space(4.0);
        }
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut room = ui.available_rect_before_wrap().width();
        if room < TYPING_MIN {
            ui.end_row();
            room = width;
        }
        ui.allocate_ui_with_layout(vec2((room - 1.0).max(1.0), ROW_H - 4.0), crate::bidi::row_from_start(Align::Center), edit).inner
    });
    // A chip of another box dropped on this one.
    let area = ui.interact(inner.response.rect, ui.scope_id().with(("recipient-drop", field.code())), Sense::hover());
    let carrying = egui::DragAndDrop::payload::<DragRecipient>(ui.ctx()).is_some_and(|p| p.from != field);
    if carrying && area.contains_pointer() {
        ui.painter().rect_stroke(inner.response.rect.expand(2.0), 4.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    if carrying && let Some(p) = area.dnd_release_payload::<DragRecipient>() {
        done.push(ChipDone::Dropped((*p).clone()));
    }
    (inner.inner, done)
}

/// One entry's chip: its name or address, × to take it out, dragged to another box, its menu.
fn chip(ui: &mut Ui, t: &Tokens, field: Recipients, i: usize, entry: &str, done: &mut Vec<ChipDone>) {
    let person = addr::parse_list(entry).ok().and_then(|list| list.into_iter().next());
    let label = person.as_ref().map_or_else(|| entry.to_string(), |p| p.display().to_string());
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(label.clone(), font, t.text);
    let w = (galley.size().x + 32.0).clamp(48.0, CHIP_MAX.min(ui.available_width().max(48.0)));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H - 8.0), Sense::click_and_drag());
    let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
    let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
    widgets::named(ui, &x, &format!("{}: {label}", tl!("Remove")), true, None);
    widgets::named(ui, &resp, &label, true, None);
    let p = ui.painter();
    let line = if person.is_none() { egui::Stroke::new(1.5, t.danger) } else { egui::Stroke::new(1.0, t.faint.gamma_multiply(0.6)) };
    p.rect(rect, (ROW_H - 8.0) / 2.0, if resp.hovered() || resp.dragged() { t.hover } else { t.hover_row }, line, egui::StrokeKind::Inside);
    let text_room = Rect::from_min_max(egui::pos2(rect.left() + 9.0, rect.top()), egui::pos2(x_rect.left() - 2.0, rect.bottom()));
    p.with_clip_rect(text_room).galley(egui::pos2(text_room.left(), rect.center().y - galley.size().y / 2.0), galley, t.text);
    if x.hovered() {
        p.rect_filled(x_rect, 8.0, t.hover);
    }
    icons::paint(p, x_rect.shrink(4.0), Icon::Close, t.dim);
    let resp = match &person {
        Some(_) => resp.on_hover_text(entry),
        None => resp.on_hover_text(tf!("Not an email address: {address}", address = entry)),
    };
    if x.clicked() {
        done.push(ChipDone::Remove(i));
    }
    if resp.double_clicked() {
        done.push(ChipDone::Edit(i));
    }
    if resp.drag_started() {
        resp.dnd_set_drag_payload(DragRecipient { from: field, entry: entry.to_string() });
    }
    // Who's being moved, beside the pointer, as a dragged message says how many.
    if resp.dragged() {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(label.as_str());
        });
    }
    resp.context_menu(|ui| {
        for (to, name) in [(Recipients::To, "Move to To"), (Recipients::Cc, "Move to Cc"), (Recipients::Bcc, "Move to Bcc")] {
            if to != field && ui.button(tl!(name)).clicked() {
                done.push(ChipDone::Move(i, to));
            }
        }
        ui.separator();
        if ui.button(tl!("Copy address")).clicked() {
            ui.ctx().copy_text(person.as_ref().map_or_else(|| entry.to_string(), |p| p.email.clone()));
        }
        if ui.button(tl!("Edit")).clicked() {
            done.push(ChipDone::Edit(i));
        }
        if ui.button(tl!("Remove")).clicked() {
            done.push(ChipDone::Remove(i));
        }
    });
}

/// What was done to `field`'s chips, done to the message's boxes. A chip put back to edit is
/// the text being typed (what was being typed becomes a chip of its own); one moved to Bcc
/// shows Bcc. Whether one was put back to edit (its box then takes the keyboard).
pub(crate) fn apply(c: &mut mailcraft_engine::Compose, field: Recipients, done: Vec<ChipDone>) -> bool {
    let mut editing = false;
    for d in done {
        let (mut list, typing) = entries(field.of(&mut c.to, &mut c.cc, &mut c.bcc));
        match d {
            ChipDone::Remove(i) if i < list.len() => {
                list.remove(i);
                *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
            }
            ChipDone::Edit(i) if i < list.len() => {
                let entry = list.remove(i);
                if !typing.trim().is_empty() {
                    list.push(typing.trim().to_string());
                }
                *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &entry);
                editing = true;
            }
            ChipDone::Move(i, to) if i < list.len() && to != field => {
                let entry = list.remove(i);
                *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
                add_entry(c, to, entry);
            }
            ChipDone::Dropped(p) if p.from != field => {
                let (mut from, from_typing) = entries(p.from.of(&mut c.to, &mut c.cc, &mut c.bcc));
                let Some(at) = from.iter().position(|e| *e == p.entry) else { continue };
                let entry = from.remove(at);
                *p.from.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&from, &from_typing);
                add_entry(c, field, entry);
            }
            _ => {}
        }
    }
    editing
}

/// `entry` put last among `to`'s chips, before what's being typed there; Bcc shown for it.
fn add_entry(c: &mut mailcraft_engine::Compose, to: Recipients, entry: String) {
    if to == Recipients::Bcc {
        c.show_bcc = true;
    }
    let (mut list, typing) = entries(to.of(&mut c.to, &mut c.cc, &mut c.bcc));
    list.push(entry);
    *to.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
}
