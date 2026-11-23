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
    /// What a close does when MailCraft is kept running.
    tray: Option<crate::KeptOnClose>,
}

struct Updating(Rc<RefCell<Asked>>);

impl crate::Services for Updating {
    stub_services!(clock, pick_files, save_path);
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
    fn keeps_running(&self) -> Option<crate::KeptOnClose> {
        Some(crate::KeptOnClose::Hidden)
    }
    fn kept_on_close(&self) -> Option<crate::KeptOnClose> {
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
    let (mut a, _) = app("update-ready");
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
}

/// Restart now with a message being written: the save question comes first, and Cancel there
/// leaves MailCraft as it was (no restart, and the next close asks about the update again).
#[test]
fn restart_now_can_still_be_called_off() {
    let (mut a, _) = app("update-restart-cancel");
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
}

/// Closing with an update ready asks what becomes of it; Close without updating keeps it for
/// next time. Closing while it downloads says the download carries on next time, and the question
/// follows the download if it finishes meanwhile.
#[test]
fn closing_says_what_becomes_of_the_update() {
    let (mut a, _) = app("update-close");
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

    let (mut a, _) = app("update-close-downloading");
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
}

/// A start that brought the mail over from MailCraft's Flatpak under its id before 0.36.
struct CarriedOver;

impl crate::Services for CarriedOver {
    stub_services!(clock, pick_files, save_path);
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("no".into())
    }
    fn carried_over_from(&self) -> Option<&'static str> {
        Some("ai.storyteller.mailcraft")
    }
}

/// The texts drawn in a frame (AccessKit on) and where each is: `(label, centre)`.
fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2)> {
    named(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter_map(|n| Some((n.name()?.to_string(), n.centre()?))).collect()
}

/// A bar says the mail came over from the old Flatpak and how to remove that one, until it's
/// dismissed; a start that brought nothing over has no bar.
#[test]
fn the_old_flatpak_is_named_once_its_mail_came_over() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut a = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(CarriedOver));
    // Settled: the bar's button finds its place over the first frames (clicked from the
    // second's, the press missed it).
    for _ in 0..3 {
        drawn(&ctx, &mut a);
    }
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|(t, _)| t.contains("flatpak uninstall --delete-data ai.storyteller.mailcraft")), "the bar says how to remove it");
    let at = texts.iter().find(|(t, _)| t == "Dismiss").map(|(_, p)| *p).expect("a Dismiss button");
    click(&ctx, &mut a, at);
    let texts = drawn(&ctx, &mut a);
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
    let (mut a, _) = app("keep-running");
    let asked = updating(&mut a);
    asked.borrow_mut().tray = Some(crate::KeptOnClose::Hidden);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // The window asked to close: what it's told to do then.
    let closed = |a: &mut MailApp| close(&ctx, a).viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
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
    // Where a hidden window couldn't be brought back (Wayland, no notification area), it's
    // minimized: the taskbar is the way back.
    a.show_window(&ctx);
    a.run("compose.discard", json!({}));
    asked.borrow_mut().tray = Some(crate::KeptOnClose::Minimized);
    let commands = closed(&mut a);
    assert!(commands.contains(&egui::ViewportCommand::CancelClose) && commands.contains(&egui::ViewportCommand::Minimized(true)), "{commands:?}");
    assert!(!hides(&commands) && !a.ui.in_tray);
    // Where the app can't keep running at all, a close is a close.
    asked.borrow_mut().tray = None;
    assert!(!closed(&mut a).contains(&egui::ViewportCommand::CancelClose));
}

/// Issues #116 and #117, with MailCraft kept running in the notification area: its icon is told
/// the Inboxes' unread mail (none with that off); started put away, MailCraft is in the
/// notification area, and comes back minimized where no icon could be made to bring it back; the
/// File page offers both under keeping it running (the count not on a Mac, whose Dock's icon has
/// it already), and neither without it.
#[test]
fn the_tray_icon_counts_unread_mail_and_mailcraft_can_start_there() {
    let (mut a, _) = app("tray-unread");
    let asked = updating(&mut a);
    asked.borrow_mut().tray = Some(crate::KeptOnClose::Hidden);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let unread = a.session.inbox_unread();
    assert!(unread > 0, "the sample has unread mail");
    assert_eq!(a.tray_words().3, unread, "the icon's count is the title's");
    a.run("view.trayUnread", json!({"on": false}));
    assert_eq!(a.tray_words().3, 0, "off: a plain icon");
    a.run("view.trayUnread", json!({"on": true}));
    // Started put away: in the notification area; with no icon there, back within reach.
    a.started_away(true);
    assert!(a.in_tray());
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| a.back_minimized(ui.ctx()));
    out.textures_delta.clear();
    let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::Visible(true)) && commands.contains(&egui::ViewportCommand::Minimized(true)), "{commands:?}");
    assert!(!a.in_tray());
    // The File page's options: under keeping it running, once that's on.
    let tall = egui::vec2(1280.0, 2400.0);
    let _tall = super::windows::layout::Dressed::new(tall, "");
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let offered = |a: &mut MailApp| {
        let texts: Vec<String> = (0..3).map(|_| drawn_strings(&ctx, a, tall)).last().unwrap_or_default();
        (texts.iter().any(|t| t == "Start MailCraft in the notification area"), texts.iter().any(|t| t == "Show the unread mail on its icon"))
    };
    assert_eq!(offered(&mut a), (false, false), "not kept running: neither");
    a.run("view.closeToTray", json!({"on": true}));
    assert_eq!(offered(&mut a), (true, !cfg!(target_os = "macos")), "kept running: both (a Mac's Dock counts unread mail)");
    a.run("view.startInTray", json!({"on": true}));
    assert!(a.session.prefs.start_in_tray);
}

/// The system's starting at sign-in, as a test has it: on or not, and why setting it fails.
#[derive(Default)]
struct System {
    at_sign_in: bool,
    refuses: Option<String>,
}

/// Services where starting at sign-in can be set.
struct SignsIn(Rc<RefCell<System>>);

impl crate::Services for SignsIn {
    stub_services!();
    fn starts_at_sign_in(&self) -> Option<bool> {
        Some(self.0.borrow().at_sign_in)
    }
    fn set_start_at_sign_in(&mut self, on: bool) -> Result<(), String> {
        let mut s = self.0.borrow_mut();
        match s.refuses.clone() {
            Some(e) => Err(e),
            None => {
                s.at_sign_in = on;
                Ok(())
            }
        }
    }
}

/// Issue #87: File › Options › General offers starting MailCraft as you sign in where the system
/// lets it be set from here (not in the browser, a portable copy or a sandbox). Its switch says
/// what the system has, read as MailCraft starts and again after each change; a change the
/// system refuses is said, and the switch stays as the system has it.
#[test]
fn mailcraft_can_start_as_you_sign_in() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let tall = egui::vec2(1280.0, 2400.0);
    let _tall = super::windows::layout::Dressed::new(tall, "");
    let label = "Start MailCraft when you sign in";
    let switch = |a: &mut MailApp| {
        let nodes = (0..3).map(|_| named(&ctx, a, tall)).last().unwrap_or_default();
        nodes.iter().find(|n| n.label.as_deref() == Some(label) && n.role == egui::accesskit::Role::CheckBox).and_then(Named::centre)
    };
    let mut a = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(NullServices(NOW)));
    a.act(Action::Backstage(crate::backstage::Page::Options));
    assert_eq!(switch(&mut a), None, "not where it can't be set");

    let system = Rc::new(RefCell::new(System { at_sign_in: true, refuses: None }));
    let mut a = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(SignsIn(system.clone())));
    assert_eq!(a.ui.at_sign_in, Some(true), "read as it starts");
    a.act(Action::Backstage(crate::backstage::Page::Options));
    let at = switch(&mut a).expect("the switch");
    click(&ctx, &mut a, at);
    assert!(!system.borrow().at_sign_in, "turned off");
    assert_eq!(a.ui.at_sign_in, Some(false));
    let at = switch(&mut a).expect("the switch");
    click(&ctx, &mut a, at);
    assert!(system.borrow().at_sign_in, "on again");
    assert_eq!(a.ui.at_sign_in, Some(true));

    system.borrow_mut().refuses = Some("access is denied".into());
    a.act(Action::StartAtSignIn(false));
    assert_eq!(a.ui.at_sign_in, Some(true), "as the system has it");
    let said = a.session.notices.last().filter(|n| n.error).map(|n| n.text.clone()).unwrap_or_default();
    assert!(said.contains("access is denied"), "{said:?}");
}

/// Issue #169: a window MailCraft minimized itself (closed to the Dock, or started put away) is
/// told it isn't minimized once the user brings it back, that is once it has the focus again
/// after losing it. egui notes a window minimized when told to minimize it, and on a Mac never
/// asks again: the window came back keeping its last frame and taking no input, as no pass drew
/// it. One minimized by the user (the yellow button) is left to egui, as it was.
#[test]
fn a_window_put_away_minimized_is_drawn_again_when_brought_back() {
    let (mut a, _) = app("minimized-back");
    let asked = updating(&mut a);
    asked.borrow_mut().tray = Some(crate::KeptOnClose::Minimized);
    a.run("view.closeToTray", json!({"on": true}));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // A pass of `logic` alone (the window counted as minimized), its focus as given.
    let mut t = 100.0;
    let mut logic = |a: &mut MailApp, focused: bool| -> Vec<egui::ViewportCommand> {
        t += 1.0;
        let mut input = RawInput { time: Some(t), ..Default::default() };
        let v = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        (v.minimized, v.focused) = (Some(true), Some(focused));
        ctx.run_logic(&input, |ctx| a.logic(ctx)).viewport_commands.remove(&egui::ViewportId::ROOT).unwrap_or_default()
    };
    let unminimized = |commands: &[egui::ViewportCommand]| commands.contains(&egui::ViewportCommand::Minimized(false));
    for start in [false, true] {
        if start {
            a.started_away(false);
        } else {
            let commands = close(&ctx, &mut a).viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
            assert!(commands.contains(&egui::ViewportCommand::Minimized(true)), "closed: minimized, {commands:?}");
        }
        // Still with the focus as it goes, then without it: left as it is.
        assert!(!unminimized(&logic(&mut a, true)), "start {start}: it hasn't gone yet");
        assert!(!unminimized(&logic(&mut a, false)));
        // Brought back (the Dock): it has the focus again, and is told it isn't minimized.
        assert!(unminimized(&logic(&mut a, true)), "start {start}: brought back");
        assert!(!unminimized(&logic(&mut a, true)), "once");
    }
    // Minimized by the user: egui's to follow, nothing said.
    assert!(!unminimized(&logic(&mut a, false)) && !unminimized(&logic(&mut a, true)));
}

/// A start after the updater installed a version, or tried to.
struct AfterUpdate(Option<crate::UpdateOutcome>);

impl crate::Services for AfterUpdate {
    stub_services!(clock, pick_files, save_path);
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("no".into())
    }
    fn update_outcome(&mut self) -> Option<crate::UpdateOutcome> {
        self.0.take()
    }
}

/// The first start after an update says how it ended in a bar of its own, until it's
/// dismissed: this is the new version, or that one didn't install and is offered again.
#[test]
fn the_first_start_after_an_update_says_how_it_ended() {
    use crate::UpdateOutcome::{NotInstalled, Updated};
    for (outcome, said) in [
        (Updated { version: "0.43.0".into() }, "MailCraft is updated to 0.43.0."),
        (NotInstalled { version: "0.43.0".into() }, "MailCraft 0.43.0 didn't install. It's offered again."),
    ] {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut a = MailApp::new(Session::open(None, Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(AfterUpdate(Some(outcome))));
        for _ in 0..3 {
            drawn(&ctx, &mut a);
        }
        let texts = drawn(&ctx, &mut a);
        assert!(texts.iter().any(|(t, _)| t == said), "{said}");
        let at = texts.iter().find(|(t, _)| t == "Dismiss").map(|(_, p)| *p).expect("a Dismiss button");
        click(&ctx, &mut a, at);
        let texts = drawn(&ctx, &mut a);
        assert_eq!(a.ui.update_outcome, None);
        assert!(!texts.iter().any(|(t, _)| t == said));
    }
}
