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
        let r = ui.add_sized([w, h], line_edit(text).password(!shown).hint_text(hint));
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

/// A one-line text box, its words in its middle top to bottom. egui's own put them at the top,
/// so a box taller than its line (a dialog's 32 points) had its spare height all under them.
/// Every one-line box is made here (`tests::one_line_boxes_are_made_here`).
pub(crate) fn line_edit(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).align(egui::Align2::LEFT_CENTER)
}

/// A checkbox whose words wrap: the box beside their first line (egui's centred it on all of
/// them, two lines of German put it between them), a click on the words ticking it too.
pub(crate) fn checkbox_wrapped(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    ui.horizontal_top(|ui| {
        // The box as tall as the words' first line, so it's in that line's middle: a dialog's
        // controls are 32 points, and in their middle it sat lower than the line.
        ui.spacing_mut().interact_size.y = ui.text_style_height(&egui::TextStyle::Body);
        let mut r = ui.checkbox(on, "");
        let words = ui.add(egui::Label::new(text).wrap().sense(Sense::click()));
        if words.clicked() {
            *on = !*on;
            r.mark_changed();
        }
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *on, text));
        r | words
    })
    .inner
}

/// A list's last row: a box and Add, Add on while what's typed is `ok`, and `why` it isn't under
/// the box once something is typed. True when it's to be added (Add, or Enter in the box).
pub(crate) fn add_row(ui: &mut Ui, text: &mut String, hint: &str, ok: bool, why: &str) -> bool {
    let t = Tokens::of(ui);
    ui.add_space(8.0);
    let mut add = false;
    ui.horizontal(|ui| {
        let r = ui.add(line_edit(text).hint_text(hint).desired_width((ui.available_width() - 90.0).clamp(160.0, 360.0)));
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
        ui.add_sized([w, h], line_edit(text).hint_text(hint));
        clicked = ui.add(egui::Button::new(button).min_size(vec2(button_w, h))).clicked();
    });
    clicked
}

#[cfg(test)]
mod tests {
    /// Every one-line text box is a [`super::line_edit`], so its words sit in its middle: egui's
    /// own put them at its top.
    #[test]
    fn one_line_boxes_are_made_here() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if p.is_dir() && name != "tests" {
                    walk(&p, out);
                } else if name.ends_with(".rs") && name != "tests.rs" && name != "form_fields.rs" {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 100, "the window's code found: {}", files.len());
        let mut bare = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                if line.contains("TextEdit::singleline(") || line.contains("text_edit_singleline(") {
                    bare.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(bare.is_empty(), "one-line boxes not made with widgets::line_edit:\n{}", bare.join("\n"));
    }
}
