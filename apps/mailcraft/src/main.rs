//! MailCraft desktop app.
//!
//! Usage: `mailcraft [--version] [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]…`
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

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{AccountConfig, Event, Job};
use mailcraft_engine::store::{Store, location};
use mailcraft_net::worker::{Credentials, Pool};
use mailcraft_ui_egui::{Backend, MailApp, Services, VideoFrame};

/// Passwords in the system keychain (Windows Credential Manager, macOS Keychain, Secret Service).
struct Keychain;

#[cfg(windows)]
mod toast;
mod video;
#[cfg(all(unix, not(target_os = "macos")))]
mod xdnd;

const SERVICE: &str = "MailCraft";

fn entry(a: &AccountConfig) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{}|{}|{}", a.id, a.username, a.imap.host)).map_err(|e| e.to_string())
}

impl Credentials for Keychain {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        entry(account).ok()?.get_password().ok()
    }
    fn set(&self, account: &AccountConfig, password: &str) -> Result<(), String> {
        entry(account)?.set_password(password).map_err(|e| e.to_string())
    }
    fn delete(&self, account: &AccountConfig) {
        if let Ok(e) = entry(account) {
            let _ = e.delete_credential();
        }
    }
}

struct PoolBackend(Pool);

impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        self.0.submit(job);
    }
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
}

#[derive(Default)]
struct Desktop {
    /// Where video frames come from (looked up once).
    frame_source: Option<Option<video::Source>>,
    /// Video frames being made or made, by file.
    frames: std::collections::HashMap<PathBuf, Arc<std::sync::Mutex<Option<VideoFrame>>>>,
    /// The data files' folder in use, and the app folder whose data files can be moved (`None`
    /// when the place was chosen at start: `--data`, `--sample`, `MAILCRAFT_DATA`).
    data: Option<PathBuf>,
    base: Option<PathBuf>,
    /// How a move carried out at this start went (told once).
    moved: Option<Result<PathBuf, String>>,
}

impl Services for Desktop {
    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        rfd::FileDialog::new().set_title("Attach files").pick_files().unwrap_or_default()
    }
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("Save as").set_file_name(suggested).save_file()
    }
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        shell_open(path.as_os_str())
    }
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        self.data.clone().map(|d| (d, self.base.is_some()))
    }
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        self.moved.take()
    }
    fn pending_data_move(&self) -> Option<PathBuf> {
        self.base.as_deref().and_then(location::pending_move)
    }
    fn move_data_folder(&mut self, to: Option<&Path>) -> Result<Option<PathBuf>, String> {
        let base = self.base.as_deref().ok_or("the data folder was chosen when MailCraft started")?;
        match to {
            Some(to) => location::schedule_move(base, to).map(Some),
            None => {
                location::cancel_move(base);
                Ok(None)
            }
        }
    }
    fn craft_app(&mut self, command: &str) -> Option<PathBuf> {
        let exe = if cfg!(windows) { format!("{command}.exe") } else { command.to_string() };
        // On the PATH (the CLI installers and Linux packages put it there)…
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                let p = dir.join(&exe);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        // …or where each platform's installer puts apps.
        let title = match command {
            "pdfcraft" => "PdfCraft",
            "wordcraft" => "WordCraft",
            "gridcraft" => "GridCraft",
            "deckcraft" => "DeckCraft",
            "photocraft" => "PhotoCraft",
            "filmcraft" => "FilmCraft",
            "soundcraft" => "SoundCraft",
            _ => return None,
        };
        let mut places: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            for var in ["ProgramFiles", "LOCALAPPDATA"] {
                if let Some(base) = std::env::var_os(var) {
                    let base = PathBuf::from(base);
                    places.push(base.join(title).join(&exe));
                    places.push(base.join("Programs").join(title).join(&exe));
                }
            }
        } else if cfg!(target_os = "macos") {
            places.push(PathBuf::from(format!("/Applications/{title}.app")));
        } else {
            places.push(PathBuf::from(format!("/opt/{command}/bin/{command}")));
        }
        places.into_iter().find(|p| p.exists())
    }
    fn open_with(&mut self, app: &Path, file: &Path) -> Result<(), String> {
        let mut cmd = if cfg!(target_os = "macos") && app.extension().is_some_and(|e| e == "app") {
            let mut c = std::process::Command::new("open");
            c.arg("-a").arg(app).arg(file);
            c
        } else {
            let mut c = std::process::Command::new(app);
            c.arg(file);
            c
        };
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    fn cjk_font(&mut self, lang: &str) -> Option<(Vec<u8>, u32)> {
        // The language's own font first (Chinese, Japanese and Korean draw shared characters
        // differently), then any that has the characters at all.
        let (windows, macos, noto_index): (&[&str], &[&str], u32) = match lang {
            "ja" => (&["YuGothM.ttc", "msgothic.ttc"], &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans GB.ttc"], 0),
            "ko" => (&["malgun.ttf", "gulim.ttc"], &["AppleSDGothicNeo.ttc"], 1),
            "zh-hant" => (&["msjh.ttc", "mingliu.ttc"], &["PingFang.ttc"], 3),
            _ => (&["msyh.ttc", "simsun.ttc"], &["PingFang.ttc", "Hiragino Sans GB.ttc"], 2),
        };
        let mut candidates: Vec<(String, u32)> = Vec::new();
        if cfg!(target_os = "windows") {
            let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            for f in windows.iter().chain(["msyh.ttc", "YuGothM.ttc", "malgun.ttf"].iter()) {
                candidates.push((format!("{dir}\\Fonts\\{f}"), 0));
            }
        } else if cfg!(target_os = "macos") {
            for f in macos.iter().chain(["PingFang.ttc", "AppleSDGothicNeo.ttc"].iter()) {
                candidates.push((format!("/System/Library/Fonts/{f}"), 0));
            }
        } else {
            // Noto Sans CJK collections hold Japanese, Korean, Simplified and Traditional faces.
            for p in [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            ] {
                candidates.push((p.to_string(), noto_index));
            }
            candidates.push(("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf".into(), 0));
        }
        candidates.iter().find_map(|(p, i)| std::fs::read(p).ok().map(|b| (b, *i)))
    }
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        // Fonts with both Arabic and Hebrew first.
        let candidates: &[&str] = if cfg!(target_os = "windows") {
            &["C:\\Windows\\Fonts\\tahoma.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
        } else if cfg!(target_os = "macos") {
            &["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf", "/System/Library/Fonts/Supplemental/Tahoma.ttf"]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            ]
        };
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }
    fn can_drag_out(&self) -> bool {
        // Elsewhere, under X11 only: a Wayland window isn't an X window.
        cfg!(any(target_os = "windows", target_os = "macos"))
            || (std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_none())
    }
    fn video_frame(&mut self, path: &Path) -> VideoFrame {
        if let Some(slot) = self.frames.get(path) {
            return slot.lock().map(|g| g.clone().unwrap_or(VideoFrame::Pending)).unwrap_or(VideoFrame::Unavailable);
        }
        let source = self.frame_source.get_or_insert_with(video::source).clone();
        let Some(source) = source else { return VideoFrame::Unavailable };
        let slot = Arc::new(std::sync::Mutex::new(None));
        self.frames.insert(path.to_path_buf(), slot.clone());
        let video = path.to_path_buf();
        let spawned = std::thread::Builder::new().name("video-thumbnail".into()).spawn(move || {
            let png = video::frame(&source, &video);
            if let Ok(mut g) = slot.lock() {
                *g = Some(png.map_or(VideoFrame::Unavailable, VideoFrame::Ready));
            }
        });
        if spawned.is_err() {
            return VideoFrame::Unavailable;
        }
        VideoFrame::Pending
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("Save attachments to").pick_folder()
    }
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Open data file")
            .add_filter("Data files", &["pst", "mailcraft"])
            .add_filter("Outlook data files", &["pst"])
            .add_filter("MailCraft data files", &["mailcraft"])
            .pick_file()
    }
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Insert pictures")
            .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
            .pick_files()
            .unwrap_or_default()
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        let img = arboard::Clipboard::new().ok()?.get_image().ok()?;
        let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
        let rgba = image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(png)
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        // Web addresses go to Windows' URL handler: ShellExec_RunDLL (right for files) started
        // and quietly showed nothing, so sign-in pages never reached the browser.
        #[cfg(windows)]
        if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
            return std::process::Command::new("rundll32.exe")
                .arg("url.dll,FileProtocolHandler")
                .arg(url)
                .spawn()
                .map(drop)
                .map_err(|e| e.to_string());
        }
        shell_open(std::ffi::OsStr::new(url))
    }
    fn notify(&mut self, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        // Off the UI thread: the notification service can be slow, or missing.
        std::thread::spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&body).appname("MailCraft");
            // Windows names a toast after its app id (PowerShell's when none is given).
            #[cfg(windows)]
            if toast::registered() {
                n.app_id(toast::APP_ID);
            }
            if let Err(e) = n.show() {
                log::warn!("notification: {e}");
            }
        });
    }
}

/// Open a file or a web address with the system's choice of app, as Explorer would.
///
/// On Windows this goes through `rundll32 shell32.dll,ShellExec_RunDLL`, a process of its own:
/// opening from MailCraft (directly, or through the hidden PowerShell the `open` crate starts)
/// silently did nothing for apps such as Acrobat, and a file no app opens gets Windows' "How do
/// you want to open this file?", as Outlook does.
fn shell_open(target: &std::ffi::OsStr) -> Result<(), String> {
    #[cfg(windows)]
    {
        // A folder opens in Explorer: ShellExec_RunDLL quietly does nothing with one (Options ›
        // Data files › Open folder never opened).
        if std::path::Path::new(target).is_dir() {
            return std::process::Command::new("explorer.exe").arg(target).spawn().map(drop).map_err(|e| e.to_string());
        }
        std::process::Command::new("rundll32.exe").arg("shell32.dll,ShellExec_RunDLL").arg(target).spawn().map(drop).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open::that_detached(target).map_err(|e| e.to_string())
    }
}

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

/// `--version`: the version, plus the commit and date of release builds (packaging/common.sh).
fn version_line() -> String {
    let v = format!("mailcraft {}", env!("CARGO_PKG_VERSION"));
    match (option_env!("MAILCRAFT_BUILD_SHA"), option_env!("MAILCRAFT_BUILD_DATE")) {
        (Some(sha), Some(date)) if !sha.is_empty() => format!("{v} ({}, {date})", sha.get(..9).unwrap_or(sha)),
        _ => v,
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui);
        if let Some(path) = self.app.take_drag_out() {
            drag_out(frame, path);
        }
    }
    fn on_exit(&mut self) {
        if let Some(p) = &self.state_path
            && let Ok(bytes) = serde_json::to_vec_pretty(&self.app.saved_state())
        {
            let _ = mailcraft_engine::store::write_atomic(p, &bytes);
        }
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", version_line());
        return Ok(());
    }
    let cmd_args = args.clone();
    let sample = args.iter().any(|a| a == "--sample");
    // Each account is a data file in Documents\MailCraft Files (as Outlook keeps .pst files in
    // Outlook Files); the account list and the app's state stay in the app folder. The data files'
    // folder can be moved (File › Options) when it's the usual one: a move asked for is carried
    // out now, before anything opens them.
    let base = (!sample && !args.iter().any(|a| a == "--data") && std::env::var_os("MAILCRAFT_DATA").is_none_or(|d| d.is_empty()))
        .then(location::base_root)
        .flatten();
    let mut moved = match base.as_deref().map(location::apply_pending_move) {
        Some(Ok(Some(to))) => Some(Ok(to)),
        Some(Err(e)) => Some(Err(e)),
        _ => None,
    };
    let data =
        args.iter().position(|a| a == "--data").and_then(|i| args.get(i + 1)).map(PathBuf::from).or_else(mailcraft_engine::store::default_root);
    let root = data.map(|d| if sample { d.join("sample") } else { d });
    // The sample mailbox stays a folder of files; anything else keeps its data files beside it
    // (`--data`) or in Documents.
    let files =
        (!sample).then(|| root.as_ref().map(|r| base.as_deref().map_or_else(|| r.join(location::FILES_FOLDER), location::files_dir))).flatten();

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
                eprintln!("mailcraft: the mail cache can't be opened ({e}); running without one");
                None
            }
        }
    });
    if sample
        && let Some(s) = &store
        && let Err(e) = mailcraft_engine::sample::seed_if_empty_at(s, now, i64::from(offset))
    {
        eprintln!("mailcraft: the sample mailbox couldn't be created: {e}");
    }
    let mut session = Session::open(store.clone(), Clock::new(now, offset));
    let desktop_data = store.as_ref().and_then(Store::data_files_dir);
    session.oauth_available = mailcraft_net::oauth::available();
    let state_path = root.as_ref().map(|r| r.join("ui.json"));

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
    let options = match main_place {
        Some(p) => eframe::NativeOptions { viewport: p.apply(options.viewport), ..options },
        None => options,
    };
    eframe::run_native(
        "MailCraft",
        options,
        Box::new(move |cc| {
            let args = cmd_args;
            let backend: Box<dyn Backend> = match store {
                Some(store) => {
                    let pool = Pool::new(store, Arc::new(Keychain));
                    let ctx = cc.egui_ctx.clone();
                    pool.set_waker(Arc::new(move || ctx.request_repaint()));
                    Box::new(PoolBackend(pool))
                }
                None => Box::new(mailcraft_ui_egui::NullBackend),
            };
            let mut app = MailApp::new(session, backend, Box::new(Desktop { data: desktop_data, base, moved, ..Default::default() }));
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
                    _ => match serde_json::from_str(params) {
                        Ok(p) => app.run(id, p),
                        Err(e) => eprintln!("--cmd {id}: bad JSON: {e}"),
                    },
                }
            }
            Ok(Box::new(App { app, state_path }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
