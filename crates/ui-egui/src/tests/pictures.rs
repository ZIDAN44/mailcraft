//! Pictures in a message being written: their size, and making them bigger or smaller (issue #10).

use super::*;
use crate::rich_edit::{FIT_WIDTH, own_size};

fn pointer(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) {
    frame(ctx, a, events);
    frame(ctx, a, vec![]);
}

fn press(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// The picture's width (its style's) and where it was drawn.
fn picture(a: &MailApp) -> (Option<u16>, egui::Rect) {
    let c = a.session.compose.as_ref().unwrap();
    let ed = a.ui.editor.as_ref().unwrap();
    let (at, rect) = *ed.placed.first().expect("a picture drawn");
    (c.styles.style(at).picture_width, rect)
}

/// A pasted picture wider than a page goes in at a page's width, on a line of its own, as big
/// as that (not a thumbnail); clicked, it's selected; its corner handle drags it smaller, and
/// undo takes that back; its menu sets it to half its own size.
#[test]
fn pictures_show_at_their_size_and_resize() {
    let (mut a, dir) = app("picture-size");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    // The message as wide as the window.
    a.session.prefs.reading_pane = mailcraft_engine::ReadingPane::Off;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    // A screenshot 1600 wide, pasted.
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(1600, 800)).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    assert_eq!(own_size(&png), Some((1600, 800)));
    let size = own_size(&png);
    let r = a.session.run(
        "compose.insertPicture",
        &json!({"name": "pasted.png", "data": png, "at": 0, "width": crate::rich_edit::fitted_width(size), "line": crate::rich_edit::own_line(size)}),
    );
    a.place_picture(r);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let (width, rect) = picture(&a);
    assert_eq!(width, Some(FIT_WIDTH as u16), "a page's width");
    assert!((rect.width() - FIT_WIDTH as f32).abs() < 1.0 && (rect.height() - FIT_WIDTH as f32 / 2.0).abs() < 1.0, "shown that big: {rect:?}");
    // Clicked: selected.
    pointer(&ctx, &mut a, vec![Event::PointerMoved(rect.center()), press(rect.center(), true), press(rect.center(), false)]);
    let at = a.ui.editor.as_ref().unwrap().placed[0].0;
    assert_eq!(a.ui.editor.as_ref().unwrap().sel, at..at + 1, "the picture selected");
    // Its corner dragged 300 to the left: 300 narrower, its shape kept.
    let corner = rect.right_bottom() - egui::vec2(4.0, 4.0);
    let to = corner - egui::vec2(300.0, 0.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(corner)]);
    frame(&ctx, &mut a, vec![press(corner, true)]);
    for k in 1..=10 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(corner - egui::vec2(30.0 * k as f32, 0.0))]);
    }
    pointer(&ctx, &mut a, vec![press(to, false)]);
    let (width, rect) = picture(&a);
    assert!(width.is_some_and(|w| (i32::from(w) - (FIT_WIDTH as i32 - 300)).abs() <= 2), "{width:?}");
    assert!((rect.height() - rect.width() / 2.0).abs() < 1.0, "its shape kept: {rect:?}");
    // Undo: back to a page's width.
    let body = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(picture(&a).0, Some(FIT_WIDTH as u16), "undone");
    // Sent at the width it shows.
    let html = a.session.compose.as_ref().unwrap().bodies().1.unwrap();
    assert!(html.contains(&format!("width=\"{FIT_WIDTH}\"")), "{html}");
    let _ = std::fs::remove_dir_all(dir);
}
