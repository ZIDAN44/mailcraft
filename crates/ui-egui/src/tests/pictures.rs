//! Pictures in a message being written: their size, and making them bigger or smaller (issue #10);
//! and those of a message read: its files taken out once, decoded once each, within a budget.

use super::*;
use crate::rich_edit::{FIT_WIDTH, own_size};

mod bodies;

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

/// A PDF of `pages` pages (100 x 50 points), each with a box in it.
fn pdf(pages: usize) -> Vec<u8> {
    let content = "0 0 1 rg 10 10 40 30 re f";
    let kids: Vec<String> = (0..pages).map(|k| format!("{} 0 R", 3 + 2 * k)).collect();
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" "))];
    for k in 0..pages {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents {} 0 R >>", 4 + 2 * k));
        objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba([40, 90, 200, 255])))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A message with these files put first in the Inbox, and the Inbox shown.
fn with_files(a: &mut MailApp, uid: u32, files: Vec<(String, &str, Vec<u8>)>) -> MessageKey {
    use mailcraft_engine::mime::{self, Draft, OutAttachment};
    let d = Draft {
        subject: format!("Files {uid}"),
        body: "Here they are.".into(),
        attachments: files.into_iter().map(|(name, mime, data)| OutAttachment { name, mime: mime.into(), data }).collect(),
        ..Default::default()
    };
    let raw = mime::build(&d, &mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example"), NOW, &format!("<files{uid}@x>")).unwrap();
    with_raw(a, uid, &raw)
}

/// A message of this source put first in the Inbox, and the Inbox shown.
fn with_raw(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    use mailcraft_engine::mime;
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mime::summary_from_full(raw, key.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    key
}

fn pictures(n: usize) -> Vec<(String, &'static str, Vec<u8>)> {
    (0..n).map(|i| (format!("photo {i}.png"), "image/png", png(8 + (i % 5) as u32, 8))).collect()
}

/// Frames in a window this big until `done` (at most `max`); how many it took, and the most
/// pictures decoded in one of them.
fn pump(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2, max: usize, done: impl Fn(&MailApp) -> bool) -> (usize, usize) {
    let mut most = 0;
    for n in 0..max {
        if done(a) {
            return (n, most);
        }
        let before = a.ui.pictures.counts().1;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        most = most.max(a.ui.pictures.counts().1 - before);
    }
    (max, most)
}

const WINDOW: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A message with more than 64 pictures cleared every thumbnail each time one more was made,
/// so it read, parsed and decoded all of them again every frame: each is decoded once, from
/// one read of the message, a picture a frame (none of it in one long frame).
#[test]
fn seventy_pictures_decode_once_each() {
    let (mut a, dir) = app("seventy-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9001, pictures(70));
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, most) = pump(&ctx, &mut a, WINDOW, 80, |a| a.ui.pictures.ready() == 70);
    assert!(frames < 80, "all 70 thumbnails in {frames} frames ({} ready)", a.ui.pictures.ready());
    assert_eq!(most, 1, "a picture a frame");
    assert_eq!(a.ui.pictures.counts(), (1, 70), "one read, each picture decoded once");
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (1, 70), "and not again");
    assert_eq!(a.ui.pictures.working(), 0);
    assert_eq!(a.ui.pictures.steps().1, 0, "no threads here: done in the window");
    let _ = std::fs::remove_dir_all(dir);
}

/// The 36 thumbnails, a picture opened big and a PDF's ten pages all come from one read of the
/// message (each was a read and a parse of the whole message, per picture and per page).
#[test]
fn one_extraction_per_message() {
    let (mut a, dir) = app("one-extraction");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut files = pictures(36);
    files.push(("report.pdf".into(), "application/pdf", pdf(10)));
    let k = with_files(&mut a, 9002, files);
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, most) = pump(&ctx, &mut a, WINDOW, 60, |a| a.ui.pictures.ready() == 37);
    assert!(frames < 60, "36 thumbnails and the PDF's first page ({} ready)", a.ui.pictures.ready());
    assert_eq!(most, 1);
    // A picture shown at the pane's size.
    a.ui.attachment_view = Some((k.clone(), 3));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 10, |a| a.ui.pictures.ready() == 38);
    assert!(frames < 10, "the picture big");
    // The PDF's pages, all on a tall screen.
    a.ui.attachment_view = Some((k.clone(), 36));
    let (frames, _) = pump(&ctx, &mut a, egui::vec2(1280.0, 8000.0), 30, |a| a.ui.pictures.ready() == 48);
    assert!(frames < 30, "its ten pages ({} ready)", a.ui.pictures.ready());
    assert_eq!(a.ui.pictures.pages(&k, 36), Some(10));
    assert_eq!(a.ui.pictures.counts(), (1, 48), "one read of the message for all of them");
    let _ = std::fs::remove_dir_all(dir);
}

/// A 60-page PDF draws the pages on screen only, and what it keeps stays within the budget
/// even when every page shows (those that don't fit come smaller, or not at all).
#[test]
fn a_long_pdf_stays_within_the_budget() {
    let (mut a, dir) = app("long-pdf");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9003, vec![("manual.pdf".into(), "application/pdf", pdf(60))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    pump(&ctx, &mut a, WINDOW, 40, |_| false);
    assert_eq!(a.ui.pictures.pages(&k, 0), Some(60));
    let (_, decodes) = a.ui.pictures.counts();
    assert!(decodes <= 6, "the chip and the pages on screen only: {decodes}");
    // Every page on screen, with room for a few.
    let budget = 1 << 20;
    a.ui.pictures.set_budget(budget);
    for _ in 0..150 {
        pump(&ctx, &mut a, egui::vec2(1280.0, 24_000.0), 1, |_| false);
        assert!(a.ui.pictures.bytes() <= budget, "{} kept", a.ui.pictures.bytes());
    }
    assert!(a.ui.pictures.ready() > 3, "the pages that fit ({})", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture bigger than a frame's uploads (2048 pixels square, 16 MiB) is still made, alone.
#[test]
fn a_single_big_upload() {
    let (mut a, dir) = app("big-upload");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9004, vec![("poster.png".into(), "image/png", png(2048, 2048))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() == 2);
    assert!(frames < 20, "the thumbnail and the picture big ({} ready)", a.ui.pictures.ready());
    assert!(a.ui.pictures.bytes() >= 16 << 20, "{}", a.ui.pictures.bytes());
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture that breaks the decoder fails, with the rest of its message's, and the next
/// message's pictures show.
#[test]
fn a_panicking_decode_leaves_later_work_alone() {
    let (mut a, dir) = app("panicking-decode");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let bad = with_files(&mut a, 9005, pictures(5));
    let good = with_files(&mut a, 9006, pictures(4));
    a.ui.pictures.panic_at = Some(2);
    a.run("list.open", json!({"key": bad.to_id()}));
    // (Once it started: the message was read.)
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.counts().0 == 1 && a.ui.pictures.working() == 0);
    assert!(frames < 20, "its work ends");
    assert_eq!(a.ui.pictures.ready(), 2, "the pictures before it");
    assert_eq!(a.ui.pictures.counts(), (1, 2), "and none after it decoded");
    a.ui.pictures.panic_at = None;
    a.run("list.open", json!({"key": good.to_id()}));
    let before = a.ui.pictures.ready();
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() >= before + 4);
    assert!(frames < 20, "the next message's four ({} ready)", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}

/// Moving on to another message stops the work on the last one's pictures.
#[test]
fn pictures_of_a_message_left_are_cancelled() {
    let (mut a, dir) = app("pictures-left");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let many = with_files(&mut a, 9007, pictures(70));
    let next = with_files(&mut a, 9008, pictures(2));
    a.run("list.open", json!({"key": many.to_id()}));
    pump(&ctx, &mut a, WINDOW, 5, |_| false);
    a.run("list.open", json!({"key": next.to_id()}));
    pump(&ctx, &mut a, WINDOW, 30, |_| false);
    let (reads, decodes) = a.ui.pictures.counts();
    assert_eq!(reads, 2);
    assert!(decodes < 10, "the first message's stopped: {decodes} decoded");
    assert_eq!(a.ui.pictures.working(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A window resized a pixel a frame with a PDF's pages shown: the pages are made again only
/// at each 100 pixels, what's kept for the message stays small, and the pages show at the
/// width it ends at (each width was a new page kept, until the message's 200 failed them all).
#[test]
fn resizing_keeps_a_pdf_showing() {
    let (mut a, dir) = app("pdf-resized");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9009, vec![("plan.pdf".into(), "application/pdf", pdf(3))]);
    a.run("list.open", json!({"key": k.to_id()}));
    a.ui.attachment_view = Some((k.clone(), 0));
    pump(&ctx, &mut a, egui::vec2(1000.0, 800.0), 20, |_| false);
    let settled = a.ui.pictures.counts().1;
    for w in 0..300 {
        pump(&ctx, &mut a, egui::vec2(1000.0 + w as f32, 800.0), 1, |_| false);
        assert!(a.ui.pictures.of(&k).0 <= 12, "pictures kept at {w}: {:?}", a.ui.pictures.of(&k));
    }
    pump(&ctx, &mut a, egui::vec2(900.0, 800.0), 20, |_| false);
    let decodes = a.ui.pictures.counts().1 - settled;
    assert!(decodes <= 20, "the pages made again at each 100 pixels only: {decodes}");
    let (kept, shown) = a.ui.pictures.of(&k);
    assert!(shown >= 3 && kept <= 12, "the three pages show at the last width: {shown} shown, {kept} kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Past 200 pictures the chips show their kind's icon, but one opened still shows.
#[test]
fn a_picture_past_the_chips_opens() {
    let (mut a, dir) = app("many-chips");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9010, pictures(205));
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 5, |_| false);
    a.ui.attachment_view = Some((k.clone(), 204));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert!(matches!(a.ui.pictures.get(&k, 204, crate::reading::Want::Big), crate::reading::Got::Ready(_)), "the last picture big");
    let _ = std::fs::remove_dir_all(dir);
}

/// Pictures left out of a read for room are read again, not failed: with room for two files
/// taken out, all six show.
#[test]
fn pictures_left_out_for_room_show() {
    let (mut a, dir) = app("sources-room");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let files = pictures(6);
    let most = files.iter().map(|f| f.2.len()).max().unwrap();
    let k = with_files(&mut a, 9011, files);
    a.ui.pictures.set_sources_max(2 * most);
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 40, |a| a.ui.pictures.ready() == 6);
    assert!(frames < 40, "all six ({} ready)", a.ui.pictures.ready());
    let (reads, decodes) = a.ui.pictures.counts();
    assert!(decodes == 6 && reads <= 3, "two at a time, each decoded once: {reads} reads, {decodes} decodes");
    let _ = std::fs::remove_dir_all(dir);
}

/// More web pictures shown at once than are kept: each is decoded once and kept while shown,
/// not decoded again every frame.
#[test]
fn many_web_pictures_are_kept_while_shown() {
    let (mut a, dir) = app("web-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let urls: Vec<String> = (0..250).map(|i| format!("https://cdn.shop.example/{i}.png")).collect();
    let imgs: String = urls.iter().map(|u| format!("<img src=\"{u}\" width=\"4\" height=\"4\">")).collect();
    let raw = format!("From: Shop <news@shop.example>\r\nSubject: Sale\r\nContent-Type: text/html\r\n\r\n<p>Sale</p>{imgs}");
    let k = with_raw(&mut a, 9012, raw.as_bytes());
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("message.showPictures", json!({}));
    for u in &urls {
        a.session.apply(MailEvent::Fetched { url: u.clone(), data: Some(png(4, 4)), error: None });
    }
    pump(&ctx, &mut a, egui::vec2(1280.0, 8000.0), 3, |_| false);
    assert_eq!(a.ui.web_pictures.len(), 250, "all of them kept while shown");
    let _ = std::fs::remove_dir_all(dir);
}

/// One read looks for 256 files at most: the pictures past them (an HTML message's 300) are
/// looked for by a later read, not taken as not in the message.
#[test]
fn more_pictures_than_one_read_takes_show() {
    use crate::reading::Want;
    let (mut a, dir) = app("past-one-read");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9013, pictures(300));
    let store = a.session.store.clone();
    for _ in 0..700 {
        if a.ui.pictures.ready() == 300 {
            break;
        }
        for i in 0..300 {
            let _ = a.ui.pictures.get(&k, i, Want::Body);
        }
        a.ui.pictures.end_frame(&ctx, store.as_ref(), None, &a.ui.fonts, &mut crate::NullServices(NOW));
    }
    assert_eq!(a.ui.pictures.ready(), 300, "all of them");
    assert_eq!(a.ui.pictures.counts(), (2, 300), "two reads, each picture decoded once");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture that can't be shown is tried again when its message is opened again, after its
/// work ended (it was only when the work was still running).
#[test]
fn a_failed_picture_is_tried_again_when_opened_again() {
    let (mut a, dir) = app("failed-again");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let broken = with_files(&mut a, 9014, vec![("broken.png".into(), "image/png", b"not a picture at all".to_vec())]);
    let other = with_files(&mut a, 9015, pictures(1));
    a.run("list.open", json!({"key": broken.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!((a.ui.pictures.counts(), a.ui.pictures.working()), ((1, 1), 0), "tried, and its work ended");
    a.run("list.open", json!({"key": other.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (2, 2));
    a.run("list.open", json!({"key": broken.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts(), (3, 3), "read and tried again");
    let _ = std::fs::remove_dir_all(dir);
}

/// A picture bigger than all the room there is lets go of no other: the last message's
/// thumbnails are kept (all of them went, for a picture then thrown away).
#[test]
fn a_picture_too_big_for_the_budget_evicts_nothing() {
    let (mut a, dir) = app("too-big");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let small = with_files(&mut a, 9016, pictures(3));
    let big = with_files(&mut a, 9017, vec![("poster.png".into(), "image/png", png(400, 400))]);
    a.run("list.open", json!({"key": small.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 20, |a| a.ui.pictures.ready() == 3);
    assert!(frames < 20, "the three thumbnails");
    // Room for the three, not for a thumbnail 160 pixels square.
    a.ui.pictures.set_budget(64 << 10);
    a.run("list.open", json!({"key": big.to_id()}));
    pump(&ctx, &mut a, WINDOW, 10, |_| false);
    assert_eq!(a.ui.pictures.counts().1, 4, "the big one decoded");
    assert_eq!(a.ui.pictures.ready(), 3, "and the three kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Services whose platform gives every video a frame.
struct Frames;

impl crate::Services for Frames {
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
    fn video_frame(&mut self, _: &std::path::Path) -> crate::VideoFrame {
        crate::VideoFrame::Ready(png(32, 18))
    }
}

/// More videos in a message than video thumbnails are kept: those past them show their kind's
/// icon, and no frame makes a texture once the others show (one past them was decoded again
/// every frame).
#[test]
fn videos_past_those_kept_make_no_texture_each_frame() {
    let (mut a, dir) = app("many-videos");
    let ctx = egui::Context::default();
    a.services = Box::new(Frames);
    frame(&ctx, &mut a, vec![]);
    let files = (0..70).map(|i| (format!("clip {i}.mp4"), "video/mp4", vec![i as u8; 64])).collect();
    let k = with_files(&mut a, 9018, files);
    a.run("list.open", json!({"key": k.to_id()}));
    let input = || RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW)), ..Default::default() };
    for _ in 0..10 {
        ctx.run_ui(input(), |ui| a.ui(ui)).textures_delta.clear();
    }
    assert_eq!(a.ui.video_thumbs.len(), 64);
    for _ in 0..5 {
        let out = ctx.run_ui(input(), |ui| a.ui(ui));
        assert!(out.textures_delta.set.is_empty(), "{} textures made", out.textures_delta.set.len());
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Services with threads for slow work: each piece on a thread of its own.
struct Threads;

impl crate::Services for Threads {
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
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        std::thread::spawn(work.run);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Services that keep the work handed to them, for the test to run when it says.
struct Held(std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>);

impl crate::Services for Held {
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
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        self.0.lock().unwrap().push(work);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Services that say there are threads, but give the work back (one couldn't be started).
struct Refused;

impl crate::Services for Refused {
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
    fn background(&mut self, _: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        Err(work)
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// Work a thread gives back is done in the window, a step a frame: every picture still shows.
#[test]
fn work_given_back_is_done_in_the_window() {
    let (mut a, dir) = app("pictures-refused");
    let ctx = egui::Context::default();
    a.services = Box::new(Refused);
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9020, pictures(12));
    a.run("list.open", json!({"key": k.to_id()}));
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 200, |a| a.ui.pictures.ready() == 12);
    assert!(frames < 200, "all shown ({} ready)", a.ui.pictures.ready());
    let (inline, wakes) = a.ui.pictures.steps();
    assert!(inline > 0, "stepped in the window");
    assert_eq!(wakes, 0, "nothing ran on a thread");
    let _ = std::fs::remove_dir_all(dir);
}

/// With a thread to work on, the 36 pictures are read and decoded there, none of it in a frame:
/// each comes once, from one read.
#[test]
fn pictures_are_decoded_on_a_thread() {
    let (mut a, dir) = app("pictures-thread");
    let ctx = egui::Context::default();
    a.services = Box::new(Threads);
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9013, pictures(36));
    a.run("list.open", json!({"key": k.to_id()}));
    let started = std::time::Instant::now();
    while a.ui.pictures.ready() < 36 && started.elapsed() < std::time::Duration::from_secs(10) {
        pump(&ctx, &mut a, WINDOW, 1, |_| false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(a.ui.pictures.ready(), 36);
    assert_eq!(a.ui.pictures.counts(), (1, 36), "one read, each picture decoded once");
    let (inline, wakes) = a.ui.pictures.steps();
    assert_eq!(inline, 0, "no step of it in a frame");
    assert_eq!(wakes, 36, "a frame asked for after each picture");
    let _ = std::fs::remove_dir_all(dir);
}

/// The work handed to a thread asks for a frame after each picture it makes (not once at its
/// end), a picture opened big while it waits goes first, and work for a message left stops.
#[test]
fn work_on_a_thread_asks_for_frames_and_stops_when_left() {
    let (mut a, dir) = app("pictures-held");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Held(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9014, pictures(6));
    let left = with_files(&mut a, 9015, pictures(70));
    // Opened and left: its work, handed over, does nothing when it runs.
    a.run("list.open", json!({"key": left.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    // The picture opened big, asked while the work waits.
    a.ui.attachment_view = Some((k.clone(), 4));
    pump(&ctx, &mut a, WINDOW, 1, |_| false);
    let work: Vec<crate::Work> = std::mem::take(&mut *held.lock().unwrap());
    assert_eq!(work.len(), 2, "one piece of work for each message");
    assert!(work[0].cancelled.load(std::sync::atomic::Ordering::Relaxed), "the one left is cancelled");
    assert_eq!(a.ui.pictures.counts(), (0, 0), "nothing done in the window");
    for w in work {
        (w.run)();
    }
    assert_eq!(a.ui.pictures.counts(), (1, 7), "the six thumbnails and the one big, from one read; none of the one left");
    assert_eq!(a.ui.pictures.waiting().first(), Some(&(4, crate::reading::Want::Big)), "the picture opened big first");
    assert_eq!(a.ui.pictures.steps(), (0, 7), "a frame asked for after each");
    let (frames, _) = pump(&ctx, &mut a, WINDOW, 5, |a| a.ui.pictures.ready() == 7);
    assert!(frames < 5, "all shown ({} ready)", a.ui.pictures.ready());
    let _ = std::fs::remove_dir_all(dir);
}
