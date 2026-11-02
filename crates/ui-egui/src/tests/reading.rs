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

/// A text file in another charset previews in it: the pane read its bytes as they came (Save's)
/// as UTF-8, so Windows-1252 and UTF-16 files showed replacement characters.
#[test]
fn a_text_attachment_previews_in_its_charset() {
    let raw = b"From: Sam <sam@studio.example>\r\nSubject: Notes\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHere.\r\n\
        --b1\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Disposition: attachment; filename=\"latin.txt\"\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\nCaf\xe9 au lait\r\n\
        --b1\r\nContent-Type: text/plain; charset=utf-16\r\nContent-Disposition: attachment; filename=\"wide.txt\"\r\n\
        Content-Transfer-Encoding: quoted-printable\r\n\r\n=FF=FEh=00i=00\r\n--b1--\r\n";
    let p = mailcraft_engine::mime::parse(raw).unwrap();
    assert_eq!(p.attachments.len(), 2);
    let shown: Vec<String> = p.attachments.iter().map(|a| crate::reading::previewed_text(raw, a).unwrap()).collect();
    assert!(shown[0].starts_with("Caf\u{e9} au lait"), "{:?}", shown[0]);
    assert_eq!(shown[1].trim_end(), "hi");
    assert!(shown.iter().all(|t| !t.contains('\u{fffd}')));
    // Saved, the Windows-1252 file is still its own bytes.
    assert!(mailcraft_engine::mime::attachment_data(raw, 0).unwrap().starts_with(b"Caf\xe9"));
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

/// Issue #10: a picture of an HTML message shows at the size the message sets, as Outlook and
/// browsers show it: both sides as set, one side with its shape kept, no wider than its
/// `max-width` or the room; a spacer that sets a size keeps its room, a tracking pixel has none.
#[test]
fn html_pictures_show_at_the_size_the_message_sets() {
    use crate::reading::body::shown_size;
    use mailcraft_engine::mime::ImageSize;
    use mailcraft_engine::mime::layout::Width;
    let v = egui::vec2;
    let set = |w: Option<Width>, h: Option<u32>, m: Option<u32>| Some(ImageSize { width: w, height: h, max_width: m });
    // A logo twice as sharp as it says: at what it says.
    assert_eq!(shown_size(v(400.0, 100.0), set(Some(Width::Px(200)), Some(50), None), 800.0), Some(v(200.0, 50.0)));
    // `width: 100%; max-width: 600px`: the room, at most 600.
    assert_eq!(shown_size(v(1200.0, 600.0), set(Some(Width::Percent(100)), None, Some(600)), 800.0), Some(v(600.0, 300.0)));
    assert_eq!(shown_size(v(1200.0, 600.0), set(Some(Width::Percent(100)), None, Some(600)), 400.0), Some(v(400.0, 200.0)));
    // One side: the other keeps its shape.
    assert_eq!(shown_size(v(300.0, 150.0), set(None, Some(100), None), 800.0), Some(v(200.0, 100.0)));
    // Both sides that aren't its shape: stretched, as set.
    assert_eq!(shown_size(v(300.0, 300.0), set(Some(Width::Px(300)), Some(150), None), 800.0), Some(v(300.0, 150.0)));
    // Wider than the room: the room, its shape kept.
    assert_eq!(shown_size(v(560.0, 400.0), set(Some(Width::Px(560)), None, None), 280.0), Some(v(280.0, 200.0)));
    // Nothing set: its own size, at most 900 wide.
    assert_eq!(shown_size(v(1200.0, 600.0), None, 2000.0), Some(v(900.0, 450.0)));
    // A spacer keeps its room; a tracking pixel has none.
    assert_eq!(shown_size(v(1.0, 1.0), set(Some(Width::Px(20)), Some(20), None), 800.0), Some(v(20.0, 20.0)));
    assert_eq!(shown_size(v(1.0, 1.0), None, 800.0), None);
    assert_eq!(shown_size(v(10.0, 10.0), set(Some(Width::Px(0)), Some(0), None), 800.0), None);
}

/// Reading pane › Bottom only hid the pane (issue #17): the list across the top took the size the
/// list beside it had saved, the whole height, under the same name.
#[test]
fn the_reading_pane_shows_below_the_list() {
    let (mut a, dir) = app("reading-bottom");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    // Where the reading pane's own Reply all shows (the ribbon's is higher up), if it does.
    let reply_all = |a: &mut MailApp| {
        let mut lowest = None;
        for _ in 0..3 {
            let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| a.ui(ui));
            out.textures_delta.clear();
            let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
            lowest = nodes.iter().filter(|(_, n)| n.label() == Some("Reply all")).filter_map(|(_, n)| n.bounds()).map(|b| b.y0).reduce(f64::max);
        }
        lowest
    };
    let right = reply_all(&mut a).unwrap_or(0.0);
    a.run("view.readingPane", json!({"value": "bottom"}));
    let bottom = reply_all(&mut a);
    assert!(bottom.is_some_and(|y| y > right + 150.0 && y < 760.0), "below the list, on screen: {bottom:?} (beside it: {right})");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #24: a message whose text came first shows it, its files named and sized, and says
/// they're downloading; the whole message, once it comes, takes its place.
#[test]
fn a_message_text_first_says_its_files_are_coming() {
    let (mut a, dir) = app("text-first");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    let d = mailcraft_engine::mime::Draft { subject: "Rushes".into(), body: "Here's the reel.".into(), ..Default::default() };
    let file = mailcraft_engine::mime::PendingFile { name: "reel.mov".into(), mime: "video/quicktime".into(), size: 12 << 20, ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example");
    let raw = mailcraft_engine::mime::text_first(&d, &from, NOW, "<r@x>", &[file]).unwrap();
    a.session.apply(mailcraft_engine::model::Event::Body { key: key.clone(), raw });
    let labels = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
        tree.nodes.iter().filter_map(|(_, n)| n.label().or(n.value()).map(str::to_string)).collect()
    };
    frame(&ctx, &mut a, vec![]);
    let shown = labels(&ctx, &mut a);
    assert!(shown.iter().any(|l| l.contains("Downloading the attachments")), "{shown:?}");
    assert!(shown.iter().any(|l| l.contains("reel.mov")));
    // The whole message: no more saying so.
    let full = a.session.store.as_ref().and_then(|st| st.read_body(&key)).unwrap();
    a.session.apply(mailcraft_engine::model::Event::Body { key, raw: full });
    frame(&ctx, &mut a, vec![]);
    let shown = labels(&ctx, &mut a);
    assert!(!shown.iter().any(|l| l.contains("Downloading the attachments")), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #29: a contact's name on a free-mail address shows that address in the list with a
/// warning sign, the reading pane says whose name it is, and the External tag shows when on.
#[test]
fn a_known_name_on_a_strange_address_shows_in_the_list_and_the_reading_pane() {
    let (mut a, dir) = app("known-name");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let d = mailcraft_engine::mime::Draft { subject: "Quick favour".into(), body: "Are you at your desk?".into(), ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new("Zoe Park", "zoe.park.director@gmail.com");
    let raw = mailcraft_engine::mime::build(&d, &from, NOW, "<favour@x>").unwrap();
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9900 };
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("view.externalTag", json!({"on": true}));
    let labels = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
        tree.nodes.iter().filter_map(|(_, n)| n.label().or(n.value()).map(str::to_string)).collect()
    };
    frame(&ctx, &mut a, vec![]);
    let shown = labels(&ctx, &mut a);
    let row = shown.iter().find(|l| l.contains("Quick favour")).cloned().unwrap_or_default();
    assert!(row.contains("Zoe Park <zoe.park.director@gmail.com>"), "{row}");
    assert!(row.contains("Check who sent it") && row.contains("External"), "{row}");
    // Zoe's own mail: her name alone, nothing to check, not external.
    let own = shown.iter().find(|l| l.starts_with("Zoe Park,")).cloned().unwrap_or_default();
    assert!(!own.is_empty(), "{shown:?}");
    assert!(!own.contains('<') && !own.contains("Check who sent it") && !own.contains("External"), "{own}");
    a.run("list.open", json!({"key": key.to_id()}));
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
    }
    let shown = labels(&ctx, &mut a);
    assert!(shown.iter().any(|l| l.contains("you know Zoe Park as zoe@studio.example")), "{shown:?}");
    assert!(shown.iter().any(|l| l == "External"), "{shown:?}");
    // Addresses always, never, and the compact list.
    for show in ["always", "never"] {
        a.run("view.senderAddresses", json!({"show": show}));
        a.run("view.compact", json!({}));
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #29: with the sender's address shown in a narrow list, the name keeps room enough to be
/// read: the address, laid out first, took up to seven tenths of the line and cut the name to
/// a few letters.
#[test]
fn a_narrow_row_keeps_the_senders_name_readable() {
    let (mut a, dir) = app("narrow-sender");
    let ctx = egui::Context::default();
    let name = "Alexandra Whitfield";
    let d = mailcraft_engine::mime::Draft { subject: "Invoice".into(), body: "Attached.".into(), ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new(name, "alexandra.whitfield.accounts@example-billing.com");
    let raw = mailcraft_engine::mime::build(&d, &from, NOW, "<narrow@x>").unwrap();
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9901 };
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(&raw, key, Default::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.unified", json!({"role": "inbox"}));
    a.run("view.senderAddresses", json!({"show": "always"}));
    a.run("view.externalTag", json!({"on": true}));
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 700.0))), ..Default::default() };
    let mut widths = Vec::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(input.clone(), |ui| a.ui(ui));
        out.textures_delta.clear();
        widths = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == name => Some(t.galley.size().x),
                _ => None,
            })
            .collect();
    }
    let full = ctx.fonts_mut(|f| f.layout_no_wrap(name.into(), crate::theme::regular(14.0), egui::Color32::WHITE).size().x);
    assert!(widths.iter().any(|w| *w >= full * 0.6), "the name drawn {widths:?} wide of {full}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The External tag stays one line in a pane too narrow for it (wrapped, its word stood in a
/// column a letter wide beside the sender's name), in English and Arabic.
#[test]
fn the_external_tag_never_wraps() {
    let ctx = egui::Context::default();
    let t = crate::theme::Tokens::light();
    for lang in ["en", "ar"] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let mut tag = egui::Rect::NOTHING;
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            ui.allocate_ui(egui::vec2(12.0, 200.0), |ui| tag = crate::reading::external_tag(ui, &t).rect);
        });
        out.textures_delta.clear();
        assert!(tag.height() < 22.0, "{lang}: {tag:?}");
        assert!(tag.width() > 30.0, "{lang}: {tag:?}");
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// Long text is cut in chunks at line ends, the break in neither, and a line longer than a
/// chunk at a character's edge: put back together, it is the text.
#[test]
fn long_text_is_cut_at_line_ends_and_character_edges() {
    use crate::reading::{CHUNK, chunks};
    let text = format!("{}{}\nend", "ab\ncd\n".repeat(2000), "\u{e9}".repeat(5000));
    let r = chunks(&text, CHUNK);
    assert!(r.len() > 3);
    assert!(r.iter().all(|c| !c.is_empty() && c.len() <= CHUNK && text.is_char_boundary(c.start) && text.is_char_boundary(c.end)));
    let (mut back, mut at) = (String::new(), 0);
    for c in &r {
        if c.start > at {
            assert_eq!(&text[at..c.start], "\n");
            back.push('\n');
        }
        back.push_str(&text[c.clone()]);
        at = c.end;
    }
    assert_eq!(back, text);
    assert!(r.iter().any(|c| text[c.clone()].chars().all(|ch| ch == '\u{e9}')), "the long line is cut");
    assert_eq!(chunks("short", CHUNK), vec![0..5]);
    assert!(chunks("", CHUNK).is_empty());
}

/// A message of a few hundred thousand characters, its lines numbered.
fn long_message(lines: usize) -> Vec<u8> {
    let body: String = (0..lines).map(|i| format!("Line {i:05} of a long message, with words enough to wrap now and then.\n")).collect();
    let d = mailcraft_engine::mime::Draft { subject: "Long".into(), body, ..Default::default() };
    let from = mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example");
    mailcraft_engine::mime::build(&d, &from, NOW, "<long@x>").unwrap()
}

/// `raw` as the body of the first message, opened; its text, and how many parts the first frame
/// that showed it laid out.
fn open_long(a: &mut MailApp, ctx: &egui::Context, raw: Vec<u8>) -> (String, usize) {
    frame(ctx, a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.session.apply(MailEvent::Body { key, raw });
    // Until it has come in (it fades up from a little below).
    let mut first = None;
    for _ in 0..30 {
        frame(ctx, a, vec![]);
        if first.is_none() && !a.ui.long_text.is_empty() {
            first = Some(long_state(a).laid);
        }
    }
    let text = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap();
    (text, first.expect("laid out by what's in view"))
}

fn long_state(a: &MailApp) -> &crate::reading::LongText {
    let mut all = a.ui.long_text.values();
    let v = all.next().expect("laid out by what's in view");
    assert!(all.next().is_none());
    v
}

/// A frame with `events`; what it copied (the last copy, which is what the clipboard keeps).
fn copied(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Option<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.platform_output
        .commands
        .into_iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
        .next_back()
}

/// A click at `at`, with `modifiers` held.
fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, modifiers: Modifiers) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(ctx, a, vec![Event::PointerMoved(at), Event::ModifiersChanged(modifiers)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false), Event::ModifiersChanged(Modifiers::NONE)]);
}

fn scroll_to_end(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    for _ in 0..60 {
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Page,
            delta: egui::vec2(0.0, -3.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        };
        frame(ctx, a, vec![Event::PointerMoved(at), wheel]);
    }
}

/// A 300,000-character message (127,000 took a 179 ms frame, laid out whole): the first frame
/// lays out at most 3 of its chunks, and scrolled to its end, the last one. A chunk's guessed
/// height is the one it's laid out at, so the text in view stays put as those above are measured.
#[test]
fn a_long_message_lays_out_what_is_in_view() {
    let (mut a, dir) = app("long-text");
    let ctx = egui::Context::default();
    let (text, first) = open_long(&mut a, &ctx, long_message(4400));
    assert!(text.len() > 300_000, "{}", text.len());
    let n = crate::reading::chunks(&text, crate::reading::CHUNK).len();
    assert!((1..=3).contains(&first), "{first} of {n} chunks laid out in the first frame");
    let v = long_state(&a);
    let measured: Vec<(f32, f32)> = v.heights().filter_map(|(g, m)| Some((g, m?))).collect();
    assert!(measured.len() > 3);
    // The last chunk ends the text, which ends with a line break of its own.
    for (g, m) in measured.iter().take(measured.len().min(n - 1)) {
        assert!((g - m).abs() < 3.0, "guessed {g}, laid out at {m}");
    }
    let at = v.shown.first().map(|(_, r)| egui::pos2(r.left() + 20.0, 400.0)).unwrap();
    scroll_to_end(&ctx, &mut a, at);
    let v = long_state(&a);
    assert!(v.shown.iter().any(|(i, _)| *i + 1 == n), "the last chunk, at the end: {:?}", v.shown);
    assert!(v.laid < n / 4, "{} of {n}", v.laid);
    let _ = std::fs::remove_dir_all(dir);
}

/// A selection from the first chunk to one far below (scrolled to, Shift+clicked) copies all the
/// text between, chunks passed over included; Ctrl+A in the text copies all of it.
#[test]
fn a_selection_over_chunks_passed_over_copies_them() {
    let (mut a, dir) = app("long-select");
    let ctx = egui::Context::default();
    let (text, _) = open_long(&mut a, &ctx, long_message(4400));
    let ranges = crate::reading::chunks(&text, crate::reading::CHUNK);
    let first = long_state(&a).shown.first().map(|(_, r)| *r).unwrap();
    click(&ctx, &mut a, first.left_top() + egui::vec2(2.0, 10.0), Modifiers::NONE);
    scroll_to_end(&ctx, &mut a, egui::pos2(first.left() + 20.0, 400.0));
    let (last, rect) = long_state(&a).shown.last().copied().unwrap();
    assert!(last > 10);
    let at = egui::pos2(rect.left() + 2.0, rect.center().y.clamp(300.0, 700.0));
    assert!(rect.contains(at));
    click(&ctx, &mut a, at, Modifiers::SHIFT);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    let before = ranges.get(last - 1).map(|r| r.end).unwrap();
    assert!(got.len() > before, "from the start into the chunk clicked: {} of {before}", got.len());
    assert!(text.starts_with(&got), "the text as it is");
    // Ctrl+A, then a copy: the whole text, and the mail selection is left alone.
    click(&ctx, &mut a, at, Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(a.ui.text_selection.all());
    assert_eq!(a.session.selection.len(), 1);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got == text, "{} of {}", got.len(), text.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Formatted mail of thousands of lines is laid out in blocks of lines in the same way.
#[test]
fn a_long_formatted_message_lays_out_what_is_in_view() {
    let (mut a, dir) = app("long-rich");
    let ctx = egui::Context::default();
    let mut raw = b"From: Sam <sam@studio.example>\r\nSubject: Long\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body>".to_vec();
    for i in 0..3000 {
        raw.extend_from_slice(format!("<div>Line {i:05} of a long <b>formatted</b> message.</div>\r\n").as_bytes());
    }
    raw.extend_from_slice(b"</body></html>\r\n");
    let (_, first) = open_long(&mut a, &ctx, raw);
    let lines = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.rich.len());
    assert!(lines > 2_000 && a.session.prefs.show_html, "{lines} lines");
    let n = lines.div_ceil(64);
    assert!((1..=6).contains(&first), "{first} of {n} blocks laid out");
    let v = long_state(&a);
    let at = v.shown.first().map(|(_, r)| egui::pos2(r.left() + 20.0, 400.0)).unwrap();
    scroll_to_end(&ctx, &mut a, at);
    let v = long_state(&a);
    assert!(v.shown.iter().any(|(i, _)| *i + 1 == n), "the last block, at the end: {:?}", v.shown);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A in the long text, then Ctrl+E to the search box: Ctrl+A and a copy there are the box's,
/// and no longer copy the whole message (that copy came last, so it was the one kept).
#[test]
fn ctrl_a_in_the_search_box_after_a_long_message_does_not_copy_it_whole() {
    let (mut a, dir) = app("long-select-all");
    let ctx = egui::Context::default();
    let (text, _) = open_long(&mut a, &ctx, long_message(4400));
    let first = long_state(&a).shown.first().map(|(_, r)| *r).unwrap();
    click(&ctx, &mut a, first.left_top() + egui::vec2(2.0, 10.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(a.ui.text_selection.all());
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_focused);
    frame(&ctx, &mut a, vec![Event::Text("zebra".into())]);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(!a.ui.text_selection.keys, "the search box has the keys");
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.len() < text.len(), "{} of {}", got.len(), text.len());
    let _ = std::fs::remove_dir_all(dir);
}

mod text_selection;
