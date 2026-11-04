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
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::CTRL), Event::Text("Hello! ".into())]);
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
