//! Android entry point for MailCraft's shared egui front end.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod panic_boundary;

#[cfg(target_os = "android")]
use mailcraft_engine::model::{Event, Job};
#[cfg(target_os = "android")]
use mailcraft_net::worker::Pool;
#[cfg(target_os = "android")]
use mailcraft_ui_egui::Backend;
#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "android")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(target_os = "android")]
use std::time::Instant;
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod activity;
#[cfg(target_os = "android")]
mod background;
#[cfg(target_os = "android")]
mod graphics;
#[cfg(target_os = "android")]
mod lanes;
#[cfg(target_os = "android")]
mod notifications;
#[cfg(target_os = "android")]
mod platform;
#[cfg(target_os = "android")]
mod state;
#[cfg(target_os = "android")]
use state::{AndroidApp, AndroidStateWriter, PendingDraftRestore};
#[cfg(target_os = "android")]
#[path = "../../mailcraft/src/logfile.rs"]
mod logfile;

#[cfg(target_os = "android")]
// SAFETY: NativeActivity looks up this exact exported symbol. This is the only Android entry
// point in the APK; the function body does not use pointers or otherwise rely on unsafe behavior.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use std::sync::Arc;

    use mailcraft_engine::model::dates::Clock;
    use mailcraft_engine::store::Store;
    use mailcraft_ui_egui::MailApp;

    let data_root = app_data_root(&app);
    let private_root = data_root.clone();
    let log_dir = data_root.as_ref().map(|root| root.join("mailcraft"));
    initialize_logging(log_dir.as_deref());
    match activity::system_trust_roots(&app) {
        Ok(certs) => {
            if let Err(error) = mailcraft_net::stream::set_android_system_roots(certs) {
                log::warn!("Android system TLS roots unavailable: {error}");
            }
        }
        Err(error) => log::warn!("Could not read Android system TLS roots: {error}"),
    }
    let ui_state_path = data_root.as_ref().map(|root| root.join("mailcraft").join("ui.json"));
    let incoming_dir = data_root.map(|root| root.join("mailcraft").join("imports"));
    let (handoff_route, handoff_rx) = register_handoff_route();
    let incoming = incoming_android_intent(&app);
    if handoff_rx.is_some() {
        cleanup_shared_attachment_staging(incoming_dir.as_deref());
    }
    let (handoff_pending, handoff_worker) = route_android_handoff(app.clone(), incoming, incoming_dir.clone(), handoff_route.clone());
    let Some(handoff_rx) = handoff_rx else {
        // Shared content URI grants last only while this receiving Activity is alive.
        if handoff_worker.is_some_and(|worker| worker.join().is_err()) {
            log::warn!("Android's shared-file handoff stopped before it finished staging documents");
        }
        platform::finish_activity(&app);
        return;
    };
    cleanup_stale_open_files();
    if let Err(error) = activity::register_middleware_activity(&app) {
        log::warn!("Android could not register the system file picker's Activity: {error}");
    }
    platform::cleanup_pending_downloads(&app);
    let restored_ui = ui_state_path.as_deref().and_then(state::read_saved_state);
    let saved_work_offline = restored_ui
        .as_ref()
        .and_then(|state| state.get("prefs"))
        .and_then(|prefs| serde_json::from_value::<mailcraft_engine::Prefs>(prefs.clone()).ok())
        .is_some_and(|prefs| prefs.work_offline);
    set_work_offline(saved_work_offline);
    let persistent_store = private_root.and_then(|root| {
        let cache_root = root.join("mailcraft");
        let retry_until = Instant::now().checked_add(std::time::Duration::from_secs(6));
        loop {
            match Store::open_alone(&cache_root) {
                Ok(store) => break Some(store),
                Err(mailcraft_engine::store::StoreError::Io { source, .. })
                    if source.to_string().contains("open in another copy of MailCraft")
                        && retry_until.is_some_and(|deadline| Instant::now() < deadline) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(error) => {
                    eprintln!("MailCraft could not open its private offline cache: {error}");
                    break None;
                }
            }
        }
    });
    let using_memory_cache = persistent_store.is_none();
    let store = persistent_store.unwrap_or_else(Store::memory);
    if using_memory_cache {
        eprintln!("MailCraft is using a temporary in-memory cache for this run");
    }
    let local_now = chrono::Local::now();
    let mut session = mailcraft_engine::Session::open(Some(store.clone()), Clock::new(local_now.timestamp(), local_now.offset().local_minus_utc()));
    let retained_imports: Vec<_> = session
        .accounts
        .iter()
        .filter_map(|account| session.data_source(&account.config.id).map(|source| std::path::PathBuf::from(&source.path)))
        .chain(session.imports().map(|(_, import)| std::path::PathBuf::from(&import.path)))
        .collect();
    cleanup_interrupted_picker_staging(incoming_dir.as_deref(), &retained_imports);
    session.oauth_available = mailcraft_net::oauth::available();
    session.set_trust_roots(mailcraft_net::trust_roots());
    session.set_card_modules(mailcraft_net::card_modules());
    let (credentials, keyring_warning): (Arc<dyn mailcraft_net::worker::Credentials>, bool) = match android_native_keyring_store::Store::new() {
        Ok(keyring) => {
            keyring_core::set_default_store(keyring);
            (Arc::new(AndroidCredentials), false)
        }
        Err(error) => {
            log::error!("Android Keystore credentials could not be initialized: {error}");
            (Arc::new(mailcraft_net::worker::MemoryCredentials::default()), true)
        }
    };
    let pool = Arc::new(Mutex::new(Pool::new(store, credentials)));
    set_active_pool(pool.clone());
    let active_pool_on_exit = pool.clone();

    let mut options = eframe::NativeOptions { android_app: Some(app.clone()), ..Default::default() };
    graphics::configure(&mut options);
    let app_handoff_route = handoff_route.clone();
    let create_app = Box::new(move |cc: &eframe::CreationContext<'_>| -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
        let pool = pool;
        let ctx = cc.egui_ctx.clone();
        let repaint = cc.egui_ctx.clone();
        activity::set_system_back_context(Some(repaint.clone()));
        app_handoff_route.set_repaint(repaint.clone());
        if let Ok(pool) = pool.lock() {
            pool.set_waker(Arc::new(move || ctx.request_repaint()));
        }
        let mut mail = MailApp::new(session, Box::new(PoolBackend(pool)), Box::new(AndroidServices::new(app.clone(), repaint, incoming_dir)));
        let pending_draft_restore = restored_ui.as_ref().and_then(PendingDraftRestore::from_saved_state);
        if let Some(state) = &restored_ui {
            mail.restore_state(state);
        }
        let was_work_offline = mail.session.prefs.work_offline;
        set_work_offline(was_work_offline);
        if keyring_warning {
            let warning = mailcraft_ui_egui::i18n::t(
                "Android's encrypted credential store is unavailable. MailCraft will keep credentials in memory until it closes.",
            );
            mail.session.notice(warning.to_owned(), true);
        }
        Ok(Box::new(AndroidApp {
            mail,
            pending_draft_restore,
            state_writer: AndroidStateWriter::new(ui_state_path),
            handoff_rx,
            handoff_route: app_handoff_route.clone(),
            activity: app.clone(),
            handoff_remaining: handoff_pending,
            cache_warning: using_memory_cache,
            system_insets: platform::system_insets(&app),
            system_bar_colors: None,
            insets_checked_at: Instant::now(),
            network_available: platform::network_available(&app),
            network_checked_at: Instant::now(),
            network_sync_pending: false,
            resume_sync_pending: false,
            was_work_offline,
            was_focused: None,
            // Session::open already queues Start jobs, which perform the initial full sync.
            // Throttle the first focus event so it doesn't queue a second full pass.
            last_resume_sync: Some(Instant::now()),
            draft_checkpoint_at: Instant::now(),
            last_input_at: Instant::now(),
            display_checked_at: Instant::now(),
        }))
    });
    if let Err(error) = eframe::run_native("MailCraft", options, create_app) {
        eprintln!("MailCraft Android window failed: {error}");
    }
    activity::set_system_back_context(None);
    clear_active_pool(&active_pool_on_exit);
    handoff_route.deactivate();
}

#[cfg(target_os = "android")]
type SharedPool = Arc<Mutex<Pool>>;

#[cfg(target_os = "android")]
static ACTIVE_POOL: OnceLock<Mutex<Option<SharedPool>>> = OnceLock::new();
#[cfg(target_os = "android")]
static WORK_OFFLINE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "android")]
static ANDROID_LOGGING: OnceLock<()> = OnceLock::new();
#[cfg(target_os = "android")]
static ANDROID_PANIC_HOOK: OnceLock<()> = OnceLock::new();

#[cfg(target_os = "android")]
fn ensure_panic_hook() {
    ANDROID_PANIC_HOOK.get_or_init(logfile::log_panics);
}

#[cfg(target_os = "android")]
fn initialize_logging(log_dir: Option<&std::path::Path>) {
    ANDROID_LOGGING.get_or_init(|| {
        if let Some(path) = logfile::start(log_dir, cfg!(debug_assertions)) {
            log::info!("MailCraft for Android started; log at {}", path.display());
        }
        ensure_panic_hook();
    });
}

#[cfg(target_os = "android")]
fn set_work_offline(offline: bool) {
    WORK_OFFLINE.store(offline, Ordering::Relaxed);
}

#[cfg(target_os = "android")]
fn is_work_offline() -> bool {
    WORK_OFFLINE.load(Ordering::Relaxed)
}

#[cfg(target_os = "android")]
fn set_active_pool(pool: SharedPool) {
    if let Ok(mut active) = ACTIVE_POOL.get_or_init(|| Mutex::new(None)).lock() {
        *active = Some(pool);
    }
}

#[cfg(target_os = "android")]
fn active_pool() -> Option<SharedPool> {
    ACTIVE_POOL.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().cloned()
}

#[cfg(target_os = "android")]
fn clear_active_pool(pool: &SharedPool) {
    if let Ok(mut active) = ACTIVE_POOL.get_or_init(|| Mutex::new(None)).lock()
        && active.as_ref().is_some_and(|current| Arc::ptr_eq(current, pool))
    {
        *active = None;
    }
}

#[cfg(target_os = "android")]
fn app_data_root(app: &winit::platform::android::activity::AndroidApp) -> Option<std::path::PathBuf> {
    app_data_root_from_paths(app.internal_data_path(), platform::no_backup_data_path(app))
}

/// Resolve and, when possible, migrate the cache directory used by both foreground and scheduled work.
#[cfg(target_os = "android")]
pub(crate) fn app_data_root_from_paths(original: Option<std::path::PathBuf>, no_backup: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    let Some(no_backup) = no_backup else { return original };
    let Some(original) = original else { return Some(no_backup) };
    if original == no_backup {
        return Some(no_backup);
    }
    // The foreground bootstrap and WorkManager can resolve paths concurrently on first launch.
    static RESOLVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RESOLVE_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let old_mailcraft = original.join("mailcraft");
    let new_mailcraft = no_backup.join("mailcraft");
    if !old_mailcraft.exists() {
        return Some(no_backup);
    }
    if new_mailcraft.exists() {
        eprintln!("MailCraft found data in both Android storage directories; keeping the existing files directory");
        return Some(original);
    }
    let migration = std::fs::create_dir_all(&no_backup).and_then(|()| std::fs::rename(&old_mailcraft, &new_mailcraft));
    match migration {
        Ok(()) => Some(no_backup),
        // A second process may have completed the same migration while this rename was running.
        Err(_) if !old_mailcraft.exists() && new_mailcraft.exists() => Some(no_backup),
        Err(error) => {
            eprintln!("MailCraft could not move its data into Android's no-backup directory; keeping the existing files: {error}");
            Some(original)
        }
    }
}

#[cfg(target_os = "android")]
struct PoolBackend(SharedPool);

#[cfg(target_os = "android")]
impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).submit(job);
    }

    fn poll(&mut self) -> Vec<Event> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).poll()
    }

    fn shutdown(&mut self) {
        clear_active_pool(&self.0);
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).shutdown();
    }

    fn flush_changes(&mut self, timeout: std::time::Duration) -> bool {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).flush_changes(timeout)
    }
}

#[cfg(target_os = "android")]
struct AndroidCredentials;

#[cfg(target_os = "android")]
impl AndroidCredentials {
    const SERVICE: &'static str = "ai.storyteller.mailcraft";

    fn user(account: &mailcraft_engine::model::AccountConfig, kind: &str) -> String {
        format!("{}|{kind}", account.id)
    }

    fn get(account: &mailcraft_engine::model::AccountConfig, kind: &str) -> Option<String> {
        keyring_core::Entry::new(Self::SERVICE, &Self::user(account, kind)).ok()?.get_password().ok()
    }

    fn set(account: &mailcraft_engine::model::AccountConfig, kind: &str, secret: &str) -> Result<(), String> {
        keyring_core::Entry::new(Self::SERVICE, &Self::user(account, kind))
            .and_then(|entry| entry.set_password(secret))
            .map_err(|error| error.to_string())
    }

    fn delete(account: &mailcraft_engine::model::AccountConfig, kind: &str) {
        let result = keyring_core::Entry::new(Self::SERVICE, &Self::user(account, kind)).and_then(|entry| entry.delete_credential());
        if let Err(error) = result
            && !matches!(error, keyring_core::Error::NoEntry)
        {
            // Account removal has already committed in the session; leave a diagnostic without
            // writing the credential name or any secret into the log.
            log::warn!("MailCraft could not remove an Android Keystore credential after account removal");
        }
    }
}

#[cfg(target_os = "android")]
impl mailcraft_net::worker::Credentials for AndroidCredentials {
    fn get(&self, account: &mailcraft_engine::model::AccountConfig) -> Option<String> {
        Self::get(account, "imap")
    }

    fn set(&self, account: &mailcraft_engine::model::AccountConfig, password: &str) -> Result<(), String> {
        Self::set(account, "imap", password)
    }

    fn delete(&self, account: &mailcraft_engine::model::AccountConfig) {
        Self::delete(account, "imap");
    }

    fn get_smtp(&self, account: &mailcraft_engine::model::AccountConfig) -> Option<String> {
        Self::get(account, "smtp")
    }

    fn set_smtp(&self, account: &mailcraft_engine::model::AccountConfig, password: &str) -> Result<(), String> {
        Self::set(account, "smtp", password)
    }

    fn delete_smtp(&self, account: &mailcraft_engine::model::AccountConfig) {
        Self::delete(account, "smtp");
    }

    fn get_secret(&self, account: &mailcraft_engine::model::AccountConfig, what: &str) -> Option<String> {
        Self::get(account, what)
    }

    fn set_secret(&self, account: &mailcraft_engine::model::AccountConfig, what: &str, secret: &str) -> Result<(), String> {
        Self::set(account, what, secret)
    }

    fn delete_secret(&self, account: &mailcraft_engine::model::AccountConfig, what: &str) {
        Self::delete(account, what);
    }
}

#[cfg(target_os = "android")]
mod services;
#[cfg(target_os = "android")]
use services::{
    AndroidOpen, AndroidServices, HandoffRoute, cleanup_interrupted_picker_staging, cleanup_shared_attachment_staging, cleanup_stale_open_files,
    incoming_android_intent, register_handoff_route, route_android_handoff,
};
