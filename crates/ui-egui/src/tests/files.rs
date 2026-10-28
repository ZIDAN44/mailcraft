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

fn attached(a: &MailApp) -> Vec<String> {
    a.session.compose.as_ref().map(|c| c.attachments.iter().map(|x| x.name.clone()).collect()).unwrap_or_default()
}

/// Files copied in a file manager and pasted into the text are attached: on Linux, where their
/// names come as text too (and stay out of the text), and by the right-click Paste (Explorer's,
/// for which egui sends no paste). Text pasted with no files on the clipboard goes into the text.
#[test]
fn files_copied_in_a_file_manager_are_attached_when_pasted() {
    let (mut a, dir) = app("paste-files");
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
    // Text alone pastes as text.
    a.services = Box::new(Copied(Vec::new()));
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![Event::Paste("hello there".into())]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().unwrap().body.contains("hello there"));
    assert_eq!(attached(&a).len(), 2);
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
