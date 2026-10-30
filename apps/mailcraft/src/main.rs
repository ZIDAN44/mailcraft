//! MailCraft desktop app.
//!
//! Usage: `mailcraft [--version] [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]… [mailto:… | file]…`
//!
//! `--sample` opens a separate, local-only sample mailbox (no server needed). `--data` (or the
//! `MAILCRAFT_DATA` environment variable) chooses where mail is cached. `--cmd` runs engine
//! commands at start-up (`--cmd 'list.open={"key":"sample:1:1:INBOX"}'`); `ui.file` (or
//! `ui.settings`), `ui.openExport`, `ui.options` and `ui.about` open the File page at that page;
//! `ui.addAccount` (optionally `={"email":…,"advanced":true,"graph":true}` or `"pop":true`), `ui.newEvent` and
//! `ui.newContact` open those dialogs.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use mailcraft_ui_egui::backstage::{Backstage, Page};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use desktop::{Desktop, PoolBackend};
use mailcraft_engine::Session;
#[cfg(all(test, windows))]
use mailcraft_engine::model::AccountConfig;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::{Store, location};
use mailcraft_net::worker::{Credentials, Pool};
use mailcraft_ui_egui::{Backend, MailApp, Services, VideoFrame};
use secrets::Keychain;
#[cfg(all(test, windows))]
use secrets::SERVICE;

mod appearance;
#[cfg(target_os = "macos")]
mod apple_events;
mod args;
mod desktop;
mod display;
mod graphics;
mod logfile;
mod portable;
mod secrets;
mod self_update;
mod single_instance;
#[cfg(windows)]
mod toast;
mod video;
#[cfg(test)]
mod windows_manifest;
#[cfg(all(unix, not(target_os = "macos")))]
mod xdnd;

/// The system's UI font, if installed (never bundled).
fn system_fonts() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let candidates: &[(&str, &str)] = if cfg!(target_os = "windows") {
        &[("C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf")]
    } else if cfg!(target_os = "macos") {
        &[("/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf")]
    } else {
        &[
            ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf"),
            ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-Bold.ttf"),
            ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ]
    };
    for (r, b) in candidates {
        if let Ok(reg) = std::fs::read(r) {
            return (Some(reg), std::fs::read(b).ok());
        }
    }
    (None, None)
}

/// Runs MailCraft's own keys on each window's input: Cut, Copy and Paste
/// ([`mailcraft_ui_egui::clipboard_keys`]) and, on Windows, Alt+numpad character codes
/// ([`mailcraft_ui_egui::alt_codes`], in the code pages for the user's language).
struct ClipboardKeys {
    #[cfg_attr(not(windows), allow(dead_code))]
    pages: (u16, u16),
}

impl eframe::ViewportHooks for ClipboardKeys {
    fn raw_input(&self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(windows)]
        mailcraft_ui_egui::alt_codes(ctx, raw_input, self.pages);
        mailcraft_ui_egui::clipboard_keys(ctx, raw_input, &mut SystemClipboard);
    }
}

/// The system clipboard, read as egui-winit reads it for a paste: its text, else its picture.
struct SystemClipboard;

impl mailcraft_ui_egui::ClipboardRead for SystemClipboard {
    fn files(&mut self) -> Vec<PathBuf> {
        clipboard_files()
    }

    fn paste(&mut self) -> Option<egui::Event> {
        if let Some(text) = read_clipboard(|c| c.get_text().ok()) {
            let text = text.replace("\r\n", "\n");
            return (!text.is_empty()).then_some(egui::Event::Paste(text));
        }
        let img = read_clipboard(|c| c.get_image().ok())?;
        // Its size and its bytes must agree (egui stops on a mismatch).
        if img.width.checked_mul(img.height).and_then(|n| n.checked_mul(4)) != Some(img.bytes.len()) {
            return None;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.bytes);
        Some(egui::Event::PasteImage(Arc::new(image)))
    }
}

/// The window icon: the committed app icon (`cargo xtask icons`), or egui's default if it can't be decoded.
fn app_icon() -> egui::IconData {
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.mailcraft.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.into_rgba8();
            egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// Hand a file to the system's drag and drop (Windows and macOS; elsewhere egui has no way to).
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn drag_out(frame: &eframe::Frame, path: PathBuf) {
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.mailcraft.png");
    let options = drag::Options { skip_animatation_on_cancel_or_failure: false, mode: drag::DragMode::Copy };
    // The drag crate is native code; a failure there must not take the app down.
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drag::start_drag(frame, drag::DragItem::Files(vec![path]), drag::Image::Raw(PNG.to_vec()), |_, _| {}, options)
    }));
    match started {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("drag out: {e}"),
        Err(_) => log::warn!("drag out failed"),
    }
}

/// Hand a file to X11's drag and drop (XDND), on a thread of its own while the button is held.
#[cfg(all(unix, not(target_os = "macos")))]
fn drag_out(frame: &eframe::Frame, path: PathBuf) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let own = match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => u32::try_from(h.window).ok(),
        Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
        _ => None,
    };
    let Some(own) = own else {
        log::info!("drag out: not an X11 window");
        return;
    };
    let spawned = std::thread::Builder::new().name("drag-out".into()).spawn(move || {
        if let Err(e) = xdnd::drag(None, own, &path) {
            log::warn!("drag out: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("drag out: {e}");
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn drag_out(_frame: &eframe::Frame, _path: PathBuf) {}

struct App {
    app: MailApp,
    state_path: Option<PathBuf>,
    /// The window's GPU device, until its errors are only logged (`graphics::log_errors`).
    gpu: Option<eframe::wgpu::Device>,
    frames: u32,
    /// Answering later launches on the same data folder (`single_instance.rs`).
    instance: Option<single_instance::Server>,
    /// Documents macOS sent over since the last frame (`apple_events.rs`).
    #[cfg(target_os = "macos")]
    finder: fmv_macos_events::Inbox,
    native_wayland: bool,
    /// Installs a downloaded update as MailCraft closes.
    updater: Option<self_update::Updater>,
    /// The settings as last written, and when they were last looked at ([`App::keep_state`]).
    saved: Vec<u8>,
    looked: std::time::Instant,
}

/// How often changed settings are written while MailCraft runs.
const KEEP_STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

impl App {
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
        let Some(p) = &self.state_path else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.app.saved_state()) else { return };
        if bytes != self.saved {
            match mailcraft_engine::store::write_atomic(p, &bytes) {
                Ok(()) => self.saved = bytes,
                Err(e) => log::warn!("the settings weren't saved: {e}"),
            }
        }
    }
}

impl eframe::App for App {
    // Every frame, the window minimized or hidden too (when eframe runs no `ui`): started again,
    // this window comes forward instead of a second one, and opens what that launch handed over.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(handed) = self.instance.as_ref().and_then(single_instance::Server::take) {
            single_instance::show(ctx, self.native_wayland);
            for h in &handed {
                self.app.open_handed(h);
            }
        }
        #[cfg(target_os = "macos")]
        apple_events::deliver(&self.finder, &mut self.app, ctx);
        // An update ready while the window is minimized or behind others: the system says so.
        self.app.follow_update(ctx);
        // A close from the taskbar or the Dock while minimized: asked about here, as `ui` doesn't run.
        self.app.logic(ctx);
        // On a Mac a close nothing held open quits through AppKit (`apple_events::quit`); last, so
        // it sees whether a question held the window.
        #[cfg(target_os = "macos")]
        MailApp::quit_instead_of_close(ctx, apple_events::quit);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frames = self.frames.saturating_add(1);
        if self.frames >= graphics::STARTUP_FRAMES
            && let Some(device) = self.gpu.take()
        {
            graphics::log_errors(&device);
        }
        self.app.ui(ui);
        self.keep_state(Some(ui.ctx()), false);
        if let Some(path) = self.app.take_drag_out() {
            drag_out(frame, path);
        }
    }
    fn on_exit(&mut self) {
        if let Some(server) = &self.instance {
            server.close();
        }
        // A test build downloaded and checked: installed now that MailCraft closes (unless it was
        // kept for later); a download still going stops, to carry on next time.
        if let Some(u) = &self.updater {
            u.closing();
        }
        self.keep_state(None, true);
        // Issue #4: attachments handed to other apps, and pictures made of them, go with
        // MailCraft when it's portable or keeps an account online only (one still open in an
        // app stays until that app lets it go).
        let s = &self.app.session;
        if s.portable || s.accounts.iter().any(|a| a.config.keeps_online()) {
            mailcraft_engine::store::clear_scratch();
        }
        // macOS: end here, before eframe tears the window down. AccessKit's macOS adapter
        // subclasses the window's content view and puts its class back on teardown; AppKit's
        // Touch Bar support observes that view, so in the next display cycle it throws ("not
        // registered") and quitting ends in a crash report (DeckCraft and GridCraft found). Every
        // way of quitting comes here, and nothing after it is needed: the window's state is
        // saved, the log is unbuffered and the data files keep what they committed.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

fn main() -> eframe::Result {
    // `args()` panics on an argument that isn't Unicode (a file name in another encoding).
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        say_out(&args::version_line());
        return Ok(());
    }
    // Asking how to start it prints that, and no window opens.
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "/?")) {
        say_out(args::USAGE);
        return Ok(());
    }
    let cmd_args = args.clone();
    // `mailto:` links and mail files: MailCraft as the system's mail app, or Open with (`args.rs`).
    let handed = args::handed(&args);
    let sample = args.iter().any(|a| a == "--sample");
    // Text pasted from Outlook, Word or a web page keeps its look.
    // A logo copied with it from older Word or Outlook, a file of Office's own, comes along too.
    mailcraft_ui_egui::set_clipboard_html(|| read_clipboard(|c| c.get().html().ok()).filter(|h| !h.trim().is_empty()).map(desktop::office_pictures));
    // Each account is a data file in Documents\MailCraft Files (as Outlook keeps .pst files in
    // Outlook Files); the account list and the app's state stay in the app folder. The data files'
    // folder can be moved (File › Options) when it's the usual one: a move asked for is carried
    // out now, before anything opens them.
    let chosen = sample || args.iter().any(|a| a == "--data") || std::env::var_os("MAILCRAFT_DATA").is_some_and(|d| !d.is_empty());
    // Portable (issue #4): `portable.txt` beside the app keeps everything in MailCraftData there.
    let portable = if chosen { portable::Portable::default() } else { portable::current().clone() };
    let base = (!chosen && portable.dir.is_none()).then(location::base_root).flatten();
    let data = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| portable.dir.clone())
        .or_else(mailcraft_engine::store::default_root);
    let root = data.map(|d| if sample { d.join("sample") } else { d });
    // One MailCraft per data folder: started again, it brings the running one forward and leaves,
    // before it moves, logs or opens anything there (`single_instance.rs`).
    let instance = match root.as_deref().map(|r| single_instance::claim(r, &handed)) {
        Some(single_instance::Claim::Shown) => {
            say(if !handed.is_empty() {
                "MailCraft is already running: what it was started with opens there"
            } else if args.iter().any(|a| a == "--cmd") {
                "MailCraft is already running: its window is brought forward (--cmd is run only by the first)"
            } else {
                "MailCraft is already running: its window is brought forward"
            });
            return Ok(());
        }
        Some(single_instance::Claim::Primary(server)) => Some(server),
        _ => None,
    };
    let mut moved = match base.as_deref().map(location::apply_pending_move) {
        Some(Ok(Some(to))) => Some(Ok(to)),
        Some(Err(e)) => Some(Err(e)),
        _ => None,
    };
    // The sample mailbox stays a folder of files; anything else keeps its data files beside it
    // (`--data`) or in Documents.
    let files =
        (!sample).then(|| root.as_ref().map(|r| base.as_deref().map_or_else(|| r.join(location::FILES_FOLDER), location::files_dir))).flatten();
    // Diagnostics go to `mailcraft.log` in the data folder.
    let echo = cfg!(debug_assertions) || args.iter().any(|a| a == "--log");
    if let Some(path) = logfile::start(root.as_deref(), echo) {
        log::info!("{} started; log at {}", args::version_line(), path.display());
    }
    if let Some(dir) = &portable.dir {
        log::info!("portable: everything is kept in {}", dir.display());
    }
    if let Some(why) = &portable.unwritable {
        log::warn!("{why}");
        say(why);
    }

    let (now, offset) = Desktop::default().clock();
    let store = root.as_ref().and_then(|r| {
        let opened = match &files {
            Some(f) => Store::open_packed(r, f).or_else(|e| {
                // Folders from before that couldn't come into data files stay as they are.
                moved = Some(Err(e.to_string()));
                Store::open(r)
            }),
            None => Store::open(r),
        };
        match opened {
            Ok(s) => Some(s),
            Err(e) => {
                say(&format!("the mail cache can't be opened ({e}); running without one"));
                None
            }
        }
    });
    if sample
        && let Some(s) = &store
        && let Err(e) = mailcraft_engine::sample::seed_if_empty_at(s, now, i64::from(offset))
    {
        say(&format!("the sample mailbox couldn't be created: {e}"));
    }
    let mut session = Session::open(store.clone(), Clock::new(now, offset));
    let desktop_data = store.as_ref().and_then(Store::data_files_dir);
    session.oauth_available = mailcraft_net::oauth::available();
    // S/MIME signers are trusted through the roots this computer trusts.
    session.set_trust_roots(mailcraft_net::trust_roots());
    session.set_card_modules(mailcraft_net::card_modules());
    session.portable = portable.dir.is_some();
    // A release build's commit, so File › About can tell its test build from a later one.
    session.build_sha = option_env!("MAILCRAFT_BUILD_SHA").filter(|s| !s.trim().is_empty()).map(str::to_string);
    let state_path = root.as_ref().map(|r| r.join("ui.json"));
    // Test builds update themselves (`self_update.rs`); not the sample mailbox's run, unless it's
    // trying the updater out.
    let updater =
        root.clone().filter(|_| !sample || self_update::trying_out()).and_then(|r| self_update::Updater::start(r, session.build_sha.clone()));

    // The main window opens where it was, at the size it was, maximized if it was.
    let main_place: Option<mailcraft_ui_egui::WindowPlace> = state_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("windows")?.get("main").cloned())
        .and_then(|m| serde_json::from_value(m).ok());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(if sample { "MailCraft – Sample mailbox" } else { "MailCraft" })
            // Matches the desktop entry (StartupWMClass) and icon name in packaging/linux.
            .with_app_id("ai.storyteller.mailcraft")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    let mut options = match main_place {
        Some(p) => eframe::NativeOptions { viewport: p.apply(options.viewport), ..options },
        None => options,
    };
    graphics::configure(&mut options);
    // Cut, Copy and Paste keys are MailCraft's, in every window: Ctrl+V attaches files copied
    // in a file manager that puts no text with them (Explorer, PCManFM; issue #10). Not in a
    // Wayland window, where egui-winit reads the clipboard without waiting on the event loop.
    #[cfg(all(unix, not(target_os = "macos")))]
    let own_keys = display::choose_from_env() == display::Choice::X11 || std::env::var_os("WAYLAND_DISPLAY").is_none();
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let own_keys = true;
    if own_keys {
        options.clipboard_shortcuts = false;
        options.viewport_hooks = Some(Arc::new(ClipboardKeys { pages: mailcraft_ui_egui::alt_code_pages() }));
    }
    // Linux and the BSDs: through XWayland on a Wayland desktop, so attachments drag in and out.
    #[cfg(all(unix, not(target_os = "macos")))]
    if display::choose_from_env() == display::Choice::X11 {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // A window that can't open says why, rather than the app leaving without a word (or
    // staying in the background); a panic on the way is caught for that too.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(options, sample, store, session, desktop_data, base, moved, state_path, (cmd_args, handed), instance, updater)
    }));
    let failed = match ran {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => e.to_string(),
        Err(p) => p
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "it stopped unexpectedly".into()),
    };
    log::error!("the window couldn't open: {failed}");
    say(&format!("the window couldn't open: {failed}"));
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("MailCraft")
        .set_description(cant_open(&failed))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(1)
}

/// How long a clipboard read may wait on the app that copied (Linux and the BSDs).
#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Read the clipboard. On Linux and the BSDs the read runs on a thread of its own and gives up
/// after [`CLIPBOARD_WAIT`]: a Wayland or X11 read waits on the app that copied, with no time
/// limit of its own, so an app that hangs would hang MailCraft's window too (FigCraft found).
#[cfg(all(unix, not(target_os = "macos")))]
fn read_clipboard<T: Send + 'static>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let _ = tx.send(arboard::Clipboard::new().ok().and_then(|mut c| read(&mut c)));
        })
        .ok()?;
    rx.recv_timeout(CLIPBOARD_WAIT).ok().flatten()
}

/// Files copied in a file manager. PCManFM ends its list with a NUL byte, which stayed on the
/// last file's name: of two files copied, one was pasted (issue #10).
fn clipboard_files() -> Vec<PathBuf> {
    let files = read_clipboard(|c| c.get().file_list().ok()).unwrap_or_default();
    let nul = char::from(0);
    files.into_iter().map(|p| p.to_str().filter(|s| s.ends_with(nul)).map_or(p.clone(), |s| PathBuf::from(s.trim_end_matches(nul)))).collect()
}

/// Read the clipboard (Windows' and macOS's reads don't wait on the app that copied).
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn read_clipboard<T>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    read(&mut arboard::Clipboard::new().ok()?)
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

/// Open the window and run the app until it closes.
#[allow(clippy::too_many_arguments)]
fn run(
    options: eframe::NativeOptions,
    sample: bool,
    store: Option<Store>,
    session: Session,
    desktop_data: Option<PathBuf>,
    base: Option<PathBuf>,
    moved: Option<Result<PathBuf, String>>,
    state_path: Option<PathBuf>,
    (cmd_args, handed): (Vec<String>, Vec<String>),
    instance: Option<single_instance::Server>,
    updater: Option<self_update::Updater>,
) -> eframe::Result {
    // The guard stays here, outside the closure, so the handlers live as long as the loop.
    #[cfg(target_os = "macos")]
    let (_finder_guard, finder) = apple_events::listen();
    eframe::run_native(
        "MailCraft",
        options,
        Box::new(move |cc| {
            let args = cmd_args;
            let app_portable = session.portable;
            let backend: Box<dyn Backend> = match store {
                Some(store) => {
                    // Portable: passwords and sign-ins in memory only, never the system's keychain.
                    let creds: Arc<dyn Credentials> =
                        if app_portable { Arc::new(mailcraft_net::worker::MemoryCredentials::default()) } else { Arc::new(Keychain) };
                    let pool = Pool::new(store, creds);
                    let ctx = cc.egui_ctx.clone();
                    pool.set_waker(Arc::new(move || ctx.request_repaint()));
                    Box::new(PoolBackend(pool))
                }
                None => Box::new(mailcraft_ui_egui::NullBackend),
            };
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
                    ..Default::default()
                }),
            );
            if sample {
                app.ui.title_base = "MailCraft – Sample mailbox".into();
            }
            let (regular, bold) = system_fonts();
            // PDFs (print, save as PDF) embed the same font.
            if let Some(r) = &regular {
                app.session.print_fonts = Some((r.clone(), bold.clone()));
            }
            app.install_fonts(&cc.egui_ctx, regular, bold);
            if let Some(v) = state_path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()) {
                app.restore_state(&v);
            }
            if args.iter().any(|a| a == "--light") {
                app.ui.dark = Some(false);
            } else if args.iter().any(|a| a == "--dark") {
                app.ui.dark = Some(true);
            }
            for c in args.windows(2).filter(|w| w.first().is_some_and(|a| a == "--cmd")).filter_map(|w| w.get(1)) {
                let (id, params) = c.split_once('=').unwrap_or((c.as_str(), "{}"));
                match id {
                    "ui.addAccount" => {
                        let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                        let mut d = mailcraft_ui_egui::Dialog::add_account_with(
                            p.get("email").and_then(|v| v.as_str()).unwrap_or_default(),
                            p.get("advanced").and_then(|v| v.as_bool()).unwrap_or(false),
                        );
                        if let mailcraft_ui_egui::Dialog::AddAccount(f) = &mut d {
                            f.graph = p.get("graph").and_then(|v| v.as_bool()).unwrap_or(false);
                            f.pop = p.get("pop").and_then(|v| v.as_bool()).unwrap_or(false);
                        }
                        app.ui.dialog = Some(d);
                    }
                    "ui.file" | "ui.settings" => app.ui.backstage = Some(Backstage::new(Page::Info)),
                    "ui.options" => app.ui.backstage = Some(Backstage::new(Page::Options)),
                    "ui.openExport" => app.ui.backstage = Some(Backstage::new(Page::Open)),
                    "ui.about" => app.ui.backstage = Some(Backstage::new(Page::About)),
                    "ui.newEvent" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::new_event(&app.session, None)),
                    "ui.newContact" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::new_contact()),
                    "ui.signature" => app.ui.dialog = mailcraft_ui_egui::Dialog::signature(&app.session),
                    _ => match serde_json::from_str(params) {
                        Ok(p) => app.run(id, p),
                        Err(e) => say(&format!("--cmd {id}: bad JSON: {e}")),
                    },
                }
            }
            for h in &handed {
                app.open_handed(h);
            }
            let gpu = cc.wgpu_render_state.as_ref().map(|rs| rs.device.clone());
            if let Some(server) = &instance {
                server.connect(&cc.egui_ctx);
            }
            // A Wayland window can't bring itself forward: it asks for attention instead.
            let native_wayland = cfg!(all(unix, not(target_os = "macos")))
                && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
                && display::choose_from_env() != display::Choice::X11;
            Ok(Box::new(App {
                app,
                state_path,
                gpu,
                frames: 0,
                instance,
                #[cfg(target_os = "macos")]
                finder: {
                    apple_events::repaint_on_arrival(&finder, &cc.egui_ctx);
                    // ⌘Q asks before what's being written is lost; without the menu it still
                    // runs, quitting as winit's does.
                    if let Err(e) = apple_events::install_menu(&cc.egui_ctx) {
                        log::warn!("the app menu wasn't put in place, so Quit won't ask first: {e}");
                    }
                    finder
                },
                native_wayland,
                updater,
                saved: Vec::new(),
                looked: std::time::Instant::now(),
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #5: a Microsoft 365 sign-in (a refresh token of about 2,000 characters, and more
    /// with more claims) couldn't be kept in Windows' Credential Manager. Run here against the
    /// real keychain, under an entry of the test's own that it removes.
    #[cfg(windows)]
    #[test]
    fn a_long_sign_in_is_kept_in_the_keychain() {
        let token: String = (0..2000).map(|i| char::from(b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_."[i * 7 % 65])).collect();
        let mut a = AccountConfig::guess("keychain-test@contoso.example");
        a.id = format!("test-{}", std::process::id());
        let k = Keychain;
        let kept = k.set(&a, &token);
        let back = k.get(&a);
        k.delete(&a);
        assert_eq!(kept, Ok(()));
        assert_eq!(back.as_deref(), Some(token.as_str()));
        assert_eq!(k.get(&a), None, "removed with the account");
    }

    /// Trying to break it, on the real keychain: a password kept by an earlier version (UTF-16,
    /// one entry) still reads and is replaced; a secret growing and shrinking leaves no stale
    /// parts; a very long one and one in other scripts come back whole; removing leaves nothing.
    #[cfg(windows)]
    #[test]
    fn the_keychain_keeps_secrets_growing_shrinking_and_old() {
        let user = format!("test-{}|grow|example", std::process::id());
        let parts_left = || {
            (2..=secrets::MAX_PARTS)
                .filter(|n| keyring::Entry::new(SERVICE, &format!("{user}|part{n}")).is_ok_and(|e| e.get_secret().is_ok()))
                .count()
        };
        // As an earlier MailCraft kept it.
        keyring::Entry::new(SERVICE, &user).unwrap().set_password("old pässword").unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some("old pässword"));
        let long = "t".repeat(7000);
        secrets::store(SERVICE, &user, &long).unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some(long.as_str()));
        assert_eq!(parts_left(), 2, "7000 bytes in three entries");
        secrets::store(SERVICE, &user, "short").unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some("short"));
        assert_eq!(parts_left(), 0, "the longer one's parts went");
        let huge: String = "密码🔑-".repeat(3000);
        secrets::store(SERVICE, &user, &huge).unwrap();
        assert_eq!(secrets::load(SERVICE, &user).as_deref(), Some(huge.as_str()));
        secrets::remove(SERVICE, &user);
        assert_eq!(secrets::load(SERVICE, &user), None);
        assert_eq!(parts_left(), 0, "every part removed");
        assert!(secrets::store(SERVICE, &user, &"x".repeat(200_000)).is_err(), "too long is refused, not cut");
        assert_eq!(secrets::load(SERVICE, &user), None, "and nothing half-kept");
        secrets::remove(SERVICE, &user);
    }

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

    #[test]
    fn video_thumbnails_are_made_off_the_ui_thread() {
        let Some(ffmpeg) = video::find_tool("ffmpeg", &[]) else { return };
        let dir = std::env::temp_dir().join(format!("mailcraft-desktop-video-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        // The system's thumbnailer, as when ffmpeg isn't installed.
        let mut d = Desktop { frame_source: Some(video::system()), ..Default::default() };
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match d.video_frame(&clip) {
                VideoFrame::Pending if std::time::Instant::now() < end => std::thread::sleep(std::time::Duration::from_millis(50)),
                f => {
                    assert!(matches!(f, VideoFrame::Ready(_)) || video::system().is_none(), "{f:?}");
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
