//! MailCraft desktop app.
//!
//! Usage: `mailcraft [--version] [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]… [mailto:… | file]…`
//!
//! `--sample` opens a separate, local-only sample mailbox (no server needed). `--data` (or the
//! `MAILCRAFT_DATA` environment variable) chooses where mail is cached. `--cmd` runs engine
//! commands at start-up (`--cmd 'list.open={"key":"sample:1:1:INBOX"}'`); `ui.file` (or
//! `ui.settings`), `ui.openExport`, `ui.options` and `ui.about` open the File page at that page;
//! `ui.addAccount` (optionally `={"email":…,"advanced":true,"graph":true}`, `"pop":true` or
//! `"calendarOnly":true`), `ui.newEvent`,
//! `ui.newContact`, `ui.newTask` and `ui.openTask` (`={"uid":…}`) open those dialogs.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use desktop::launch::{self, Launch};
use desktop::start::Made;
use desktop::{Desktop, PoolBackend};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::{OpenProgress, Store, location};
use mailcraft_net::worker::{Credentials, Pool};
use mailcraft_ui_egui::{Backend, BootCard, FramePart, MailApp, Services, VideoFrame};
use secrets::Keychain;

#[cfg(target_os = "macos")]
use platform::apple_events;
#[cfg(windows)]
use platform::toast;
use platform::{appearance, secrets};
use startup::{args, logfile, portable, single_instance};
use window::{ClipboardKeys, app_icon, clipboard_files, drag_out, read_clipboard};
use window::{display, graphics};

mod desktop;
mod platform;
mod self_update;
mod startup;
mod window;
#[cfg(test)]
mod windows_manifest;

/// The system's UI font, if installed (never bundled).
fn system_fonts() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let candidates: Vec<(PathBuf, PathBuf)> = if cfg!(target_os = "windows") {
        let fonts = desktop::windows_fonts_dir();
        vec![(fonts.join("segoeui.ttf"), fonts.join("segoeuib.ttf"))]
    } else {
        let places: &[(&str, &str)] = if cfg!(target_os = "macos") {
            &[("/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf")]
        } else {
            &[
                ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf"),
                ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-Bold.ttf"),
                ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
            ]
        };
        places.iter().map(|(r, b)| (PathBuf::from(r), PathBuf::from(b))).collect()
    };
    for (r, b) in candidates {
        if let Ok(reg) = std::fs::read(r) {
            return (Some(reg), std::fs::read(b).ok());
        }
    }
    (None, None)
}

struct App {
    app: MailApp,
    /// Writes the settings (`ui.json`) on a thread of its own.
    settings: Option<desktop::state_writer::StateWriter>,
    /// The window's GPU device, until its errors are only logged (`graphics::log_errors`).
    gpu: Option<eframe::wgpu::Device>,
    /// The window's GPU device, watched for its loss (`graphics::LostWatch`).
    lost: Option<graphics::LostWatch>,
    frames: u32,
    /// Answering later launches on the same data folder (`single_instance.rs`).
    instance: Option<single_instance::Server>,
    /// Documents macOS sent over since the last frame (`apple_events.rs`).
    #[cfg(target_os = "macos")]
    finder: fmv_macos_events::Inbox,
    /// The unread count the Dock icon shows (issue #93), once it's been set.
    #[cfg(target_os = "macos")]
    badge: Option<u32>,
    native_wayland: bool,
    /// Installs a downloaded update as MailCraft closes.
    updater: Option<self_update::Updater>,
    /// When the settings were last looked at ([`App::keep_state`]).
    looked: std::time::Instant,
    /// When the system was last asked where the monitors are ([`App::look_at_monitors`]).
    monitors_looked: Option<std::time::Instant>,
    /// MailCraft's icon in the notification area, while it's kept running there.
    tray: desktop::tray::Tray,
}

/// How often changed settings are written while MailCraft runs.
const KEEP_STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// How often the system is asked where the monitors are.
const MONITORS_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

impl App {
    /// Tell MailCraft's windows where the monitors are, asking the system every few seconds (a
    /// monitor plugged in or out shows soon enough): a window restored where none is any more is
    /// brought back on screen (`WindowPlace::rescue`).
    fn look_at_monitors(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.monitors_looked.is_some_and(|at| at.elapsed() < MONITORS_EVERY) {
            return;
        }
        self.monitors_looked = Some(std::time::Instant::now());
        let rect = |(x, y): (i32, i32), (w, h): (u32, u32)| egui::Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32));
        let monitors = frame.winit_window().map(|w| mailcraft_ui_egui::Monitors {
            all: w.available_monitors().map(|m| rect((m.position().x, m.position().y), (m.size().width, m.size().height))).collect(),
            primary: w.primary_monitor().map(|m| rect((m.position().x, m.position().y), (m.size().width, m.size().height))),
        });
        mailcraft_ui_egui::set_monitors(ctx, monitors);
    }

    /// Write the settings when they've changed: a reminder dismissed or a sender blocked isn't
    /// lost if MailCraft is ended without closing (a crash, a power cut, a forced quit).
    fn keep_state(&mut self, ctx: Option<&egui::Context>, force: bool) {
        let since = self.looked.elapsed();
        if !force && since < KEEP_STATE_EVERY {
            // Something may have changed in this frame: look again when it's time, even if
            // nothing else draws a frame by then.
            if let Some(ctx) = ctx {
                ctx.request_repaint_after(KEEP_STATE_EVERY.saturating_sub(since));
            }
            return;
        }
        self.looked = std::time::Instant::now();
        // Written (when they changed) on the settings' thread: flushed to the disk, the write
        // held the window's frame, for 197 ms once while a data file was being flushed.
        if let Some(w) = &mut self.settings {
            w.keep(self.app.saved_state());
        }
    }
}

// Each part of a frame runs under the guard: a panic in it is said and MailCraft goes on, rather
// than ending with the messages being written (`MailApp::frame_part_ended`).
impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.frame_part_starts(FramePart::Logic);
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.logic_unguarded(ctx, frame)));
        self.app.frame_part_ended(ctx, FramePart::Logic, ran);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.frame_part_starts(FramePart::Ui);
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.ui_unguarded(ui, frame)));
        self.app.frame_part_ended(ui.ctx(), FramePart::Ui, ran);
    }
    fn on_exit(&mut self) {
        self.exiting();
    }
}

impl App {
    // Every frame, the window minimized or hidden too (when eframe runs no `ui`): started again,
    // this window comes forward instead of a second one, and opens what that launch handed over.
    fn logic_unguarded(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The GPU device is gone: what's being written is kept as drafts and MailCraft closes, as
        // nothing can be drawn any more.
        if self.lost.as_ref().is_some_and(graphics::LostWatch::lost) {
            self.app.graphics_lost(ctx);
        }
        if let Some(handed) = self.instance.as_ref().and_then(single_instance::Server::take) {
            single_instance::show(ctx, self.native_wayland);
            handed.give(&mut self.app);
        }
        #[cfg(target_os = "macos")]
        {
            apple_events::deliver(&self.finder, &mut self.app, ctx);
            let unread = self.app.session.inbox_unread();
            if self.badge != Some(unread) {
                apple_events::dock_badge(unread);
                self.badge = Some(unread);
            }
        }
        // An update ready while the window is minimized or behind others: the system says so.
        self.app.follow_update(ctx);
        self.tray.follow(&mut self.app, ctx);
        // A close from the taskbar or the Dock while minimized: asked about here, as `ui` doesn't run.
        self.app.logic(ctx);
        // On a Mac a close nothing held open quits through AppKit (`apple_events::quit`); last, so
        // it sees whether a question held the window.
        #[cfg(target_os = "macos")]
        MailApp::quit_instead_of_close(ctx, apple_events::quit);
    }
    fn ui_unguarded(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Nothing is drawn with a lost device (egui-wgpu would panic making its buffers).
        if self.app.ui.graphics_lost {
            return;
        }
        self.frames = self.frames.saturating_add(1);
        if self.frames >= graphics::STARTUP_FRAMES
            && let Some(device) = self.gpu.take()
        {
            graphics::log_errors(&device);
        }
        self.look_at_monitors(ui.ctx(), frame);
        self.app.ui(ui);
        self.keep_state(Some(ui.ctx()), false);
        let dragged = self.app.take_drag_out();
        if !dragged.is_empty() {
            drag_out(frame, dragged);
        }
    }
    fn exiting(&mut self) {
        if let Some(server) = &self.instance {
            server.close();
        }
        // A test build downloaded and checked: installed now that MailCraft closes (unless it was
        // kept for later); a download still going stops, to carry on next time.
        if let Some(u) = &self.updater {
            u.closing();
        }
        self.keep_state(None, true);
        if let Some(w) = &mut self.settings {
            w.close();
        }
        // An import running pauses: what it read is written before the data files are sealed
        // below (waited for 10 seconds at most; an import stuck longer is let go).
        self.app.backend.shutdown();
        // Issue #4: attachments handed to other apps, and pictures made of them, go with
        // MailCraft when it's portable or keeps an account online only (one still open in an
        // app stays until that app lets it go).
        let s = &self.app.session;
        if s.portable || s.accounts.iter().any(|a| a.config.keeps_online()) {
            mailcraft_engine::store::clear_scratch();
        }
        // The data files sealed, so quitting just after a write (a message marked read) or
        // during an import doesn't cost a full check of the file at the next start. Closing them
        // would, but the accounts' threads hold the store too and are let go, not waited for.
        if let Some(store) = &s.store
            && let Err(e) = store.seal_all()
        {
            log::warn!("closing: {e}");
        }
        // The lines logged are written on the log's thread: written before MailCraft ends.
        log::logger().flush();
        // macOS: end here, before eframe tears the window down. AccessKit's macOS adapter
        // subclasses the window's content view and puts its class back on teardown; AppKit's
        // Touch Bar support observes that view, so in the next display cycle it throws ("not
        // registered") and quitting ends in a crash report (DeckCraft and GridCraft found). Every
        // way of quitting comes here, and nothing after it is needed: the window's state is
        // saved, the log written and the data files keep what they committed.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

fn main() -> eframe::Result {
    // `args()` panics on an argument that isn't Unicode (a file name in another encoding).
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    // Started for a link or a file (`--open`): nothing after it is an option.
    let (args, opened) = args::opened(args);
    if args.iter().any(|a| a == "--version" || a == "-V") {
        say_out(&args::version_line());
        return Ok(());
    }
    // Asking how to start it prints that, and no window opens.
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "/?")) {
        say_out(args::USAGE);
        return Ok(());
    }
    // Text pasted from Outlook, Word or a web page keeps its look.
    // A logo copied with it from older Word or Outlook, a file of Office's own, comes along too.
    mailcraft_ui_egui::set_clipboard_html(|| read_clipboard(|c| c.get().html().ok()).filter(|h| !h.trim().is_empty()).map(desktop::office_pictures));
    let launch = Launch::resolve(args, opened);
    // MailCraft's Flatpak under its id before 0.36 kept its mail in that id's sandbox: brought
    // across before anything is made in this one's.
    if !launch.chosen
        && launch.portable.dir.is_none()
        && let Some(r) = launch.root.as_deref()
    {
        self_update::renamed::carry_over_flatpak(r);
    }
    // One MailCraft per data folder: started again, it brings the running one forward and leaves,
    // before it moves, logs or opens anything there (`single_instance.rs`).
    let instance = match launch.root.as_deref().map(|r| single_instance::claim(r, &launch.handed)) {
        Some(single_instance::Claim::Shown) => {
            say(if !launch.handed.is_empty() {
                "MailCraft is already running: what it was started with opens there"
            } else if launch.has("--cmd") {
                "MailCraft is already running: its window is brought forward (--cmd is run only by the first)"
            } else {
                "MailCraft is already running: its window is brought forward"
            });
            return Ok(());
        }
        Some(single_instance::Claim::Primary(server)) => Some(server),
        _ => None,
    };
    // A move of the data files' folder asked for (File › Options) is carried out now, before
    // anything opens them.
    let mut moved = match launch.base.as_deref().map(location::apply_pending_move) {
        Some(Ok(Some(to))) => Some(Ok(to)),
        Some(Err(e)) => Some(Err(e)),
        _ => None,
    };
    let files = launch.files();
    start_logging(&launch);

    let (now, offset) = Desktop::default().clock();
    let store = launch.open_store(files.as_deref(), &mut moved);
    if launch.sample
        && let Some(s) = &store
        && let Err(e) = mailcraft_engine::sample::seed_if_empty_at(s, now, i64::from(offset))
    {
        say(&format!("the sample mailbox couldn't be created: {e}"));
    }
    let build_sha = option_env!("MAILCRAFT_BUILD_SHA").filter(|s| !s.trim().is_empty()).map(str::to_string);
    let made = Made { clock: Clock::new(now, offset), portable: launch.portable.dir.is_some(), build_sha: build_sha.clone() };
    // The data files open on a thread of their own, and the session is made there from them,
    // while the window shows how that goes (a file not closed properly is checked: minutes).
    let (progress, opened) = {
        let made = made.clone();
        desktop::boot::open(store, launch.root.clone(), moved, move |store| desktop::start::session(store, made))
    };
    let state_path = launch.state_path();
    // Test builds update themselves (`self_update.rs`); not the sample mailbox's run, unless it's
    // trying the updater out.
    let updater = launch.root.clone().filter(|_| !launch.sample || self_update::trying_out()).and_then(|r| self_update::Updater::start(r, build_sha));

    // The settings, read once: the window opens with them, and MailCraft once it's in it.
    let saved_ui = state_path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let theme = launch::theme_asked(&launch.args);
    // Linux and the BSDs: whether the window opens on X11 (`display.rs`), chosen once (a stale
    // `DISPLAY` is logged as it's chosen).
    let x11 = cfg!(all(unix, not(target_os = "macos"))) && display::choose_from_env() == display::Choice::X11;
    let (options, card) = launch::window(launch.sample, saved_ui.as_ref(), theme, x11);
    let started =
        Started { launch, opening: Opening { progress, opened, card, made }, state_path, instance, updater, looks: Looks { saved_ui, theme, x11 } };
    // A window that can't open says why, rather than the app leaving without a word (or
    // staying in the background); a panic on the way is caught for that too.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(options, started)));
    let failed = match ran {
        Ok(Ok(())) => {
            log::logger().flush();
            return Ok(());
        }
        Ok(Err(e)) => e.to_string(),
        // Where it happened is in the log already (`logfile.rs`); what it said isn't written
        // there, here or on the screen: it can quote what it was working on (a subject, a line
        // of a message).
        Err(_) => "it stopped unexpectedly (the log says where)".into(),
    };
    log::error!("the window couldn't open: {failed}");
    log::logger().flush();
    say(&format!("the window couldn't open: {failed}"));
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("MailCraft")
        .set_description(cant_open(&failed))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(1)
}

/// Diagnostics to `mailcraft.log` in the data folder, panics logged; then what this start should
/// note there.
fn start_logging(launch: &Launch) {
    let echo = cfg!(debug_assertions) || launch.has("--log");
    if let Some(path) = logfile::start(launch.root.as_deref(), echo) {
        log::info!("{} started; log at {}", args::version_line(), path.display());
    }
    logfile::log_panics();
    // What MailCraft's id before 0.36 left on this computer (`self_update/renamed.rs`).
    if !launch.sample
        && let Err(e) = std::thread::Builder::new().name("renamed".into()).spawn(self_update::renamed::tidy)
    {
        log::warn!("renamed: not tidied: {e}");
    }
    if let Some(dir) = &launch.portable.dir {
        log::info!("portable: everything is kept in {}", dir.display());
    }
    if let Some(why) = &launch.portable.unwritable {
        log::warn!("{why}");
        say(why);
    }
}

/// Tell standard error (and the log). `eprintln!` panics when standard error is a broken pipe,
/// which a launch from another program can leave it as (PdfCraft and GridCraft found).
fn say(what: &str) {
    log::warn!("{what}");
    let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("mailcraft: {what}\n"));
}

/// Tell standard output, as `println!` would without its panic on a closed pipe.
fn say_out(what: &str) {
    let _ = std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("{what}\n"));
}

/// What the message box says when the window can't open.
fn cant_open(why: &str) -> String {
    let (help, log) = if cfg!(windows) {
        (
            "Updating it may help, or starting MailCraft with the environment variable WGPU_FORCE_FALLBACK_ADAPTER=1, which draws it with \
             Windows' software renderer.",
            "%APPDATA%\\MailCraft\\mailcraft.log",
        )
    } else {
        ("Updating it may help.", "mailcraft.log, in MailCraft's data folder")
    };
    format!("MailCraft's window couldn't open:\n{why}\n\nThis is usually the graphics driver. {help}\n\nThe details are in {log}.")
}

/// The data files opening (`desktop/boot.rs`), what the window shows meanwhile, and what the
/// session is made with if they couldn't be.
struct Opening {
    progress: Arc<OpenProgress>,
    opened: std::sync::mpsc::Receiver<desktop::boot::Opened>,
    card: BootCard,
    made: Made,
}

/// What was decided of the window's look before it opened, for MailCraft once it's in it.
struct Looks {
    /// The settings (`ui.json`) as they were read for the window.
    saved_ui: Option<serde_json::Value>,
    /// `--light` (false) or `--dark` (true), over the settings.
    theme: Option<bool>,
    /// The window is on X11 (`display.rs`; Linux and the BSDs).
    x11: bool,
}

/// What the window runs with, besides its options.
struct Started {
    launch: Launch,
    opening: Opening,
    /// Where the window's settings are written (`ui.json`).
    state_path: Option<PathBuf>,
    /// Answering later launches on the same data folder.
    instance: Option<single_instance::Server>,
    updater: Option<self_update::Updater>,
    looks: Looks,
}

/// Open the window and run the app until it closes: the boot card while the data files open,
/// then MailCraft.
fn run(options: eframe::NativeOptions, started: Started) -> eframe::Result {
    let Started { launch, opening: Opening { progress, opened, card, made }, state_path, instance, updater, looks: Looks { saved_ui, theme, x11 } } =
        started;
    let put_away = launch.starts_away(saved_ui.as_ref());
    let Launch { args: cmd_args, handed, sample, base, .. } = launch;
    // The guard stays here, outside the closure, so the handlers live as long as the loop.
    #[cfg(target_os = "macos")]
    let (_finder_guard, finder) = apple_events::listen();
    // The window drawing its own title bar (the system's was left off it, `main`).
    let own_title_bar = options.viewport.decorations == Some(false);
    eframe::run_native(
        "MailCraft",
        options,
        Box::new(move |cc| {
            // What the window needs from the start, before MailCraft is in it.
            // Its thread is never kept waiting by a compaction that has a data file to itself:
            // what it reads of that file fails at once ("being compacted") until it's given back.
            mailcraft_engine::store::no_waiting();
            // The updater wakes the window as the update downloads and is ready.
            if let Some(u) = &updater {
                let ctx = cc.egui_ctx.clone();
                u.set_waker(Arc::new(move || ctx.request_repaint()));
            }
            // Linux and the BSDs: the desktop's dark or light, which winit doesn't report there.
            let dark = appearance::SystemDark::default();
            {
                let ctx = cc.egui_ctx.clone();
                dark.follow(move || ctx.request_repaint());
            }
            let (regular, bold) = system_fonts();
            mailcraft_ui_egui::theme::install_fonts(&cc.egui_ctx, regular.clone(), bold.clone());
            let gpu = cc.wgpu_render_state.as_ref().map(|rs| rs.device.clone());
            if let Some(server) = &instance {
                server.connect(&cc.egui_ctx);
            }
            // A Wayland window can't bring itself forward: it asks for attention instead.
            let native_wayland =
                cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) && !x11;
            #[cfg(target_os = "macos")]
            {
                apple_events::repaint_on_arrival(&finder, &cc.egui_ctx);
                // ⌘Q asks before what's being written is lost; without the menu it still runs,
                // quitting as winit's does.
                if let Err(e) = apple_events::install_menu(&cc.egui_ctx) {
                    log::warn!("the app menu wasn't put in place, so Quit won't ask first: {e}");
                }
            }
            // Borderless, its title bar MailCraft's own: kept so from the first frame (issue #22).
            let frame_guard = desktop::window_frame::service(cc, own_title_bar);
            let booting_guard = frame_guard.clone();
            let booting_updater = updater.clone();
            // MailCraft, once its data files are open.
            let start: desktop::boot::Start = Box::new(move |ctx, opened, instance| {
                let args = cmd_args;
                let desktop::boot::Opened { store, session: made_session, moved } = opened;
                // The store opened with: the plain cache when folders from before couldn't come
                // into data files, none if opening failed altogether.
                let desktop_data = store.as_ref().and_then(Store::data_files_dir);
                let app_portable = made.portable;
                let session = made_session.unwrap_or_else(|| desktop::start::session(None, made));
                let backend: Box<dyn Backend> = match store {
                    Some(store) => {
                        // Portable: passwords and sign-ins in memory only, never the system's keychain.
                        let creds: Arc<dyn Credentials> =
                            if app_portable { Arc::new(mailcraft_net::worker::MemoryCredentials::default()) } else { Arc::new(Keychain) };
                        let pool = Pool::new(store, creds);
                        let ctx = ctx.clone();
                        pool.set_waker(Arc::new(move || ctx.request_repaint()));
                        Box::new(PoolBackend(pool))
                    }
                    None => Box::new(mailcraft_ui_egui::NullBackend),
                };
                let mut app = MailApp::new(
                    session,
                    backend,
                    Box::new(Desktop {
                        dark,
                        data: desktop_data,
                        base,
                        moved,
                        #[cfg(windows)]
                        portable: app_portable,
                        updater: updater.clone(),
                        own_title_bar,
                        frame_guard,
                        x11,
                        ..Default::default()
                    }),
                );
                if sample {
                    app.ui.title_base = "MailCraft – Sample mailbox".into();
                }
                // PDFs (print, save as PDF) embed the same font.
                if let Some(r) = &regular {
                    app.session.print_fonts = Some((r.clone(), bold.clone()));
                }
                app.install_fonts(ctx, regular, bold);
                // The settings the window opened with (nothing writes them until MailCraft runs).
                if let Some(v) = &saved_ui {
                    app.restore_state(v);
                }
                if theme.is_some() {
                    app.ui.dark = theme;
                }
                desktop::start::commands(&mut app, &args);
                handed.give(&mut app);
                // From here the open message's body is read and parsed on a thread of its own (the
                // commands above answered at once, as they do in the CLI).
                app.defer_bodies();
                App {
                    app,
                    settings: state_path.map(desktop::state_writer::StateWriter::new),
                    lost: gpu.as_ref().map(graphics::LostWatch::new),
                    gpu,
                    frames: 0,
                    instance,
                    #[cfg(target_os = "macos")]
                    finder,
                    #[cfg(target_os = "macos")]
                    badge: None,
                    native_wayland,
                    updater,
                    looked: std::time::Instant::now(),
                    monitors_looked: None,
                    tray: desktop::tray::Tray::new(native_wayland),
                }
            });
            Ok(Box::new(desktop::boot::Window::Booting(Box::new(desktop::boot::Booting {
                card: card.titled(own_title_bar),
                progress,
                opened,
                came: None,
                start: Some(start),
                instance,
                updater: booting_updater,
                handed: args::Handed::default(),
                native_wayland,
                frame_guard: booting_guard,
                put_away,
                away: None,
            }))))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use desktop::video;

    /// eframe is used without its default features, and copy and paste are one of them since
    /// 0.37: the app had no clipboard (nothing pasted anywhere) until it was named.
    #[test]
    fn the_window_has_a_clipboard() {
        let manifest = include_str!("../Cargo.toml");
        let eframe = manifest.lines().find(|l| l.trim_start().starts_with("eframe")).unwrap_or("");
        assert!(eframe.contains("\"clipboard\""), "eframe without its clipboard feature: {eframe}");
    }

    /// A shortcut that names an MSI `<Icon>` shows a blank icon once an update deletes the
    /// installer's cached copy, and a taskbar pin (a copy of the shortcut) is never repaired:
    /// shortcuts take the icon embedded in mailcraft.exe, and package.ps1 refuses an exe without
    /// one. Only Apps and features (`ARPPRODUCTICON`) uses the MSI `<Icon>`.
    #[test]
    fn windows_shortcuts_use_the_icon_embedded_in_the_exe() {
        let wxs = include_str!("../../../packaging/windows/mailcraft.wxs");
        let shortcuts: Vec<&str> = wxs.split("<Shortcut ").skip(1).filter_map(|s| s.split('>').next()).collect();
        assert!(!shortcuts.is_empty(), "mailcraft.wxs has no shortcut");
        for shortcut in shortcuts {
            assert!(!shortcut.contains("Icon="), "a shortcut names an MSI <Icon>: {shortcut}");
            assert!(shortcut.contains("Advertise=\"no\""), "an advertised shortcut shows the MSI's icon: {shortcut}");
        }
        assert!(wxs.contains("<Property Id=\"ARPPRODUCTICON\""), "Apps and features keeps its icon");
        let script = include_str!("../../../packaging/windows/package.ps1");
        assert!(script.contains("ExtractIconEx") && script.contains("has no embedded icon"), "package.ps1 doesn't refuse an exe without an icon");
    }

    /// A window that can't open says why, in plain sentences (no gaps left from the source).
    #[test]
    fn a_window_that_cant_open_says_why() {
        let m = cant_open("no graphics adapter");
        assert!(m.starts_with("MailCraft's window couldn't open:\nno graphics adapter\n"), "{m}");
        assert!(m.contains("mailcraft.log") && !m.contains("  "), "{m}");
    }

    /// The system's thumbnailer (as when ffmpeg isn't installed) makes a video's frame through
    /// the desktop's services: the window's first ask is answered at once, the frame comes later.
    /// It needs ffmpeg to make the clip and a thumbnailer on the system; without either it says
    /// so and checks nothing (`desktop/video_frames.rs` checks the rest with a stand-in tool).
    /// Found in audit (#233): it checked nothing of what its name says, and passed with no frame
    /// when the system had no thumbnailer.
    #[test]
    fn video_thumbnails_are_made_off_the_ui_thread() {
        let skipped = |why: &str| eprintln!("video_thumbnails_are_made_off_the_ui_thread: SKIPPED, {why}");
        let Some(ffmpeg) = video::find_tool("ffmpeg", &[]) else { return skipped("no ffmpeg to make the clip") };
        let Some(system) = video::system() else { return skipped("no system thumbnailer") };
        let dir = std::env::temp_dir().join(format!("mailcraft-desktop-video-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return skipped("ffmpeg made no clip");
        }
        let mut d = Desktop { frames: desktop::video_frames::VideoFrames::with_source(Some(system)), ..Default::default() };
        let asked = std::time::Instant::now();
        assert_eq!(d.video_frame(&clip), VideoFrame::Pending, "the first ask waits for nothing");
        assert!(asked.elapsed() < std::time::Duration::from_secs(5), "the window waited {:?}", asked.elapsed());
        // Longer than the tool is given: a frame that never comes is the tool's to give up on.
        let end = std::time::Instant::now() + video::TIMEOUT.saturating_mul(2);
        let frame = loop {
            match d.video_frame(&clip) {
                VideoFrame::Pending if std::time::Instant::now() < end => std::thread::sleep(std::time::Duration::from_millis(50)),
                f => break f,
            }
        };
        assert!(matches!(&frame, VideoFrame::Ready(png) if png.starts_with(b"\x89PNG")), "{frame:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
