//! A form's text boxes and checkboxes as they read: the words in a box in its middle, top to
//! bottom (they sat high, the box's spare height all under them), and a checkbox beside the first
//! line of its words (it sat lower, in the middle of a control's height).

use egui::accesskit::Role;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 1400.0);

/// The account form, filled in (changing an account), drawn until it has settled.
fn form(name: &str, dialog: Dialog) -> (MailApp, egui::Context) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(dialog);
    for _ in 0..4 {
        let _ = drawn_text(&ctx, &mut a, SIZE);
    }
    (a, ctx)
}

#[test]
fn a_text_boxs_words_sit_in_its_middle() {
    let config = {
        let (a, _) = app("fields-look-config");
        a.session.accounts[0].config.clone()
    };
    let (mut a, ctx) = form("fields-look-boxes", Dialog::edit_account(&config));
    let texts = drawn_text(&ctx, &mut a, SIZE);
    let boxes: Vec<egui::Rect> = named(&ctx, &mut a, SIZE).iter().filter(|n| n.role == Role::TextInput).filter_map(|n| n.rect).collect();
    let mut checked = 0;
    for b in &boxes {
        // The words drawn inside this box (the address, the name, a server's host).
        for d in texts.iter().filter(|d| !d.text.trim().is_empty() && b.contains_rect(d.rect)) {
            let off = d.rect.center().y - b.center().y;
            assert!(off.abs() <= 1.0, "\"{}\" is {off:+.1} points from the middle of its box {b:?}", d.text);
            checked += 1;
        }
    }
    assert!(checked >= 3, "boxes with words in them: {checked} of {}", boxes.len());
}

#[test]
fn a_checkbox_sits_beside_its_first_line() {
    let (mut a, ctx) = form("fields-look-checkbox", Dialog::add_account());
    let words = "Online only: keep none of this account's mail on this computer";
    let text = drawn_text(&ctx, &mut a, SIZE).into_iter().find(|d| d.text == words).map(|d| d.rect).expect("the checkbox's words");
    let check = named(&ctx, &mut a, SIZE)
        .into_iter()
        .filter(|n| n.role == Role::CheckBox)
        .filter_map(|n| n.rect)
        .find(|r| (r.center().y - text.center().y).abs() < 30.0 && r.right() <= text.left() + 1.0)
        .expect("the box before the words");
    let line = ctx.fonts_mut(|f| f.row_height(&crate::theme::regular(crate::sizes::BODY)));
    let first_line = text.top() + line / 2.0;
    let off = check.center().y - first_line;
    assert!(off.abs() <= 1.0, "the box {check:?} is {off:+.1} points from the middle of the first line ({first_line})");
}
