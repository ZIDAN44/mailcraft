//! UI tests: writing messages (the editor, recipients, the address book, date and words, several at once).

use super::*;

#[test]
fn recipient_boxes_suggest_people() {
    let (mut a, dir) = app("suggest");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "zo");
    assert_eq!(a.ui.suggest_field, Some("to"));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>, ");
    // Picked with the mouse: the box keeps the cursor after the click, so the next address is
    // typed straight in.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![]);
    let list =
        ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)));
    let row = list.map(|r| r.left_top() + egui::vec2(20.0, 16.0)).expect("the suggestions show");
    let button = |pressed| Event::PointerButton { pos: row, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(row)]);
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    let to = a.session.compose.as_ref().unwrap().to.clone();
    assert!(to.starts_with("Zoe Park <zoe@studio.example>, ") && to.ends_with(">, x") && to.matches('@').count() == 2, "{to}");
    a.session.compose.as_mut().unwrap().to = "Zoe Park <zoe@studio.example>, ".into();

    // The next address starts fresh; Escape hides the list.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.compose.as_ref().unwrap().to.ends_with(", sh"), "{}", a.session.compose.as_ref().unwrap().to);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_editor_formats_as_you_see_it_with_outlooks_keys() {
    use mailcraft_engine::mime::rich::{Align, List, PICTURE};
    let (mut a, dir) = app("editor");
    a.services = Box::new(Recorder(Default::default()));
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": ""}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    let compose_id = a.session.compose.as_ref().unwrap().id;
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let c = |a: &MailApp| a.session.compose.clone().unwrap();
    // Bold on, then typing: bold text, and no marks in it.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("Hello".into())]);
    assert_eq!(c(&a).body, "Hello");
    assert!(c(&a).styles.style(0).bold && c(&a).styles.style(4).bold);
    // Off again: plain.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text(" world".into())]);
    assert_eq!(c(&a).body, "Hello world");
    assert!(!c(&a).styles.style(7).bold);
    // Select "Hello" and underline it.
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::U, Modifiers::COMMAND)]);
    assert!(c(&a).styles.style(2).underline && !c(&a).styles.style(7).underline);
    // Bullets (Ctrl+Shift+L), then undo (Ctrl+Z) and redo (Ctrl+Y).
    frame(&ctx, &mut a, vec![key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::None);
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    // Ctrl+R aligns right while writing (it doesn't reply), Ctrl+E centres.
    frame(&ctx, &mut a, vec![key(Key::R, Modifiers::COMMAND)]);
    assert_eq!(c(&a).id, compose_id, "still the same message");
    assert_eq!(c(&a).styles.para(0).align, Align::Right);
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).align, Align::Center);
    // Bigger (Ctrl+]), on "Hello" selected again (undo leaves the cursor collapsed).
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::CloseBracket, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.style(0).size(), 12);
    // Sent as HTML with what was seen.
    let (text, html, _) = c(&a).bodies();
    let html = html.unwrap();
    assert!(text.starts_with("• Hello world"), "{text}");
    assert!(html.contains("<ul>") && html.contains("<u>Hello</u>") && html.contains("text-align:center"), "{html}");
    // Ctrl+Shift+B: the address book.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND | Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.ui.dialog, Some(Dialog::SelectNames(_))));
    a.ui.dialog = None;
    // A pasted picture goes in at the cursor.
    a.act(crate::Action::PastePicture);
    frame(&ctx, &mut a, vec![]);
    assert!(c(&a).body.contains(PICTURE) && c(&a).inline.len() == 1);
    a.run("compose.format", json!({"plain": true}));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_address_book_adds_people_to_to_cc_and_bcc() {
    let (mut a, dir) = app("names");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.list.iter().any(|x| x.email == "zoe@studio.example"), "everyone mail was exchanged with: {:?}", f.list);
    f.query = "zoe".into();
    f.stale = true;
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(!f.list.is_empty() && f.list.iter().all(|x| x.email.contains("zoe") || x.name.to_lowercase().contains("zoe")));
    f.to = "Zoe Park <zoe@studio.example>; ".into();
    f.cc = "ann@x.example".into();
    // OK: the fields go to the message.
    let p = json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": ""});
    a.act(crate::Action::Submit("compose.update", p));
    let c = a.session.compose.clone().unwrap();
    assert!(c.to.contains("zoe@studio.example") && c.cc == "ann@x.example");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn delay_delivery_and_replies_to_draw_while_writing() {
    let (mut a, dir) = app("compose-delay");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // Delay Delivery shows for a server account (the Outbox sends it later), with its time.
    let id = a.session.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.prefs.new_mail_window = false;
    a.run("mail.new", json!({}));
    a.run(
        "compose.update",
        json!({"account": id, "to": "ann@example.org", "subject": "Later", "replyTo": "desk@example.org", "sendAfter": NOW + 3600}),
    );
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.compose.as_ref().is_some_and(|c| c.send_after == Some(NOW + 3600) && c.reply_to == "desk@example.org"));
    assert_eq!(crate::reading::parse_local("2026-10-16", "08:30"), Some((20_742, 510)));
    assert_eq!(crate::reading::parse_local("2026-02-30", "08:30"), None);
    assert_eq!(crate::reading::parse_local("2026-10-16", "24:00"), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Writing: today's date or the time goes in at the cursor (a plain message stays plain, and
/// Undo takes it out); Word count counts the message, or what's selected.
#[test]
fn the_date_goes_in_and_words_are_counted() {
    let (mut a, dir) = app("date-words");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let choices = crate::rich_edit::date_choices(&a.session.clock);
    assert_eq!(choices.len(), 4);
    let iso = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400));
    assert_eq!(choices[1], iso);
    assert!(choices[0].contains(&mailcraft_engine::model::dates::civil(NOW, &a.session.clock).year.to_string()));
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_mut().unwrap();
    c.body = "Hello  world.\n\nSecond paragraph here".into();
    c.plain = true;
    let mut ed = crate::rich_edit::Editor::new(c.id);
    ed.sel = 5..5;
    crate::rich_edit::apply(&mut ed, c, crate::rich_edit::Op::Insert(format!(" {iso}")));
    assert_eq!(c.body, format!("Hello {iso}  world.\n\nSecond paragraph here"));
    assert!(c.plain, "a plain message stays plain");
    assert_eq!(ed.sel, 5 + 1 + iso.chars().count()..5 + 1 + iso.chars().count());
    // Past the end, or a selection replaced.
    ed.sel = 10_000..10_000;
    crate::rich_edit::apply(&mut ed, c, crate::rich_edit::Op::Insert("!".into()));
    assert!(c.body.ends_with('!'));
    let n = crate::rich_edit::count_words(&c.body);
    assert_eq!((n.words, n.paragraphs, n.lines), (6, 2, 3), "{n:?}");
    assert_eq!(crate::rich_edit::count_words(""), crate::rich_edit::Counts::default());
    let w = crate::rich_edit::count_words("naïve café — 東京 2026 don't");
    assert_eq!((w.words, w.chars_no_spaces), (6, "naïvecafé—東京2026don't".chars().count()), "{w:?}");
    // Shown where the message is being written (its own window), as Address Book is.
    assert!(Dialog::WordCount(n, false).for_compose());
    a.ui.dialog = Some(Dialog::WordCount(n, false));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Several messages written at once, each in its own window: a second New mail keeps the first;
/// what a window does goes to its own message; each keeps its editor (selection, undo); a
/// dialog opened from one shows in that one.
#[test]
fn several_messages_are_written_at_once() {
    let (mut a, dir) = app("several-composes");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "First", "to": "ana@soundlab.example"}));
    let first = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.ui.editor.as_mut().unwrap().sel = 3..3;
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let second = a.session.compose.as_ref().unwrap().id;
    assert_ne!(first, second);
    assert_eq!(a.session.all_composes().count(), 2, "the first is kept");
    // Its window's action goes to it, not to the one acted on last.
    a.act(crate::Action::ForCompose(first, Box::new(crate::Action::Run("compose.update", json!({"subject": "First, changed"})))));
    let subjects: Vec<String> = a.session.all_composes().map(|c| c.subject.clone()).collect();
    assert!(subjects.contains(&"First, changed".to_string()) && subjects.contains(&String::new()), "{subjects:?}");
    // Its editor came back with it.
    assert_eq!(a.ui.editor.as_ref().map(|e| (e.id, e.sel.clone())), Some((first, 3..3)));
    // A dialog from the first's window belongs to it.
    a.act(crate::Action::ForCompose(first, Box::new(crate::Action::Dialog(Dialog::SaveChanges))));
    assert_eq!(a.ui.dialog_for, Some(first));
    a.ui.dialog = None;
    // Closing one leaves the other.
    a.act(crate::Action::ForCompose(second, Box::new(crate::Action::Run("compose.close", json!({"save": false})))));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.all_composes().map(|c| c.id).collect::<Vec<_>>(), vec![first]);
    // An action for a message gone does nothing.
    a.act(crate::Action::ForCompose(second, Box::new(crate::Action::Run("compose.update", json!({"subject": "ghost"})))));
    assert_eq!(a.session.compose.as_ref().unwrap().subject, "First, changed");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Closing MailCraft with messages being written asks first; Save keeps each as a draft.
#[test]
fn closing_with_messages_written_asks_and_keeps_them() {
    let (mut a, dir) = app("save-on-close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Also half done"}));
    let close = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(close, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(2))), "asked");
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.keepAll", json!({})));
    let drafts = a.session.account("sample").and_then(|x| x.folder(mailcraft_engine::model::FolderRole::Drafts)).map(|f| f.path.clone()).unwrap();
    let subjects: Vec<String> = a.session.folder_state("sample", &drafts).messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Half done".into()) && subjects.contains(&"Also half done".into()), "{subjects:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A file dragged from the desktop onto a message being written is attached to it.
#[test]
fn files_dropped_on_a_message_are_attached() {
    use std::path::{Path, PathBuf};
    /// What the desktop hands over for one file let go over the window.
    #[derive(Debug)]
    struct LetGo {
        from: PathBuf,
    }
    impl egui::DroppedFile for LetGo {
        fn bytes(&self) -> Result<Vec<u8>, String> {
            let read = std::fs::read(&self.from);
            read.map_err(|why| format!("{}: {why}", self.from.display()))
        }

        fn path(&self) -> &Path {
            self.from.as_path()
        }
    }
    let (mut a, dir) = app("drop-attach");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let file = dir.join("dropped notes.txt");
    std::fs::write(&file, b"hello").unwrap();
    let dropped: Vec<egui::DroppedFileHandle> =
        vec![std::sync::Arc::new(LetGo { from: file }), std::sync::Arc::new(LetGo { from: "relative.txt".into() })];
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        dropped_files: dropped,
        ..Default::default()
    };
    // The message is in a window of its own, or in the main one where there are none: either
    // way the drop reaches it.
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    let names: Vec<String> = a.session.all_composes().flat_map(|c| c.attachments.iter().map(|x| x.name.clone())).collect();
    assert_eq!(names, ["dropped notes.txt"], "the file with no path to read is left");
    let _ = std::fs::remove_dir_all(dir);
}
