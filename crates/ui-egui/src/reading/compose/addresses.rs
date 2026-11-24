//! The message being written: Send and the rows beside it (From, To, Cc, Bcc), the people
//! suggested while an address is typed, Replies to, who can read it encrypted, and the subject.

use super::*;
use crate::dialogs::Recipients;
use crate::look::people_box;

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
            // Each box's text box, and whether chips were taken out, moved or put in another
            // order: then none stays selected (they're kept by their places, which changed).
            let (mut boxes, mut changed) = (Vec::new(), false);
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
                let (book, check, done, edit_id) =
                    recipient_field(ui, st, field, label, value, hint, field == Recipients::To && focus_to, &f.suggestions);
                boxes.push(edit_id);
                changed |= !done.is_empty();
                // A chip put back to edit: its box has the keyboard, at the end of it.
                if recipients::apply(c, field, done) {
                    ui.memory_mut(|m| m.request_focus(edit_id));
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit_id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(usize::MAX))));
                        state.store(ui.ctx(), edit_id);
                    }
                }
                if book {
                    let d = crate::Dialog::select_names(c, field);
                    acts.push(Action::Dialog(d));
                }
                // Ctrl+K checks the names typed, as Outlook's does in these boxes (issue #211).
                if check {
                    acts.push(Action::Run("compose.checkNames", json!({})));
                }
            }
            if changed {
                for edit_id in boxes {
                    recipients::let_go(ui.ctx(), edit_id);
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
            // Escape put them away for what's being typed after the chips (all the box hands
            // the suggestions).
            let typing = recipients::entries(&text).1;
            if mailcraft_engine::model::contacts::last_token(&text).is_empty() || st.suggest_dismissed.as_deref() == Some(typing.as_str()) {
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

/// A recipient box (To, Cc, Bcc): its people a chip each ([`recipients`], issue #269), then what's
/// being typed, which suggests people from mail already seen ([`crate::look::people_box`]).
/// Whether its button was pressed (the address book), whether Ctrl+K was (Check Names), what was
/// done to its chips, and its text box's id.
#[allow(clippy::too_many_arguments)]
fn recipient_field(
    ui: &mut Ui,
    st: &mut UiState,
    field: Recipients,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
    suggestions: &[Contact],
) -> (bool, bool, Vec<recipients::ChipDone>, egui::Id) {
    let t = st.tokens;
    let showing = st.suggest_field == Some(field) && !suggestions.is_empty();
    let refocus = st.suggest_refocus == Some(field);
    let edit_id = ui.scope_id().with(("recipient", field.code()));
    let (pressed, typed, done) = ui
        .horizontal(|ui| {
            // The field's button opens the address book (the caller's to show).
            let b = label_button(ui, &t, label, true);
            let (list, _) = recipients::entries(value);
            let (_, at) = addr::split_entries(value);
            let mut typing = value.get(at..).unwrap_or_default().to_string();
            let before = typing.clone();
            // The keys on the chips, before the text box sees them (Backspace with nothing typed
            // selects the last, a second takes it out).
            let mut keyed = Vec::new();
            recipients::keys(ui, edit_id, list.len(), &typing, &mut keyed);
            // A list pasted a person a line (from a spreadsheet or another mail program) is a
            // chip each: its lines apart by `;` (the one-line box ran them together, one entry).
            if ui.memory(|m| m.has_focus(edit_id)) {
                ui.input_mut(|i| {
                    for e in &mut i.events {
                        if let egui::Event::Paste(pasted) = e
                            && pasted.contains(['\n', '\r', '\t'])
                        {
                            *pasted = pasted.split(['\n', '\r', '\t']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ");
                        }
                    }
                });
            }
            let typing_state = people_box::Typing { index: &mut st.suggest_index, dismissed: &mut st.suggest_dismissed, refocus };
            let shown = if showing { suggestions } else { &[] };
            let hint = if list.is_empty() { tl!(hint) } else { "" };
            let (typed, mut done) = recipients::chips_and_typing(ui, &t, field, value, edit_id, |ui| {
                people_box::edit(ui, &t, edit_id, &mut typing, shown, typing_state, |ui, e| underlined(ui, &t, e.hint_text(hint)))
            });
            done.extend(keyed);
            if typing != before {
                *value = format!("{}{typing}", value.get(..at).unwrap_or_default());
            }
            (b.clicked(), typed, done)
        })
        .inner;
    let r = &typed.response;
    if focus {
        r.request_focus();
    }
    if refocus && people_box::click_over(ui) {
        st.suggest_refocus = None;
    }
    if typed.took {
        st.suggest_refocus = Some(field);
    }
    if r.has_focus() || typed.took || st.suggest_refocus == Some(field) {
        st.suggest_field = Some(field);
    } else if st.suggest_field == Some(field) {
        st.suggest_field = None;
    }
    // Half-typed addresses are fine while typing; check once the box is left.
    address_check(ui, &t, value, !r.has_focus() && st.suggest_refocus != Some(field));
    (pressed, typed.check, done, edit_id)
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
