//! UI tests: writing messages (the editor, recipients, the address book, date and words, several at once,
//! files from the desktop, signatures).

use super::*;

mod check_names;
mod files;
mod fonts;
mod form;
mod names;
mod quote;
mod sending;
mod signatures;
mod suggest;
mod undo_send;

#[test]
fn recipient_boxes_suggest_people() {
    let (mut a, _) = app("suggest");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "zo");
    assert_eq!(a.ui.suggest_field, Some(crate::dialogs::Recipients::To));
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
    click(&ctx, &mut a, row);
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    let to = a.session.compose.as_ref().unwrap().to.clone();
    assert!(to.starts_with("Zoe Park <zoe@studio.example>, ") && to.ends_with(">, x") && to.matches('@').count() == 2, "{to}");
    a.session.compose.as_mut().unwrap().to = "Zoe Park <zoe@studio.example>, ".into();

    // The next address starts fresh; Escape hides the list.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.compose.as_ref().unwrap().to.ends_with(", sh"), "{}", a.session.compose.as_ref().unwrap().to);
}

#[test]
fn the_editor_formats_as_you_see_it_with_outlooks_keys() {
    use mailcraft_engine::mime::rich::{Align, List, PICTURE};
    let (mut a, _) = app("editor");
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
}

#[test]
fn the_address_book_adds_people_to_to_cc_and_bcc() {
    let (mut a, _) = app("names");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
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
    click_named(&ctx, &mut a, "OK");
    assert!(a.ui.dialog.is_none(), "{:?}", a.ui.dialog);
    let c = a.session.compose.clone().unwrap();
    assert!(c.to.contains("zoe@studio.example") && c.cc == "ann@x.example");
}

/// A dialog's OK refused by the command is said, even by a dialog with no error line of its
/// own (Select Names), and with no dialog open at all.
#[test]
fn a_refused_dialog_submit_is_said() {
    let (mut a, _) = app("names-refused");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(&ctx, &mut a, vec![]);
    // The message went to be sent while the names were being picked: OK can't change it.
    if let Some(c) = a.session.compose.as_mut() {
        c.sending = true;
    }
    let before = a.session.notices.len();
    a.act(crate::Action::Submit("compose.update", json!({"to": "zoe@studio.example", "cc": "", "bcc": ""})));
    assert!(matches!(a.ui.dialog, Some(Dialog::SelectNames(_))), "the dialog stays, to be put right or cancelled");
    let said = a.session.notices.get(before).unwrap_or_else(|| panic!("nothing said: {:?}", a.session.notices));
    assert!(said.error && said.text.contains("being sent"), "{said:?}");
    // No dialog open: said all the same.
    a.ui.dialog = None;
    a.act(crate::Action::Submit("rules.set", json!({"rules": "not a list"})));
    assert_eq!(a.session.notices.len(), before + 2, "{:?}", a.session.notices);
    assert!(a.session.notices.last().is_some_and(|n| n.error));
    // A dialog with its own error line keeps showing it there, and only there.
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example"));
    a.act(crate::Action::Submit("rules.set", json!({"rules": "not a list"})));
    assert_eq!(a.session.notices.len(), before + 2, "{:?}", a.session.notices);
    let Some(Dialog::EditRule(f)) = &a.ui.dialog else { panic!("no dialog") };
    assert!(f.error.as_ref().is_some_and(|e| !e.starts_with("bad parameters")), "{:?}", f.error);
}

#[test]
fn delay_delivery_and_replies_to_draw_while_writing() {
    let (mut a, _) = app("compose-delay");
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
    // Gap 9: Delay delivery's time, typed on a 12-hour clock too.
    assert_eq!(crate::reading::parse_local("2026-10-16", "8:30 pm"), Some((20_742, 1230)));
    assert_eq!(crate::reading::parse_local("2026-10-16", "12 AM"), Some((20_742, 0)));
    assert_eq!(crate::reading::parse_local("2026-10-16", "13 PM"), None);
}

/// Writing: today's date or the time goes in at the cursor (a plain message stays plain, and
/// Undo takes it out); Word count counts the message, or what's selected.
#[test]
fn the_date_goes_in_and_words_are_counted() {
    let (mut a, _) = app("date-words");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let choices = crate::rich_edit::date_choices(&a.session.clock);
    assert_eq!(choices.len(), 4);
    let iso = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400));
    assert_eq!(choices[1], iso);
    assert!(choices[0].contains(&mailcraft_engine::model::dates::civil(NOW, &a.session.clock).year.to_string()));
    // As File › Options writes dates and times (issue #46).
    let mut clock = a.session.clock;
    clock.style.order = mailcraft_engine::model::dates::DateOrder::DayMonthYear;
    clock.style.hours12 = true;
    let styled = crate::rich_edit::date_choices(&clock);
    assert!(styled[1].matches('/').count() == 2 && (styled[3].ends_with(" AM") || styled[3].ends_with(" PM")), "{styled:?}");
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
}

/// Several messages written at once, each in its own window: a second New mail keeps the first;
/// what a window does goes to its own message; each keeps its editor (selection, undo); a
/// dialog opened from one shows in that one.
#[test]
fn several_messages_are_written_at_once() {
    let (mut a, _) = app("several-composes");
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
}

/// Closing MailCraft with messages being written asks first; Save keeps each as a draft.
#[test]
fn closing_with_messages_written_asks_and_keeps_them() {
    let (mut a, _) = app("save-on-close");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Also half done"}));
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(2))), "asked");
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.keepAll", json!({})));
    let drafts = a.session.account("sample").and_then(|x| x.folder(mailcraft_engine::model::FolderRole::Drafts)).map(|f| f.path.clone()).unwrap();
    let subjects: Vec<String> = a.session.folder_state("sample", &drafts).messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.contains(&"Half done".into()) && subjects.contains(&"Also half done".into()), "{subjects:?}");
}

/// A message closed with its draft saved is held, out of sight, while a server's save is on its
/// way (#53). MailCraft closing meanwhile keeps it on this computer, however it closes: with no
/// message open and nothing to ask, and through Don't save, which lets go of the one still open
/// only (found in review: the held ones were dropped with the pool's queue).
#[test]
fn closing_keeps_the_drafts_on_their_way_to_a_server() {
    let mut s = Session::open(Some(scratch_store(Store::open(scratch("held-drafts")).unwrap())), Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "ana@studio.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let jobs = Jobs::default();
    let mut a = MailApp::new(s, Box::new(jobs.clone()), Box::new(NullServices(NOW)));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let write = |a: &mut MailApp, subject: &str| {
        a.run("mail.new", json!({}));
        a.run("compose.update", json!({"account": id, "to": "ben@studio.test", "subject": subject}));
    };
    let kept = |jobs: &Jobs| -> Vec<String> {
        jobs.take()
            .into_iter()
            .filter_map(|j| match j {
                mailcraft_engine::model::Job::KeepDraft { raw, .. } => Some(String::from_utf8_lossy(&raw).into_owned()),
                _ => None,
            })
            .collect()
    };

    write(&mut a, "On its way");
    a.run("compose.close", json!({}));
    assert!(a.session.compose.is_none() && a.session.saving.len() == 1, "held");
    frame(&ctx, &mut a, vec![]);
    jobs.take();
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "nothing to ask");
    assert!(a.session.saving.is_empty());
    let raws = kept(&jobs);
    assert!(raws.len() == 1 && raws[0].contains("Subject: On its way"), "kept on this computer: {raws:?}");

    // Don't save: the message open goes, the one held is kept.
    write(&mut a, "Held too");
    a.run("compose.close", json!({}));
    write(&mut a, "Not wanted");
    frame(&ctx, &mut a, vec![]);
    jobs.take();
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked about the one open");
    a.act(crate::Action::CloseAnyway);
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    let raws = kept(&jobs);
    assert!(raws.len() == 1 && raws[0].contains("Subject: Held too"), "only the held one: {raws:?}");
}

/// Closing while a message is still being sent asks first: it isn't in the Outbox yet, and was
/// lost (found testing issue #19).
#[test]
fn closing_while_a_message_is_sent_asks() {
    let (mut a, _) = app("close-while-sending");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "On its way"}));
    if let Some(c) = a.session.compose.as_mut() {
        c.sending = true;
    }
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::LostOnClose { sending: 1, unsent: 0, changes: 0 })), "{:?}", a.ui.dialog);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
}

/// Closed from the taskbar while minimized, when eframe runs `logic` alone and no `ui`: the close
/// still waits for the question, and the window comes back to ask it (craft-libs' quit crate:
/// eframe closed the window, and the messages being written were lost).
#[test]
fn closing_while_minimized_still_asks() {
    let (mut a, _) = app("save-on-close-minimized");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Written while minimized"}));
    frame(&ctx, &mut a, vec![]);
    let mut close = RawInput { time: Some(99.0), ..close_input() };
    if let Some(viewport) = close.viewports.get_mut(&egui::ViewportId::ROOT) {
        viewport.minimized = Some(true);
    }
    let mut out = ctx.run_logic(&close, |ctx| a.logic(ctx));
    let commands = out.viewport_commands.remove(&egui::ViewportId::ROOT).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::CancelClose), "the close waits: {commands:?}");
    assert!(commands.contains(&egui::ViewportCommand::Minimized(false)), "the window comes back: {commands:?}");
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "asked");
    // The window back, the same close comes in again with the next pass (eframe keeps the
    // input logic alone saw): asked once, still.
    let mut out = ctx.run_ui(close, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))));
}

/// macOS quits through `terminate:` rather than closing the window: a close nothing holds open
/// is taken over (once, the window kept for the quit), one a question holds open isn't, and a
/// frame with no close leaves the quit alone. Elsewhere `quit` says no and eframe closes.
#[test]
fn a_close_becomes_the_platform_quit_unless_something_asks() {
    let ctx = egui::Context::default();
    // Nothing being written: the quit runs, and the window waits for it.
    let (mut a, _) = app("quit-takes-over");
    let quits = std::cell::Cell::new(0);
    let mut out = ctx.run_logic(&close_input(), |ctx| {
        a.logic(ctx);
        assert!(MailApp::quit_instead_of_close(ctx, || {
            quits.set(quits.get() + 1);
            true
        }));
    });
    assert_eq!(quits.get(), 1);
    assert!(out.viewport_commands.remove(&egui::ViewportId::ROOT).unwrap_or_default().contains(&egui::ViewportCommand::CancelClose));
    // A platform without a quit of its own: nothing taken over, eframe closes.
    let out = ctx.run_logic(&close_input(), |ctx| assert!(!MailApp::quit_instead_of_close(ctx, || false)));
    assert!(out.viewport_commands.get(&egui::ViewportId::ROOT).is_none_or(|c| !c.contains(&egui::ViewportCommand::CancelClose)));
    // No close asked for: no quit.
    let _ = ctx.run_logic(&RawInput::default(), |ctx| assert!(!MailApp::quit_instead_of_close(ctx, || panic!("no close, no quit"))));
    // A message being written: the question holds the window, and the quit waits for the answer.
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Not lost to Cmd+Q"}));
    frame(&ctx, &mut a, vec![]);
    let _ = ctx.run_logic(&RawInput { time: Some(50.0), ..close_input() }, |ctx| {
        a.logic(ctx);
        assert!(!MailApp::quit_instead_of_close(ctx, || panic!("asked first")));
    });
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))));
}

/// Issue #10: a picture in a message shows as big as it is (it showed 72 points tall), and the
/// handle at its corner, shown while it's hovered, resizes it (a small square edged in the accent).
#[test]
fn pictures_in_a_message_show_their_size_and_resize_by_their_handle() {
    use egui::{Pos2, Rect, Shape, vec2};
    let (mut a, _) = app("picture-size");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Poster", "body": ""}));
    let png = super::pictures::png(320, 200);
    a.run("compose.insertPicture", json!({"name": "poster.png", "data": png, "at": 0}));
    // What a frame paints: where the picture is (the one image not of the fonts), and whether a
    // handle shows (a small square in the accent colour).
    let accent = a.ui.tokens.accent;
    let paint = |a: &mut MailApp, events: Vec<Event>| -> (Option<Rect>, bool) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        fn walk(s: &Shape, picture: &mut Option<Rect>, handle: &mut bool, accent: egui::Color32) {
            match s {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, picture, handle, accent)),
                Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => *picture = Some(m.calc_bounds()),
                Shape::Rect(r) if (r.fill == accent || r.stroke.color == accent) && r.rect.width() < 12.0 && r.rect.height() < 12.0 => *handle = true,
                _ => {}
            }
        }
        let (mut picture, mut handle) = (None, false);
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut picture, &mut handle, accent));
        (picture, handle)
    };
    paint(&mut a, vec![]);
    let (r, handle) = paint(&mut a, vec![]);
    let r = r.expect("the picture is drawn");
    assert!((r.width() - 320.0).abs() < 1.0 && (r.height() - 200.0).abs() < 1.0, "its own size: {r:?}");
    assert!(!handle, "no handle while the pointer is elsewhere");
    let (_, handle) = paint(&mut a, vec![Event::PointerMoved(r.center())]);
    assert!(handle, "hovered, the handle shows");
    // Dragged 120 points in from its corner: 200 wide, as tall as that makes it.
    let corner = r.right_bottom() - vec2(4.0, 4.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    paint(&mut a, vec![Event::PointerMoved(corner)]);
    paint(&mut a, vec![button(corner, true)]);
    for k in 1..=6 {
        paint(&mut a, vec![Event::PointerMoved(corner - vec2(20.0 * k as f32, 0.0))]);
    }
    paint(&mut a, vec![button(corner - vec2(120.0, 0.0), false)]);
    let (r2, _) = paint(&mut a, vec![Event::PointerMoved(Pos2::new(5.0, 5.0))]);
    let r2 = r2.expect("still drawn");
    assert!((r2.width() - 200.0).abs() < 1.5 && (r2.height() - 125.0).abs() < 1.5, "resized: {r2:?}");
    let c = a.session.compose.clone().unwrap();
    let at = c.body.chars().position(|ch| ch == mailcraft_engine::mime::rich::PICTURE).unwrap();
    assert!(c.styles.style(at).picture_width.is_some_and(|w| w.abs_diff(200) <= 1), "{:?}", c.styles.style(at));
}

/// Issue #9: with an Exchange account, Select Names offers its Global Address List, the people
/// the directory finds come into the open list as they're answered, and the person selected
/// stays selected when they do (the list moving under the selection added someone else).
#[test]
fn the_directory_comes_into_select_names() {
    let (mut a, _) = app("names-directory");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    let id = a
        .session
        .run(
            "account.add",
            &json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example", "userAgent": "Evolution/3.46.4"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.directory, "an Exchange account has a directory");
    f.source = "directory".into();
    f.query = "zo".into();
    f.stale = true;
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.list.is_empty() && !f.searching, "two letters aren't sent by themselves");
    f.query = "zoe".into();
    f.stale = true;
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.searching, "asked of the server");
    a.session.apply(mailcraft_engine::model::Event::Directory {
        account: id.clone(),
        query: "zoe".into(),
        people: vec![mailcraft_engine::model::DirectoryPerson {
            name: "Zoe Park".into(),
            email: "zoe.park@contoso.example".into(),
            ..Default::default()
        }],
        error: None,
        consent: false,
    });
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert_eq!(f.list.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), ["zoe.park@contoso.example"]);
    assert!(!f.searching);
    f.selected = Some(0);
    // Asked again (Go), and answered with someone sorted before her: she stays the one selected.
    a.run("contact.searchDirectory", json!({"query": "zoe"}));
    a.session.apply(mailcraft_engine::model::Event::Directory {
        account: id.clone(),
        query: "zoe".into(),
        people: [("Zoe Park", "zoe.park@contoso.example"), ("Zoe Abbott", "zoe.a@contoso.example")]
            .map(|(n, e)| mailcraft_engine::model::DirectoryPerson { name: n.into(), email: e.into(), ..Default::default() })
            .to_vec(),
        error: None,
        consent: false,
    });
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert_eq!(f.list.len(), 2);
    assert_eq!(f.selected.and_then(|i| f.list.get(i)).map(|x| x.email.as_str()), Some("zoe.park@contoso.example"));
    // Refresh: forgotten, and asked again.
    a.run("contact.refreshDirectory", json!({}));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert!(f.searching);
}

/// Issue #9, online: with an Exchange Online account still to allow it, Select Names asks for the
/// permission (Allow searching the directory…) rather than showing an error.
#[test]
fn select_names_asks_to_allow_the_directory() {
    let (mut a, _) = app("names-consent");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    a.session.oauth_available = vec!["microsoft".into()];
    frame(&ctx, &mut a, vec![]);
    let id = a.session.run("account.add", &json!({"email": "me@contoso.example", "graph": true})).unwrap()["account"].as_str().unwrap().to_string();
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    f.source = "directory".into();
    f.query = "alice".into();
    f.stale = true;
    frame(&ctx, &mut a, vec![]);
    a.session.apply(mailcraft_engine::model::Event::Directory {
        account: id,
        query: "alice".into(),
        people: Vec::new(),
        error: Some("searching your organisation's directory needs your permission first".into()),
        consent: true,
    });
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert!(f.consent && f.directory_errors.is_empty(), "{:?}", f.directory_errors);
}

/// Issue #11: a picture pasted into the signature editor (or added with Pictures) is kept with
/// the signature when it's saved, and goes into the next message with it.
#[test]
fn a_signature_keeps_a_picture_pasted_into_it() {
    let (mut a, _) = app("signature-picture");
    // Its clipboard holds a picture.
    a.services = Box::new(Recorder(Default::default()));
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Dialog(Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false)))));
    frame(&ctx, &mut a, vec![]);
    let form = |a: &MailApp| match &a.ui.dialog {
        Some(Dialog::Signature(f)) => Some(f.compose.clone()),
        _ => None,
    };
    let id = crate::rich_edit::body_id(&form(&a).unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::RED)))]);
    frame(&ctx, &mut a, vec![]);
    let c = form(&a).unwrap();
    assert_eq!((c.body.matches('\u{FFFC}').count(), c.inline.len()), (1, 1), "{:?}", c.body);
    // Saved with the button a screen reader names Save.
    let nodes = named(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    let save = nodes.iter().find(|n| n.role == egui::accesskit::Role::Button && n.label.as_deref() == Some("Save")).and_then(Named::centre);
    click(&ctx, &mut a, save.expect("a Save button"));
    assert!(form(&a).is_none(), "saved and closed");
    let html = a.session.default_signature("sample", false).unwrap().html;
    assert!(html.contains("src=\"data:image/png;base64,"), "{html}");
    a.run("mail.new", json!({}));
    let c = a.session.compose.clone().unwrap();
    assert_eq!((c.body.matches('\u{FFFC}').count(), c.inline.len()), (1, 1), "the next message has it");
}

/// A reply drawn in the editor and left as MailCraft made it closes asking nothing: the frames
/// that draw it (and fit its styles to its text) change nothing in it.
#[test]
fn a_reply_drawn_and_left_untouched_closes_without_asking() {
    let (mut a, _) = app("reply-untouched");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.compose.is_some() && !a.session.compose_written(), "as made");
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "nothing asked: {:?}", a.ui.dialog);
}

/// A `mailto:` link clicked in a message is read as one the system hands MailCraft is: its
/// address decoded, its subject and copies taken (only `%40` was decoded, and everything after
/// the `?` was dropped).
#[test]
fn a_mailto_link_clicked_in_a_message_fills_in_the_new_message() {
    let (mut a, _) = app("mailto-link");
    let link = "mailto:first%2Elast@studio.example?subject=Tickets%20for%20Friday&cc=zoe%2Bshows@studio.example";
    a.act(crate::Action::OpenUrl(link.into()));
    let c = a.session.compose.as_ref().expect("a new message");
    assert_eq!((c.to.as_str(), c.subject.as_str(), c.cc.as_str()), ("first.last@studio.example", "Tickets for Friday", "zoe+shows@studio.example"));
}

/// Issue #213: the Address Book has Job title and Department columns where the directory gives
/// them, as Outlook's has, and none where it gives neither.
#[test]
fn the_address_book_shows_job_titles() {
    let (mut a, _) = app("names-titles");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    let id =
        a.session.run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(&ctx, &mut a, vec![]);
    let answer = |a: &mut MailApp, query: &str, people: Vec<mailcraft_engine::model::DirectoryPerson>| {
        if let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() {
            (f.source, f.query, f.stale) = ("directory".into(), query.into(), true);
        }
        frame(&ctx, a, vec![]);
        a.session.apply(mailcraft_engine::model::Event::Directory { account: id.clone(), query: query.into(), people, error: None, consent: false });
        frame(&ctx, a, vec![]);
        drawn_strings(&ctx, a, egui::vec2(1280.0, 900.0))
    };
    let person = |name: &str, email: &str, title: &str, department: &str| mailcraft_engine::model::DirectoryPerson {
        name: name.into(),
        email: email.into(),
        title: title.into(),
        department: department.into(),
    };
    let shown = answer(
        &mut a,
        "zoe",
        vec![person("Zoe Park", "zoe.park@contoso.example", "Colourist", "Post-production"), person("Zoe Abbott", "zoe.a@contoso.example", "", "")],
    );
    for what in ["Job title", "Department", "Colourist", "Post-production", "Zoe Abbott"] {
        assert!(shown.iter().any(|s| s == what), "{what}: {shown:?}");
    }
    let shown = answer(&mut a, "sam", vec![person("Sam Lee", "sam@contoso.example", "", "")]);
    assert!(!shown.iter().any(|s| s == "Job title" || s == "Department"), "none given, no columns: {shown:?}");
}
