//! macOS document opening: an `.eml`, `.ics` or `.vcf` sent to MailCraft from the Finder
//! (double-click, Open With, dropped on the Dock tile) or from `open -a MailCraft invite.ics`.
//!
//! Such files never show up in `argv`. LaunchServices delivers them as an `odoc` Apple event,
//! and winit 0.30 drops that event on the floor. Catching it means declaring an Objective-C
//! class, which can't be done without `unsafe`, and this workspace forbids `unsafe`. The
//! `fmv-macos-events` crate (shared with the other Craft apps) does that part and gives back a
//! plain queue we read from the egui thread.
//!
//! Links are out of scope here: a `mailto:` click arrives as a `GURL` event, which the crate
//! ignores, so `packaging/macos/Info.plist.in` doesn't register the scheme at all.
//!
//! And quitting: an app menu whose Quit (⌘Q) asks first, and the quit itself through
//! `terminate:` ([`install_menu`], [`quit`]).

use fmv_macos_events::{Event, Inbox, Registration};

/// Hooks MailCraft into the Apple event manager. Call it on the main thread, before eframe
/// spins up its loop, so a file that is the reason for this very launch still gets queued.
///
/// The returned guard unregisters on drop: keep it in scope until `run_native` comes back.
pub fn listen() -> (Registration, Inbox) {
    Registration::install()
}

/// Ask for a repaint whenever an event lands in `queue`, so it's handled even while idle.
pub fn repaint_on_arrival(queue: &Inbox, ctx: &egui::Context) {
    let waker = ctx.clone();
    queue.set_wake(move || waker.request_repaint());
}

/// Called once per frame: every queued file goes to the app just like a path given at
/// start-up, and a Quit from the Dock or the menu closes the window the normal way (so a
/// draft still gets its "save?" question).
pub fn deliver(queue: &Inbox, app: &mut mailcraft_ui_egui::MailApp, ctx: &egui::Context) {
    for event in queue.drain() {
        let files = match event {
            Event::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                continue;
            }
            Event::Open(files) => files,
        };
        files.iter().for_each(|file| app.open_handed(&file.to_string_lossy()));
    }
}

/// Quit the Mac way: `-[NSApplication terminate:]`, sent from the main queue, after the event
/// being handled (winit can't deliver its exit event in the middle of another). Leaving by
/// closing the eframe window instead drops the window while AppKit's run loop still runs, and in
/// the next display cycle its Touch Bar code reaches for the view that's gone: an uncaught
/// Objective-C exception (craft-libs' quit crate found it). winit's `applicationWillTerminate:`
/// still runs eframe's exit, so `on_exit` saves as it does after a close. Always quits: the
/// caller cancels the close and keeps the window for it. All safe objc2 calls.
pub fn quit() -> bool {
    dispatch2::DispatchQueue::main().exec_async(|| {
        // The main queue runs on the main thread, so there's always a marker.
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(mtm).terminate(None);
        }
    });
    true
}

thread_local! {
    /// The installed app menu: muda's items must outlive the native menu that shows them.
    static MENU: std::cell::RefCell<Option<muda::Menu>> = const { std::cell::RefCell::new(None) };
}

/// muda's id for the menu's Quit.
const QUIT_ITEM: &str = "mailcraft.quit";

/// Puts in place of winit's app menu one with the same items, but whose Quit (⌘Q) asks the
/// window to close: winit's sends `terminate:` straight away, and winit never asks the app
/// (`applicationShouldTerminate:`), so ⌘Q ended MailCraft with a message half written and no
/// "save?" question. Now ⌘Q goes the close button's way: the question when something would be
/// lost, then [`quit`]. Call it once, from the app creator (winit builds its menu before then).
pub fn install_menu(ctx: &egui::Context) -> muda::Result<()> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    if MENU.with_borrow(Option::is_some) {
        return Ok(());
    }
    let quit = MenuItem::with_id(QUIT_ITEM, "Quit MailCraft", true, Some(Accelerator::new(Modifiers::META, Code::KeyQ)));
    let app_menu = Submenu::new("MailCraft", true);
    app_menu.append_items(&[
        &PredefinedMenuItem::about(Some("About MailCraft"), None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(Some("Hide MailCraft")),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let menu = Menu::new();
    menu.append(&app_menu)?;
    menu.init_for_nsapp();
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.as_ref() == QUIT_ITEM {
            // Between frames: egui keeps the command for the next one, which this wakes.
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            ctx.request_repaint();
        }
    }));
    MENU.set(Some(menu));
    Ok(())
}
