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
    let (mut a, dir) = app("picture-size-shown");
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

/// Services whose clipboard holds a picture.
struct Clip(Vec<u8>);

impl crate::Services for Clip {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

/// Issue #11: the signature editor has a bar of its own and takes pictures; saved, the
/// signature keeps its picture, and a new message has it.
#[test]
fn a_picture_goes_into_the_signature() {
    let (mut a, dir) = app("signature-picture-pasted");
    let ctx = egui::Context::default();
    a.services = Box::new(Clip(mailcraft_engine::sample::sample_png()));
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::SignaturePicture { paste: true });
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_ref() else { panic!("the editor is open") };
    assert_eq!(f.compose.inline.len(), 1);
    assert!(f.compose.body.contains(mailcraft_engine::mime::rich::PICTURE));
    // Saved (its Save button is the dialog's first).
    let Some(Dialog::Signature(f)) = a.ui.dialog.take() else { panic!() };
    let text = f.compose.body.trim_end().to_string();
    let html = mailcraft_engine::mime::embed_pictures(&mailcraft_engine::mime::rich::to_html(&text, &f.compose.styles), &f.compose.inline);
    a.run("account.setSignature", json!({"account": "sample", "signature": text, "html": html}));
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    assert_eq!(c.inline.len(), 1, "the new message has the signature's picture");
    assert!(c.bodies().1.is_some_and(|h| h.contains("cid:signature1.")));
    // Too big for a signature: said, and left out.
    a.ui.dialog = Some(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))));
    a.services = Box::new(Clip(vec![0; mailcraft_engine::mime::SIGNATURE_PICTURE_MAX + 1]));
    a.act(crate::Action::SignaturePicture { paste: true });
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_ref() else { panic!() };
    assert_eq!(f.compose.inline.len(), 1, "only the one it had");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #11: a signature copied from Word or Outlook with its logo, pasted into the signature
/// editor or a message, brings the logo along (only the text and its look came), at its size.
#[test]
fn a_logo_pasted_with_a_signature_comes_along() {
    let (mut a, dir) = app("pasted-logo");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    // The picture written into the HTML, as Word writes it (`data:`).
    let logo = mailcraft_engine::mime::InlinePicture {
        cid: "x".into(),
        name: "x.png".into(),
        mime: "image/png".into(),
        data: mailcraft_engine::sample::sample_png(),
    };
    let png = mailcraft_engine::mime::embed_pictures("src=\"cid:x\"", &[logo]);
    let html = format!("<p class=MsoNormal><![if !vml]><img width=120 height=36 {png}><![endif]></p><p class=MsoNormal><b>Riley Moreno</b></p>");
    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = Some(html));
    frame(&ctx, &mut a, vec![]);
    // Into the signature editor.
    a.ui.dialog = Some(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!("the editor is open") };
    f.compose.body.clear();
    f.compose.styles = mailcraft_engine::mime::rich::Styles::default();
    let id = crate::rich_edit::body_id(&f.compose);
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Paste("Riley Moreno".into())]);
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_ref() else { panic!() };
    assert_eq!(f.compose.body, "\u{FFFC}\nRiley Moreno");
    assert_eq!(f.compose.inline.len(), 1, "the logo with it");
    let st = f.compose.styles.style(0);
    assert_eq!((st.picture.as_deref(), st.picture_width), (Some(f.compose.inline[0].cid.as_str()), Some(120)));
    assert!(f.compose.styles.style(3).bold);
    // Into a message: the same, and sent with it.
    a.ui.dialog = None;
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Paste("Riley Moreno".into())]);
    let c = a.session.compose.as_ref().unwrap();
    let cids: Vec<&str> = c.inline.iter().map(|p| p.cid.as_str()).collect();
    assert!(cids.iter().any(|cid| c.bodies().1.is_some_and(|h| h.contains(&format!("cid:{cid}")))), "{cids:?}");
    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(dir);
}
