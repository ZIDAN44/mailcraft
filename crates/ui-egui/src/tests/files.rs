//! Files from the desktop: pasted into a message, dropped on the main window (issue #10).

use super::*;
use std::path::{Path, PathBuf};

/// Services whose clipboard holds files copied in a file manager.
pub(super) struct Copied(pub(super) Vec<PathBuf>);

impl crate::Services for Copied {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
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
    let _ = std::fs::remove_dir_all(dir);
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
    let dropped: Vec<egui::DroppedFileHandle> = vec![std::sync::Arc::new(LetGo(file)), std::sync::Arc::new(LetGo("relative.txt".into()))];
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: dropped, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(attached(&a), ["brief.pdf"]);
    // Dropped again while it's being written: attached to it, not another message.
    let again = dir.join("notes.txt");
    std::fs::write(&again, b"n").unwrap();
    let id = a.session.compose.as_ref().map(|c| c.id);
    let dropped: Vec<egui::DroppedFileHandle> = vec![std::sync::Arc::new(LetGo(again))];
    let mut out = ctx.run_ui(RawInput { screen_rect: screen, dropped_files: dropped, ..Default::default() }, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.id), id);
    assert_eq!(attached(&a), ["brief.pdf", "notes.txt"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A backend that keeps the jobs it's given.
#[derive(Clone, Default)]
struct Kept(std::sync::Arc<std::sync::Mutex<Vec<mailcraft_engine::model::Job>>>);

impl crate::Backend for Kept {
    fn submit(&mut self, job: mailcraft_engine::model::Job) {
        if let Ok(mut j) = self.0.lock() {
            j.push(job);
        }
    }
    fn poll(&mut self) -> Vec<mailcraft_engine::model::Event> {
        Vec::new()
    }
}

/// Issue #38: Outlook message files and `.eml` files dropped on the main window go into the
/// folder open (it says so while they're held over it); other files dropped with them start a
/// message with them attached. What went in is said once the folder has them.
#[test]
fn message_files_dropped_on_the_window_go_into_the_folder_open() {
    let (mut a, dir) = app("drop-messages");
    let kept = Kept::default();
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
    let jobs = kept.0.lock().unwrap().clone();
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
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #56: a mail file opened to be looked at, picked in MailCraft itself. What it's inspected
/// for is there (who else it went to, what's attached, where a link really goes); nothing of it
/// is filed, and a file that's no message says so.
#[test]
fn a_mail_file_is_opened_to_be_looked_at() {
    struct Picks(std::path::PathBuf);
    impl crate::Services for Picks {
        fn clock(&self) -> (i64, i32) {
            (NOW, 0)
        }
        fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
        fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
        fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
            Ok(())
        }
        fn notify(&mut self, _: &str, _: &str) {}
        fn pick_mail_file(&mut self) -> Option<std::path::PathBuf> {
            Some(self.0.clone())
        }
    }
    let (mut a, dir) = app("open-mail-file");
    let kept = Kept::default();
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
    a.services = Box::new(Picks(eml));
    a.act(crate::Action::OpenMailFile);
    let Some(crate::Dialog::Preview { title, header, text, .. }) = &a.ui.dialog else { panic!("{:?}", a.session.notices) };
    assert_eq!(title, "Invoice");
    let row = |k: &str| header.iter().find(|(h, _)| h == k).map(|(_, v)| v.as_str());
    assert_eq!(row("Cc"), Some("Ben <ben@x.example>"));
    assert_eq!(row("Attachments"), Some("update.exe"));
    assert!(text.contains("Dear customer") && !text.contains("**"), "{text}");
    assert!(text.contains("https://bank.example/login <https://evil.example/login>"), "{text}");
    assert!(kept.0.lock().unwrap().is_empty(), "nothing fetched or filed: {:?}", kept.0.lock().unwrap());

    // Bytes that are no message: said, in one line that names it once.
    a.ui.dialog = None;
    let junk = dir.join("junk.eml");
    std::fs::write(&junk, [0xFFu8, 0xFE, 0x00, 0x01, 0x80].repeat(400)).unwrap();
    a.services = Box::new(Picks(junk));
    a.act(crate::Action::OpenMailFile);
    assert!(a.ui.dialog.is_none());
    let said = a.session.notices.last().unwrap();
    assert!(said.error && said.text.matches("junk.eml").count() == 1, "{}", said.text);

    // Past what's read: said by its name, not its path, and not as a program's error.
    let huge = dir.join("huge.eml");
    std::fs::File::create(&huge).unwrap().set_len(65 << 20).unwrap();
    a.services = Box::new(Picks(huge));
    a.act(crate::Action::OpenMailFile);
    assert_eq!(a.session.notices.last().unwrap().text, "huge.eml is too big");
    let _ = std::fs::remove_dir_all(dir);
}
