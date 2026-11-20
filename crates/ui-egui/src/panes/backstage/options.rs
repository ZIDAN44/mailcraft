//! File › Options: the app's settings, a card per subject.

use super::*;

/// MailCraft's own colour, the accent when none is chosen ([`theme::APP_COLOR`]).
const APP_RGB: [u8; 3] = [theme::APP_COLOR.r(), theme::APP_COLOR.g(), theme::APP_COLOR.b()];

mod calendar;
mod general;
mod inbox;
mod mail;
mod safety;

pub(super) fn options(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    heading(ui, &t, "Options");
    ui.set_max_width(ui.available_width().min(760.0));
    general::general(s, st, ui, acts);
    general::data_files(s, st, ui, acts);
    mail::reading(s, &t, ui, acts);
    mail::kept(s, &t, ui, acts);
    mail::checking(s, &t, ui, acts);
    inbox::inbox(s, &t, ui, acts);
    calendar::calendar(s, &t, ui, acts);
    super::meetings::meetings(s, st, ui, acts);
    safety::senders(s, &t, ui, acts);
    safety::safety(s, &t, ui, acts);
    safety::privacy(s, &t, ui, acts);
}

/// One setting of a card, one per row, by its name and the line about it (in the rows Info and
/// Open & Export draw too).
fn setting(ui: &mut Ui, t: &Tokens, first: bool, label: &str, about: Option<&str>, control: impl FnOnce(&mut Ui)) {
    setting_row(ui, t, first, name(t, label), about.map(|a| note(t, a)), control);
}

/// A setting that is a switch: its name, also the switch's to a screen reader; the line about
/// it; whether it's on; the command told whether it's turned on.
struct Switch<'a> {
    label: &'a str,
    about: Option<&'a str>,
    on: bool,
    cmd: &'static str,
}

/// A setting that is a switch, its name given once.
fn switch_row(ui: &mut Ui, t: &Tokens, first: bool, w: Switch<'_>, acts: &mut Vec<Action>) {
    let Switch { label, about, on, cmd } = w;
    setting(ui, t, first, label, about, |ui| {
        let mut v = on;
        if widgets::switch(ui, t, &mut v, label).changed() {
            acts.push(Action::Run(cmd, json!({"on": v})));
        }
    });
}

/// The accent colours as swatches (MailCraft's teal first), then any colour with a picker.
fn accent_choice(ui: &mut Ui, t: &Tokens, current: &str, acts: &mut Vec<Action>) {
    let names = [
        ("teal", tl!("Teal")),
        ("blue", tl!("Blue")),
        ("indigo", tl!("Indigo")),
        ("violet", tl!("Violet")),
        ("rose", tl!("Rose")),
        ("red", tl!("Red")),
        ("orange", tl!("Orange")),
        ("green", tl!("Green")),
    ];
    // The row is laid out from its end: the swatches go in the other way round, so teal reads
    // first (and the picker last).
    let from_end = ui.layout().prefer_right_to_left() != crate::i18n::current().rtl();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let picker = |ui: &mut Ui, acts: &mut Vec<Action>| {
            // Any colour: what's chosen now, or the accent in use, to start from.
            let mut rgb = mailcraft_engine::accent_rgb(current).unwrap_or(APP_RGB);
            let r = egui::widgets::color_picker::color_edit_button_srgb(ui, &mut rgb).on_hover_text(tl!("Any colour"));
            if current.starts_with('#') {
                ui.painter().rect_stroke(r.rect.expand(2.0), 4.0, egui::Stroke::new(2.0, t.text), egui::StrokeKind::Outside);
            }
            if r.changed() {
                acts.push(Action::Run("view.accent", json!({"color": format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])})));
            }
        };
        if from_end {
            picker(ui, acts);
            ui.add_space(8.0);
        }
        let ordered: Vec<(&str, &str)> = if from_end { names.iter().rev().copied().collect() } else { names.to_vec() };
        for (code, label) in ordered {
            let rgb = mailcraft_engine::accent_rgb(code).unwrap_or(APP_RGB);
            let on = current == code || (code == "teal" && current.is_empty());
            let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            widgets::named(ui, &resp, label, true, Some(on));
            let fill = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            if on {
                ui.painter().circle_stroke(rect.center(), 11.5, egui::Stroke::new(2.0, t.text));
            } else if resp.hovered() {
                ui.painter().circle_stroke(rect.center(), 11.5, egui::Stroke::new(1.0, t.border));
            }
            ui.painter().circle_filled(rect.center(), 8.5, fill);
            if resp.on_hover_text(label).clicked() && !on {
                acts.push(Action::Run("view.accent", json!({"color": code})));
            }
        }
        if !from_end {
            ui.add_space(8.0);
            picker(ui, acts);
        }
    });
}
