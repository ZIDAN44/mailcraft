//! Files from the desktop: pasted into a message, dropped on the main window (issue #10).

use super::*;
use std::path::{Path, PathBuf};

/// Services whose clipboard holds files copied in a file manager.
struct Copied(Vec<PathBuf>);

impl crate::Services for Copied {
    stub_services!();
    fn clipboard_files(&mut self) -> Vec<PathBuf> {
        self.0.clone()
    }
}

/// What the desktop hands over for one file let go over the window.
#[derive(Debug)]
struct LetGo(PathBuf);

impl egui::DroppedFile for LetGo {
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|why| format!("{}: {why}", self.0.display()))
    }

    fn path(&self) -> &Path {
        self.0.as_path()
    }
}

/// A clipboard holding files, or text.
struct Board(Vec<PathBuf>, Option<String>);

impl crate::ClipboardRead for Board {
    fn files(&mut self) -> Vec<PathBuf> {
        self.0.clone()
    }
    fn paste(&mut self) -> Option<Event> {
        self.1.clone().map(Event::Paste)
    }
}

/// A frame whose keys go through MailCraft's clipboard keys first, as the app's windows do.
fn keyed(ctx: &egui::Context, a: &mut MailApp, board: &mut Board, events: Vec<Event>) {
    let mut input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    crate::clipboard_keys(ctx, &mut input, board);
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

fn attached(a: &MailApp) -> Vec<String> {
    a.session.compose.as_ref().map(|c| c.attachments.iter().map(|x| x.name.clone()).collect()).unwrap_or_default()
}

/// Files copied in a file manager and pasted into the text are attached: on Linux, where their
/// names come as text too (and stay out of the text), and by the right-click Paste (Explorer's,
/// for which egui sends no paste). Text pasted with no files on the clipboard goes into the text.
#[test]
fn files_copied_in_a_file_manager_are_attached_when_pasted() {
    let (mut a, dir) = app("paste-copied");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    let (report, photo) = (dir.join("report.txt"), dir.join("photo one.webp"));
    std::fs::write(&report, b"numbers").unwrap();
    std::fs::write(&photo, b"RIFF....WEBP").unwrap();
    a.services = Box::new(Copied(vec![report.clone(), PathBuf::from("relative.txt")]));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let body = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    let before = a.session.compose.as_ref().unwrap().body.clone();
    // Linux: the paste brings the files' addresses as text.
    frame(&ctx, &mut a, vec![Event::Paste(format!("file://{}", report.display()))]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a), ["report.txt"], "the one with no path to read is left");
    assert_eq!(a.session.compose.as_ref().unwrap().body, before, "the file's address stays out of the text");
    // Explorer: the right-click Paste.
    a.services = Box::new(Copied(vec![photo]));
    a.ui.editor.as_mut().unwrap().paste = true;
    a.act(crate::Action::PasteFiles);
    assert!(!a.ui.editor.as_ref().unwrap().paste, "no text paste after it");
    assert_eq!(attached(&a), ["report.txt", "photo one.webp"]);
    // Ctrl+V with only the files there (Explorer, PCManFM: no text with them, issue #10): the
    // key is MailCraft's, and the files are looked for first.
    let ctrl_v = || vec![super::key(egui::Key::V, egui::Modifiers::COMMAND)];
    a.services = Box::new(Copied(Vec::new()));
    let brief = dir.join("brief.pdf");
    std::fs::write(&brief, b"%PDF-1.4").unwrap();
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    keyed(&ctx, &mut a, &mut Board(vec![brief.clone()], None), ctrl_v());
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a), ["report.txt", "photo one.webp", "brief.pdf"]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, before, "nothing typed");
    // None there: its text, as egui pasted it on the key.
    keyed(&ctx, &mut a, &mut Board(Vec::new(), Some("from the keys".into())), ctrl_v());
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().unwrap().body.contains("from the keys"));
    // Somewhere else with the keyboard: the text, the files not looked for.
    ctx.memory_mut(|m| m.surrender_focus(body));
    frame(&ctx, &mut a, vec![]);
    keyed(&ctx, &mut a, &mut Board(vec![brief], Some("elsewhere".into())), ctrl_v());
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a).len(), 3);
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    // Text alone pastes as text.
    a.services = Box::new(Copied(Vec::new()));
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![Event::Paste("hello there".into())]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().unwrap().body.contains("hello there"));
    assert_eq!(attached(&a).len(), 3);
}

/// Files dropped on the main window with nothing being written start a message with them
/// attached, as Outlook does; while they're held over it, it says so.
#[test]
fn files_dropped_on_the_main_window_start_a_message() {
    let (mut a, dir) = app("drop-main");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none());
    let file = dir.join("brief.pdf");
    std::fs::write(&file, b"%PDF-1.4").unwrap();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
    let held = RawInput {
        screen_rect: screen,
        hovered_files: vec![egui::HoveredFile { path: Some(file.clone()), mime: String::new() }],
        ..Default::default()
    };
    let mut out = ctx.run_ui(held, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(a.session.compose.is_none(), "held over it, nothing yet");
    // A folder alone: nothing to attach, no message.
    let folder: Vec<egui::DroppedFileHandle> = vec![std::sync::Arc::new(LetGo(dir.clone()))];
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: folder, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none(), "a folder starts no message");
    // A path relative to nowhere is left, even one that names a file from where MailCraft runs
    // (the tests run in the crate's folder): it isn't what was dropped.
    let relative = PathBuf::from("Cargo.toml");
    assert!(relative.is_file(), "a relative path to a file");
    let dropped: Vec<egui::DroppedFileHandle> = vec![std::sync::Arc::new(LetGo(file)), std::sync::Arc::new(LetGo(relative.clone()))];
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: dropped, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a), ["brief.pdf"]);
    // Dropped again while it's being written: attached to it, not another message (and again
    // a relative path is left, by the message's own form).
    let again = dir.join("notes.txt");
    std::fs::write(&again, b"n").unwrap();
    let id = a.session.compose.as_ref().map(|c| c.id);
    let dropped: Vec<egui::DroppedFileHandle> = vec![std::sync::Arc::new(LetGo(again)), std::sync::Arc::new(LetGo(relative))];
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: dropped, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.id), id);
    assert_eq!(attached(&a), ["brief.pdf", "notes.txt"]);
}

/// Issue #96: files handed to MailCraft to send (Windows' Send to, the file managers' Send with
/// MailCraft, the Dock icon) start a new message with them attached, one for each time they're
/// sent. A folder isn't attached, and says so; a file that can't be read is said over the
/// message; with no account, nothing starts and it says one is needed first.
#[test]
fn files_handed_to_send_start_a_message() {
    let (mut a, dir) = app("send-to");
    let (report, photo) = (dir.join("report.pdf"), dir.join("photo one.jpg"));
    std::fs::write(&report, b"%PDF-1.4").unwrap();
    std::fs::write(&photo, b"\xFF\xD8\xFF").unwrap();
    let path = |p: &Path| p.to_string_lossy().to_string();
    a.attach_handed(&[path(&report), path(&dir), path(&photo)]);
    assert_eq!(attached(&a), ["report.pdf", "photo one.jpg"]);
    let folder = dir.file_name().unwrap().to_string_lossy().to_string();
    assert!(a.session.notices.last().is_some_and(|n| n.error && n.text.contains(&folder)), "{:?}", a.session.notices.last());
    // Sent again: a message of its own.
    let first = a.session.compose.as_ref().map(|c| c.id);
    a.attach_handed(&[path(&photo)]);
    assert_ne!(a.session.compose.as_ref().map(|c| c.id), first);
    assert_eq!(attached(&a), ["photo one.jpg"]);
    // A folder alone: no message.
    let now = a.session.compose.as_ref().map(|c| c.id);
    a.attach_handed(&[path(&dir)]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.id), now, "a folder starts no message");
    // Gone by the time it's read: the message, and why it has nothing, over it.
    a.ui.compose_alert = None;
    a.attach_handed(&[path(&dir.join("gone.pdf"))]);
    assert_ne!(a.session.compose.as_ref().map(|c| c.id), now);
    assert!(attached(&a).is_empty() && a.ui.compose_alert.is_some());
    // No account yet.
    let (mut none, _) = app("send-to-no-account");
    none.session.accounts.clear();
    none.attach_handed(&[path(&report)]);
    assert!(none.session.compose.is_none());
    assert!(none.session.notices.last().is_some_and(|n| n.error && n.text.starts_with("Add an account first")), "{:?}", none.session.notices.last());
}

/// Issue #96: the window of a message started from files sent to MailCraft comes to the front
/// once (found on Linux: it opened behind the main window, which the launch had brought
/// forward); one started with New mail is left where it opens.
#[test]
fn a_message_from_files_sent_comes_to_the_front() {
    let (mut a, dir) = app("send-to-front");
    a.session.prefs.new_mail_window = true;
    let file = dir.join("report.pdf");
    std::fs::write(&file, b"%PDF-1.4").unwrap();
    let ctx = egui::Context::default();
    // Windows of their own, as on the desktop (not drawn in the main one).
    ctx.set_embed_viewports(false);
    let focused = |a: &mut MailApp, frames: usize| {
        let mut n = 0;
        for _ in 0..frames {
            let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| a.ui(ui));
            out.textures_delta.clear();
            n += out.viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, egui::ViewportCommand::Focus)).count();
        }
        n
    };
    assert_eq!(focused(&mut a, 3), 0);
    a.run("mail.new", json!({}));
    assert_eq!(focused(&mut a, 6), 0, "New mail: where it opens");
    a.attach_handed(&[file.to_string_lossy().to_string()]);
    assert_eq!(attached(&a), ["report.pdf"]);
    assert_eq!(focused(&mut a, 6), 1, "brought forward once");
    assert_eq!(focused(&mut a, 6), 0, "and left alone after");
}

/// Found testing issue #96 on X11 (Openbox): started for files to send, MailCraft's main window,
/// shown once it had drawn, took the focus 40 ms after the message's window had it, so the
/// message stayed behind (a mail file MailCraft was started with too). The message comes forward
/// again once, if nothing was clicked or typed in the main window first; then it's left alone.
/// The message's window is a window of its own here, with its own input, as eframe runs it.
#[test]
fn a_message_window_comes_back_in_front_of_the_main_window_once() {
    use std::cell::Cell;
    use std::rc::Rc;
    // Whether the main window has the focus (the message's has it otherwise), and how many
    // times the message's window asked for it.
    let main = Rc::new(Cell::new(false));
    let asked = Rc::new(Cell::new(0usize));
    let focus = |out: &egui::FullOutput| {
        out.viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, egui::ViewportCommand::Focus)).count()
    };
    {
        let (main, asked) = (Rc::clone(&main), Rc::clone(&asked));
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
            let egui::ImmediateViewport { ids, mut viewport_ui_cb, .. } = viewport;
            let mut input = RawInput {
                viewport_id: ids.this,
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 720.0))),
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(main.get());
            input.viewports.entry(ids.this).or_default().focused = Some(!main.get());
            let mut out = ctx.run_ui(input, |ui| viewport_ui_cb(ui));
            out.textures_delta.clear();
            asked.set(asked.get() + focus(&out));
        });
    }
    // A frame of the main window, with the focus there or not and what was done there.
    let frame = |ctx: &egui::Context, a: &mut MailApp, focused: bool, events: Vec<Event>| {
        main.set(focused);
        let mut input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(focused);
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        assert_eq!(focus(&out), 0, "the main window isn't brought forward");
        asked.replace(0)
    };
    let started = |name: &str| {
        let (mut a, dir) = app(name);
        a.session.prefs.new_mail_window = true;
        let file = dir.join("report.pdf");
        std::fs::write(&file, b"%PDF-1.4").unwrap();
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        a.attach_handed(&[file.to_string_lossy().to_string()]);
        let first: usize = (0..3).map(|_| frame(&ctx, &mut a, false, vec![])).sum();
        assert_eq!(first, 1, "{name}: in front as it opens");
        (a, ctx)
    };
    // The main window takes the focus: the message comes back, once.
    let (mut a, ctx) = started("send-to-taken");
    assert_eq!(frame(&ctx, &mut a, true, vec![]), 1, "back in front");
    let after: usize = (0..4).map(|_| frame(&ctx, &mut a, true, vec![])).sum();
    assert_eq!(after, 0, "then left alone");
    // Something clicked in the main window first: that's where the user is.
    let (mut b, ctx) = started("send-to-clicked");
    let click =
        Event::PointerButton { pos: egui::pos2(600.0, 400.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
    assert_eq!(frame(&ctx, &mut b, true, vec![click]), 0);
    let after: usize = (0..4).map(|_| frame(&ctx, &mut b, true, vec![])).sum();
    assert_eq!(after, 0, "left in the main window");
}

/// Issue #38: Outlook message files and `.eml` files dropped on the main window go into the
/// folder open (it says so while they're held over it); other files dropped with them start a
/// message with them attached. What went in is said once the folder has them.
#[test]
fn message_files_dropped_on_the_window_go_into_the_folder_open() {
    let (mut a, dir) = app("drop-messages");
    let kept = Jobs::default();
    a.backend = Box::new(kept.clone());
    let ctx = egui::Context::default();
    a.session.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    frame(&ctx, &mut a, vec![]);
    let (msg, eml, pdf) = (dir.join("figures.msg"), dir.join("hello.eml"), dir.join("brief.pdf"));
    for (p, b) in [(&msg, &b"\xD0\xCF\x11\xE0"[..]), (&eml, b"Subject: x\r\n\r\ny"), (&pdf, b"%PDF-1.4")] {
        std::fs::write(p, b).unwrap();
    }
    let dropped: Vec<egui::DroppedFileHandle> =
        vec![std::sync::Arc::new(LetGo(msg.clone())), std::sync::Arc::new(LetGo(eml.clone())), std::sync::Arc::new(LetGo(pdf))];
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: dropped, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    let jobs = kept.take();
    let filed: Vec<&Vec<String>> = jobs
        .iter()
        .filter_map(|j| {
            if let mailcraft_engine::model::Job::FileMessages { folder, paths, .. } = j { (folder == "INBOX").then_some(paths) } else { None }
        })
        .collect();
    assert_eq!(filed, [&vec![msg.to_string_lossy().into_owned(), eml.to_string_lossy().into_owned()]], "{jobs:?}");
    assert_eq!(attached(&a), ["brief.pdf"], "the rest attached to a new message");
    a.session.apply(mailcraft_engine::model::Event::MessagesFiled { account: "sample".into(), folder: "INBOX".into(), added: 2, failed: Vec::new() });
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.notices.last().is_some_and(|n| n.text.contains('2') && !n.error), "{:?}", a.session.notices.last());
}

/// Issue #56: the pictures a mail file opened to be looked at carries in itself are shown, as a
/// message's from a folder are: it's in no cache, so they come out of what was read of the file.
#[test]
fn a_mail_files_own_pictures_are_shown() {
    use mailcraft_engine::mime::{self, Draft, InlinePicture};
    let (mut a, dir) = app("open-mail-file-pictures");
    std::fs::create_dir_all(&dir).unwrap();
    let d = Draft {
        subject: "Stamped".into(),
        body: "Stamp".into(),
        html: Some("<p>Stamp <img src=\"cid:stamp@x\" alt=\"stamp\"></p>".into()),
        inline: vec![InlinePicture { cid: "stamp@x".into(), name: "stamp.png".into(), mime: "image/png".into(), data: super::pictures::png(40, 20) }],
        ..Default::default()
    };
    let raw = mime::build(&d, &mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example"), NOW, "<stamped@x>").unwrap();
    let eml = dir.join("stamped.eml");
    std::fs::write(&eml, raw).unwrap();
    a.open_handed(&eml.to_string_lossy());
    let w = a.session.windows.first().cloned().unwrap_or_else(|| panic!("{:?}", a.session.notices));
    let index = w.parsed.as_ref().and_then(|p| mime::picture_attachment(p, "cid:stamp@x")).expect("the picture is the message's");
    let ctx = egui::Context::default();
    let mut got = false;
    for _ in 0..40 {
        drawn_text(&ctx, &mut a, egui::vec2(1600.0, 1000.0));
        if matches!(a.ui.pictures.get(&w.key, index, crate::reading::Want::Body), crate::reading::Got::Ready(_)) {
            got = true;
            break;
        }
    }
    assert!(got, "the file's own picture is shown");
}

/// Issue #56: a mail file opened to be looked at, picked in MailCraft itself, shows in a message
/// window as one double-clicked in a folder does. What it's inspected for is there (who else it
/// went to, what's attached, where a link really goes); nothing of it is filed or fetched, its
/// toolbar answers or files nothing, and a file that's no message says so.
#[test]
fn a_mail_file_is_opened_to_be_looked_at() {
    struct PicksMailFile(std::path::PathBuf);
    impl crate::Services for PicksMailFile {
        stub_services!();
        fn pick_mail_file(&mut self) -> Option<std::path::PathBuf> {
            Some(self.0.clone())
        }
    }
    let (mut a, dir) = app("open-mail-file");
    let kept = Jobs::default();
    a.backend = Box::new(kept.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("inspect me.eml");
    std::fs::write(
        &eml,
        "From: Billing <billing@paypa1.example>\r\nTo: Ana <ana@x.example>\r\nCc: Ben <ben@x.example>\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>Dear <b>customer</b></p>\
         <a href=\"https://evil.example/login\">https://bank.example/login</a><img src=\"https://t.example/o.gif\">\r\n\
         --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=update.exe\r\n\r\nMZ\r\n--b--\r\n",
    )
    .unwrap();
    a.services = Box::new(PicksMailFile(eml));
    // Even from a sender whose pictures always load.
    a.session.prefs.trusted_senders.insert("billing@paypa1.example".into());
    a.act(crate::Action::OpenMailFile);
    assert!(a.ui.dialog.is_none(), "a window, not a dialog");
    let key = a.session.windows.first().map(|w| w.key.clone()).unwrap_or_else(|| panic!("{:?}", a.session.notices));
    let ctx = egui::Context::default();
    let size = egui::vec2(1600.0, 1000.0);
    for _ in 0..3 {
        drawn_text(&ctx, &mut a, size);
    }
    // What the window draws: from its title on.
    let drawn: Vec<String> = drawn_strings(&ctx, &mut a, size).into_iter().skip_while(|t| t != "Invoice – Message").collect();
    let has = |want: &str| drawn.iter().any(|t| t.contains(want));
    // Its pictures from the web are there to ask for, this once: not to always show its sender's,
    // who may be anyone.
    for want in [
        "Invoice – Message",
        "Print",
        "Ben",
        "update.exe",
        "customer",
        " <https://evil.example/login>",
        "Opened from inspect me.eml",
        "Show pictures",
    ] {
        assert!(has(want), "{want:?} not drawn: {drawn:?}");
    }
    for gone in ["Reply", "Archive", "Move to", "Always show from", "1970"] {
        assert!(!has(gone), "{gone:?} drawn: {drawn:?}");
    }
    // (The window's own search index may be built meanwhile.)
    let asked = kept.take();
    assert!(asked.iter().all(|j| matches!(j, mailcraft_engine::model::Job::Index { .. })), "nothing fetched or filed: {asked:?}");
    // In a narrow window everything wraps within it: the line saying where it was opened from
    // didn't, and widened the whole message past the window's edge.
    let narrow = egui::vec2(640.0, 1000.0);
    for _ in 0..3 {
        drawn_text(&ctx, &mut a, narrow);
    }
    let past: Vec<(String, egui::Rect)> = drawn_text(&ctx, &mut a, narrow)
        .into_iter()
        .skip_while(|d| d.text != "Invoice – Message")
        .filter(|d| d.ink.max.x > narrow.x + 1.0)
        .map(|d| (d.text, d.ink))
        .collect();
    assert!(past.is_empty(), "drawn past the window: {past:?}");
    // Its buttons act on it: saved as a PDF, it's this message.
    let pdf = dir.join("invoice.pdf");
    a.act(crate::Action::InWindow(key.clone(), Box::new(crate::Action::Run("message.exportPdf", json!({"path": pdf.to_string_lossy()})))));
    assert!(std::fs::read(&pdf).is_ok_and(|b| b.starts_with(b"%PDF")), "{:?}", a.session.notices);
    // Closed: gone, file and all.
    a.act(crate::Action::Run("message.closeWindow", json!({"key": key.to_id()})));
    assert!(a.session.windows.is_empty() && a.session.opened_file_name(&key).is_none());

    // Bytes that are no message: said, in one line that names it once.
    let junk = dir.join("junk.eml");
    std::fs::write(&junk, [0xFFu8, 0xFE, 0x00, 0x01, 0x80].repeat(400)).unwrap();
    a.services = Box::new(PicksMailFile(junk));
    a.act(crate::Action::OpenMailFile);
    assert!(a.ui.dialog.is_none());
    let said = a.session.notices.last().unwrap();
    assert!(said.error && said.text.matches("junk.eml").count() == 1, "{}", said.text);

    // Past what's read: said by its name, not its path, and not as a program's error.
    let huge = dir.join("huge.eml");
    std::fs::File::create(&huge).unwrap().set_len(65 << 20).unwrap();
    a.services = Box::new(PicksMailFile(huge));
    a.act(crate::Action::OpenMailFile);
    assert_eq!(a.session.notices.last().unwrap().text, "huge.eml is too big");
}

/// A mail file handed to MailCraft opens in front: its window is brought forward once, when it's
/// on screen (it opened behind the main window, which the launch handing it over had just
/// brought forward), and not again while it stays open. Opened again after closing, again.
#[test]
fn a_mail_files_window_comes_to_the_front() {
    let (mut a, dir) = app("open-mail-file-front");
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("note.eml");
    std::fs::write(&eml, "From: Sam <sam@studio.example>\r\nSubject: Note\r\nMessage-ID: <note@x>\r\n\r\nHello.\r\n").unwrap();
    let ctx = egui::Context::default();
    let focused = |a: &mut MailApp, frames: usize| {
        let mut n = 0;
        for _ in 0..frames {
            let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| a.ui(ui));
            out.textures_delta.clear();
            n += out.viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, egui::ViewportCommand::Focus)).count();
        }
        n
    };
    assert_eq!(focused(&mut a, 4), 0, "nothing to bring forward yet");
    a.open_handed(&eml.to_string_lossy());
    let key = a.session.windows.first().map(|w| w.key.clone()).unwrap_or_else(|| panic!("{:?}", a.session.notices));
    assert_eq!(focused(&mut a, 6), 1, "brought forward once");
    assert_eq!(focused(&mut a, 6), 0, "and left alone after");
    a.run("message.closeWindow", json!({"key": key.to_id()}));
    assert_eq!(focused(&mut a, 3), 0);
    a.open_handed(&eml.to_string_lossy());
    assert_eq!(focused(&mut a, 6), 1, "opened again: in front again");
}

/// A message dragged from the list onto the message being written (issue #270) is attached to
/// it, whole, an `.eml`: here a reply written in the reading pane, its own message dragged onto
/// it; the drag says it attaches. Let go anywhere else in the message's room, nothing is.
#[test]
fn a_message_dragged_onto_the_message_is_attached() {
    let (mut a, _) = app("drag-attach");
    a.session.prefs.reply_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let m = a.session.list.first().cloned().expect("a message");
    a.run("list.open", json!({"key": m.key.to_id()}));
    a.run("mail.reply", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let size = crate::tests::windows::layout::SIZE.with(std::cell::Cell::get);
    let row = drawn_text(&ctx, &mut a, size).into_iter().filter(|d| d.text == m.subject).min_by(|x, y| x.rect.left().total_cmp(&y.rect.left()));
    let from = row.expect("its row in the list").rect.center();
    let (_, area) = a.ui.compose_area.expect("the reply in the reading pane");
    let to = area.center();
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    for k in 1..=4 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    assert!(crate::reading::dragged::over_message(&ctx), "the drag says it attaches");
    frame(&ctx, &mut a, vec![press(to, false)]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().expect("the reply");
    assert_eq!(c.attachments.len(), 1, "{:?}", c.attachments.iter().map(|x| &x.name).collect::<Vec<_>>());
    assert_eq!(c.attachments[0].mime, "message/rfc822");
    assert!(!egui::DragAndDrop::has_any_payload(&ctx), "the drag is over");
}
