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
