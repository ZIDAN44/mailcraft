//! The message being written: the row of options under its ribbon (Save draft, Bcc, the format,
//! importance, a read receipt, signing and encryption, Delay delivery, Replies to; Discard and
//! Pop out at the right), and when to send a held-back message.

use super::*;

/// The options under the ribbon: Drafts, Bcc and the format at the left (wrapping onto a second
/// line in a narrow pane); Discard and Pop out (or in) kept at the right, as in Outlook.
pub(super) fn options_row(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form, acts: &mut Vec<Action>) {
    let (icon, label, id) = if f.in_window { (Icon::PopIn, "Pop in", "compose.popIn") } else { (Icon::PopOut, "Pop out", "compose.popOut") };
    // Where windows can't open (the browser), messages stay in the reading pane.
    let can_pop = !ui.ctx().embed_viewports();
    let width_of = |ui: &Ui, l: &str| ui.painter().layout_no_wrap(tl!(l).to_string(), theme::regular(13.0), t.text).size().x + 36.0;
    let gap = ui.spacing().item_spacing.x;
    let right = width_of(ui, "Discard") + if can_pop { width_of(ui, label) + gap } else { 0.0 } + 2.0 * gap;
    // A pane too narrow for both halves side by side has them one under the other (the
    // left one was drawn under the right one).
    let beside = ui.available_width() - right >= 160.0;
    let halves = if beside { egui::Layout::left_to_right(egui::Align::Center) } else { egui::Layout::top_down(egui::Align::Min) };
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), halves, |ui| {
        let left_w = if beside { ui.available_width() - right } else { ui.available_width() };
        ui.allocate_ui_with_layout(vec2(left_w, 30.0), crate::bidi::row_from_start(egui::Align::Center).with_main_wrap(true), |ui| {
            left_options(ui, t, c, f, acts);
        });
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Min), |ui| {
            if can_pop
                && widgets::tool_button(ui, icon, label, t, true)
                    .on_hover_text(tl!(if f.in_window { "Back to the reading pane" } else { "Write it in a window of its own" }))
                    .clicked()
            {
                acts.push(Action::Run(id, json!({})));
            }
            if widgets::tool_button(ui, Icon::Delete, "Discard", t, true).clicked() {
                acts.push(Action::Run("compose.discard", json!({})));
            }
        });
    });
}

/// The options at the left of the row.
fn left_options(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form, acts: &mut Vec<Action>) {
    if widgets::tool_button(ui, Icon::Drafts, "Save draft", t, !c.sending).on_hover_text(tl!("Save draft (Ctrl+S)")).clicked() {
        acts.push(Action::Run("compose.saveDraft", json!({})));
    }
    if widgets::tool_button(ui, Icon::Person, if c.show_bcc { "Hide Bcc" } else { "Bcc" }, t, !c.sending).clicked() {
        c.show_bcc = !c.show_bcc;
    }
    let mut plain = c.plain;
    if ui.checkbox(&mut plain, RichText::new(tl!("Plain text")).small()).on_hover_text(tl!("Send without formatting")).changed() {
        c.plain = plain;
    }
    // Importance and a read receipt, as Outlook's Tags and Options.
    ui.add_space(6.0);
    if ui.selectable_label(c.importance > 0, RichText::new("!").strong().color(t.danger)).on_hover_text(tl!("High importance")).clicked() {
        c.importance = if c.importance > 0 { 0 } else { 1 };
    }
    if ui.selectable_label(c.importance < 0, RichText::new("↓").strong().color(t.accent)).on_hover_text(tl!("Low importance")).clicked() {
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
    if f.has_cert || f.has_pgp {
        sign_and_encrypt(ui, t, c, f);
    }
    // Outlook's Delay Delivery and Direct Replies To.
    if f.can_delay {
        let label = match c.send_after {
            Some(at) => tf!("Sends {when}", when = crate::i18n::date(&dates::full_time(at, &f.clock))),
            None => tl!("Delay delivery").to_string(),
        };
        let r = widgets::tool_toggle(ui, Icon::Clock, &label, t, c.send_after.is_some())
            .on_hover_text(tl!("Keep it in the Outbox and send it later (MailCraft has to be running then)"));
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| delay_menu(ui, t, c, &f.clock));
    }
    if widgets::tool_toggle(ui, Icon::Reply, "Replies to", t, c.show_reply_to || !c.reply_to.is_empty())
        .on_hover_text(tl!("Have replies sent to another address"))
        .clicked()
    {
        c.show_reply_to = !c.show_reply_to;
    }
}

/// Sign and Encrypt, for an account with a certificate or an OpenPGP key; which of the two,
/// when it has both.
fn sign_and_encrypt(ui: &mut Ui, t: &Tokens, c: &mut mailcraft_engine::Compose, f: &Form) {
    let toggle = |ui: &mut Ui, on: &mut bool, icon: Icon, label: &str, about: &str| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, icon, if *on { t.accent } else { t.dim });
        ui.checkbox(on, RichText::new(tl!(label)).small()).on_hover_text(tl!(about)).changed()
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
