//! A dialog's buttons: its main one, one that deletes for good, and the others; and an outlined
//! button with an icon, another way to do what the main one does.

use super::*;

/// An outlined button with an icon before its label: another way to do what a dialog's main
/// button does (Sign in with Google beside Connect), not filled in the app colour as that one is.
pub(crate) fn outline_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool) -> Response {
    let font = theme::regular(crate::sizes::BODY);
    let ink = if enabled { t.text } else { t.faint };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, ink);
    let h = ui.spacing().interact_size.y;
    let size = vec2(galley.size().x + 44.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    let fill = if enabled && resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, crate::sizes::CONTROL_RADIUS, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let m = crate::bidi::Mirror::new(rect);
    let ir = m.rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 12.0, rect.center().y - 8.0), vec2(16.0, 16.0)));
    icons::paint(ui.painter(), ir, icon, ink);
    let at = egui::pos2(m.start(rect.min.x + 34.0, galley.size().x), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, ink);
    resp
}

/// What a dialog's button is: its main one, filled in the app colour; one that deletes or
/// removes for good, filled red; or another (Cancel, Close), outlined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Danger,
    Secondary,
}

/// A dialog's button, words only, of `kind`: every kind one height and at least
/// [`crate::sizes::BUTTON_MIN_W`] wide (Cancel was shorter and narrower than the button beside
/// it). One that can't be used now is drawn quiet, not as a darker button.
pub fn dialog_button(ui: &mut Ui, kind: ButtonKind, label: &str, t: &Tokens, enabled: bool) -> Response {
    use crate::sizes::{BUTTON_MIN_W, CONTROL_H, CONTROL_RADIUS};
    let label = tl!(label);
    let font = if kind == ButtonKind::Secondary { theme::regular(13.5) } else { theme::semibold(13.5) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, t.text);
    let size = vec2((galley.size().x + 32.0).max(BUTTON_MIN_W), CONTROL_H);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    let hovered = enabled && resp.hovered();
    let r = f32::from(CONTROL_RADIUS);
    let (fill, stroke, ink) = match (kind, enabled) {
        // Quiet, outlined: in the dark the style's own grey was a light slab, brighter than Cancel.
        (_, false) => (t.chrome, Stroke::new(1.0, t.border), t.faint),
        (ButtonKind::Primary, true) => (accent_fill(t, true, hovered), Stroke::NONE, t.on_accent),
        // The light theme's red in both (the dark theme's lighter red is for text, and white on
        // it was hard to read).
        (ButtonKind::Danger, true) => {
            let red = Color32::from_rgb(0xC4, 0x2B, 0x1C);
            (if hovered { red.gamma_multiply(0.88) } else { red }, Stroke::NONE, Color32::WHITE)
        }
        (ButtonKind::Secondary, true) => (if hovered { t.hover } else { t.surface }, ui.visuals().widgets.inactive.bg_stroke, t.text),
    };
    ui.painter().rect(rect, r, fill, stroke, egui::StrokeKind::Inside);
    if resp.has_focus() {
        ui.painter().rect_stroke(rect.expand(2.0), r + 2.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Outside);
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    resp
}

/// A dialog's main button (OK, Connect, Save…): filled in the app colour, words only.
pub fn primary_button(ui: &mut Ui, label: &str, t: &Tokens, enabled: bool) -> Response {
    dialog_button(ui, ButtonKind::Primary, label, t, enabled)
}

/// A dialog's other button (Cancel, Close…): outlined, as tall and wide as its main one.
pub fn secondary_button(ui: &mut Ui, label: &str, t: &Tokens, enabled: bool) -> Response {
    dialog_button(ui, ButtonKind::Secondary, label, t, enabled)
}

/// [`secondary_button`] in the theme the window is drawn in.
pub fn secondary(ui: &mut Ui, label: &str) -> Response {
    let t = Tokens::of(ui);
    secondary_button(ui, label, &t, true)
}
