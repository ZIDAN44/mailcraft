//! MailCraft desktop app.
//!
//! Usage: `mailcraft [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]…`
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
use mailcraft_ui_egui::{Backend, MailApp, Services, theme};

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

/// The window icon, drawn in code: a white envelope on the app colour.
fn app_icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let c = theme::APP_COLOR;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32, y as f32);
            let corner = 12.0f32;
            let dx = (corner - fx).max(fx - (n as f32 - 1.0 - corner)).max(0.0);
            let dy = (corner - fy).max(fy - (n as f32 - 1.0 - corner)).max(0.0);
            let inside = dx * dx + dy * dy <= corner * corner;
            let env = (12..=52).contains(&x) && (18..=46).contains(&y);
            let edge = env && (x == 12 || x == 52 || y == 18 || y == 46 || x == 13 || x == 51 || y == 19 || y == 45);
            let flap = env && {
                let mid = 32.0;
                let yy = 18.0 + (fx - mid).abs().mul_add(-0.7, 20.0);
                (fy - yy).abs() < 1.5
            };
            let px = if !inside {
                [0, 0, 0, 0]
            } else if edge || flap {
                [255, 255, 255, 255]
            } else {
                [c.r(), c.g(), c.b(), 255]
            };
            let i = (y * n + x) * 4;
            if let Some(slot) = rgba.get_mut(i..i + 4) {
                slot.copy_from_slice(&px);
            }
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
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
