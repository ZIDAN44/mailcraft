//! MailCraft desktop app.
//!
//! Usage: `mailcraft [--version] [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]…`
//!
//! `--sample` opens a separate, local-only sample mailbox (no server needed). `--data` (or the
//! `MAILCRAFT_DATA` environment variable) chooses where mail is cached. `--cmd` runs engine
//! commands at start-up (`--cmd 'list.open={"key":"sample:1:1:INBOX"}'`); `ui.addAccount` (optionally
//! `={"email":…,"advanced":true}`) and `ui.settings` open those dialogs.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{AccountConfig, Event, Job};
use mailcraft_engine::store::Store;
use mailcraft_net::worker::{Credentials, Pool};
use mailcraft_ui_egui::{Backend, MailApp, Services};

/// Passwords in the system keychain (Windows Credential Manager, macOS Keychain, Secret Service).
struct Keychain;

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

struct Desktop;

impl Services for Desktop {
    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        rfd::FileDialog::new().set_title("Attach files").pick_files().unwrap_or_default()
    }
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("Save attachment").set_file_name(suggested).save_file()
    }
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        open::that_detached(path).map_err(|e| e.to_string())
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
        open::that_detached(url).map_err(|e| e.to_string())
    }
    fn notify(&mut self, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        // Off the UI thread: the notification service can be slow, or missing.
        std::thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new().summary(&title).body(&body).appname("MailCraft").show() {
                log::warn!("notification: {e}");
            }
        });
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

struct App {
    app: MailApp,
    state_path: Option<PathBuf>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
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
    let data =
        args.iter().position(|a| a == "--data").and_then(|i| args.get(i + 1)).map(PathBuf::from).or_else(mailcraft_engine::store::default_root);
    let root = data.map(|d| if sample { d.join("sample") } else { d });

    let (now, offset) = Desktop.clock();
    let store = root.as_ref().and_then(|r| match Store::open(r) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("mailcraft: the mail cache can't be opened ({e}); running without one");
            None
        }
    });
    if sample
        && let Some(s) = &store
        && let Err(e) = mailcraft_engine::sample::seed_if_empty(s, now)
    {
        eprintln!("mailcraft: the sample mailbox couldn't be created: {e}");
    }
    let session = Session::open(store.clone(), Clock::new(now, offset));
    let state_path = root.as_ref().map(|r| r.join("ui.json"));

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
            let mut app = MailApp::new(session, backend, Box::new(Desktop));
            if sample {
                app.ui.title_base = "MailCraft – Sample mailbox".into();
            }
            let (regular, bold) = system_fonts();
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
                        app.ui.dialog = Some(mailcraft_ui_egui::Dialog::add_account_with(
                            p.get("email").and_then(|v| v.as_str()).unwrap_or_default(),
                            p.get("advanced").and_then(|v| v.as_bool()).unwrap_or(false),
                        ));
                    }
                    "ui.settings" => app.ui.dialog = Some(mailcraft_ui_egui::Dialog::Settings),
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
