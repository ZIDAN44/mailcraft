//! Android entry point for MailCraft's shared egui front end.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
use eframe::egui;
#[cfg(target_os = "android")]
use mailcraft_engine::model::{Event, Job};
#[cfg(target_os = "android")]
use mailcraft_ui_egui::{Backend, Services};

#[cfg(target_os = "android")]
// SAFETY: NativeActivity looks up this exact exported symbol. This is the only Android entry
// point in the APK; the function body does not use pointers or otherwise rely on unsafe behavior.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use mailcraft_engine::model::dates::Clock;
    use mailcraft_engine::store::Store;
    use mailcraft_net::worker::{MemoryCredentials, Pool};
    use mailcraft_ui_egui::MailApp;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs().min(i64::MAX as u64) as i64);
    let store = app.internal_data_path().and_then(|root| Store::open_alone(root.join("mailcraft")).ok());
    if store.is_none() {
        eprintln!("MailCraft could not open its private offline cache; using a temporary cache for this run");
    }
    let session = mailcraft_engine::Session::open(store.clone(), Clock::new(now, 0));
    let pool = store.map(|store| Pool::new(store, Arc::new(MemoryCredentials::default())));

    let options = eframe::NativeOptions { android_app: Some(app), ..Default::default() };
    let create_app = Box::new(move |cc: &eframe::CreationContext<'_>| -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
        let pool = pool;
        let ctx = cc.egui_ctx.clone();
        if let Some(pool) = &pool {
            pool.set_waker(Arc::new(move || ctx.request_repaint()));
        }
        let mail = MailApp::new(session, Box::new(PoolBackend(pool)), Box::new(AndroidServices));
        Ok(Box::new(AndroidApp { mail }))
    });
    if let Err(error) = eframe::run_native("MailCraft", options, create_app) {
        eprintln!("MailCraft Android window failed: {error}");
    }
}

#[cfg(target_os = "android")]
struct PoolBackend(Option<mailcraft_net::worker::Pool>);

#[cfg(target_os = "android")]
impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        if let Some(pool) = &mut self.0 {
            pool.submit(job);
        }
    }

    fn poll(&mut self) -> Vec<Event> {
        self.0.as_mut().map(mailcraft_net::worker::Pool::poll).unwrap_or_default()
    }

    fn shutdown(&mut self) {
        if let Some(pool) = &mut self.0 {
            pool.shutdown();
        }
    }
}

#[cfg(target_os = "android")]
struct AndroidServices;

#[cfg(target_os = "android")]
impl Services for AndroidServices {
    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }

    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn save_path(&mut self, _suggested: &str) -> Option<std::path::PathBuf> {
        None
    }

    fn open_path(&mut self, _path: &std::path::Path) -> Result<(), String> {
        Err("Opening files is not available on Android yet".into())
    }
}

#[cfg(target_os = "android")]
struct AndroidApp {
    mail: mailcraft_ui_egui::MailApp,
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.mail.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Leave room for Android's status bar; window safe-area support is not available in the
        // currently pinned winit release.
        egui::Panel::top("android_status_bar_inset").show(ui, |ui| ui.set_height(28.0));
        self.mail.ui(ui);
    }
}

#[cfg(not(target_os = "android"))]
pub fn android_target_only() {}
