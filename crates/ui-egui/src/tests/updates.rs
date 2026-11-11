//! A newer MailCraft on its way: the bar and the notification when it's ready, and what closing
//! says about it.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::{Action, UpdateState};

/// What the fake updater was asked.
#[derive(Default)]
struct Asked {
    state: UpdateState,
    notified: Vec<String>,
    later: bool,
    restart: bool,
    /// MailCraft's icon is in the notification area.
    tray: bool,
}

struct Updating(Rc<RefCell<Asked>>);

impl crate::Services for Updating {
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
        Err("no".into())
    }
    fn update_state(&mut self) -> UpdateState {
        self.0.borrow().state.clone()
    }
    fn restart_to_update(&mut self) -> bool {
        let mut a = self.0.borrow_mut();
        a.restart = a.state.ready().is_some();
        a.restart
    }
    fn keep_update_for_later(&mut self) {
        self.0.borrow_mut().later = true;
    }
    fn tray_offered(&self) -> bool {
        true
    }
    fn tray_shown(&self) -> bool {
        self.0.borrow().tray
    }
    fn notify(&mut self, title: &str, _body: &str) {
        self.0.borrow_mut().notified.push(title.to_string());
    }
}

fn updating(a: &mut MailApp) -> Rc<RefCell<Asked>> {
    let asked = Rc::new(RefCell::new(Asked::default()));
    a.services = Box::new(Updating(asked.clone()));
    asked
}

fn ready(admin: bool) -> UpdateState {
    UpdateState::Ready { version: "0.25.0-test.22".into(), asks: if admin { crate::InstallAsks::Administrator } else { crate::InstallAsks::Nothing } }
}

/// The window asked to close, as its close button does.
fn close(ctx: &egui::Context, a: &mut MailApp) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// The frames with the update in each state, for the tour (light and dark, every language).
pub(crate) fn tour(a: &mut MailApp, ctx: &egui::Context) {
    let asked = updating(a);
    for state in [UpdateState::Downloading { version: "0.25.0".into(), done: 3 << 20, total: 30 << 20 }, ready(true), ready(false)] {
        asked.borrow_mut().state = state.clone();
        frame(ctx, a, vec![]);
        a.ui.dialog = Some(Dialog::UpdateOnClose(state));
        frame(ctx, a, vec![]);
        a.ui.dialog = None;
        a.act(Action::Backstage(crate::backstage::Page::About));
        frame(ctx, a, vec![]);
        a.ui.backstage = None;
    }
    a.services = Box::new(NullServices(NOW));
    a.ui.update_told = None;
    frame(ctx, a, vec![]);
}

/// Ready: a bar to restart into it (put away with Later), and the system told once, only when
/// MailCraft isn't in front.
#[test]
fn a_ready_update_is_said_once() {
    let (mut a, dir) = app("update-ready");
    let asked = updating(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(!a.update_bar_wanted());
    asked.borrow_mut().state = UpdateState::Downloading { version: "0.25.0".into(), done: 1, total: 4 };
    frame(&ctx, &mut a, vec![]);
    assert!(!a.update_bar_wanted(), "downloading is said in the status bar only");
    asked.borrow_mut().state = ready(false);
    // MailCraft in the background: the system's notification.
    let behind =
        RawInput { focused: false, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(behind, |ui| a.ui(ui));
    out.textures_delta.clear();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(asked.borrow().notified, ["MailCraft 0.25.0-test.22 is ready"], "told once");
    assert!(a.update_bar_wanted());
    a.ui.update_later = Some("0.25.0-test.22".into());
    frame(&ctx, &mut a, vec![]);
    assert!(!a.update_bar_wanted(), "Later puts it away");
    // A newer one ready: said again.
    asked.borrow_mut().state = UpdateState::Ready { version: "0.25.0-test.23".into(), asks: crate::InstallAsks::Nothing };
    frame(&ctx, &mut a, vec![]);
    assert!(a.update_bar_wanted());
    assert_eq!(asked.borrow().notified.len(), 1, "in front: the bar says it");
    // Restart now: closing asks nothing more, and the restart holds once the window closes.
    a.act(Action::RestartToUpdate);
    assert!(a.ui.close_now && a.ui.update_answered && !asked.borrow().restart, "not before the close goes ahead");
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none() && asked.borrow().restart);
    let _ = std::fs::remove_dir_all(dir);
}

/// Restart now with a message being written: the save question comes first, and Cancel there
/// leaves MailCraft as it was (no restart, and the next close asks about the update again).
#[test]
fn restart_now_can_still_be_called_off() {
    let (mut a, dir) = app("update-restart-cancel");
    let asked = updating(&mut a);
    asked.borrow_mut().state = ready(false);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Half done"}));
    a.act(Action::RestartToUpdate);
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "{:?}", a.ui.dialog);
    // Cancel.
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    assert!(!a.ui.restart_wanted && !a.ui.update_answered && !asked.borrow().restart);
    // Close again, and Save there (as its button does): the update is asked about again, and
    // nothing restarts.
    close(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveOnClose(1))), "{:?}", a.ui.dialog);
    a.ui.dialog = None;
    a.act(Action::Run("compose.keepAll", json!({})));
    a.act(Action::CloseAnyway);
    assert!(matches!(a.ui.dialog, Some(Dialog::UpdateOnClose(_))), "{:?}", a.ui.dialog);
    a.act(Action::UpdateOnClose(true));
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(!asked.borrow().restart, "installed as it closes, not reopened");
    let _ = std::fs::remove_dir_all(dir);
}

/// Closing with an update ready asks what becomes of it; Close without updating keeps it for
/// next time. Closing while it downloads says the download carries on next time, and the question
/// follows the download if it finishes meanwhile.
#[test]
fn closing_says_what_becomes_of_the_update() {
    let (mut a, dir) = app("update-close");
    let asked = updating(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "nothing on its way: nothing asked");

    asked.borrow_mut().state = ready(true);
    a.ui.closing_anyway = false;
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::UpdateOnClose(s)) if s.ready().is_some()), "asked");
    a.ui.dialog = None;
    a.act(Action::UpdateOnClose(false));
    assert!(asked.borrow().later && a.ui.close_now, "kept for later, and closing");

    let (mut a, dir2) = app("update-close-downloading");
    let asked = updating(&mut a);
    asked.borrow_mut().state = UpdateState::Downloading { version: "0.25.0".into(), done: 1 << 20, total: 4 << 20 };
    frame(&ctx, &mut a, vec![]);
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::UpdateOnClose(UpdateState::Downloading { .. }))), "{:?}", a.ui.dialog);
    asked.borrow_mut().state = ready(false);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::UpdateOnClose(s)) if s.ready().is_some()), "the question follows the download");
    a.ui.dialog = None;

    // Close anyway (online-only accounts' warning) still says what becomes of the update.
    a.act(Action::CloseAnyway);
    assert!(matches!(&a.ui.dialog, Some(Dialog::UpdateOnClose(_))) && !a.ui.close_now);
    // Cancel there: the next close asks everything again (online-only mail isn't dropped).
    a.ui.dialog = None;
    assert!(!a.ui.closing_anyway, "cancelled, nothing is closing");
    close(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::UpdateOnClose(_))), "asked again");
    a.act(Action::UpdateOnClose(true));
    assert!(!asked.borrow().later && a.ui.close_now && a.ui.update_answered);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// A start that brought the mail over from MailCraft's Flatpak under its id before 0.36.
struct CarriedOver;

impl crate::Services for CarriedOver {
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
        Err("no".into())
    }
    fn carried_over_from(&self) -> Option<&'static str> {
        Some("ai.storyteller.mailcraft")
    }
}

/// The texts drawn in a frame (AccessKit on) and where each is: `(label, centre)`.
fn drawn(ctx: &egui::Context, a: &mut MailApp, events: Vec<egui::Event>) -> Vec<(String, egui::Pos2)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let label = n.label().or(n.value())?.to_string();
            Some((label, egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)))
        })
        .collect()
}

/// A bar says the mail came over from the old Flatpak and how to remove that one, until it's
/// dismissed; a start that brought nothing over has no bar.
#[test]
fn the_old_flatpak_is_named_once_its_mail_came_over() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut a = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(CarriedOver));
    drawn(&ctx, &mut a, vec![]);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(texts.iter().any(|(t, _)| t.contains("flatpak uninstall --delete-data ai.storyteller.mailcraft")), "the bar says how to remove it");
    let at = texts.iter().find(|(t, _)| t == "Dismiss").map(|(_, p)| *p).expect("a Dismiss button");
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    drawn(&ctx, &mut a, vec![egui::Event::PointerMoved(at), press(true)]);
    drawn(&ctx, &mut a, vec![press(false)]);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.old_flatpak, None);
    assert!(!texts.iter().any(|(t, _)| t.contains("flatpak uninstall")));
    let b = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(NullServices(NOW)));
    assert_eq!(b.ui.old_flatpak, None);
}

/// Issue #67: with "Keep MailCraft running when its window is closed" on, and its icon in the
/// notification area, closing the window hides it and ends nothing; Exit closes it, and asks
/// what a close asks. Off, or with no icon to come back from, a close is a close.
#[test]
fn closing_the_window_keeps_mailcraft_running_when_asked_to() {
    let (mut a, dir) = app("keep-running");
    let asked = updating(&mut a);
    asked.borrow_mut().tray = true;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // The window asked to close: what it's told to do then.
    let closed = |a: &mut MailApp| {
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() },
            ))
            .collect(),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()
    };
    let hides = |commands: &[egui::ViewportCommand]| {
        commands.contains(&egui::ViewportCommand::CancelClose) && commands.contains(&egui::ViewportCommand::Visible(false))
    };
    assert!(!hides(&closed(&mut a)) && !a.ui.in_tray, "off: the window closes");
    a.run("view.closeToTray", json!({"on": true}));
    assert!(hides(&closed(&mut a)) && a.ui.in_tray, "on: hidden, still running");
    a.show_window(&ctx);
    assert!(!a.ui.in_tray);
    // A message being written doesn't stop the window hiding: nothing is lost by it.
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Later", "body": "Not sent yet."}));
    assert!(hides(&closed(&mut a)) && a.ui.dialog.is_none());
    // Exit asks about it, as a close does, with the window back; cancelled, the window hides again.
    a.exit(&ctx);
    let commands = closed(&mut a);
    assert!(!hides(&commands) && matches!(&a.ui.dialog, Some(Dialog::SaveOnClose(1))), "{commands:?}");
    assert!(commands.contains(&egui::ViewportCommand::Visible(true)) && !a.ui.in_tray, "the question is shown: {commands:?}");
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    assert!(hides(&closed(&mut a)), "cancelled: closing hides again");
    // No icon there (it couldn't be made): closing closes, rather than leave a window nobody can find.
    a.show_window(&ctx);
    a.run("compose.discard", json!({}));
    asked.borrow_mut().tray = false;
    assert!(!hides(&closed(&mut a)));
    let _ = std::fs::remove_dir_all(dir);
}
