//! UI tests: selecting several messages from the keyboard and with the mouse (issue #26).

use egui::accesskit::Role;
use mailcraft_engine::Row;

use super::*;

/// The rows of the message list as a screen reader is told of them: what each says, and where.
fn list_rows(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    let nodes = named(ctx, a, egui::vec2(1280.0, 800.0));
    let mut rows: Vec<(String, egui::Rect)> =
        nodes.into_iter().filter(|n| n.role == Role::ListItem).filter_map(|n| Some((n.label.unwrap_or_default(), n.rect?))).collect();
    rows.sort_by(|x, y| x.1.min.y.total_cmp(&y.1.min.y));
    rows
}

/// Whether some text drawn says `what` (a label, as a screen reader reads it).
fn says(ctx: &egui::Context, a: &mut MailApp, what: &str) -> bool {
    named(ctx, a, egui::vec2(1280.0, 800.0)).iter().filter_map(Named::name).any(|l| l.contains(what))
}

fn subjects(a: &MailApp) -> Vec<String> {
    let rows = a.session.row_keys();
    let mut sel: Vec<&MessageKey> = a.session.selection.iter().collect();
    sel.sort_by_key(|k| rows.iter().position(|r| r == *k));
    sel.iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect()
}

/// Shift+Down and Shift+Up select from the keyboard, and the reading pane says how many are
/// selected; Shift+End to the last; Page Down goes as many rows as the list shows; in a text
/// box Shift+Down selects text, not mail.
#[test]
fn shift_arrows_select_from_the_keyboard() {
    let (mut a, _) = app("shift-keys");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let all: Vec<String> = a.session.row_keys().iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect();
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all[..3].to_vec());
    assert!(a.session.reading.is_none());
    assert!(says(&ctx, &mut a, "3 messages selected"), "the reading pane says how many");
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all[..2].to_vec());
    frame(&ctx, &mut a, vec![key(Key::End, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all.clone());
    // Home alone: the first, by itself, read.
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    assert!(a.session.reading.is_some());
    // A page: the rows the list shows at once (more than one at this window's size).
    assert!(a.ui.list_page > 1, "{}", a.ui.list_page);
    frame(&ctx, &mut a, vec![key(Key::PageDown, Modifiers::NONE)]);
    let page = usize::try_from(a.ui.list_page).unwrap().min(all.len() - 1);
    assert_eq!(subjects(&a), vec![all[page].clone()]);
    // Typing in the search box: Shift+Down is the text's.
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let before = a.session.selection.clone();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    assert_eq!(a.session.selection, before);
}

/// Shift+click selects from the row clicked first to the one Shift+clicked; right-clicking one
/// of them keeps them all selected, so the menu acts on all of them (opening the one clicked
/// selected it alone: Delete took just that one).
#[test]
fn shift_click_and_a_right_click_on_the_selection() {
    let (mut a, _) = app("shift-click-ui");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let rows = list_rows(&ctx, &mut a);
    assert!(rows.len() >= 4, "{rows:?}");
    let all: Vec<String> = a.session.row_keys().iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect();
    click_at(&ctx, &mut a, rows[0].1.center(), egui::PointerButton::Primary, Modifiers::NONE);
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    click_at(&ctx, &mut a, rows[2].1.center(), egui::PointerButton::Primary, Modifiers::SHIFT);
    assert_eq!(subjects(&a), all[..3].to_vec());
    // A right-click on the middle one: all three stay selected.
    click_at(&ctx, &mut a, rows[1].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(subjects(&a), all[..3].to_vec(), "the menu is for all of them");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // A right-click on one outside them: that one alone.
    click_at(&ctx, &mut a, rows[3].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(subjects(&a), vec![all[3].clone()]);
}

/// The list's keys are Mail's: in Calendar the list isn't there, and End read its last message,
/// Delete and Backspace took the mail selected away unseen (found reviewing issue #26).
#[test]
fn list_keys_stay_in_mail() {
    let (mut a, _) = app("keys-in-calendar");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let first = a.session.row_keys()[0].clone();
    a.run("list.open", json!({"key": first.to_id()}));
    let n = a.session.list.len();
    a.run("view.calendar", json!({}));
    frame(&ctx, &mut a, vec![]);
    for k in [Key::End, Key::ArrowDown, Key::Delete, Key::Backspace] {
        frame(&ctx, &mut a, vec![key(k, Modifiers::NONE)]);
        frame(&ctx, &mut a, vec![key(k, Modifiers::SHIFT)]);
    }
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(a.ui.dialog.is_none());
    a.run("view.mail", json!({}));
    assert_eq!(a.session.list.len(), n, "nothing deleted or archived");
    assert_eq!(a.session.selection, vec![first.clone()]);
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(first));
    // Back in Mail they work.
    frame(&ctx, &mut a, vec![key(Key::End, Modifiers::NONE)]);
    assert_eq!(a.session.selection.last(), a.session.row_keys().last());
}

/// A right-click on one of several selected: Reply, Reply all, Forward and Quick print are for
/// one message, so they're off (they failed, or asked where to save a PDF of nothing).
#[test]
fn the_menu_of_several_has_no_reply() {
    let (mut a, _) = app("menu-several");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let rows = list_rows(&ctx, &mut a);
    click_at(&ctx, &mut a, rows[0].1.center(), egui::PointerButton::Primary, Modifiers::NONE);
    click_at(&ctx, &mut a, rows[2].1.center(), egui::PointerButton::Primary, Modifiers::SHIFT);
    click_at(&ctx, &mut a, rows[1].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(a.session.selection.len(), 3);
    let nodes = named(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    for name in ["Quick print", "Forward", "Delete"] {
        let found: Vec<bool> = nodes.iter().filter(|n| n.label.as_deref() == Some(name)).map(|n| n.disabled).collect();
        assert!(!found.is_empty(), "the menu has {name}");
        if name == "Delete" {
            assert!(found.iter().any(|d| !d), "Delete is for all of them");
        } else {
            assert!(found.iter().all(|d| *d), "{name} is off: {found:?}");
        }
    }
}

/// Down held: the presses queued in one frame (faster than frames were drawn) are one step of
/// as many rows, so one message is read and marked read; those passed over stay unread, as in
/// Outlook's list. More presses than rows stop at the last. J is Down too; Shift+Down after
/// them is a step of its own.
#[test]
fn queued_downs_read_only_the_message_landed_on() {
    let (mut a, _) = app("queued-downs");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    a.session.run("list.selectAll", &json!({})).unwrap();
    a.session.run("mail.markUnread", &json!({})).unwrap();
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let keys = a.session.row_keys();
    assert!(keys.len() >= 6, "{}", keys.len());
    let unread = |a: &MailApp, i: usize| a.session.summary(&keys[i]).is_some_and(|m| !m.flags.seen);
    let shown = a.session.messages_shown();
    let downs = |n: usize| {
        (0..n).map(|i| Event::Key { key: Key::ArrowDown, physical_key: None, pressed: true, repeat: i > 0, modifiers: Modifiers::NONE }).collect()
    };
    frame(&ctx, &mut a, downs(4));
    assert_eq!(a.session.reading.as_ref().map(|r| &r.key), Some(&keys[4]), "four rows on");
    assert_eq!(a.session.messages_shown(), shown + 1, "one message shown for four presses");
    assert!((1..4).all(|i| unread(&a, i)), "passed over: still unread");
    assert!(!unread(&a, 4), "the one landed on is read");
    // 20 presses, more than the list has rows: the last, and still one message shown.
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let shown = a.session.messages_shown();
    frame(&ctx, &mut a, downs(20));
    let last = keys.len() - 1;
    assert!(last < 20);
    assert_eq!(a.session.reading.as_ref().map(|r| &r.key), Some(&keys[last]));
    assert_eq!(a.session.messages_shown(), shown + 1, "one message shown for 20 presses");
    assert!((5..last).all(|i| unread(&a, i)), "passed over: still unread");
    // From the top, Down, J (a Down too) and Down: one step of 3; then Up, and Shift+Down twice
    // after it, a step of 2 from there.
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let shown = a.session.messages_shown();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::J, Modifiers::NONE), key(Key::ArrowDown, Modifiers::NONE)]);
    assert_eq!(a.session.focus.as_ref(), Some(&keys[3]));
    assert_eq!(a.session.messages_shown(), shown + 1);
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE), key(Key::ArrowDown, Modifiers::SHIFT), key(Key::ArrowDown, Modifiers::SHIFT)]);
    assert_eq!(a.session.anchor.as_ref(), Some(&keys[2]));
    assert_eq!(a.session.focus.as_ref(), Some(&keys[4]));
    assert_eq!(a.session.selection.len(), 3);
}

/// The same on an Inbox longer than the presses (the sample's is shorter, so the test above stops
/// at its last row): 20 Downs in one frame read the 21st message, and only it.
#[test]
fn twenty_queued_downs_land_twenty_rows_on() {
    let store = scratch_store(Store::open(scratch("twenty-downs")).unwrap());
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut st = store.load_folder(sample::SAMPLE_ACCOUNT, "INBOX").unwrap();
    let first = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    for uid in first..first + 30 {
        let key = MessageKey { account: sample::SAMPLE_ACCOUNT.into(), folder: "INBOX".into(), validity: st.validity, uid };
        store.write_body(&key, format!("From: a@x.example\r\nSubject: Message {uid}\r\n\r\nText.\r\n").as_bytes()).unwrap();
        let date = NOW - 86_400 * i64::from(uid);
        st.messages.push(Summary { key, subject: format!("Message {uid}"), date, ..Default::default() }.into());
    }
    store.save_folder(sample::SAMPLE_ACCOUNT, "INBOX", &st).unwrap();
    let mut a = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(NullServices(NOW)));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let keys = a.session.row_keys();
    assert!(keys.len() > 21, "{}", keys.len());
    let shown = a.session.messages_shown();
    let downs =
        (0..20).map(|i| Event::Key { key: Key::ArrowDown, physical_key: None, pressed: true, repeat: i > 0, modifiers: Modifiers::NONE }).collect();
    frame(&ctx, &mut a, downs);
    assert_eq!(a.session.reading.as_ref().map(|r| &r.key), Some(&keys[20]), "20 rows on");
    assert_eq!(a.session.messages_shown(), shown + 1, "one message shown for 20 presses");
}

/// Every message of a long folder selected (Ctrl+A): drawing the window copies none of them. The
/// ribbon, the menus and the Delete check each copied the whole selection, every frame.
#[test]
fn a_long_selection_is_drawn_without_copies() {
    let (mut a, _) = app("select-all-long");
    let messages: Vec<Summary> = (1..=20_000u32)
        .rev()
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid },
            subject: format!("Message {uid}"),
            date: NOW - i64::from(20_000 - uid) * 60,
            ..Default::default()
        })
        .collect();
    archive_of(&mut a, messages);
    assert_eq!(a.session.list.len(), 20_000);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.run("list.selectAll", json!({}));
    assert_eq!(a.session.selection.len(), 20_000);
    let copies = a.session.summary_clones();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    // The context menu of the selection, and the Delete key's check.
    let rows = list_rows(&ctx, &mut a);
    click_at(&ctx, &mut a, rows[1].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    frame(&ctx, &mut a, vec![]);
    assert!(!a.needs_delete_confirm(), "not in Deleted Items");
    assert_eq!(a.session.selection.len(), 20_000, "still all of them");
    assert_eq!(a.session.summary_clones(), copies, "no message copied to draw them");
}

/// Audit #466: a frame of the message list readies nothing for a row's right-click menu, which
/// only its open menu reads. Run in release: `cargo test --release -p mailcraft-ui-egui --lib
/// list_frames_measure -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn list_frames_measure() {
    let (mut a, _) = app("list-frames-measure");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 2000.0)));
    let list = |ctx: &egui::Context, a: &mut MailApp| {
        let mut out = ctx.run_ui(RawInput { screen_rect: screen, ..Default::default() }, |ui| {
            crate::panes::list::show(&a.session, &mut a.ui, ui, &mut Vec::new());
        });
        out.textures_delta.clear();
    };
    for _ in 0..50 {
        list(&ctx, &mut a);
    }
    const ROUNDS: u32 = 3000;
    let t = std::time::Instant::now();
    for _ in 0..ROUNDS {
        list(&ctx, &mut a);
    }
    let rows = a.session.rows().len();
    println!("MEASURE a frame of the message list, {rows} rows: {:.1} µs", t.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS));
}

/// Issue #92: Enter opens the message selected in the list in a window of its own, as a
/// double-click does; a draft goes on being written. In a text box Enter is the box's.
#[test]
fn enter_opens_the_selected_message() {
    let (mut a, _) = app("enter-opens");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let first = a.session.selection[0].clone();
    assert!(a.session.windows.is_empty());
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(a.session.windows.iter().map(|w| w.key.clone()).collect::<Vec<_>>(), [first], "its own window");

    // Typed into the search box, Enter searches: nothing more opens.
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("x".into()), key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(a.session.windows.len(), 1);

    // A draft: Enter goes on writing it.
    a.run("search.clear", json!({}));
    a.run("mail.new", json!({}));
    a.run("compose.saveDraft", json!({}));
    a.run("compose.close", json!({}));
    assert_eq!(a.session.all_composes().count(), 0);
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    assert!(!a.session.selection.is_empty(), "the draft is in Drafts");
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(a.session.all_composes().count(), 1, "the draft is being written");
}

/// Issue #189: a click on a heading of the list folds its mail away, the heading counting it; a
/// click again shows it. Its menu folds and shows every heading; what's folded is kept for the
/// next start.
#[test]
fn a_heading_clicked_folds_its_mail_away() {
    let (mut a, _) = app("fold-heading");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let heading = a.session.rows().iter().find_map(|r| if let Row::Header(h) = r { Some(h.to_string()) } else { None }).unwrap();
    let (rows, shown) = (a.session.rows().len(), list_rows(&ctx, &mut a).len());
    click_named(&ctx, &mut a, &heading);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.heading_folded(0), "folded: {:?}", &a.session.rows()[..2]);
    let Some(Row::Folded { count, .. }) = a.session.rows().get(1).cloned() else { panic!("counted") };
    assert_eq!(list_rows(&ctx, &mut a).len(), shown - count, "its rows gone");
    assert!(says(&ctx, &mut a, &format!("{heading} ({count}), folded")), "the heading says how many it hides");
    // Kept for the next start.
    let saved = a.saved_state();
    let (mut b, _) = app("fold-heading-again");
    b.restore_state(&saved);
    assert!(b.session.heading_folded(0), "folded after a restart");
    // Clicked again: shown.
    click_named(&ctx, &mut a, &format!("{heading} ({count}), folded"));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.rows().len(), rows);
    // Its menu: every heading folded, then shown.
    let at = named(&ctx, &mut a, egui::vec2(1280.0, 800.0))
        .into_iter()
        .find(|n| n.label.as_deref() == Some(heading.as_str()))
        .and_then(|n| n.centre())
        .unwrap();
    click_at(&ctx, &mut a, at, egui::PointerButton::Secondary, Modifiers::NONE);
    click_named(&ctx, &mut a, "Collapse all groups");
    assert_eq!(list_rows(&ctx, &mut a).len(), 0, "every message folded away");
    assert!(a.session.rows().iter().all(|r| !matches!(r, Row::Message { .. })));
    // The list's arrangement menu shows them all again.
    click_named(&ctx, &mut a, "By Date ⏷");
    click_named(&ctx, &mut a, "Expand all groups");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.rows().len(), rows);
}

/// Issue #216: a row's own buttons (Mark read, Flag, Delete) act on that message alone; what's
/// selected and open stays as it was (the row was selected first, and the user's selection lost).
#[test]
fn a_rows_buttons_leave_the_selection_alone() {
    let (mut a, _) = app("row-buttons");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let (selected, open) = (a.session.selection.clone(), a.session.reading.as_ref().map(|r| r.key.clone()));
    assert!(open.is_some());
    let target = a.session.row_keys()[2].clone();
    let press = |a: &mut MailApp, label: &str| {
        let third = list_rows(&ctx, a)[2].1;
        frame(&ctx, a, vec![Event::PointerMoved(third.center())]);
        let nodes = named(&ctx, a, egui::vec2(1280.0, 800.0));
        let at = nodes.iter().filter(|n| n.label.as_deref() == Some(label)).filter_map(Named::centre).find(|c| third.contains(*c));
        click(&ctx, a, at.unwrap_or_else(|| panic!("no {label} on the third row")));
        frame(&ctx, a, vec![]);
    };
    let seen = a.session.summary(&target).unwrap().flags.seen;
    press(&mut a, if seen { "Mark unread" } else { "Mark read" });
    assert_eq!(a.session.summary(&target).unwrap().flags.seen, !seen, "that message marked");
    assert_eq!(a.session.selection, selected, "the selection stays");
    let flagged = a.session.summary(&target).unwrap().flags.flagged;
    press(&mut a, if flagged { "Unflag" } else { "Flag" });
    assert_eq!(a.session.summary(&target).unwrap().flags.flagged, !flagged, "that message flagged");
    assert_eq!(a.session.selection, selected);
    press(&mut a, "Delete");
    assert!(a.session.summary(&target).is_none(), "that message deleted");
    assert_eq!(a.session.selection, selected, "the selection stays");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), open, "so does the message open");
}
