//! The message being written: the row of options under its ribbon (Save draft, Bcc, the format,
//! importance, a read receipt, signing and encryption, Delay delivery, Replies to; Discard and
//! Pop out at the right), and when to send a held-back message.

use super::*;

/// The options under the ribbon: Save draft, Bcc and the message's Options (its format,
/// importance, a read receipt, signing and encryption, Delay delivery, Replies to) at the left;
/// Pop out (or in) and Discard at the right, as icons. Each option was a control of its own kind
/// on the row (buttons, checkboxes, a bare "!" and "↓"), and a long label (German's Pop in) put
/// the row on two lines.
pub(super) fn options_row(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form, acts: &mut Vec<Action>) {
    let (icon, label, id) = if f.in_window { (Icon::PopIn, "Pop in", "compose.popIn") } else { (Icon::PopOut, "Pop out", "compose.popOut") };
    // Where windows can't open (the browser), messages stay in the reading pane.
    let can_pop = !ui.ctx().embed_viewports();
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let right = if can_pop { 2.0 * 32.0 + 2.0 } else { 32.0 };
        let left_w = (ui.available_width() - right - 8.0).max(0.0);
        ui.allocate_ui_with_layout(vec2(left_w, 32.0), crate::bidi::row_from_start(egui::Align::Center), |ui| {
            left_options(ui, t, c, f, acts);
        });
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if widgets::icon_action(ui, Icon::Delete, "Discard", t).clicked() {
                acts.push(Action::Run("compose.discard", json!({})));
            }
            let pop = widgets::icon_action(ui, icon, label, t);
            if can_pop && pop.on_hover_text(tl!(if f.in_window { "Back to the reading pane" } else { "Write it in a window of its own" })).clicked() {
                acts.push(Action::Run(id, json!({})));
            }
        });
    });
}

/// The options at the left of the row.
fn left_options(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form, acts: &mut Vec<Action>) {
    if widgets::tool_button(ui, Icon::Save, "Save draft", t, !c.sending).on_hover_text(tl!("Save draft (Ctrl+S)")).clicked() {
        acts.push(Action::Run("compose.saveDraft", json!({})));
    }
    if widgets::tool_button(ui, Icon::EyeOff, if c.show_bcc { "Hide Bcc" } else { "Bcc" }, t, !c.sending).clicked() {
        c.show_bcc = !c.show_bcc;
    }
    // The rest in one menu, lit while one of them is on: Outlook's Tags and Options.
    let any = c.plain || c.importance != 0 || c.read_receipt || c.send_after.is_some() || c.show_reply_to || !c.reply_to.is_empty();
    let r = widgets::tool_toggle(ui, Icon::Gear, "Options", t, any);
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let mut plain = c.plain;
        if ui.checkbox(&mut plain, tl!("Plain text")).on_hover_text(tl!("Send without formatting")).changed() {
            c.plain = plain;
        }
        let (mut high, mut low) = (c.importance > 0, c.importance < 0);
        if ui.checkbox(&mut high, tl!("High importance")).changed() {
            c.importance = i8::from(high);
        }
        if ui.checkbox(&mut low, tl!("Low importance")).changed() {
            c.importance = if low { -1 } else { 0 };
        }
        let mut receipt = c.read_receipt;
        if ui
            .checkbox(&mut receipt, tl!("Read receipt"))
            .on_hover_text(tl!("Ask the recipients to confirm they've read it (their mail app may not, or may ask them)"))
            .changed()
        {
            c.read_receipt = receipt;
        }
        if f.has_cert || f.has_pgp {
            ui.separator();
            sign_and_encrypt(ui, t, c, f);
        }
        ui.separator();
        // Outlook's Direct Replies To and Delay Delivery.
        let mut reply_to = c.show_reply_to || !c.reply_to.is_empty();
        if ui.checkbox(&mut reply_to, tl!("Replies to")).on_hover_text(tl!("Have replies sent to another address")).changed() {
            c.show_reply_to = reply_to;
        }
        if f.can_delay {
            ui.menu_button(tl!("Delay delivery"), |ui| delay_menu(ui, t, c, &f.clock))
                .response
                .on_hover_text(tl!("Keep it in the Outbox and send it later (MailCraft has to be running then)"));
        }
    });
    // Held back: when it goes, in sight.
    if let Some(at) = c.send_after.filter(|_| f.can_delay) {
        let label = tf!("Sends {when}", when = crate::i18n::date(&dates::full_time(at, &f.clock)));
        let r = widgets::tool_toggle(ui, Icon::Clock, &label, t, true);
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| delay_menu(ui, t, c, &f.clock));
    }
}

/// Sign and Encrypt, for an account with a certificate or an OpenPGP key; which of the two,
/// when it has both.
fn sign_and_encrypt(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form) {
    // Each on its row of the Options menu, its icon before it.
    let toggle = |ui: &mut Ui, on: &mut bool, icon: Icon, label: &str, about: &str| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, if *on { t.accent } else { t.dim });
            ui.checkbox(on, tl!(label)).on_hover_text(tl!(about)).changed()
        })
        .inner
    };
    let (sign_about, encrypt_about) = if f.has_pgp {
        (
            "Signed with your certificate or OpenPGP key: the recipients can tell it's from you and unchanged",
            "Only the recipients can read it: each needs a certificate or OpenPGP key MailCraft has",
        )
    } else {
        (
            "Signed with your certificate: the recipients can tell it's from you and unchanged",
            "Only the recipients can read it: each needs to have sent you a signed message",
        )
    };
    let mut sign = c.sign;
    if toggle(ui, &mut sign, Icon::Seal, "Sign", sign_about) {
        c.sign = sign;
    }
    let mut encrypt = c.encrypt;
    if toggle(ui, &mut encrypt, Icon::Lock, "Encrypt", encrypt_about) {
        c.encrypt = encrypt;
        // Chosen: encryption no longer follows the recipients.
        c.encrypt_auto = false;
    }
    if f.has_cert && f.has_pgp {
        security::scheme_menu(ui, c);
    }
}

/// When to send a held-back message: in an hour, tomorrow or Monday morning, a time of one's
/// own, or at once again.
fn delay_menu(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, clock: &dates::Clock) {
    let today = dates::today(clock);
    let at = |day: i64, minutes: i64| clock.utc(day.saturating_mul(86_400).saturating_add(minutes * 60));
    let weekday = i64::from(dates::weekday_index(today));
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
        let at = c.send_after.unwrap_or(clock.now + 3600);
        let c0 = dates::civil(at, clock);
        (dates::day_string(clock.local_day(at)), crate::i18n::time_box(c0.hour, c0.minute, clock.style))
    });
    ui.label(RichText::new(tl!("Don't send before")).color(t.dim).small());
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut day).desired_width(90.0).hint_text("YYYY-MM-DD"));
        let (hint, width) = (crate::i18n::time_box(8, 0, clock.style), if clock.style.hours12 { 76.0 } else { 50.0 });
        ui.add(egui::TextEdit::singleline(&mut time).desired_width(width).hint_text(hint));
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

/// A local `YYYY-MM-DD` and a time as typed, on either clock (gap 9), as (day, minutes into it).
pub(crate) fn parse_local(day: &str, time: &str) -> Option<(i64, i64)> {
    if !mailcraft_engine::model::job::is_day(day.trim()) {
        return None;
    }
    let d = day.trim();
    let num = |a: usize, z: usize| d.get(a..z).and_then(|x| x.parse::<u32>().ok());
    let days = dates::days_from_civil(i64::from(num(0, 4)?), num(5, 7)?, num(8, 10)?);
    let (h, m) = crate::i18n::read_time(time)?;
    Some((days, i64::from(h) * 60 + i64::from(m)))
}
