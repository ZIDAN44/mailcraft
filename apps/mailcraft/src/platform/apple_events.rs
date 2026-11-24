//! macOS document opening: an `.eml`, `.ics` or `.vcf` sent to MailCraft from the Finder
//! (double-click, Open With, dropped on the Dock tile) or from `open -a MailCraft invite.ics`;
//! and any other file dropped on the Dock tile, attached to a new message (issue #96), as
//! `packaging/macos/Info.plist.in` lets it be dropped there.
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
/// start-up (a message, calendar or contacts file opened, the others of one drop attached to a
/// new message), and a Quit from the Dock or the menu closes the window the normal way (so a
/// draft still gets its "save?" question).
pub fn deliver(queue: &Inbox, app: &mut mailcraft_ui_egui::MailApp, ctx: &egui::Context) {
    for event in queue.drain() {
        let files = match event {
            Event::Quit => {
                // Quit ends MailCraft, where a close of its window may only put it away.
                app.exit(ctx);
                continue;
            }
            Event::Open(files) => files,
        };
        let files = files.iter().map(|file| file.to_string_lossy().into_owned()).collect();
        crate::args::Handed::from_finder(files).give(app);
    }
}

/// Quit the Mac way: `-[NSApplication terminate:]`, sent from the main queue, after the event
/// being handled (winit can't deliver its exit event in the middle of another). Leaving by
/// closing the eframe window instead drops the window while AppKit's run loop still runs, and in
/// the next display cycle its Touch Bar code reaches for the view that's gone: an uncaught
/// Objective-C exception (craft-libs' quit crate found it). winit's `applicationWillTerminate:`
/// still runs eframe's exit, so `on_exit` saves as it does after a close. Always quits: the
/// caller cancels the close and keeps the window for it. All safe objc2 calls.
///
/// The winit fork could turn a termination back into a close request (its W14, switched on by
/// `with_close_requested_on_terminate(true)` on the event loop builder). MailCraft must leave it
/// off: the close would come back here and quit again, round and round.
pub fn quit() -> bool {
    dispatch2::DispatchQueue::main().exec_async(|| {
        // The main queue runs on the main thread, so there's always a marker.
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(mtm).terminate(None);
        }
    });
    true
}

/// The Dock icon's badge: the Inboxes' unread mail, as the window's title counts it (issue #93),
/// none when there's none. All safe objc2 calls, on the main thread (where the window's logic
/// runs; elsewhere nothing happens).
pub fn dock_badge(unread: u32) {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let label = (unread > 0).then(|| objc2_foundation::NSString::from_str(&if unread > 9999 { "9999+".to_string() } else { unread.to_string() }));
    objc2_app_kit::NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
}

thread_local! {
    /// The installed app menu: muda's items must outlive the native menu that shows them.
    static MENU: std::cell::RefCell<Option<muda::Menu>> = const { std::cell::RefCell::new(None) };
}

/// muda's id for the menu's Quit.
const QUIT_ITEM: &str = "mailcraft.quit";

/// Runs `show`, a file dialog, with an Edit menu of the system's own items in the menu bar: the
/// dialog's fields take ⌘A, ⌘C, ⌘V, ⌘X and ⌘Z only through the menu bar's Select All, Copy and
/// the rest, and MailCraft's has none, so ⌘A in the Save As panel's name did nothing (as
/// EffectCraft found in its Open panel). It's there only while the dialog is: MailCraft's own
/// windows take those keys themselves.
pub fn with_edit_menu<T>(show: impl FnOnce() -> T) -> T {
    use muda::{PredefinedMenuItem, Submenu};
    let edit = Submenu::new("Edit", true);
    let added = edit
        .append_items(&[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ])
        .is_ok()
        && MENU.with_borrow(|m| m.as_ref().is_some_and(|m| m.append(&edit).is_ok()));
    let shown = show();
    if added {
        MENU.with_borrow(|m| {
            if let Some(m) = m {
                let _ = m.remove(&edit);
            }
        });
    }
    shown
}

/// Puts in place of winit's app menu one with the same items, but whose Quit (⌘Q) asks the
/// window to close: winit's sends `terminate:` straight away, and winit's
/// `applicationShouldTerminate:` always answers "now", so ⌘Q ended MailCraft with a message half
/// written and no "save?" question. Now ⌘Q goes the close button's way: the question when
/// something would be lost, then [`quit`]. `ask_exit` is the app's Exit (`desktop/tray.rs`): false
/// when MailCraft isn't in the window yet. Call it once, from the app creator (winit builds its
/// menu before then).
pub fn install_menu(ctx: &egui::Context, ask_exit: fn(&egui::Context) -> bool) -> muda::Result<()> {
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
            // Quit ends MailCraft, where a close of its window may only put it away
            // (`desktop/tray.rs`); before MailCraft is in the window, the window closes.
            if !ask_exit(&ctx) {
                // Between frames: egui keeps the command for the next one, which this wakes.
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
                ctx.request_repaint();
            }
        }
    }));
    MENU.set(Some(menu));
    Ok(())
}
