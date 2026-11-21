//! The message being written: Send and the rows beside it (From, To, Cc, Bcc), the people
//! suggested while an address is typed, Replies to, who can read it encrypted, and the subject.

use super::*;
use crate::dialogs::Recipients;

/// Send, beside From, To, Cc and Bcc (when shown); then Replies to (when shown), who can read it
/// encrypted, and the subject. `focus_to`: a new message starts in To.
pub(super) fn address_rows(ui: &mut Ui, st: &mut UiState, c: &mut mailcraft_engine::Compose, f: &Form, focus_to: bool, acts: &mut Vec<Action>) {
    let t = st.tokens;
    // Send beside the address rows (From, To, Cc and Bcc when shown).
    let rows = if c.show_bcc { 4.0 } else { 3.0 };
    let height = rows * ROW_H + (rows - 1.0) * 6.0;
    ui.horizontal_top(|ui| {
        if send_button(ui, &t, height, c.sending, f.blocked_text.as_deref()).clicked() {
            send(c, f.blocked, acts);
        }
        ui.add_space(SEND_GAP);
        crate::bidi::vertical(ui, |ui| {
            from_row(ui, &t, c, &f.accounts, acts);
            // To, Cc and Bcc open the address book, as in Outlook.
            for (field, label, hint) in [
                (Recipients::To, "To", "Names <addresses>, separated by ; or ,"),
                (Recipients::Cc, "Cc", ""),
                (Recipients::Bcc, "Bcc", "Hidden from the other recipients"),
            ] {
                if field == Recipients::Bcc && !c.show_bcc {
                    continue;
                }
                let value = field.of(&mut c.to, &mut c.cc, &mut c.bcc);
                let (book, check) = recipient_field(ui, st, field, label, value, hint, field == Recipients::To && focus_to, &f.suggestions);
                if book {
                    let d = crate::Dialog::select_names(c, field);
                    acts.push(Action::Dialog(d));
                }
                // Ctrl+K checks the names typed, as Outlook's does in these boxes (issue #211).
                if check {
                    acts.push(Action::Run("compose.checkNames", json!({})));
                }
            }
        });
    });
    if c.show_reply_to || !c.reply_to.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(PAST_SEND);
            crate::bidi::vertical(ui, |ui| field(ui, &t, "Replies to", &mut c.reply_to, "Where replies go instead of to you", true, false));
        });
    }
    if let Some(r) = f.reach.as_ref().filter(|r| !matches!(r, mailcraft_engine::Reach::NotSetUp | mailcraft_engine::Reach::NoOne)) {
        ui.horizontal(|ui| {
            ui.add_space(PAST_SEND + LABEL_W);
            security::encryption_line(ui, &t, r, c.encrypt);
        });
    }
    ui.horizontal(|ui| {
        ui.add_space(PAST_SEND);
        crate::bidi::vertical(ui, |ui| field(ui, &t, "Subject", &mut c.subject, "Add a subject", false, false));
    });
}

/// From's choices: the accounts that can send (and the one it's from, whatever it is), each with
/// its other addresses under it (issue #81): the account, the address ("" its own), and who
/// that is.
pub(super) fn from_choices(s: &Session) -> Vec<(String, String, String)> {
    let from = s.compose.as_ref().map(|c| c.account.as_str()).unwrap_or_default();
    s.accounts
        .iter()
        .filter(|a| a.config.id == from || s.send_block(&a.config.id).is_none())
        .flat_map(|a| {
            let own = (a.config.id.clone(), String::new(), a.config.sender("").to_string());
            let others = a.config.identities.iter().map(move |i| (a.config.id.clone(), i.email.clone(), a.config.sender(&i.email).to_string()));
            std::iter::once(own).chain(others)
        })
        .collect()
}

/// From, and who the message is from: a menu of `accounts` when there's more than one.
fn from_row(ui: &mut Ui, t: &Tokens, c: &mailcraft_engine::Compose, accounts: &[(String, String, String)], acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let is_from = |id: &str, address: &str| c.account == id && c.from.eq_ignore_ascii_case(address);
        let current = accounts.iter().find(|(id, address, _)| is_from(id, address)).map(|(_, _, l)| l.clone());
        // One no longer among them (an address taken out): the account's own.
        let current = current.or_else(|| accounts.iter().find(|(id, _, _)| *id == c.account).map(|(_, _, l)| l.clone())).unwrap_or_default();
        let many = accounts.len() > 1;
        let b = label_button_menu(ui, t, "From", many, many);
        if many {
            egui::Popup::menu(&b).show(|ui| {
                // Through the engine: the signature follows the account
                // (issue #11), and the address.
                for (id, address, label) in accounts {
                    let picked = is_from(id, address);
                    if ui.selectable_label(picked, label).clicked() && !picked {
                        acts.push(Action::Run("compose.update", json!({"account": id, "from": address})));
                    }
                }
            });
        }
        ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), crate::bidi::row_from_start(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(current).color(t.dim)).truncate());
        });
    });
}

/// The people suggested for the recipient box being typed in (none when what's typed there is
/// done, or Escape put them away).
pub(super) fn suggestions(s: &mut Session, st: &mut UiState) -> Vec<Contact> {
    match (st.suggest_field, s.compose.as_ref()) {
        (Some(f), Some(c)) => {
            let text = match f {
                Recipients::To => c.to.clone(),
                Recipients::Cc => c.cc.clone(),
                Recipients::Bcc => c.bcc.clone(),
            };
            if mailcraft_engine::model::contacts::last_token(&text).is_empty() || st.suggest_dismissed.as_deref() == Some(text.as_str()) {
                Vec::new()
            } else {
                suggested(s, st, text)
            }
        }
        _ => Vec::new(),
    }
}

/// The people suggested for what's typed in a recipient box, and what they were made for.
pub(crate) struct Suggested {
    text: String,
    rev: (Option<u64>, u64, i64),
    people: Vec<Contact>,
}

/// The people to suggest for `text`, typed in a recipient box (those already in it left out):
/// made again only when the text or what they're made of changes, not on every frame (each
/// time the whole address book was read, lower-cased and sorted).
fn suggested(s: &mut Session, st: &mut UiState, text: String) -> Vec<Contact> {
    if let Some(kept) = st.suggested.as_ref().filter(|k| k.text == text && k.rev == s.suggestions_rev()) {
        return kept.people.clone();
    }
    let token = mailcraft_engine::model::contacts::last_token(&text);
    let present = text.to_lowercase();
    let people: Vec<Contact> =
        s.suggest_contacts(token, 8).into_iter().filter(|x| !present.contains(&x.address.email.to_lowercase())).take(6).collect();
    st.suggested = Some(Suggested { rev: s.suggestions_rev(), people: people.clone(), text });
    people
}

/// A field box drawn the way Outlook's are: no frame, a line under it (the accent while typing).
fn underlined(ui: &mut Ui, t: &Tokens, edit: egui::TextEdit<'_>) -> egui::Response {
    let r = ui.add_sized(vec2(ui.available_width(), ROW_H - 4.0), edit.frame(egui::Frame::NONE).align(egui::Align2::LEFT_CENTER));
    let y = r.rect.bottom();
    let color = if r.has_focus() { t.accent } else { t.border };
    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(1.0, color));
    r
}

/// A small framed button naming a field (To, Cc…), as in Outlook's message window.
fn label_button(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool) -> egui::Response {
    label_button_menu(ui, t, label, enabled, false)
}

/// A label button that opens a menu: its label and a small drawn chevron, centred together.
fn label_button_menu(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool, menu: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(LABEL_W - 8.0, ROW_H - 4.0), if enabled { Sense::click() } else { Sense::hover() });
    crate::widgets::named(ui, &resp, tl!(label), enabled, None);
    let fill = if resp.hovered() && enabled { t.hover } else { t.surface };
    ui.painter().rect(rect, 3.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let color = if enabled { t.text } else { t.dim };
    let galley = ui.painter().layout_no_wrap(tl!(label).to_string(), theme::regular(13.0), color);
    let chevron = if menu { 14.0 } else { 0.0 };
    let left = rect.center().x - (galley.size().x + chevron) / 2.0;
    let text_pos = egui::pos2(left, rect.center().y - galley.size().y / 2.0);
    let text_w = galley.size().x;
    ui.painter().galley(text_pos, galley, color);
    if menu {
        let r = egui::Rect::from_center_size(egui::pos2(left + text_w + 4.0 + 5.0, rect.center().y + 0.5), vec2(10.0, 10.0));
        icons::paint(ui.painter(), r, Icon::ChevronDown, t.dim);
    }
    ui.add_space(8.0);
    resp
}

/// Outlook's tall Send button beside the address rows. `blocked`: why the message can't go from
/// its account; it's drawn off, and a click says why.
fn send_button(ui: &mut Ui, t: &Tokens, height: f32, sending: bool, blocked: Option<&str>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(SEND_W, height), if sending { Sense::hover() } else { Sense::click() });
    let on = !sending && blocked.is_none();
    crate::widgets::named(ui, &resp, if sending { tl!("Sending…") } else { tl!("Send") }, on, None);
    let fill = if resp.hovered() && on { t.hover } else { t.surface };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(24.0, 24.0));
    icons::paint(ui.painter(), icon, Icon::Send, if on { t.accent } else { t.dim });
    let label = if sending { tl!("Sending…") } else { tl!("Send") };
    ui.painter().text(rect.center() + vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, theme::regular(12.5), if on { t.text } else { t.dim });
    match blocked {
        Some(why) => resp.on_hover_text(why),
        None => resp.on_hover_text(tl!("Send (Ctrl+Enter)")),
    }
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, check_addresses: bool, focus: bool) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(LABEL_W, ROW_H), egui::Label::new(RichText::new(tl!(label)).color(t.dim)));
        let r = underlined(ui, t, egui::TextEdit::singleline(value).hint_text(tl!(hint)));
        if focus {
            r.request_focus();
        }
    });
    address_check(ui, t, value, check_addresses);
}

/// A recipient box (To, Cc, Bcc) that suggests people from mail already seen: ↑/↓ choose,
/// Enter or Tab takes the highlighted one, a click takes any. Whether its button was pressed
/// (the address book), and whether Ctrl+K was (Check Names).
fn recipient_field(
    ui: &mut Ui,
    st: &mut UiState,
    field: Recipients,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    suggestions: &[Contact],
) -> (bool, bool) {
    let t = st.tokens;
    let showing = st.suggest_field == Some(field) && !suggestions.is_empty();
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
    let edit_id = ui.scope_id().with(("recipient", field.code()));
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
    let check = ui.memory(|m| m.has_focus(edit_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K));
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
    if st.suggest_refocus == Some(field) {
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
        let area = egui::Area::new(ui.scope_id().with(("suggest", field.code())))
            .order(egui::Order::Foreground)
            .fixed_pos(r.rect.left_bottom() + vec2(0.0, 2.0));
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
                    let (row, resp) = ui.allocate_exact_size(vec2(w, 28.0), egui::Sense::click());
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
        st.suggest_refocus = Some(field);
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            let end = egui::text::CCursor::new(value.chars().count());
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.has_focus() || take.is_some() || st.suggest_refocus == Some(field) {
        st.suggest_field = Some(field);
    } else if st.suggest_field == Some(field) {
        st.suggest_field = None;
    }
    // Half-typed addresses are fine while typing; check once the box is left.
    address_check(ui, &t, value, !r.has_focus() && st.suggest_refocus != Some(field));
    (pressed, check)
}

fn address_check(ui: &mut Ui, t: &Tokens, value: &str, check_addresses: bool) {
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
