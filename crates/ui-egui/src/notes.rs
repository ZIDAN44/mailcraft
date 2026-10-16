//! Notes, as Outlook's Notes module: coloured sticky notes, written straight onto. A click on a
//! note writes on it (the text is saved when the note is left); right-click gives its colours
//! and Delete. Exchange accounts keep them in the mailbox's Notes folder, others on this computer.

use egui::{Color32, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

/// Outlook's note colours, in its order: blue, green, pink, yellow, white.
pub(crate) const NAMES: [&str; 5] = ["Blue", "Green", "Pink", "Yellow", "White"];

/// A note's paper (light in both themes, as sticky notes are), and its text.
pub(crate) fn paper(color: u8, dark: bool) -> Color32 {
    let c = match color {
        0 => [0xD4, 0xEC, 0xFB],
        1 => [0xD6, 0xF5, 0xD6],
        2 => [0xFB, 0xD8, 0xEE],
        4 => [0xFF, 0xFF, 0xFF],
        _ => [0xFF, 0xF4, 0xC2],
    };
    let c = Color32::from_rgb(c[0], c[1], c[2]);
    if dark { c.gamma_multiply(0.9) } else { c }
}

const INK: Color32 = Color32::from_rgb(0x1F, 0x23, 0x28);

/// The side pane: a new note, search, how many there are.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New note", &t, !s.accounts.is_empty()).clicked() {
        new_note(st, acts);
    }
    ui.add_space(8.0);
    ui.add(egui::TextEdit::singleline(&mut st.notes_query).hint_text(tl!("Search notes")).desired_width(f32::INFINITY));
    ui.add_space(6.0);
    let n = s.notes(&st.notes_query).len();
    ui.label(RichText::new(tf!("{n} notes", n = n)).color(t.faint).small());
}

/// Start a note and write on it at once (the app makes it, then opens it by its uid).
pub(crate) fn new_note(st: &mut UiState, acts: &mut Vec<Action>) {
    save_edit(st, acts);
    acts.push(Action::NewNote);
}

/// The main pane: the notes, side by side.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let notes = s.notes(&st.notes_query);
    // The note being written on went (deleted, or another search): keep what was typed.
    if let Some((uid, _)) = &st.note_edit
        && !notes.iter().any(|n| &n.note.uid == uid)
    {
        save_edit(st, acts);
    }
    if notes.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Note, t.border);
                ui.add_space(8.0);
                let empty = st.notes_query.trim().is_empty();
                ui.label(RichText::new(tl!(if empty { "No notes yet: New note starts one." } else { "No note matches." })).color(t.faint));
            });
        });
        return;
    }
    let mut clicked_off = ui.input(|i| i.pointer.primary_pressed()) && ui.rect_contains_pointer(ui.max_rect());
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("notes").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            let size = 210.0;
            let gap = 14.0;
            let cols = (((ui.available_width() + gap) / (size + gap)).floor() as usize).max(1);
            for row in notes.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for n in row {
                        if note(s, &t, st, ui, n, size, acts) {
                            clicked_off = false;
                        }
                    }
                });
                ui.add_space(gap);
            }
        });
    });
    // A click anywhere else leaves the note: what was typed is kept.
    if clicked_off && st.note_edit.is_some() && !ui.ctx().any_popup_open() {
        save_edit(st, acts);
    }
}

/// Save what was typed on the note being written on, and stop writing on it.
pub(crate) fn save_edit(st: &mut UiState, acts: &mut Vec<Action>) {
    if let Some((uid, text)) = st.note_edit.take() {
        acts.push(Action::Run("note.update", json!({"uid": uid, "text": text})));
    }
}

/// One note: its paper, its text (or the text box, while written on), when it last changed.
/// True when the pointer pressed on it.
fn note(s: &Session, t: &Tokens, st: &mut UiState, ui: &mut Ui, n: &mailcraft_engine::NoteRef, size: f32, acts: &mut Vec<Action>) -> bool {
    let uid = &n.note.uid;
    let editing = st.note_edit.as_ref().is_some_and(|(u, _)| u == uid);
    let id = ui.id().with(("note", uid));
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hovered = resp.hovered() || editing;
    let fill = paper(n.note.color, t.dark);
    // A shadow, the paper, its folded corner.
    ui.painter().rect_filled(rect.translate(vec2(0.0, 2.0)), 4.0, Color32::from_black_alpha(if hovered { 40 } else { 22 }));
    ui.painter().rect(
        rect,
        4.0,
        fill,
        egui::Stroke::new(if editing { 2.0 } else { 1.0 }, if editing { t.accent } else { fill.gamma_multiply(0.85) }),
        egui::StrokeKind::Inside,
    );
    let fold = 18.0;
    let corner = [rect.right_bottom() - vec2(0.0, fold), rect.right_bottom() - vec2(fold, 0.0), rect.right_bottom() - vec2(fold, fold)];
    ui.painter().add(egui::Shape::convex_polygon(corner.to_vec(), fill.gamma_multiply(0.8), egui::Stroke::NONE));
    let inner = rect.shrink2(vec2(12.0, 10.0));
    let when = crate::i18n::date(&dates::list_time(n.note.modified, &s.clock));
    ui.painter().text(egui::pos2(inner.left(), inner.bottom()), egui::Align2::LEFT_BOTTOM, when, theme::regular(11.0), INK.gamma_multiply(0.55));
    let text_rect = egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), inner.bottom() - 18.0));
    let mut pressed = resp.is_pointer_button_down_on() || resp.clicked();
    if editing {
        let mut text = st.note_edit.as_ref().map(|(_, x)| x.clone()).unwrap_or_default();
        // In a child of its own over the paper, so the row of notes doesn't move.
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
        let edit = child.add(
            egui::TextEdit::multiline(&mut text)
                .id(id.with("text"))
                .frame(egui::Frame::NONE)
                .text_color(INK)
                .font(theme::regular(14.0))
                .hint_text(RichText::new(tl!("Write your note…")).color(INK.gamma_multiply(0.45)))
                .desired_width(text_rect.width())
                .min_size(text_rect.size()),
        );
        if std::mem::take(&mut st.note_focus) {
            edit.request_focus();
        }
        pressed |= edit.is_pointer_button_down_on() || edit.has_focus() && ui.input(|i| i.pointer.primary_pressed()) && edit.contains_pointer();
        if let Some((_, buf)) = st.note_edit.as_mut() {
            *buf = text;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            save_edit(st, acts);
        }
    } else {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = text_rect.width();
        let title = n.note.title().to_string();
        let rest: String = n.note.text.trim_start().split_once('\n').map(|(_, r)| r.to_string()).unwrap_or_default();
        job.append(&title, 0.0, egui::TextFormat { font_id: theme::bold(14.0), color: INK, ..Default::default() });
        if !rest.trim().is_empty() {
            job.append(
                &format!("\n{}", rest.trim()),
                0.0,
                egui::TextFormat { font_id: theme::regular(13.0), color: INK.gamma_multiply(0.85), ..Default::default() },
            );
        }
        let galley = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(text_rect).galley(text_rect.min, galley, INK);
        if resp.clicked() {
            save_edit(st, acts);
            st.note_edit = Some((uid.clone(), n.note.text.clone()));
            st.note_focus = true;
        }
    }
    resp.context_menu(|ui| {
        for (i, name) in NAMES.iter().enumerate() {
            let c = u8::try_from(i).unwrap_or(3);
            let clicked = ui
                .horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    ui.painter().rect(r, 3.0, paper(c, false), egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    ui.selectable_label(n.note.color == c, tl!(name)).clicked()
                })
                .inner;
            if clicked {
                acts.push(Action::Run("note.update", json!({"uid": uid, "color": c})));
                ui.close();
            }
        }
        ui.separator();
        if ui.button(RichText::new(tl!("Delete")).color(t.danger)).clicked() {
            if editing {
                st.note_edit = None;
            }
            acts.push(Action::Run("note.delete", json!({"uid": uid})));
            ui.close();
        }
    });
    pressed
}
