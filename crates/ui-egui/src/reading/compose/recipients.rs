//! Recipient entry, completion suggestions and address validation.

use super::*;

/// A recipient box (To, Cc, Bcc) that suggests people from mail already seen: ↑/↓ choose,
/// Enter or Tab takes the highlighted one, a click takes any.
#[allow(clippy::too_many_arguments)]
pub(super) fn recipient_field(
    ui: &mut Ui,
    st: &mut UiState,
    name: &'static str,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    suggestions: &[Contact],
) -> bool {
    let t = st.tokens;
    let showing = st.suggest_field == Some(name) && !suggestions.is_empty();
    let mut take: Option<usize> = None;
    if showing {
        let n = suggestions.len();
        // Before the text box sees them: arrows move the highlight, Enter/Tab accept.
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                st.suggest_index = (st.suggest_index + 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                st.suggest_index = (st.suggest_index + n - 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                take = Some(st.suggest_index.min(n - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                st.suggest_dismissed = Some(value.clone());
            }
        });
    }
    // Backspace right after a whole recipient takes all of it, as Outlook does with a resolved
    // name (a half-typed one loses a character as usual).
    let edit_id = ui.scope_id().with(("recipient", name));
    if ui.memory(|m| m.has_focus(edit_id))
        && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id)
        && let Some(range) = state.cursor.char_range().filter(|r| r.is_empty())
        && let Some((rest, at)) = mailcraft_engine::model::contacts::remove_recipient(value, range.primary.index.0)
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        *value = rest;
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
        state.store(ui.ctx(), edit_id);
        st.suggest_dismissed = Some(value.clone());
    }
    let (pressed, r) = ui
        .horizontal(|ui| {
            // The field's button opens the address book (the caller's to show).
            let b = label_button(ui, &t, label, true);
            (b.clicked(), underlined(ui, &t, egui::TextEdit::singleline(value).id(edit_id).hint_text(tl!(hint))))
        })
        .inner;
    if focus {
        r.request_focus();
    }
    // Picked from the suggestions: the box stays the one being typed in, cursor at the end,
    // until the click that picked is over.
    if st.suggest_refocus == Some(name) {
        r.request_focus();
        if ui.input(|i| !i.pointer.any_down() && !i.pointer.any_released()) {
            st.suggest_refocus = None;
        }
    }
    if r.changed() {
        st.suggest_index = 0;
        st.suggest_dismissed = None;
    }
    if showing && take.is_none() {
        let area =
            egui::Area::new(ui.scope_id().with(("suggest", name))).order(egui::Order::Foreground).fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0));
        area.show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                // As wide as the longest suggestion (within the box's width), every row taking
                // a click anywhere on it: the name, and the address dimmer beside it.
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
                    let on = i == st.suggest_index;
                    let suggestion_height = if ui.ctx().input(|input| input.has_touch_screen()) { 48.0 } else { 28.0 };
                    let (row, resp) = ui.allocate_exact_size(vec2(w, suggestion_height), egui::Sense::click());
                    if on || resp.hovered() {
                        ui.painter().rect_filled(row, 4.0, if on { t.accent_soft } else { t.hover_row });
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
                    let spoken =
                        if c.address.name.is_empty() { c.address.email.clone() } else { format!("{} <{}>", c.address.name, c.address.email) };
                    widgets::named(ui, &resp, &spoken, true, Some(on));
                    // On press: the box loses focus on the same click, before a release would count.
                    if resp.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                        take = Some(i);
                    }
                }
            });
        });
    }
    if let Some(i) = take
        && let Some(c) = suggestions.get(i)
    {
        *value = mailcraft_engine::model::contacts::complete(value, &c.address);
        st.suggest_index = 0;
        st.suggest_refocus = Some(name);
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(value.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.has_focus() || take.is_some() || st.suggest_refocus == Some(name) {
        st.suggest_field = Some(name);
    } else if st.suggest_field == Some(name) {
        st.suggest_field = None;
    }
    // Half-typed addresses are fine while typing; check once the box is left.
    address_check(ui, &t, value, !r.has_focus() && st.suggest_refocus != Some(name));
    pressed
}

pub(super) fn address_check(ui: &mut Ui, t: &Tokens, value: &str, check_addresses: bool) {
    if check_addresses
        && !value.trim().is_empty()
        && let Err(bad) = addr::parse_list(value)
    {
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            ui.label(RichText::new(tf!("Not an email address: {address}", address = bad)).color(t.danger).small());
        });
    }
}
