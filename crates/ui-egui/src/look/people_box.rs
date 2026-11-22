//! A box of people as a message's To is: the people known suggested as a name is typed (↑/↓
//! choose, Enter or Tab takes the highlighted one, a click takes any, Escape hides them),
//! Backspace after a whole person taking all of them, and Ctrl+K asking for Check Names. A
//! message's boxes keep what's typed in the window's state; a dialog's (a meeting's guests,
//! issue #253; who a meeting is forwarded or delegated to, issue #254) keep it in a
//! [`PeopleBox`] of their own.

use egui::{Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::contacts::Contact;

use crate::theme::{self, Tokens};

/// The most people suggested.
const SHOWN: usize = 6;

/// What a dialog's box of people keeps between frames.
#[derive(Clone, Debug, Default)]
pub struct PeopleBox {
    /// The people suggested for what's typed (none while it isn't typed in).
    pub suggestions: Vec<Contact>,
    /// The text and the address book (its counts) they were made for.
    made_for: Option<(String, (Option<u64>, u64, i64))>,
    /// It's being typed in: its suggestions are wanted.
    pub typing: bool,
    /// The one highlighted, and the text for which Escape hid them.
    pub index: usize,
    pub dismissed: Option<String>,
    /// A suggestion was just picked: the box keeps the cursor through the rest of that click.
    pub refocus: bool,
}

impl PeopleBox {
    /// The suggestions for `text`, made again when it or what they're made of changed (before
    /// the dialog is drawn, which can't change the session): none while it isn't typed in, when
    /// what's typed is done, or Escape hid them.
    pub(crate) fn refresh(&mut self, s: &mut Session, text: &str) {
        let token = mailcraft_engine::model::contacts::last_token(text);
        if !self.typing || token.is_empty() || self.dismissed.as_deref() == Some(text) {
            self.suggestions.clear();
            self.made_for = None;
            return;
        }
        let made_for = (text.to_string(), s.suggestions_rev());
        if self.made_for.as_ref() == Some(&made_for) {
            return;
        }
        let present = text.to_lowercase();
        self.suggestions =
            s.suggest_contacts(token, SHOWN + 2).into_iter().filter(|x| !present.contains(&x.address.email.to_lowercase())).take(SHOWN).collect();
        self.made_for = Some(made_for);
    }
}

/// What typing in a box changes, wherever it's kept: the highlighted suggestion, and the text
/// for which Escape hid them.
pub(crate) struct Typing<'a> {
    pub index: &'a mut usize,
    pub dismissed: &'a mut Option<String>,
    /// A suggestion was just picked into it: it keeps the cursor.
    pub refocus: bool,
}

/// What a box of people's frame did.
pub(crate) struct Typed {
    pub response: egui::Response,
    /// Ctrl+K: check the names typed.
    pub check: bool,
    /// A suggestion was taken.
    pub took: bool,
}

/// A box of people with the id `edit_id`, its text box drawn by `draw` (the caller's look), and
/// `suggestions` shown under it (none: not shown).
pub(crate) fn edit(
    ui: &mut Ui,
    t: &Tokens,
    edit_id: egui::Id,
    value: &mut String,
    suggestions: &[Contact],
    typing: Typing<'_>,
    draw: impl FnOnce(&mut Ui, egui::TextEdit<'_>) -> egui::Response,
) -> Typed {
    let Typing { index, dismissed, refocus } = typing;
    let mut take: Option<usize> = None;
    if !suggestions.is_empty() {
        let n = suggestions.len();
        // Before the text box sees them: arrows move the highlight, Enter/Tab accept.
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                *index = (*index + 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                *index = (*index + n - 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                take = Some((*index).min(n - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                *dismissed = Some(value.clone());
            }
        });
    }
    // Backspace right after a whole person takes all of them, as Outlook does with a resolved
    // name (a half-typed one loses a character as usual).
    if ui.memory(|m| m.has_focus(edit_id))
        && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id)
        && let Some(range) = state.cursor.char_range().filter(|r| r.is_empty())
        && let Some((rest, at)) = mailcraft_engine::model::contacts::remove_recipient(value, range.primary.index.0)
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        *value = rest;
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
        state.store(ui.ctx(), edit_id);
        *dismissed = Some(value.clone());
    }
    let check = ui.memory(|m| m.has_focus(edit_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K));
    let r = draw(ui, egui::TextEdit::singleline(value).id(edit_id));
    // Picked from the suggestions: the box stays the one being typed in, cursor at the end,
    // until the click that picked is over.
    if refocus {
        r.request_focus();
    }
    if r.changed() {
        *index = 0;
        *dismissed = None;
    }
    if !suggestions.is_empty() && take.is_none() {
        take = shown(ui, t, edit_id, &r, suggestions, *index);
    }
    if let Some(i) = take
        && let Some(c) = suggestions.get(i)
    {
        *value = mailcraft_engine::model::contacts::complete(value, &c.address);
        *index = 0;
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(value.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    Typed { response: r, check, took: take.is_some() }
}

/// Whether the click that picked a suggestion is over (the box can let the cursor go again).
pub(crate) fn click_over(ui: &Ui) -> bool {
    ui.input(|i| !i.pointer.any_down() && !i.pointer.any_released())
}

/// The suggestions under the box `r`, `on` highlighted: the one pressed, if any.
fn shown(ui: &mut Ui, t: &Tokens, edit_id: egui::Id, r: &egui::Response, suggestions: &[Contact], on: usize) -> Option<usize> {
    let mut take = None;
    let area = egui::Area::new(edit_id.with("suggest")).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0));
    area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            // As wide as the longest suggestion (within the box's width), every row taking a
            // click anywhere on it: the name, and the address dimmer beside it.
            let name_font = theme::regular(13.5);
            let mail_font = theme::regular(12.5);
            let text_w = |text: &str, font: &egui::FontId| ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), t.text).size().x);
            let widest = suggestions
                .iter()
                .map(|c| {
                    let name = if c.address.name.is_empty() { 0.0 } else { text_w(&c.address.name, &name_font) + 12.0 };
                    name + text_w(&c.address.email, &mail_font)
                })
                .fold(0.0_f32, f32::max);
            let w = (widest + 24.0).clamp(200.0, r.rect.width().clamp(200.0, 520.0));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, c) in suggestions.iter().enumerate() {
                let (row, resp) = ui.allocate_exact_size(vec2(w, 28.0), egui::Sense::click());
                if i == on || resp.hovered() {
                    ui.painter().rect_filled(row, 4.0, if i == on { t.accent_soft } else { t.hover_row });
                }
                let clip = ui.painter().with_clip_rect(row.shrink2(vec2(8.0, 0.0)));
                let mut x = row.left() + 10.0;
                if !c.address.name.is_empty() {
                    let g = clip.layout_no_wrap(c.address.name.clone(), name_font.clone(), t.text);
                    let gw = g.size().x;
                    clip.galley(egui::pos2(x, row.center().y - g.size().y / 2.0), g, t.text);
                    x += gw + 12.0;
                }
                let g = clip.layout_no_wrap(c.address.email.clone(), mail_font.clone(), t.dim);
                clip.galley(egui::pos2(x, row.center().y - g.size().y / 2.0), g, t.dim);
                // On press: the box loses focus on the same click, before a release would count.
                if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                    take = Some(i);
                }
            }
        });
    });
    take
}

/// A box of people in a dialog called `name` (to a screen reader), `width` wide (the rest of the
/// row when infinite), its state in `b`, `salt` telling it from the dialog's others. Whether
/// Ctrl+K was pressed in it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn in_dialog(ui: &mut Ui, t: &Tokens, salt: &str, name: &str, value: &mut String, hint: &str, width: f32, b: &mut PeopleBox) -> bool {
    let id = ui.make_persistent_id(("people", salt));
    let suggestions = if b.typing && b.dismissed.as_deref() != Some(value.as_str()) { b.suggestions.clone() } else { Vec::new() };
    let typing = Typing { index: &mut b.index, dismissed: &mut b.dismissed, refocus: b.refocus };
    let typed = edit(ui, t, id, value, &suggestions, typing, |ui, e| {
        let w = if width.is_finite() { width } else { ui.available_width() };
        ui.add(e.hint_text(hint).desired_width(w).min_size(vec2(w, 30.0)).margin(egui::Margin::symmetric(8, 6)).align(egui::Align2::LEFT_CENTER))
            .accessible_name(name)
    });
    if b.refocus && click_over(ui) {
        b.refocus = false;
    }
    if typed.took {
        b.refocus = true;
    }
    b.typing = typed.response.has_focus() || typed.took || b.refocus;
    if !b.typing {
        b.suggestions.clear();
    }
    typed.check
}
