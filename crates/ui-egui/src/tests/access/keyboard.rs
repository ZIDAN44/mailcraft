//! What a screen reader hears as the keyboard moves (issue #175): the message list's arrows, a
//! drop-down's keys and the open message's text. A screen reader reads the focused control, so
//! each of these has to move the focus itself, not only what the window shows.

use egui::accesskit::{self, Role};

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The node a frame's update says has the keyboard: its role, its name and its value.
fn focused(out: &egui::FullOutput) -> (Role, String, String) {
    let u = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
    let n = u.nodes.iter().find(|(id, _)| *id == u.focus).map(|(_, n)| n.clone()).expect("the focus is among the nodes");
    (n.role(), n.label().unwrap_or_default().to_string(), n.value().unwrap_or_default().to_string())
}

/// Frames for what `events` do to settle (a focus asked for lands in the next one), the last
/// one's output.
fn settled(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> egui::FullOutput {
    sized_frame(ctx, a, SIZE, events);
    sized_frame(ctx, a, SIZE, vec![]);
    sized_frame(ctx, a, SIZE, vec![])
}

/// A screen reader's "focus this" for the node `id`.
fn focus_request(id: accesskit::NodeId) -> Event {
    Event::AccessKitActionRequest(accesskit::ActionRequest {
        action: accesskit::Action::Focus,
        target_node: id,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    })
}

/// The nodes of a frame's update, with their ids.
fn nodes(out: &egui::FullOutput) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    out.platform_output.accesskit_update.as_ref().expect("AccessKit is on").nodes.clone()
}

/// Down and Up in the message list take the keyboard to the next and the previous row, so a
/// screen reader reads each message as it's reached; the keyboard stayed on the window, and NVDA
/// read nothing. A click on a row gives it the keyboard too. Each press moves one row, not two
/// (egui's own arrow keys would also move to the row below). A message deleted, the next one's
/// row has the keyboard.
#[test]
fn the_list_arrows_take_the_screen_reader_from_message_to_message() {
    let (mut a, _) = app("access-list-arrows");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Down with nothing chosen: the first message, read.
    settled(&ctx, &mut a, vec![]);
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let reached = a.session.focus.clone().and_then(|k| a.session.summary(&k).map(|m| m.subject.clone())).expect("Down chose a message");
    let (role, label, _) = focused(&out);
    assert!(role == Role::ListItem && label.contains(reached.as_str()), "the first Down reads {label:?}, not {reached}");
    let first = a.session.list.first().cloned().expect("the sample has mail");
    let row = named_in(&out)
        .into_iter()
        .find(|n| n.role == Role::ListItem && n.label.as_deref().is_some_and(|l| l.contains(first.subject.as_str())))
        .and_then(|n| n.centre())
        .expect("the first message's row");
    click(&ctx, &mut a, row);
    let out = settled(&ctx, &mut a, vec![]);
    let (role, label, _) = focused(&out);
    assert_eq!(role, Role::ListItem, "a clicked row has the keyboard");
    assert!(label.contains(first.subject.as_str()), "{label}");
    // Which message a row is, counting messages only (not the date headings between them).
    let nth = |a: &MailApp, k: &MessageKey| {
        let row = a.session.row_index_of(k).expect("its row");
        a.session.rows().get(..row).unwrap_or_default().iter().filter(|r| matches!(r, mailcraft_engine::Row::Message { .. })).count()
    };
    let mut at = nth(&a, a.session.focus.as_ref().expect("a message has the list's focus"));
    for (k, step) in [(Key::ArrowDown, 1_isize), (Key::ArrowDown, 1), (Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
        let out = settled(&ctx, &mut a, vec![key(k, Modifiers::NONE)]);
        let now = a.session.focus.clone().expect("a message has the list's focus");
        let index = nth(&a, &now);
        assert_eq!(index as isize, at as isize + step, "{k:?} moves one row");
        at = index;
        let subject = a.session.summary(&now).map(|m| m.subject.clone()).unwrap_or_default();
        let (role, label, _) = focused(&out);
        assert_eq!(role, Role::ListItem, "{k:?}: the row has the keyboard, not {label:?}");
        assert!(label.contains(subject.as_str()), "{k:?}: the screen reader reads {label:?}, not the message reached ({subject})");
    }
    // Deleted, its row goes: the message the list goes on to has the keyboard (it was nowhere).
    let gone = a.session.focus.clone();
    let out = settled(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    let now = a.session.focus.clone().expect("the list goes on to another message");
    assert_ne!(Some(&now), gone.as_ref(), "Delete took the message away");
    let subject = a.session.summary(&now).map(|m| m.subject.clone()).unwrap_or_default();
    let (role, label, _) = focused(&out);
    assert!(role == Role::ListItem && label.contains(subject.as_str()), "after Delete the screen reader reads {label:?}, not {subject}");
}

/// The message toolbar's drop-downs keep the keyboard while their arrows choose: Font size goes
/// up a size a press (the text took the keyboard back after the first, and the second moved the
/// caret instead).
#[test]
fn a_toolbar_drop_down_keeps_the_keyboard_while_its_arrows_choose() {
    let (mut a, _) = app("access-toolbar-keys");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    settled(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"}));
    let out = settled(&ctx, &mut a, vec![]);
    let size =
        nodes(&out).into_iter().find(|(_, n)| n.role() == Role::ComboBox && n.label() == Some("Font size")).map(|(id, _)| id).expect("Font size");
    settled(&ctx, &mut a, vec![focus_request(size)]);
    let typed = |a: &MailApp| a.session.compose.as_ref().map(|c| c.styles.style(0).size()).unwrap_or_default();
    let before = typed(&a);
    let sizes = mailcraft_engine::mime::rich::SIZES;
    let at = sizes.iter().position(|z| *z == before).expect("a size the box offers");
    for step in 1..=2 {
        let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
        let (role, label, value) = focused(&out);
        assert_eq!((role, label.as_str()), (Role::ComboBox, "Font size"), "press {step}: the keyboard stays on the box");
        assert_eq!(value, sizes.get(at + step).map(u8::to_string).unwrap_or_default(), "press {step}: a size up");
    }
}

/// A drop-down's arrows choose, as a list box's do on Windows: Down on the account form's Mail
/// server picks the next kind of server and the keyboard stays on it (it went to the field
/// below, so the list could only be opened by trying keys). Up goes back; Alt+Down opens the
/// list on the choice made (not on its first), whose arrows go through its choices and whose
/// Enter picks one, leaving the keyboard on the drop-down again.
#[test]
fn a_drop_downs_arrows_choose_and_keep_the_keyboard() {
    let (mut a, _) = app("access-drop-down-keys");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(Dialog::add_account_with("ana@example.org", true));
    let kind = |a: &MailApp| match &a.ui.dialog {
        Some(Dialog::AddAccount(f)) => f.kind,
        _ => panic!("the account form is open"),
    };
    let out = settled(&ctx, &mut a, vec![]);
    let combo = nodes(&out)
        .into_iter()
        .find(|(_, n)| n.role() == Role::ComboBox && n.value() == Some("IMAP and SMTP"))
        .map(|(id, _)| id)
        .expect("Mail server, showing IMAP");
    let out = settled(&ctx, &mut a, vec![focus_request(combo)]);
    assert_eq!(focused(&out).0, Role::ComboBox);
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_eq!(kind(&a), crate::dialogs::AccountKind::Pop, "Down picks the next");
    let (role, _, value) = focused(&out);
    assert_eq!((role, value.as_str()), (Role::ComboBox, "POP and SMTP"), "the keyboard stays on the drop-down, which reads its new value");
    settled(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE)]);
    assert_eq!(kind(&a), crate::dialogs::AccountKind::Imap, "Up goes back");
    // The list: opened on the choice made, and on to the next, which is picked.
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(focused(&out).1, "IMAP and SMTP", "the open list has the keyboard on the choice made");
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let picked = focused(&out).1;
    assert_eq!(picked, "POP and SMTP", "and through its choices");
    let out = settled(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(kind(&a), crate::dialogs::AccountKind::Pop, "Enter picks it");
    let (role, _, value) = focused(&out);
    assert!(!egui::Popup::is_any_open(&ctx), "the list is closed");
    assert_eq!(role, Role::ComboBox, "the keyboard is back on the drop-down");
    assert_eq!(value, picked);
    // A list opened on a later choice starts there, not at its first (JMAP is the third).
    settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_eq!(kind(&a), crate::dialogs::AccountKind::Jmap);
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(focused(&out).1, "JMAP", "the list opens on the choice made");
    let out = settled(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE)]);
    assert_eq!(focused(&out).1, "POP and SMTP", "Up in it goes to the one before");
    settled(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(kind(&a), crate::dialogs::AccountKind::Jmap, "Esc leaves the choice as it was");
    // Esc in the open list closes the list, not the form with what was typed in it, and the
    // keyboard is back on the drop-down.
    settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let out = settled(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(matches!(a.ui.dialog, Some(Dialog::AddAccount(_))), "the form stays");
    assert!(!egui::Popup::is_any_open(&ctx), "the list is closed");
    assert_eq!(focused(&out).0, Role::ComboBox, "the keyboard is back on the drop-down");
}

/// The open message's text is one document a screen reader can go through by line, word and
/// letter, as Outlook's reading pane is: Tab goes from the message's row, past the reading pane's
/// buttons and attachments, each named, to the document (it was out of the keyboard's reach,
/// and NVDA read nothing of a message, then a block at a time). The whole text is its text; Down
/// and Up move its caret a line on and back, Right a letter, Shift+Down selects, and none of them
/// moves the list on to the next message under the one being read.
#[test]
fn a_screen_reader_reads_the_open_message() {
    let (mut a, _) = app("access-reading");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let out = settled(&ctx, &mut a, vec![]);
    let subject = "Your proof for the festival poster is ready";
    let row = named_in(&out)
        .into_iter()
        .find(|n| n.role == Role::ListItem && n.label.as_deref().is_some_and(|l| l.contains(subject)))
        .and_then(|n| n.centre())
        .expect("the proof's row");
    click(&ctx, &mut a, row);
    let opened = a.session.focus.clone().expect("the row clicked is the list's focus");
    let mut reached = false;
    for _ in 0..40 {
        let out = settled(&ctx, &mut a, vec![key(Key::Tab, Modifiers::NONE)]);
        let (role, label, value) = focused(&out);
        if role == Role::Document {
            assert_eq!(label, "Message text");
            reached = true;
            break;
        }
        assert_ne!(role, Role::Window, "Tab lost the keyboard");
        // Each stop on the way says what it is: the attachments were "custom", and a stop under
        // the text said nothing at all.
        assert!(role != Role::Unknown && !(label.is_empty() && value.is_empty()), "a stop a screen reader can't name: {role:?} {label:?} {value:?}");
    }
    assert!(reached, "Tab reaches the message's text");
    // What a reader gets of it: the document's text, and where its caret is (a line, and a
    // letter in the whole text).
    let read = |out: &egui::FullOutput| {
        let update = out.platform_output.accesskit_update.clone().expect("AccessKit is on");
        let tree = accesskit_consumer::Tree::new(update, true);
        let state = tree.state();
        let doc = state.focus().expect("the document has the keyboard");
        assert_eq!(doc.role(), Role::Document);
        assert!(doc.supports_text_ranges(), "read as text, by line, word and letter");
        let text = doc.document_range().text();
        let caret = doc.text_selection_focus().map(|p| (p.to_line_index(), p.to_global_usv_index()));
        let selected = doc.text_selection().is_some_and(|r| !r.is_degenerate());
        (text, caret, selected)
    };
    let (text, _, _) = read(&settled(&ctx, &mut a, vec![]));
    assert!(
        text.starts_with("Hello,") && text.contains("The proof for order 4471") && text.trim_end().ends_with("Print Shop"),
        "the whole text: {text:?}"
    );
    let (_, start, _) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::NONE)]));
    let (line0, at0) = start.expect("the caret is told");
    let (_, next, _) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::NONE)]));
    let (line1, at1) = next.expect("the caret is told");
    assert_eq!((line1, at1), (line0, at0 + 1), "Right moves it a letter");
    let (_, down, _) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]));
    let (line2, _) = down.expect("the caret is told");
    assert!(line2 > line1, "Down moves it on a line: {line1} to {line2}");
    assert_eq!(a.session.focus.as_ref(), Some(&opened), "the list stays on the message being read");
    let (_, up, _) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE)]));
    assert_eq!(up.map(|c| c.0), Some(line1), "Up moves it back");
    let (_, _, selected) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]));
    assert!(selected, "Shift+Down selects from the caret");
    let (_, _, selected) = read(&settled(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]));
    assert!(!selected, "Down alone lets the selection go");
    assert_eq!(a.session.focus.as_ref(), Some(&opened));
}

/// A formatted message is read as one text too, in its reading order: its heading, its
/// paragraphs whose bold and italic words are runs of their own, its list and its link.
#[test]
fn a_formatted_message_reads_as_one_text() {
    let (mut a, _) = app("access-reading-html");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    settled(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).expect("the sample's newsletter");
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..4 {
        sized_frame(&ctx, &mut a, SIZE, vec![]);
    }
    let out = settled(&ctx, &mut a, vec![]);
    let doc = nodes(&out).into_iter().find(|(_, n)| n.role() == Role::Document).map(|(id, _)| id).expect("the message's text is a document");
    let out = settled(&ctx, &mut a, vec![focus_request(doc)]);
    let tree = accesskit_consumer::Tree::new(out.platform_output.accesskit_update.clone().expect("AccessKit is on"), true);
    let state = tree.state();
    let focus = state.focus().expect("the document has the keyboard");
    assert_eq!(focus.role(), Role::Document);
    let text = focus.document_range().text();
    let order = [
        "Winter group show: open call",
        "Dear artists,",
        "Our winter group show is open for submissions until the end of the month",
        "three works",
        "Deadline: 30 November",
        "full guidelines on our website",
        "See you at the opening!",
    ];
    let mut from = 0;
    for part in order {
        let at = text.get(from..).and_then(|t| t.find(part)).unwrap_or_else(|| panic!("{part:?} after the one before it in {text:?}"));
        from += at + part.len();
    }
}

/// F6 goes from pane to pane, as in Outlook: from the message list to the reading pane, the
/// To-Do Bar when it's shown, the ribbon, the folders and back to the list's message; Shift+F6
/// the other way. Each pane's control has the keyboard in turn, so a screen reader reads it.
#[test]
fn f6_goes_from_pane_to_pane() {
    for todo_bar in [false, true] {
        f6_round(todo_bar);
    }
}

fn f6_round(todo_bar: bool) {
    let (mut a, _) = app(if todo_bar { "access-f6-todo" } else { "access-f6" });
    if todo_bar {
        a.run("view.todoBar", json!({"part": "tasks", "on": true}));
    }
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let out = settled(&ctx, &mut a, vec![]);
    let first = a.session.list.first().cloned().expect("the sample has mail");
    let row = named_in(&out)
        .into_iter()
        .find(|n| n.role == Role::ListItem && n.label.as_deref().is_some_and(|l| l.contains(first.subject.as_str())))
        .and_then(|n| n.centre())
        .expect("the first message's row");
    click(&ctx, &mut a, row);
    settled(&ctx, &mut a, vec![]);
    // Which pane the keyboard is in, by where its control is.
    let pane_of = |a: &MailApp, out: &egui::FullOutput| {
        let u = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
        let b = u.nodes.iter().find(|(id, _)| *id == u.focus).and_then(|(_, n)| n.bounds())?;
        let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
        let bars = a.ui.top_bar_rect.map(|r| a.ui.ribbon_rect.map_or(r, |x| r.union(x)));
        if bars.is_some_and(|r| r.contains(at)) {
            return Some(crate::Pane::Bars);
        }
        a.ui.panes.iter().find(|(_, r)| r.contains(at)).map(|(p, _)| *p)
    };
    let shown: Vec<crate::Pane> = a.ui.panes.iter().map(|(p, _)| *p).collect();
    assert_eq!(shown.contains(&crate::Pane::TodoBar), todo_bar, "the To-Do Bar is shown: {shown:?}");
    let mut want = vec![crate::Pane::Reading];
    if shown.contains(&crate::Pane::TodoBar) {
        want.push(crate::Pane::TodoBar);
    }
    want.extend([crate::Pane::Bars, crate::Pane::Folders, crate::Pane::List]);
    let mut went = Vec::new();
    let mut last = None;
    for _ in 0..want.len() {
        let out = settled(&ctx, &mut a, vec![key(Key::F6, Modifiers::NONE)]);
        let pane = pane_of(&a, &out);
        if pane == Some(crate::Pane::Folders) {
            // The folder shown, not the pane's first button.
            let (role, label, _) = focused(&out);
            assert!(role == Role::TreeItem && label.starts_with("Inbox"), "F6 in the folders is on the folder shown: {role:?} {label:?}");
        }
        went.push(pane);
        last = Some(out);
    }
    assert_eq!(went, want.iter().map(|p| Some(*p)).collect::<Vec<_>>(), "F6 goes through every pane in turn");
    let (role, label, _) = focused(&last.expect("F6 was pressed"));
    assert!(role == Role::ListItem && label.contains(first.subject.as_str()), "back in the list, on its message: {role:?} {label:?}");
    // Shift+F6: the other way round.
    let mut back = Vec::new();
    for _ in 0..want.len() {
        let out = settled(&ctx, &mut a, vec![key(Key::F6, Modifiers::SHIFT)]);
        back.push(pane_of(&a, &out));
    }
    let mut want_back: Vec<Option<crate::Pane>> = want.iter().rev().skip(1).map(|p| Some(*p)).collect();
    want_back.push(Some(crate::Pane::List));
    assert_eq!(back, want_back, "Shift+F6 goes back through them");
    assert_eq!(a.session.focus.as_ref(), Some(&first.key), "the list stayed on its message all the way round");
    // On the folder shown, the window laid out anew (Ctrl+F1 hiding the ribbon) makes its row
    // again under another id: the keyboard stays on it (it went nowhere).
    settled(&ctx, &mut a, vec![key(Key::F6, Modifiers::SHIFT)]);
    for _ in 0..2 {
        let out = settled(&ctx, &mut a, vec![key(Key::F1, Modifiers::COMMAND)]);
        let (role, label, _) = focused(&out);
        assert!(role == Role::TreeItem && label.starts_with("Inbox"), "Ctrl+F1 kept the keyboard on the folder: {role:?} {label:?}");
    }
}
