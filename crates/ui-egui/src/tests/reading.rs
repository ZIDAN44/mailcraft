//! UI tests: the list, the reading pane, attachments and notices.

use super::*;

#[test]
fn renders_every_state_without_panicking() {
    tour();
}

/// A newsletter with a picture behind its heading and a label placed over its poster draws in
/// both themes and in a narrow reading pane (where its columns stack).
#[test]
fn a_banner_and_a_placed_label_draw() {
    let (mut a, dir) = app("banner");
    let ctx = egui::Context::default();
    let k = a.session.list.iter().find(|m| m.subject.contains("winter group show")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let p = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert!(p.images.iter().any(|i| i == "cid:banner@gallery.example"), "the banner is one of its pictures: {:?}", p.images);
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        for pane in [ReadingPane::Right, ReadingPane::Bottom] {
            a.session.prefs.reading_pane = pane;
            for _ in 0..3 {
                frame(&ctx, &mut a, vec![]);
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Dialogs kept the size of the last big one (their buttons' row took all the room left): Add
/// account stayed tall after Server settings closed, Remove account opened as tall as Options.
#[test]
fn dialogs_fit_what_they_show() {
    let (mut a, dir) = app("dialog-size");
    let ctx = egui::Context::default();
    let height = |ctx: &egui::Context, d: &Dialog| {
        // Its own window (or, before, the one all dialogs shared).
        let ids = [egui::Id::unique(("dialog", std::mem::discriminant(d))), egui::Id::unique("dialog")];
        ids.iter().find_map(|id| ctx.memory(|m| m.area_rect(*id))).map_or(0.0, |r| r.height())
    };
    let shown = |a: &mut MailApp, d: Dialog| {
        a.ui.dialog = Some(d);
        for _ in 0..4 {
            frame(&ctx, a, vec![]);
        }
        a.ui.dialog.clone().unwrap()
    };
    let mut d = Dialog::add_account_with("zoe@gmail.com", true);
    let open = height(&ctx, &shown(&mut a, d.clone()));
    if let Dialog::AddAccount(f) = &mut d {
        f.advanced = false;
    }
    let closed = height(&ctx, &shown(&mut a, d));
    assert!(open > 0.0 && closed < open - 60.0, "Server settings closed: {closed} (open: {open})");
    let edit = Dialog::edit_account(&a.session.accounts[0].config);
    shown(&mut a, edit);
    let remove = Dialog::RemoveAccount { account: "sample".into(), email: "x".into() };
    let small = height(&ctx, &shown(&mut a, remove));
    assert!(small > 0.0 && small < 220.0, "Remove account: {small}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keyboard_shortcuts_run_commands() {
    let (mut a, dir) = app("keys");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let first = a.session.focus.clone().unwrap();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_ne!(a.session.focus.clone().unwrap(), first);
    let was = a.session.selected()[0].flags.flagged;
    frame(&ctx, &mut a, vec![key(Key::Insert, Modifiers::NONE)]);
    assert_eq!(a.session.selected()[0].flags.flagged, !was, "Insert toggles the flag");
    let before = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), before - 1);
    frame(&ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    assert!(a.session.compose.is_some());
    // The new message's To box takes focus, so Delete edits text instead of deleting mail.
    frame(&ctx, &mut a, vec![]);
    let n = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), n);
    frame(&ctx, &mut a, vec![key(Key::M, Modifiers::COMMAND | Modifiers::SHIFT)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn folder_dialogs_create_rename_and_report_errors() {
    let (mut a, dir) = app("folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::new_folder("sample", None));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::FolderName { name, .. }) = a.ui.dialog.as_mut() {
        *name = "Clients".into();
    }
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.folder("sample", "Clients").is_some());
    assert!(a.ui.dialog.is_none(), "the dialog closes once the folder exists");
    // A duplicate keeps the dialog open with the error.
    a.act(crate::Action::Dialog(Dialog::new_folder("sample", None)));
    a.act(crate::Action::Submit("folder.create", json!({"account": "sample", "name": "Clients"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::FolderName { error: Some(e), .. }) if e.contains("already")));
    frame(&ctx, &mut a, vec![]);
    // Favorites show the new folder once pinned.
    a.run("folder.favorite", json!({"account": "sample", "folder": "Clients"}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.favorites().iter().any(|f| f.folder == "Clients"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn attachments_drag_out_as_files() {
    let (mut a, dir) = app("drag-out");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    // Not offered where the system can't take the file.
    a.act(crate::Action::DragOut(0));
    assert!(a.take_drag_out().is_none());
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.act(crate::Action::DragOut(index));
    let path = a.take_drag_out().expect("a file to drag");
    assert!(path.is_file() && path.metadata().unwrap().len() > 0, "{path:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn new_mail_is_announced_and_counted_in_the_title() {
    let (mut a, dir) = app("announce");
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.title_shown, "(3) Inbox – MailCraft");
    assert!(shown.lock().unwrap().is_empty(), "the mail already there isn't news");
    let m = |uid: u32, who: &str, subject: &str| Summary {
        key: MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: 1, uid },
        from: mailcraft_engine::model::EmailAddress::new(who, "x@y.example"),
        subject: subject.into(),
        date: NOW - 5,
        ..Default::default()
    };
    a.session.new_mail = vec![m(90, "Zoe Park", "Lunch?")];
    frame(&ctx, &mut a, vec![]);
    a.session.new_mail = vec![m(91, "Ann", "a"), m(92, "Bo", "b"), m(93, "Ann", "c")];
    frame(&ctx, &mut a, vec![]);
    let got = shown.lock().unwrap().clone();
    assert_eq!(got[0], ("Zoe Park".to_string(), "Lunch?".to_string()));
    assert_eq!(got[1], ("3 new messages".to_string(), "From Ann and Bo".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn long_folders_render_quickly() {
    let (mut a, dir) = app("long");
    let store = a.session.store.clone().unwrap();
    let messages: Vec<Summary> = (1..=2000u32)
        .rev()
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid },
            subject: format!("Message {uid}"),
            date: NOW - i64::from(2000 - uid) * 3600,
            preview: "Lorem".into(),
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Archive", &FolderState { validity: 1, uid_next: 2001, highest_modseq: 0, messages, window: 0 }).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    assert_eq!(a.session.list.len(), 2000);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    let per = t.elapsed() / 10;
    // Virtualised: only visible rows are laid out (debug builds are slow; this is generous).
    assert!(per.as_millis() < 400, "frame took {per:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, dir) = app("attachment-preview");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let n = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.attachments.len());
    assert_eq!(n, 4, "notes, a PDF, a document and a workbook");
    for i in 0..n {
        a.ui.attachment_view = Some((k.clone(), i));
        // A PDF draws a page a frame.
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        assert_eq!(a.ui.attachment_view, Some((k.clone(), i)));
    }
    // Escape: back to the message.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    // One that isn't there any more (another message's index) is just the message.
    a.ui.attachment_view = Some((k.clone(), 99));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A goes to what was last clicked: the attachments when they were, else the mail list.
#[test]
fn ctrl_a_selects_attachments_when_they_have_the_keyboard() {
    let (mut a, dir) = app("attach-all");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let picked = a.ui.attachment_selected.as_ref().map(|(_, v)| v.len());
    assert_eq!(picked, Some(4), "all four attachments");
    assert_eq!(a.session.selection.len(), 1, "the mail selection is left alone");
    // Escape lets go of them.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_selected.is_none() && !a.ui.attachments_active);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.ui.attachment_selected.as_ref().map(|(_, v)| v.len()), Some(4));
    // Elsewhere (a press off the attachments), Ctrl+A selects the mail again.
    let off = egui::pos2(5.0, 400.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(off)]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert!(!a.ui.attachments_active);
    assert!(a.ui.attachment_selected.is_none(), "a click elsewhere lets go of them");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.session.selection.len(), a.session.list.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, dir) = app("attach-back");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    a.ui.attachment_view = Some((k.clone(), index));
    a.ui.attachment_selected = Some((k.clone(), vec![index]));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none() && a.ui.attachment_selected.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Enter in a dialog's one field submits it: the dialog gave the field its focus back before
/// looking for the Enter that had just ended input, so only the button worked.
#[test]
fn enter_submits_a_one_field_dialog() {
    let (mut a, dir) = app("enter-submits");
    let ctx = egui::Context::default();
    // The sample account isn't an Exchange one, so the submit comes back as the dialog's error.
    a.ui.dialog = Some(Dialog::AddSharedMailbox { account: "sample".into(), address: "sales@contoso.example".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddSharedMailbox { error: Some(_), .. })), "submitted: {:?}", a.ui.dialog.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Motion: the folder pane slides between its two widths and the rest draws through it; Play
/// animations off makes egui's own motion instant too; a message just opened fades in.
#[test]
fn motion_plays_and_stops() {
    let (mut a, dir) = app("motion");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(crate::motion::enabled(&ctx));
    for minimized in [true, false, true, false] {
        a.run("view.folderPane", json!({"minimized": minimized}));
        for _ in 0..4 {
            frame(&ctx, &mut a, vec![]);
        }
    }
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    frame(&ctx, &mut a, vec![]);
    a.run("view.animations", json!({"on": false}));
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::motion::enabled(&ctx));
    assert_eq!(ctx.global_style().animation_time, 0.0);
    a.run("view.folderPane", json!({"minimized": true}));
    frame(&ctx, &mut a, vec![]);
    a.run("view.animations", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.animations && crate::motion::enabled(&ctx));
    let _ = std::fs::remove_dir_all(dir);
}
