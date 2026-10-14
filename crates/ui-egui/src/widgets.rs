//! Small shared widgets: toolbar buttons, avatars, the welcome screen.

use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// A ribbon button: icon over nothing, label to the right.
pub fn tool_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let font = theme::regular(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let size = vec2(galley.size().x + 36.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let color = if enabled { t.text } else { t.faint };
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        ui.painter().rect_filled(rect, 6.0, if resp.is_pointer_button_down_on() { t.accent_soft } else { t.hover });
    }
    let ir = egui::Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, if enabled { t.accent } else { t.faint });
    ui.painter().galley_with_override_text_color(rect.min + vec2(30.0, (30.0 - galley.size().y) / 2.0), galley, color);
    resp
}

/// The filled "New mail" button.
pub fn accent_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let label = tl!(label);
    let font = theme::bold(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.on_accent);
    let size = vec2(galley.size().x + 40.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        t.faint
    } else if resp.hovered() {
        t.accent.gamma_multiply(0.88)
    } else {
        t.accent
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let ir = egui::Rect::from_min_size(rect.min + vec2(10.0, 7.0), vec2(16.0, 16.0));
    icons::paint(ui.painter(), ir, icon, t.on_accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(32.0, (30.0 - galley.size().y) / 2.0), galley, t.on_accent);
    resp
}

/// An icon button on the coloured title bar.
pub fn bar_icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(40));
    }
    icons::paint(ui.painter(), icons::centered(rect, 17.0), icon, Color32::WHITE);
    resp.on_hover_text(tl!(tip))
}

/// A small icon button inside panes.
pub fn tiny_icon_button(ui: &mut Ui, icon: Icon, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    icons::paint(ui.painter(), icons::centered(rect, 13.0), icon, if resp.hovered() { t.text } else { t.dim });
    resp.on_hover_text(tl!(tip))
}

/// A labelled icon button for the reading pane's action row.
pub fn pill_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
    let size = vec2(galley.size().x + 34.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(9.0, 6.0), vec2(16.0, 16.0)), icon, t.accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(29.0, (28.0 - galley.size().y) / 2.0), galley, t.text);
    resp
}

pub fn group_sep(ui: &mut Ui, t: &Tokens) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 26.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border);
    ui.add_space(4.0);
}

/// A round avatar with initials.
pub fn avatar(ui: &Ui, center: egui::Pos2, radius: f32, initials: &str, color: Color32) {
    ui.painter().circle_filled(center, radius, color);
    ui.painter().text(center, egui::Align2::CENTER_CENTER, initials, theme::bold(radius * 0.8), Color32::WHITE);
}

pub fn human_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    }
}

pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// First run: no accounts yet.
pub fn welcome(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.18).max(20.0));
        egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(32)).show(ui, |ui| {
            ui.set_max_width(460.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().rect_filled(r, 14.0, t.accent);
                icons::paint(ui.painter(), icons::centered(r, 38.0), Icon::Unread, t.on_accent);
                ui.add_space(12.0);
                ui.label(RichText::new(tl!("Welcome to MailCraft")).font(theme::bold(22.0)).color(t.text));
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("Your mail, on your computer. Add any IMAP account: messages are kept on this device so they're fast to search and readable offline. Your password stays in the system keychain.")).color(t.dim));
                ui.add_space(18.0);
                if accent_button(ui, Icon::Plus, "Add an account", t, true).clicked() {
                    acts.push(Action::Dialog(Dialog::add_account()));
                }
            });
        });
    });
}
