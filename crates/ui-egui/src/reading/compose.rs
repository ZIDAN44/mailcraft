//! The message being written: its form (from, to, cc, bcc, subject), its ribbon and its body.

use super::*;

/// Height of an attachment chip.
pub(super) const CHIP_H: f32 = 42.0;

/// When to send a held-back message: in an hour, tomorrow or Monday morning, a time of one's
/// own, or at once again.
pub(super) fn delay_menu(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, clock: &dates::Clock) {
    let today = dates::today(clock);
    let at = |day: i64, minutes: i64| clock.utc(day.saturating_mul(86_400).saturating_add(minutes * 60));
    let weekday = (today + 3).rem_euclid(7);
    let presets = [
        ("In 1 hour", (clock.now + 3600) / 60 * 60),
        ("Tomorrow morning", at(today + 1, 8 * 60)),
        ("Monday morning", at(today + 7 - weekday, 8 * 60)),
    ];
    for (label, when) in presets {
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(c.send_after == Some(when), tl!(label));
                ui.label(RichText::new(crate::i18n::date(&dates::full_time(when, clock))).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            c.send_after = Some(when);
            ui.close();
        }
    }
    ui.separator();
    // A day and time of one's own (local), kept while the menu is open.
    let id = egui::Id::unique(("delay-custom", c.id));
    let (mut day, mut time) = ui.data(|d| d.get_temp::<(String, String)>(id)).unwrap_or_else(|| {
        let c0 = dates::civil(c.send_after.unwrap_or(clock.now + 3600), clock);
        (format!("{:04}-{:02}-{:02}", c0.year, c0.month, c0.day), format!("{:02}:{:02}", c0.hour, c0.minute))
    });
    ui.label(RichText::new(tl!("Don't send before")).color(t.dim).small());
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut day).desired_width(90.0).hint_text("YYYY-MM-DD"));
        ui.add(egui::TextEdit::singleline(&mut time).desired_width(50.0).hint_text("HH:MM"));
        let parsed = parse_local(&day, &time).map(|(d, m)| at(d, m));
        if ui.add_enabled(parsed.is_some_and(|w| w > clock.now), egui::Button::new(tl!("Set"))).clicked() {
            chosen = parsed;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (day, time)));
    if let Some(w) = chosen {
        c.send_after = Some(w);
        ui.close();
    }
    if c.send_after.is_some() {
        ui.separator();
        if ui.button(tl!("Send right away")).clicked() {
            c.send_after = None;
            ui.close();
        }
    }
}

/// A local `YYYY-MM-DD` and `HH:MM` as (day, minutes into it).
pub(crate) fn parse_local(day: &str, time: &str) -> Option<(i64, i64)> {
    if !mailcraft_engine::model::job::is_day(day.trim()) {
        return None;
    }
    let d = day.trim();
    let num = |a: usize, z: usize| d.get(a..z).and_then(|x| x.parse::<u32>().ok());
    let days = dates::days_from_civil(i64::from(num(0, 4)?), num(5, 7)?, num(8, 10)?);
    let (h, m) = time.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((days, h * 60 + m))
}

/// Width of the labels and label buttons left of the message's fields.
pub(super) const LABEL_W: f32 = 76.0;

pub(super) const ROW_H: f32 = 30.0;

/// A field box drawn the way Outlook's are: no frame, a line under it (the accent while typing).
pub(super) fn underlined(ui: &mut Ui, t: &Tokens, edit: egui::TextEdit<'_>) -> egui::Response {
    let r = ui.add_sized(vec2(ui.available_width(), ROW_H - 4.0), edit.frame(egui::Frame::NONE).align(egui::Align2::LEFT_CENTER));
    let y = r.rect.bottom();
    let color = if r.has_focus() { t.accent } else { t.border };
    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(1.0, color));
    r
}

/// A small framed button naming a field (To, Cc…), as in Outlook's message window.
pub(super) fn label_button(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool) -> egui::Response {
    label_button_menu(ui, t, label, enabled, false)
}

/// A label button that opens a menu: its label and a small drawn chevron, centred together.
pub(super) fn label_button_menu(ui: &mut Ui, t: &Tokens, label: &str, enabled: bool, menu: bool) -> egui::Response {
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

/// Outlook's tall Send button beside the address rows.
pub(super) fn send_button(ui: &mut Ui, t: &Tokens, height: f32, sending: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(66.0, height), if sending { Sense::hover() } else { Sense::click() });
    crate::widgets::named(ui, &resp, if sending { tl!("Sending…") } else { tl!("Send") }, !sending, None);
    let fill = if resp.hovered() && !sending { t.hover } else { t.surface };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(24.0, 24.0));
    icons::paint(ui.painter(), icon, Icon::Send, if sending { t.dim } else { t.accent });
    let label = if sending { tl!("Sending…") } else { tl!("Send") };
    ui.painter().text(rect.center() + vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, theme::regular(12.5), t.text);
    resp.on_hover_text(tl!("Send (Ctrl+Enter)"))
}

pub(super) fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, check_addresses: bool, focus: bool) {
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

/// The message being written, in a window of its own.
pub fn compose_window(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    compose(s, st, ui, acts, true);
}

pub(super) fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    attach_dropped(ui, &t, acts);
    // Where there are no windows of their own (the browser), the other messages being written
    // are a click away above this one.
    if !in_window && !s.composes.is_empty() && ui.ctx().embed_viewports() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Also being written:")).color(t.dim).small());
            for c in &s.composes {
                let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
                if ui.small_button(subject).clicked() {
                    acts.push(Action::Run("compose.activate", json!({"id": c.id})));
                }
            }
        });
    }
    let accounts: Vec<(String, String)> =
        s.accounts.iter().map(|a| (a.config.id.clone(), EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string())).collect();
    // Suggestions for the recipient box being typed in (computed before the form borrows it).
    let suggestions: Vec<Contact> = match (st.suggest_field, s.compose.as_ref()) {
        (Some(f), Some(c)) => {
            let text = match f {
                "to" => c.to.clone(),
                "cc" => c.cc.clone(),
                _ => c.bcc.clone(),
            };
            let token = mailcraft_engine::model::contacts::last_token(&text).to_string();
            if token.chars().count() < 1 || st.suggest_dismissed.as_deref() == Some(text.as_str()) {
                Vec::new()
            } else {
                let present = text.to_lowercase();
                s.suggest_contacts(&token, 8).into_iter().filter(|x| !present.contains(&x.address.email.to_lowercase())).take(6).collect()
            }
        }
        _ => Vec::new(),
    };
    let clock = s.clock;
    // Delay Delivery holds mail in the Outbox, which only a server account's sends from.
    let can_delay = s.compose.as_ref().and_then(|c| s.account(&c.account)).is_some_and(|a| !a.is_local());
    // S/MIME's switches, for an account with a certificate.
    let has_cert = s.compose.as_ref().is_some_and(|c| s.identity(&c.account).is_some());
    let signature = s.compose.as_ref().and_then(|c| s.account(&c.account)).map(|a| a.config.signature.clone()).unwrap_or_default();
    let Some(c) = s.compose.as_mut() else { return };
    // The editor's state belongs to one message.
    if st.editor.as_ref().is_none_or(|e| e.id != c.id) {
        st.editor = Some(rich_edit::Editor::new(c.id));
    }
    let Some(mut ed) = st.editor.take() else { return };
    // Why it couldn't go: over the message, in its own window when it has one.
    if let Some(msg) = st.compose_alert.clone() {
        let modal = egui::Modal::new(egui::Id::unique(("compose-alert", c.id))).frame(crate::dialogs::dialog_frame(&t)).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Warning, t.warn);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(tl!("MailCraft")).font(theme::bold(16.0)).color(t.text));
                    let mut text = if msg == "add at least one recipient" {
                        tl!("Add at least one name in To, Cc or Bcc before sending.").to_string()
                    } else {
                        msg.clone()
                    };
                    if let Some(first) = text.get(..1) {
                        text = first.to_uppercase() + text.get(1..).unwrap_or_default();
                    }
                    if !text.ends_with('.') {
                        text.push('.');
                    }
                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                });
            });
            ui.add_space(10.0);
            ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
                ui.button(tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
            })
            .inner
        });
        if modal.inner || modal.should_close() {
            st.compose_alert = None;
        }
    }
    // A newly opened message: start in To (new mail) or the body (replies).
    let fresh = st.compose_shown.insert(c.id);
    let focus_to = fresh && c.to.trim().is_empty();
    let focus_body = fresh && !focus_to;
    let send = |c: &mailcraft_engine::Compose, acts: &mut Vec<Action>| {
        if c.subject.trim().is_empty() {
            acts.push(Action::Dialog(crate::Dialog::EmptySubject));
        } else {
            acts.push(Action::Run("compose.send", json!({})));
        }
    };
    // Alt+S sends, as in Outlook; in a window of its own the main window's shortcuts don't reach
    // it, so Ctrl+Enter and Ctrl+S are the message's own there.
    if !c.sending {
        let (alt_s, send_key, save_key) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::ALT, egui::Key::S),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
            )
        });
        if alt_s || send_key {
            send(c, acts);
        }
        if save_key {
            acts.push(Action::Run("compose.saveDraft", json!({})));
        }
    }
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 12 }).show(ui, |ui| {
        // From the right in a right-to-left interface, its rows too (the message's own text goes
        // the way it's written).
        ui.with_layout(crate::bidi::column(), |ui| {
            // The Message ribbon.
            egui::Frame::new().fill(t.chrome).corner_radius(6).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                rich_edit::ribbon(ui, &t, &mut ed, c, &signature, &clock, acts);
            });
            ui.add_space(4.0);
            // Drafts, Bcc and the format at the left (wrapping onto a second line in a narrow pane);
            // Discard and Pop out (or in) kept at the right, as in Outlook.
            ui.horizontal(|ui| {
                let (icon, label, id) =
                    if in_window { (Icon::PopIn, "Pop in", "compose.popIn") } else { (Icon::PopOut, "Pop out", "compose.popOut") };
                // Where windows can't open (the browser), messages stay in the reading pane.
                let can_pop = !ui.ctx().embed_viewports();
                let width_of = |ui: &Ui, l: &str| ui.painter().layout_no_wrap(tl!(l).to_string(), theme::regular(13.0), t.text).size().x + 36.0;
                let gap = ui.spacing().item_spacing.x;
                let right = width_of(ui, "Discard") + if can_pop { width_of(ui, label) + gap } else { 0.0 } + 2.0 * gap;
                let left_w = (ui.available_width() - right).max(120.0);
                ui.allocate_ui_with_layout(vec2(left_w, 30.0), crate::bidi::row_from_start(egui::Align::Center).with_main_wrap(true), |ui| {
                    if widgets::tool_button(ui, Icon::Drafts, "Save draft", &t, !c.sending).on_hover_text(tl!("Save draft (Ctrl+S)")).clicked() {
                        acts.push(Action::Run("compose.saveDraft", json!({})));
                    }
                    if widgets::tool_button(ui, Icon::Person, if c.show_cc { "Hide Bcc" } else { "Bcc" }, &t, !c.sending).clicked() {
                        c.show_cc = !c.show_cc;
                    }
                    let mut plain = c.plain;
                    if ui.checkbox(&mut plain, RichText::new(tl!("Plain text")).small()).on_hover_text(tl!("Send without formatting")).changed() {
                        c.plain = plain;
                    }
                    // Importance and a read receipt, as Outlook's Tags and Options.
                    ui.add_space(6.0);
                    if ui
                        .selectable_label(c.importance > 0, RichText::new("!").strong().color(t.danger))
                        .on_hover_text(tl!("High importance"))
                        .clicked()
                    {
                        c.importance = if c.importance > 0 { 0 } else { 1 };
                    }
                    if ui
                        .selectable_label(c.importance < 0, RichText::new("↓").strong().color(t.accent))
                        .on_hover_text(tl!("Low importance"))
                        .clicked()
                    {
                        c.importance = if c.importance < 0 { 0 } else { -1 };
                    }
                    let mut receipt = c.read_receipt;
                    if ui
                        .checkbox(&mut receipt, RichText::new(tl!("Read receipt")).small())
                        .on_hover_text(tl!("Ask the recipients to confirm they've read it (their mail app may not, or may ask them)"))
                        .changed()
                    {
                        c.read_receipt = receipt;
                    }
                    if has_cert {
                        let toggle = |ui: &mut Ui, on: &mut bool, icon: Icon, label: &str, about: &str| {
                            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                            icons::paint(ui.painter(), r, icon, if *on { t.accent } else { t.dim });
                            ui.checkbox(on, RichText::new(tl!(label)).small()).on_hover_text(tl!(about)).changed()
                        };
                        let mut sign = c.sign;
                        if toggle(
                            ui,
                            &mut sign,
                            Icon::Seal,
                            "Sign",
                            "Signed with your certificate: the recipients can tell it's from you and unchanged",
                        ) {
                            c.sign = sign;
                        }
                        let mut encrypt = c.encrypt;
                        if toggle(
                            ui,
                            &mut encrypt,
                            Icon::Lock,
                            "Encrypt",
                            "Only the recipients can read it: each needs to have sent you a signed message",
                        ) {
                            c.encrypt = encrypt;
                        }
                    }
                    // Outlook's Delay Delivery and Direct Replies To.
                    if can_delay {
                        let label = match c.send_after {
                            Some(at) => tf!("Sends {when}", when = crate::i18n::date(&dates::full_time(at, &clock))),
                            None => tl!("Delay delivery").to_string(),
                        };
                        let r = widgets::tool_toggle(ui, Icon::Clock, &label, &t, c.send_after.is_some())
                            .on_hover_text(tl!("Keep it in the Outbox and send it later (MailCraft has to be running then)"));
                        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| delay_menu(ui, &t, c, &clock));
                    }
                    if widgets::tool_toggle(ui, Icon::Reply, "Replies to", &t, c.show_reply_to || !c.reply_to.is_empty())
                        .on_hover_text(tl!("Have replies sent to another address"))
                        .clicked()
                    {
                        c.show_reply_to = !c.show_reply_to;
                    }
                });
                ui.with_layout(crate::bidi::row_from_end(egui::Align::Min), |ui| {
                    if can_pop
                        && widgets::tool_button(ui, icon, label, &t, true)
                            .on_hover_text(tl!(if in_window { "Back to the reading pane" } else { "Write it in a window of its own" }))
                            .clicked()
                    {
                        acts.push(Action::Run(id, json!({})));
                    }
                    if widgets::tool_button(ui, Icon::Delete, "Discard", &t, true).clicked() {
                        acts.push(Action::Run("compose.discard", json!({})));
                    }
                });
            });
            if let Some(e) = &c.error {
                ui.add_space(4.0);
                egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.label(RichText::new(tf!("Not sent: {error}", error = e)).color(t.danger));
                });
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!c.sending, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                // Send beside the address rows (From, To, Cc and Bcc when shown).
                let rows = if c.show_cc { 4.0 } else { 3.0 };
                let height = rows * ROW_H + (rows - 1.0) * 6.0;
                ui.horizontal_top(|ui| {
                    if send_button(ui, &t, height, c.sending).clicked() {
                        send(c, acts);
                    }
                    ui.add_space(10.0);
                    crate::bidi::vertical(ui, |ui| {
                        ui.horizontal(|ui| {
                            let current = accounts.iter().find(|(id, _)| *id == c.account).map(|(_, l)| l.clone()).unwrap_or_default();
                            let many = accounts.len() > 1;
                            let b = label_button_menu(ui, &t, "From", many, many);
                            if many {
                                egui::Popup::menu(&b).show(|ui| {
                                    for (id, label) in &accounts {
                                        ui.selectable_value(&mut c.account, id.clone(), label);
                                    }
                                });
                            }
                            ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), crate::bidi::row_from_start(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(current).color(t.dim)).truncate());
                            });
                        });
                        // To, Cc and Bcc open the address book, as in Outlook.
                        for (name, label, hint) in [
                            ("to", "To", "Names <addresses>, separated by ; or ,"),
                            ("cc", "Cc", ""),
                            ("bcc", "Bcc", "Hidden from the other recipients"),
                        ] {
                            if name == "bcc" && !c.show_cc {
                                continue;
                            }
                            let value = match name {
                                "to" => &mut c.to,
                                "cc" => &mut c.cc,
                                _ => &mut c.bcc,
                            };
                            if recipient_field(ui, st, name, label, value, hint, name == "to" && focus_to, &suggestions) {
                                let d = crate::Dialog::select_names(c, name);
                                acts.push(Action::Dialog(d));
                            }
                        }
                    });
                });
                if c.show_reply_to || !c.reply_to.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(76.0);
                        crate::bidi::vertical(ui, |ui| {
                            field(ui, &t, "Replies to", &mut c.reply_to, "Where replies go instead of to you", true, false)
                        });
                    });
                }
                ui.horizontal(|ui| {
                    ui.add_space(76.0);
                    crate::bidi::vertical(ui, |ui| field(ui, &t, "Subject", &mut c.subject, "Add a subject", false, false));
                });
                if !c.attachments.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(76.0 + LABEL_W);
                        let mut remove = None;
                        crate::bidi::vertical(ui, |ui| {
                            // Wrapped into rows, scrolling past two, as Outlook's are.
                            let n = c.attachments.len();
                            widgets::chip_grid(ui, egui::Id::unique(("compose-attachments", c.id)), n, 220.0, 320.0, 28.0, 2, |ui, i, w| {
                                let Some(a) = c.attachments.get(i) else { return };
                                let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                                let x_rect = Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), vec2(22.0, 22.0));
                                let x = ui.interact(x_rect, resp.id.with("remove"), Sense::click()).on_hover_text(tl!("Remove"));
                                widgets::named(ui, &x, &format!("{}: {}", tl!("Remove"), a.name), true, None);
                                let p = ui.painter();
                                p.rect(
                                    rect,
                                    4.0,
                                    if resp.hovered() { t.hover_row } else { t.surface },
                                    egui::Stroke::new(1.0, t.faint.gamma_multiply(0.55)),
                                    egui::StrokeKind::Inside,
                                );
                                if x.hovered() {
                                    p.rect_filled(x_rect, 3.0, t.hover);
                                }
                                icons::paint(p, icons::centered(x_rect, 11.0), Icon::Close, if x.hovered() { t.text } else { t.dim });
                                let ir = Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), vec2(14.0, 14.0));
                                icons::paint(p, ir, Icon::Attach, t.dim);
                                let text = format!("{} ({})", a.name, widgets::human_size(a.data.len()));
                                let g = widgets::one_line(ui, &text, theme::regular(12.0), t.text, x_rect.left() - 4.0 - (ir.right() + 6.0));
                                p.galley(egui::pos2(ir.right() + 6.0, rect.center().y - g.size().y / 2.0), g, t.text);
                                resp.on_hover_text(a.name.clone());
                                if x.clicked() {
                                    remove = Some(i);
                                }
                            });
                            // A photo that says where it was taken (looked at once per set of files).
                            let size: usize = c.attachments.iter().map(|a| a.data.len()).sum();
                            let id = egui::Id::unique(("photo-place", c.id, n, size));
                            let placed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or_else(|| {
                                let v = c.attachments.iter().any(|a| mime::without_location(&a.data).is_some());
                                ui.data_mut(|d| d.insert_temp(id, v));
                                v
                            });
                            if placed {
                                let note = if s.prefs.keep_photo_location {
                                    tl!("These photos say where they were taken, and go that way (File › Options › Safety)")
                                } else {
                                    tl!("Where these photos were taken is left out as they're sent")
                                };
                                ui.label(RichText::new(note).color(t.faint).small());
                            }
                        });
                        if let Some(i) = remove {
                            acts.push(Action::Run("compose.removeAttachment", json!({"index": i})));
                        }
                    });
                }
                ui.add_space(4.0);
                let sep = ui.available_rect_before_wrap();
                ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
                ui.add_space(8.0);
                let body_id = rich_edit::body_id(c);
                // A picture on the clipboard is pasted as a picture. On the desktop egui takes Ctrl+V as
                // a paste and never passes the key on: a picture alone on the clipboard comes as
                // `PasteImage` (so Ctrl+V pasted nothing); in the browser the key itself comes.
                if ui.memory(|m| m.has_focus(body_id))
                    && ui.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(e, egui::Event::PasteImage(_))
                                || matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command)
                        })
                    })
                {
                    acts.push(Action::PastePicture);
                }
                egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                    // Replies start above the quoted text.
                    rich_edit::body(ui, &t, &mut ed, c, focus_body.then_some(0), acts);
                });
            });
        });
    });
    st.editor = Some(ed);
}

/// Files dragged from the desktop onto the message are attached (read from where they are, as
/// Attach File reads them); while they're held over it, its edge says it takes them. In the
/// browser a dropped file has no path to read from, and nothing is attached.
fn attach_dropped(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let held_over = ui.ctx().input(|state| !state.raw.hovered_files.is_empty());
    let dropped = ui.ctx().input(|state| state.raw.dropped_files.clone());
    if held_over {
        ui.painter().rect_stroke(ui.max_rect().shrink(2.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    for f in dropped.iter().take(100).filter(|f| f.path().is_absolute()) {
        acts.push(Action::Run("compose.attach", json!({"path": f.path().to_string_lossy()})));
    }
}
