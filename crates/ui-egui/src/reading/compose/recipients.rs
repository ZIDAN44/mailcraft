//! A recipient box's people (To, Cc, Bcc), each a chip (issue #269): its name, or its address
//! when it has none, all of it on hover. The box's text is what's kept and sent: the chips are
//! its entries ([`addr::split_entries`]), the text box after them what's still being typed.
//!
//! A chip's × takes it out. A click selects it, Ctrl+click adds one to those selected and
//! Shift+click the ones between; with nothing typed, ← selects the last, ← and → move along
//! them (with Shift, more of them), Backspace or Delete takes those selected out, Esc lets them
//! go. Dragged, those selected move: onto another box, or to another place in theirs, where the
//! line shows. The right-click menu moves, copies, edits or takes them out; a double-click puts
//! one back into the text to edit. One that isn't an address is outlined in red. A box not being
//! written in shows the chips that fit on a line, and how many more.

use std::collections::BTreeSet;
use std::sync::Arc;

use mailcraft_engine::model::addr;

use crate::dialogs::Recipients;

use super::*;

/// What a drag of chips carries: the box they're from, and their entries as written there.
#[derive(Clone)]
pub(crate) struct DragRecipient {
    pub(crate) from: Recipients,
    pub(crate) entries: Vec<String>,
}

/// What was done to a box's chips this frame: done by [`apply`], as a move takes two boxes.
pub(crate) enum ChipDone {
    /// These entries taken out.
    Remove(Vec<usize>),
    /// This entry put back into the text being typed.
    Edit(usize),
    /// These entries moved to the end of another box.
    Move(Vec<usize>, Recipients),
    /// Chips dropped on this box (from it or another), to go before its entry `at` (its end
    /// when past them).
    Dropped(DragRecipient, usize),
}

/// The widest a chip is drawn; a longer name is cut short (all of it on hover).
const CHIP_MAX: f32 = 260.0;

/// The text box after the chips is at least this wide, else it goes to a line of its own.
const TYPING_MIN: f32 = 140.0;

/// The text box after the chips of a box folded to one line ([`chips_and_typing`]) is at least
/// this wide.
const FOLDED_TYPING: f32 = 40.0;

/// A box's entries: each as written, and what's being typed after them.
pub(super) fn entries(value: &str) -> (Vec<String>, String) {
    let (ranges, at) = addr::split_entries(value);
    let entries = ranges.into_iter().filter_map(|r| value.get(r)).map(str::to_string).collect();
    (entries, value.get(at..).unwrap_or_default().to_string())
}

/// A box's chips selected, by their places, kept between frames under its text box's id.
#[derive(Clone, Default)]
pub(crate) struct Selected {
    pub(crate) chips: BTreeSet<usize>,
    /// Where Shift+click or Shift+arrow selects from.
    anchor: usize,
    /// The end the arrows move.
    at: usize,
}

impl Selected {
    fn one(i: usize) -> Selected {
        Selected { chips: BTreeSet::from([i]), anchor: i, at: i }
    }

    /// From the anchor to `i`.
    fn to(&mut self, i: usize) {
        let (a, b) = (self.anchor.min(i), self.anchor.max(i));
        self.chips = (a..=b).collect();
        self.at = i;
    }

    /// Those selected that are among `n` chips.
    fn within(mut self, n: usize) -> Selected {
        self.chips.retain(|i| *i < n);
        self
    }
}

fn selected_id(edit_id: egui::Id) -> egui::Id {
    edit_id.with("chips-selected")
}

/// The box's chips selected (those among `n`).
pub(crate) fn selected(ctx: &egui::Context, edit_id: egui::Id, n: usize) -> Selected {
    ctx.data(|d| d.get_temp::<Selected>(selected_id(edit_id))).unwrap_or_default().within(n)
}

fn keep(ctx: &egui::Context, edit_id: egui::Id, s: Selected) {
    ctx.data_mut(|d| d.insert_temp(selected_id(edit_id), s));
}

/// None of the box's chips selected (its chips taken out, moved or put in another order: those
/// selected were kept by their places).
pub(super) fn let_go(ctx: &egui::Context, edit_id: egui::Id) {
    keep(ctx, edit_id, Selected::default());
}

/// The keys on the box's chips, its text box having the keyboard (taken before the text box
/// sees them): with chips selected, ← and → move along them (Shift: more of them; → past the
/// last goes back to the text), Backspace and Delete take them out, Esc lets them go, and typing
/// lets them go too; with none, ← at the start of the text, or Backspace with nothing typed,
/// selects the last.
pub(super) fn keys(ui: &mut Ui, edit_id: egui::Id, n: usize, typing: &str, done: &mut Vec<ChipDone>) {
    if !ui.memory(|m| m.has_focus(edit_id)) || n == 0 {
        return;
    }
    let mut s = selected(ui.ctx(), edit_id, n);
    let at_start = typing.is_empty()
        || egui::TextEdit::load_state(ui.ctx(), edit_id)
            .and_then(|st| st.cursor.char_range())
            .is_some_and(|r| r.is_empty() && r.primary.index.0 == 0);
    let last = n - 1;
    ui.input_mut(|i| {
        if s.chips.is_empty() {
            let left = if at_start { arrow(i, egui::Key::ArrowLeft).is_some() } else { false };
            if left || (typing.is_empty() && i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)) {
                s = Selected::one(last);
            }
            return;
        }
        if i.events.iter().any(|e| matches!(e, egui::Event::Text(_) | egui::Event::Paste(_)))
            || i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
        {
            s = Selected::default();
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) || i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
            done.push(ChipDone::Remove(s.chips.iter().copied().collect()));
            s = Selected::default();
        } else if let Some(shift) = arrow(i, egui::Key::ArrowLeft) {
            let to = s.at.saturating_sub(1);
            if shift { s.to(to) } else { s = Selected::one(to) }
        } else if let Some(shift) = arrow(i, egui::Key::ArrowRight) {
            if s.at >= last && !shift {
                s = Selected::default();
            } else {
                let to = (s.at + 1).min(last);
                if shift { s.to(to) } else { s = Selected::one(to) }
            }
        }
    });
    keep(ui.ctx(), edit_id, s);
}

/// `key` pressed, and whether with Shift (as the key event says), taken from the input.
fn arrow(i: &mut egui::InputState, key: egui::Key) -> Option<bool> {
    if i.consume_key(egui::Modifiers::SHIFT, key) { Some(true) } else { i.consume_key(egui::Modifiers::NONE, key).then_some(false) }
}

/// The label and width of an entry's chip, and the person in it (`None`: not an address).
fn measured(ui: &Ui, t: &Tokens, entry: &str) -> (String, Arc<egui::Galley>, f32, Option<mailcraft_engine::model::EmailAddress>) {
    let person = addr::parse_list(entry).ok().and_then(|list| list.into_iter().next());
    let label = person.as_ref().map_or_else(|| entry.to_string(), |p| p.display().to_string());
    let galley = ui.painter().layout_no_wrap(label.clone(), theme::regular(13.0), t.text);
    let w = (galley.size().x + 32.0).clamp(48.0, CHIP_MAX);
    (label, galley, w, person)
}

/// `field`'s chips, a row wrapped as they need, then `typing` (the text box, drawn by `edit`,
/// whose id is `edit_id`). A box not being written in (no keyboard, nothing typed, none
/// selected, nothing dragged) shows the chips that fit on one line and how many more; a click
/// there gives it the keyboard, and it shows them all. Chips dropped on it are taken, where the
/// line shows. What was done, for [`apply`].
pub(super) fn chips_and_typing<R>(
    ui: &mut Ui,
    t: &Tokens,
    field: Recipients,
    value: &str,
    edit_id: egui::Id,
    edit: impl FnOnce(&mut Ui) -> R,
) -> (R, Vec<ChipDone>) {
    let (list, typing) = entries(value);
    let mut done = Vec::new();
    let width = ui.available_width();
    let mut sel = selected(ui.ctx(), edit_id, list.len());
    let dragging = egui::DragAndDrop::has_payload_of_type::<DragRecipient>(ui.ctx());
    let busy = ui.memory(|m| m.has_focus(edit_id)) || !typing.trim().is_empty() || !sel.chips.is_empty() || dragging;
    let measures: Vec<_> = list.iter().map(|e| measured(ui, t, e)).collect();
    // Folded: the chips that fit on a line before "+N more" and a small text box.
    let shown = if busy { list.len() } else { fitting(ui, t, &measures, width) };
    let mut rects: Vec<Rect> = Vec::new();
    let mut picked = false;
    // From where the interface's text starts (the right in Arabic and Hebrew), as the rows are.
    let inner = ui.allocate_ui_with_layout(vec2(width, ROW_H - 4.0), crate::bidi::row_from_start(Align::Center).with_main_wrap(true), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, (entry, m)) in list.iter().zip(&measures).enumerate().take(shown) {
            let c = Chip { field, i, entry, list: &list, measured: m };
            let (rect, clicked) = chip(ui, t, &c, &mut sel, &mut done);
            rects.push(rect);
            picked |= clicked;
        }
        if shown < list.len() {
            let more = format!("+{}", tf!("{n} more", n = list.len() - shown));
            let b = ui.add(egui::Button::new(RichText::new(more).color(t.accent)).frame(false)).on_hover_text(tl!("Show all"));
            picked |= b.clicked();
        }
        // The text box in what's left of the row (a wrapping row's available width is all of
        // it: the box asked for that, and went to a line of its own), or on a line of its own
        // when that's too little.
        if shown > 0 {
            ui.add_space(4.0);
        }
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut room = ui.available_rect_before_wrap().width();
        if room < if shown < list.len() { FOLDED_TYPING } else { TYPING_MIN } {
            ui.end_row();
            room = width;
        }
        ui.allocate_ui_with_layout(vec2((room - 1.0).max(1.0), ROW_H - 4.0), crate::bidi::row_from_start(Align::Center), edit).inner
    });
    // A chip or "+N more" clicked: the text box has the keyboard, for the arrows. Asked for once
    // it's drawn: a click elsewhere takes the keyboard from it as it's drawn, and asked for
    // before, it was given up at once.
    // With a text box's own keys (the arrows its, not egui's to move to the next widget: ←
    // went to a chip's ×).
    if picked {
        let keys = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: false, ..Default::default() };
        ui.memory_mut(|m| m.request_focus_with_filter(edit_id, keys));
    }
    // A click elsewhere (the text box's keyboard gone) lets the selected go.
    if !picked && !ui.memory(|m| m.has_focus(edit_id)) {
        sel = Selected::default();
    }
    keep(ui.ctx(), edit_id, sel);
    // Chips dropped on this box: where the line shows, before the chip under the pointer.
    let area = ui.interact(inner.response.rect, ui.scope_id().with(("recipient-drop", field.code())), Sense::hover());
    if dragging && area.contains_pointer() {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let at = pointer.map_or(rects.len(), |p| insert_at(&rects, p));
        let from_here = egui::DragAndDrop::payload::<DragRecipient>(ui.ctx()).is_some_and(|p| p.from == field);
        if !from_here {
            ui.painter().rect_stroke(inner.response.rect.expand(2.0), 4.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        if let Some(x) = marker_x(&rects, at) {
            let y = rects.get(at.min(rects.len().saturating_sub(1))).map_or(inner.response.rect.y_range(), |r| r.y_range());
            ui.painter().vline(x, y, egui::Stroke::new(2.0, t.accent));
        }
        if let Some(p) = area.dnd_release_payload::<DragRecipient>() {
            done.push(ChipDone::Dropped((*p).clone(), at));
        }
    }
    (inner.inner, done)
}

/// How many of a box's chips fit on its one line before "+N more" and a small text box; all of
/// them when they do with the text box at its usual width.
fn fitting(ui: &Ui, t: &Tokens, measures: &[(String, Arc<egui::Galley>, f32, Option<mailcraft_engine::model::EmailAddress>)], width: f32) -> usize {
    let widths: Vec<f32> = measures.iter().map(|m| m.2.min(width)).collect();
    let all: f32 = widths.iter().map(|w| w + 4.0).sum();
    if all + 4.0 + TYPING_MIN <= width {
        return widths.len();
    }
    let more =
        |hidden: usize| ui.painter().layout_no_wrap(format!("+{}", tf!("{n} more", n = hidden)), theme::regular(13.0), t.accent).size().x + 16.0;
    let mut used = 0.0;
    let mut k = 0;
    for (i, w) in widths.iter().enumerate() {
        if used + w + 4.0 + more(widths.len() - i - 1) + 4.0 + FOLDED_TYPING > width {
            break;
        }
        used += w + 4.0;
        k = i + 1;
    }
    k
}

/// Where chips let go at `p` go among those drawn in `rects`: before the first on its line whose
/// middle is past it (in the line's direction), else after the last on a line above it.
fn insert_at(rects: &[Rect], p: egui::Pos2) -> usize {
    let rtl = crate::i18n::current().rtl();
    for (i, r) in rects.iter().enumerate() {
        let on_line = p.y <= r.bottom() + 2.0;
        let before = if rtl { p.x > r.center().x } else { p.x < r.center().x };
        if on_line && (before || p.y < r.top() - 2.0) {
            return i;
        }
    }
    rects.len()
}

/// Where the line showing where they go is drawn: before chip `at`, or after the last.
fn marker_x(rects: &[Rect], at: usize) -> Option<f32> {
    let rtl = crate::i18n::current().rtl();
    match rects.get(at) {
        Some(r) => Some(if rtl { r.right() + 2.0 } else { r.left() - 2.0 }),
        None => rects.last().map(|r| if rtl { r.left() - 2.0 } else { r.right() + 2.0 }),
    }
}

/// What a chip is drawn from.
struct Chip<'a> {
    field: Recipients,
    i: usize,
    entry: &'a str,
    /// All the box's entries (those selected with it go along).
    list: &'a [String],
    measured: &'a (String, Arc<egui::Galley>, f32, Option<mailcraft_engine::model::EmailAddress>),
}

/// One entry's chip: its name or address, × to take it out, selected by a click, dragged with
/// those selected, its menu. Where it's drawn, and whether a click selected it.
fn chip(ui: &mut Ui, t: &Tokens, c: &Chip<'_>, sel: &mut Selected, done: &mut Vec<ChipDone>) -> (Rect, bool) {
    let Chip { field, i, entry, list, measured: (label, galley, w, person) } = *c;
    let w = w.min(ui.available_width().max(48.0));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H - 8.0), Sense::click_and_drag());
    let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
    let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
    widgets::named(ui, &x, &format!("{}: {label}", tl!("Remove")), true, None);
    let is_selected = sel.chips.contains(&i);
    widgets::named(ui, &resp, label, true, Some(is_selected));
    let p = ui.painter();
    let line = match (is_selected, person.is_none()) {
        (_, true) => egui::Stroke::new(1.5, t.danger),
        (true, false) => egui::Stroke::new(1.5, t.accent),
        (false, false) => egui::Stroke::new(1.0, t.faint.gamma_multiply(0.6)),
    };
    let fill = if is_selected {
        t.accent.gamma_multiply(0.22)
    } else if resp.hovered() || resp.dragged() {
        t.hover
    } else {
        t.hover_row
    };
    p.rect(rect, (ROW_H - 8.0) / 2.0, fill, line, egui::StrokeKind::Inside);
    let text_room = Rect::from_min_max(egui::pos2(rect.left() + 9.0, rect.top()), egui::pos2(x_rect.left() - 2.0, rect.bottom()));
    p.with_clip_rect(text_room).galley(egui::pos2(text_room.left(), rect.center().y - galley.size().y / 2.0), galley.clone(), t.text);
    if x.hovered() {
        p.rect_filled(x_rect, 8.0, t.hover);
    }
    icons::paint(p, x_rect.shrink(4.0), Icon::Close, t.dim);
    let resp = match person {
        Some(_) => resp.on_hover_text(entry),
        None => resp.on_hover_text(tf!("Not an email address: {address}", address = entry)),
    };
    // A click selects it (Ctrl: adds it to, or takes it from, those selected; Shift: those
    // between), the text box keeping the keyboard for the arrows.
    let mut clicked = false;
    if resp.clicked() {
        // The click's own keys (Ctrl held only for the click, both in one frame, were gone from
        // the input's by the time it was read).
        let m = ui.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|e| match e {
                    egui::Event::PointerButton { pressed: false, modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(i.modifiers)
        });
        if m.command {
            if !sel.chips.remove(&i) {
                sel.chips.insert(i);
            }
            sel.anchor = i;
            sel.at = i;
        } else if m.shift && !sel.chips.is_empty() {
            sel.to(i);
        } else {
            *sel = Selected::one(i);
        }
        clicked = true;
    }
    // Those it's done to: the selected, when it's one of them; else it alone.
    let group: Vec<usize> = if is_selected && sel.chips.len() > 1 { sel.chips.iter().copied().collect() } else { vec![i] };
    if x.clicked() {
        done.push(ChipDone::Remove(vec![i]));
    }
    if resp.double_clicked() {
        done.push(ChipDone::Edit(i));
    }
    if resp.drag_started() {
        if !is_selected {
            *sel = Selected::one(i);
        }
        let entries = group.iter().filter_map(|&j| list.get(j).cloned()).collect();
        resp.dnd_set_drag_payload(DragRecipient { from: field, entries });
        clicked = true;
    }
    // Who's being moved, beside the pointer, as a dragged message says how many.
    if resp.dragged() {
        let n = egui::DragAndDrop::payload::<DragRecipient>(ui.ctx()).map_or(1, |p| p.entries.len());
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("drag"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(if n > 1 { tf!("{n} people", n = n) } else { label.clone() });
        });
    }
    resp.context_menu(|ui| {
        for (to, name) in [(Recipients::To, "Move to To"), (Recipients::Cc, "Move to Cc"), (Recipients::Bcc, "Move to Bcc")] {
            if to != field && ui.button(tl!(name)).clicked() {
                done.push(ChipDone::Move(group.clone(), to));
            }
        }
        ui.separator();
        let copy = if group.len() > 1 { tl!("Copy addresses") } else { tl!("Copy address") };
        if ui.button(copy).clicked() {
            let addresses: Vec<String> = group.iter().filter_map(|&j| list.get(j)).map(|e| email_of(e)).collect();
            ui.ctx().copy_text(addresses.join("; "));
        }
        if group.len() == 1 && ui.button(tl!("Edit")).clicked() {
            done.push(ChipDone::Edit(i));
        }
        if ui.button(tl!("Remove")).clicked() {
            done.push(ChipDone::Remove(group.clone()));
        }
    });
    (rect, clicked)
}

/// An entry's address alone, or the entry as written when it isn't one.
fn email_of(entry: &str) -> String {
    addr::parse_list(entry).ok().and_then(|l| l.into_iter().next()).map_or_else(|| entry.to_string(), |p| p.email)
}

/// What was done to `field`'s chips, done to the message's boxes. A chip put back to edit is
/// the text being typed (what was being typed becomes a chip of its own); one moved to Bcc
/// shows Bcc. Whether one was put back to edit (its box then takes the keyboard).
pub(crate) fn apply(c: &mut mailcraft_engine::Compose, field: Recipients, done: Vec<ChipDone>) -> bool {
    let mut editing = false;
    for d in done {
        let (mut list, typing) = entries(field.of(&mut c.to, &mut c.cc, &mut c.bcc));
        match d {
            ChipDone::Remove(at) => {
                let taken = take(&mut list, &at);
                if !taken.is_empty() {
                    *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
                }
            }
            ChipDone::Edit(i) if i < list.len() => {
                let entry = list.remove(i);
                if !typing.trim().is_empty() {
                    list.push(typing.trim().to_string());
                }
                *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &entry);
                editing = true;
            }
            ChipDone::Move(at, to) if to != field => {
                let taken = take(&mut list, &at);
                if !taken.is_empty() {
                    *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
                    add_entries(c, to, taken, usize::MAX);
                }
            }
            ChipDone::Dropped(p, at) if p.from == field => {
                // Put in another place in their own box: before entry `at` as it was.
                let mut moved = Vec::new();
                let mut place = at.min(list.len());
                for e in &p.entries {
                    if let Some(j) = list.iter().position(|x| x == e) {
                        if j < place {
                            place -= 1;
                        }
                        moved.push(list.remove(j));
                    }
                }
                for (k, e) in moved.into_iter().enumerate() {
                    list.insert((place + k).min(list.len()), e);
                }
                *field.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
            }
            ChipDone::Dropped(p, at) => {
                let (mut from, from_typing) = entries(p.from.of(&mut c.to, &mut c.cc, &mut c.bcc));
                let mut moved = Vec::new();
                for e in &p.entries {
                    if let Some(j) = from.iter().position(|x| x == e) {
                        moved.push(from.remove(j));
                    }
                }
                if moved.is_empty() {
                    continue;
                }
                *p.from.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&from, &from_typing);
                add_entries(c, field, moved, at);
            }
            _ => {}
        }
    }
    editing
}

/// The entries at the places `at` taken out of `list`, in their order (places past it ignored).
fn take(list: &mut Vec<String>, at: &[usize]) -> Vec<String> {
    let mut at: Vec<usize> = at.iter().copied().filter(|i| *i < list.len()).collect();
    at.sort_unstable();
    at.dedup();
    let taken: Vec<String> = at.iter().filter_map(|i| list.get(*i).cloned()).collect();
    for i in at.into_iter().rev() {
        list.remove(i);
    }
    taken
}

/// `new` put among `to`'s chips before its entry `at` (after the last when past them), before
/// what's being typed there; Bcc shown for them.
fn add_entries(c: &mut mailcraft_engine::Compose, to: Recipients, new: Vec<String>, at: usize) {
    if to == Recipients::Bcc {
        c.show_bcc = true;
    }
    let (mut list, typing) = entries(to.of(&mut c.to, &mut c.cc, &mut c.bcc));
    let at = at.min(list.len());
    for (k, e) in new.into_iter().enumerate() {
        list.insert(at + k, e);
    }
    *to.of(&mut c.to, &mut c.cc, &mut c.bcc) = addr::join_entries(&list, &typing);
}
