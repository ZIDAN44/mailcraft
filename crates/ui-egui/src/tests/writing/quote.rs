use super::*;

/// The owner's report: a reply showed the original's columns as broken lines of text, then as
/// text one column under another. The sample newsletter (a band, two columns, a label over its
/// poster) is quoted as it came under the text, in the reading pane and in a window of its own,
/// in both themes, and typed into where it stands: its blocks of text are text boxes, the main
/// text keeps the keyboard's typing when it has the cursor, and the message stays one being
/// written while it's drawn.
#[test]
fn a_reply_shows_the_original_as_it_came_and_changes_it_in_place() {
    let (mut a, _) = app("quote-shown");
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
}

/// Issue #118: in a reply with its original shown as it came, Ctrl+A
/// stopped at a paragraph, selecting the text being written or the block of the original with
/// the cursor, alone. From either, it selects the text and all of the original, as Outlook does:
/// Copy takes all of it, Delete or typing takes all of it away, the original too, and Undo
/// brings it back in one step; a key that moves the cursor ends it.
#[test]
fn ctrl_a_selects_the_text_and_all_of_the_original() {
    let (mut a, _) = app("quote-select-all");
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let open = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.to_id()).expect("the sample newsletter");
    a.run("list.open", json!({"key": open}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    let body = crate::rich_edit::body_id(c);
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Thanks".into())]);
    let c = a.session.compose.clone().unwrap();
    let q = c.quote.clone().expect("its original as it came");
    let blocks = q.cell_texts();
    assert!(blocks.len() > 3, "{} blocks", blocks.len());
    // Everything selected: the text and every block of the original.
    let all_selected = |a: &MailApp| {
        let c = a.session.compose.as_ref().unwrap();
        let text = a.ui.editor.as_ref().is_some_and(|ed| ed.sel == (0..c.body.chars().count()));
        let original =
            blocks.iter().enumerate().all(|(n, (t, _))| a.ui.quote_editors.get(&(c.id, n)).is_some_and(|ed| ed.sel == (0..t.chars().count())));
        text && original
    };
    // From a block of the original.
    let n = (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.starts_with("Dear artists"))).expect("its first words' block");
    let cell = mailcraft_engine::Compose { id: crate::reading::cell_id(c.id, n), ..Default::default() };
    ctx.memory_mut(|m| m.request_focus(crate::rich_edit::body_id(&cell)));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(all_selected(&a), "all of it, from a block of the original");
    // Copy takes all of it, as the message's plain text has it. Ctrl comes first, as a key of
    // its own: it keeps all of it selected.
    frame(&ctx, &mut a, vec![key(Key::ControlLeft, Modifiers::COMMAND)]);
    let size = crate::tests::windows::layout::SIZE.with(std::cell::Cell::get);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events: vec![Event::Copy], ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let copied: Vec<String> = out
        .platform_output
        .commands
        .into_iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(copied.len(), 1, "{copied:?}");
    assert!(copied[0].starts_with("Thanks") && copied[0].contains(&q.text()), "{:?}", copied[0]);
    assert!(all_selected(&a), "still selected once copied");
    // Delete takes all of it away, the original too; Undo brings it back, Redo takes it again.
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    let now = a.session.compose.as_ref().unwrap();
    assert!(now.body.is_empty() && now.quote.is_none(), "{:?}", now.body);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    let now = a.session.compose.as_ref().unwrap();
    assert_eq!((&now.body, &now.quote), (&c.body, &c.quote), "back as it was");
    let unselected = |a: &MailApp| a.ui.quote_editors.iter().filter(|((of, _), _)| *of == c.id).all(|(_, ed)| ed.sel.is_empty());
    assert!(unselected(&a), "back with nothing in it selected, the block that had the cursor too");
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    let now = a.session.compose.as_ref().unwrap();
    assert!(now.body.is_empty() && now.quote.is_none(), "taken again: {:?}", now.body);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    // From the text: a key that moves the cursor lets the original go.
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(all_selected(&a), "all of it, from the text");
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(unselected(&a), "the original no longer selected");
    // Typed over, all of it goes for what's typed, and comes back with one Undo.
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("New".into())]);
    frame(&ctx, &mut a, vec![Event::Text(" text".into())]);
    let now = a.session.compose.as_ref().unwrap();
    assert!(now.body == "New text" && now.quote.is_none(), "{:?}", now.body);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    let now = a.session.compose.as_ref().unwrap();
    assert_eq!((&now.body, &now.quote), (&c.body, &c.quote), "back as it was");
}

/// A draft in Drafts says it isn't sent yet, and opens again to carry on writing it, as it was
/// (its original as it came under it): from its Edit button or a double-click in the list.
#[test]
fn a_draft_opens_again_from_drafts() {
    let (mut a, _) = app("draft-again");
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
}

/// Issue #74: a message with a logo from the web, centred over its text, forwarded. The logo the
/// reader was shown is shown in the forward too, where it was an empty box, and it stays in the
/// middle. A picture the reader wasn't shown stays unloaded: nothing is fetched for the editor.
#[test]
fn a_forward_shows_the_web_picture_the_reader_was_shown_centred() {
    use egui::{Pos2, Rect, Shape, vec2};
    let (mut a, _) = app("forward-web-picture");
    a.session.prefs.reply_window = false;
    let url = "https://pictures.example/logo.png";
    let html = format!(
        "<html><body><p style=\"text-align:center\"><img src=\"{url}\" width=\"160\" height=\"50\" alt=\"Logo\"></p><p style=\"text-align:center\"><b>Confirmation</b></p><p>Dear John,</p></body></html>"
    );
    let store = a.session.store.clone().unwrap();
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 1 };
    let raw = format!(
        "From: Trips <trips@x.example>\r\nSubject: Confirmation\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
    );
    store.write_body(&key, raw.as_bytes()).unwrap();
    archive_of(&mut a, vec![Summary { key: key.clone(), subject: "Confirmation".into(), date: NOW - 3600, ..Default::default() }]);
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
    let png = crate::tests::pictures::png(160, 50);
    a.session.pictures.insert(url.to_string(), mailcraft_engine::Picture::Ready(std::sync::Arc::new(png)));
    paint(&mut a);
    let shown = paint(&mut a);
    assert_eq!(shown.len(), 1, "the logo is drawn: {shown:?}");
    let logo = shown[0];
    assert!((logo.width() - 160.0).abs() < 1.0 && (logo.height() - 50.0).abs() < 1.0, "at its size: {logo:?}");
    // In the middle of the pane it's in (which starts right of the folders and the list).
    let pane = Rect::from_min_max(Pos2::new(620.0, 0.0), Pos2::new(1280.0, 800.0));
    assert!((logo.center().x - pane.center().x).abs() < 40.0, "centred: {logo:?} in {pane:?}");
}
