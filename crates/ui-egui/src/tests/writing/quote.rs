use super::*;

/// The owner's report: a reply showed the original's columns as broken lines of text, then as
/// text one column under another. The sample newsletter (a band, two columns, a label over its
/// poster) is quoted as it came under the text, in the reading pane and in a window of its own,
/// in both themes, and typed into where it stands: its blocks of text are text boxes, the main
/// text keeps the keyboard's typing when it has the cursor, and the message stays one being
/// written while it's drawn.
#[test]
fn a_reply_shows_the_original_as_it_came_and_changes_it_in_place() {
    let (mut a, dir) = app("quote-shown");
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    let c = a.session.compose.as_ref().expect("still being written after it was drawn");
    let q = c.quote.clone().expect("kept as it came");
    assert!(!q.parsed.layout.is_empty());
    assert!(q.edits.is_empty(), "drawing it changes nothing in it");
    // Typed at the top, the text takes it; the quote stays as it came.
    let id = crate::rich_edit::body_id(c);
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Thanks".into())]);
    let c = a.session.compose.as_ref().unwrap();
    assert!(c.body.contains("Thanks") && c.quote.as_ref().is_some_and(|q| q.edits.is_empty()), "{}", c.body);
    // Typed into a block of the original: that block changes, where it stands.
    let n = (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.starts_with("Dear artists"))).expect("its first words' block");
    let cell = mailcraft_engine::Compose { id: crate::reading::cell_id(c.id, n), ..Default::default() };
    ctx.memory_mut(|m| m.request_focus(crate::rich_edit::body_id(&cell)));
    frame(&ctx, &mut a, vec![]);
    // To its start the way every system has: Ctrl+Home is Windows' and Linux's, and macOS reads
    // Ctrl with a key as Emacs's; Command+Up (Ctrl+Up elsewhere) is the start on all three.
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::COMMAND), Event::Text("Hello! ".into())]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    let q = c.quote.clone().unwrap();
    let (text, _) = q.cell(n).unwrap();
    assert!(text.starts_with("Hello! Dear artists"), "{text}");
    assert_eq!(q.edits.len(), 1, "that block alone");
    assert!(!c.body.contains("Hello!"), "not in the text: {}", c.body);
    assert_eq!(a.ui.quote_focus, Some((c.id, n)), "the ribbon works on it");
    let html = c.bodies().1.unwrap_or_default();
    assert!(html.contains("Hello! Dear artists") && html.contains("<table"), "{html}");
    // In a window of its own too.
    a.run("compose.popOut", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.quote.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

/// A draft in Drafts says it isn't sent yet, and opens again to carry on writing it, as it was
/// (its original as it came under it): from its Edit button or a double-click in the list.
#[test]
fn a_draft_opens_again_from_drafts() {
    let (mut a, dir) = app("draft-again");
    a.session.prefs.reply_window = false;
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    a.run("compose.saveDraft", json!({}));
    a.session.compose = None;
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    let key = a.session.list.iter().find(|m| m.subject == "Re: Open call: winter group show").map(|m| m.key.clone()).expect("in Drafts");
    a.run("list.open", json!({"key": key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    // Its Edit button.
    a.act(crate::Action::Run("draft.edit", json!({"key": key.to_id()})));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().expect("being written again");
    assert_eq!(c.draft.as_ref(), Some(&key));
    assert!(c.quote.as_ref().is_some_and(|q| !q.parsed.layout.is_empty()), "its original as it came");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #74: a message with a logo from the web, centred over its text, forwarded. The logo the
/// reader was shown is shown in the forward too, where it was an empty box, and it stays in the
/// middle. A picture the reader wasn't shown stays unloaded: nothing is fetched for the editor.
#[test]
fn a_forward_shows_the_web_picture_the_reader_was_shown_centred() {
    use egui::{Pos2, Rect, Shape, vec2};
    let (mut a, dir) = app("forward-web-picture");
    a.session.prefs.reply_window = false;
    let url = "https://pictures.example/logo.png";
    let html = format!(
        "<html><body><p style=\"text-align:center\"><img src=\"{url}\" width=\"160\" height=\"50\" alt=\"Logo\"></p><p style=\"text-align:center\"><b>Confirmation</b></p><p>Dear John,</p></body></html>"
    );
    let store = a.session.store.clone().unwrap();
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 1 };
    let messages = vec![Summary { key: key.clone(), subject: "Confirmation".into(), date: NOW - 3600, ..Default::default() }];
    let state = FolderState { validity: 1, uid_next: 2, highest_modseq: 0, messages: messages.into_iter().map(Into::into).collect(), window: 0 };
    store.save_folder("sample", "Archive", &state).unwrap();
    let raw = format!(
        "From: Trips <trips@x.example>\r\nSubject: Confirmation\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
    );
    store.write_body(&key, raw.as_bytes()).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": key.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.run("mail.forward", json!({}));
    // The pictures a frame paints (not the fonts' texture), and where "Dear John" is.
    let paint = |a: &mut MailApp| -> Vec<Rect> {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        fn walk(s: &Shape, pictures: &mut Vec<Rect>) {
            match s {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, pictures)),
                Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => pictures.push(m.calc_bounds()),
                _ => {}
            }
        }
        let mut pictures = Vec::new();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut pictures));
        pictures
    };
    paint(&mut a);
    assert!(paint(&mut a).is_empty(), "not shown to the reader: not shown here");
    let mut png = Vec::new();
    let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(160, 50, image::Rgba([40, 120, 200, 255])));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    a.session.pictures.insert(url.to_string(), mailcraft_engine::Picture::Ready(std::sync::Arc::new(png)));
    paint(&mut a);
    let shown = paint(&mut a);
    assert_eq!(shown.len(), 1, "the logo is drawn: {shown:?}");
    let logo = shown[0];
    assert!((logo.width() - 160.0).abs() < 1.0 && (logo.height() - 50.0).abs() < 1.0, "at its size: {logo:?}");
    // In the middle of the pane it's in (which starts right of the folders and the list).
    let pane = Rect::from_min_max(Pos2::new(620.0, 0.0), Pos2::new(1280.0, 800.0));
    assert!((logo.center().x - pane.center().x).abs() < 40.0, "centred: {logo:?} in {pane:?}");
    let _ = std::fs::remove_dir_all(dir);
}
