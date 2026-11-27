//! A dialog's text boxes that take more than a box: a password's, which can be shown, and one
//! with a button after it (Browse…).

use super::*;

/// A password's box, the row's width, with a button at its end that shows what's typed and hides
/// it again (they were dots alone, a mistyped one found only when the server refused it).
pub(crate) fn password_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    let h = ui.spacing().interact_size.y;
    let gap = ui.spacing().item_spacing.x;
    let id = ui.next_auto_id().with("password-shown");
    let mut shown = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let t = Tokens::of(ui);
    let inner = ui.horizontal(|ui| {
        let w = (ui.available_width() - h - gap).max(80.0);
        let r = ui.add_sized([w, h], egui::TextEdit::singleline(text).password(!shown).hint_text(hint));
        let (icon, tip) = if shown { (Icon::EyeOff, tl!("Hide the password")) } else { (Icon::Eye, tl!("Show the password")) };
        let (rect, b) = ui.allocate_exact_size(vec2(h, h), Sense::click());
        named(ui, &b, tip, true, Some(shown));
        if b.hovered() {
            ui.painter().rect_filled(rect, crate::sizes::CONTROL_RADIUS, t.hover);
        }
        icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, t.dim);
        if b.on_hover_text(tip).clicked() {
            shown = !shown;
        }
        r
    });
    ui.data_mut(|d| d.insert_temp(id, shown));
    inner.inner
}

/// A list's last row: a box and Add, Add on while what's typed is `ok`, and `why` it isn't under
/// the box once something is typed. True when it's to be added (Add, or Enter in the box).
pub(crate) fn add_row(ui: &mut Ui, text: &mut String, hint: &str, ok: bool, why: &str) -> bool {
    let t = Tokens::of(ui);
    ui.add_space(8.0);
    let mut add = false;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width((ui.available_width() - 90.0).clamp(160.0, 360.0)));
        add |= ok && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        add |= ui.add_enabled(ok, egui::Button::new(tl!("Add"))).clicked();
    });
    if !text.trim().is_empty() && !ok {
        crate::dialogs::field_error(ui, &t, why);
    }
    ui.add_space(4.0);
    add
}

/// A text box filling its row but for a button after it (Browse…), the row ending where the
/// fields above it do (the box was the room less 100 or 112 points, the row ragged): true when
/// the button is clicked.
pub(crate) fn field_and_button(ui: &mut Ui, text: &mut String, hint: &str, button: &str) -> bool {
    let gap = ui.spacing().item_spacing.x;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label_w = ui.painter().layout_no_wrap(button.to_string(), font, Color32::PLACEHOLDER).size().x;
    let button_w = (label_w + 2.0 * ui.spacing().button_padding.x).max(ui.spacing().interact_size.x);
    let h = ui.spacing().interact_size.y;
    let mut clicked = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - button_w - gap).max(80.0);
        ui.add_sized([w, h], egui::TextEdit::singleline(text).hint_text(hint));
        clicked = ui.add(egui::Button::new(button).min_size(vec2(button_w, h))).clicked();
    });
    clicked
}
