//! Android document, notification and clipboard services.
use super::*;
use eframe::egui;
use mailcraft_ui_egui::Services;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

/// Bound Android's display name by bytes; the shared sanitizer limits characters, not UTF-8 bytes.
fn safe_download_name(name: &str) -> String {
    const MAX_BYTES: usize = 240;
    let extension_start = name.rfind('.').filter(|index| {
        *index > 0
            && name.len().saturating_sub(*index) <= 25
            && name
                .get((*index).saturating_add(1)..)
                .is_some_and(|extension| !extension.is_empty() && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    });
    let (stem, extension) = extension_start.map_or((name, ""), |index| (&name[..index], &name[index..]));
    let stem_limit = MAX_BYTES.saturating_sub(extension.len());
    let mut safe = String::with_capacity(stem.len().min(stem_limit));
    for character in stem.chars() {
        let bidi_control = matches!(character as u32, 0x061C | 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069);
        let character = if character.is_control() || bidi_control || matches!(character, '/' | '\\') { '_' } else { character };
        if safe.len().saturating_add(character.len_utf8()) > stem_limit {
            break;
        }
        safe.push(character);
    }
    let safe = safe.trim_matches([' ', '.']);
    let stem = if safe.is_empty() { "download" } else { safe };
    format!("{stem}{extension}")
}

mod display;
mod downloads;
mod handoff;
mod implementation;
mod picking;
mod save_as;
mod video;
pub(super) use downloads::cleanup_stale_open_files;
pub(super) use handoff::{
    AndroidOpen, CachedDark, CachedZone, HandoffRoute, cleanup_interrupted_picker_staging, cleanup_shared_attachment_staging,
    incoming_android_intent, register_handoff_route, route_android_handoff,
};
use picking::{picker_fallback, picker_file_limit, read_picked_file};

#[cfg(target_os = "android")]
pub(super) struct AndroidServices {
    activity: winit::platform::android::activity::AndroidApp,
    repaint: egui::Context,
    incoming_dir: Option<std::path::PathBuf>,
    clipboard_files: Sender<Result<Vec<std::path::PathBuf>, String>>,
    clipboard_files_rx: Receiver<Result<Vec<std::path::PathBuf>, String>>,
    clipboard_files_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    clipboard_picture: Sender<Option<Vec<u8>>>,
    clipboard_picture_rx: Receiver<Option<Vec<u8>>>,
    clipboard_picture_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    document_results_rx: Receiver<activity::DocumentResult>,
    active_document_picker: Option<(i32, String)>,
    pending_saves: HashMap<i32, PendingSave>,
    picked: Sender<(String, String, Vec<u8>)>,
    picked_rx: Receiver<(String, String, Vec<u8>)>,
    errors: Sender<String>,
    errors_rx: Receiver<String>,
    downloaded: Sender<String>,
    downloaded_rx: Receiver<String>,
    download_finished: Sender<()>,
    download_finished_rx: Receiver<()>,
    download_queue: VecDeque<AndroidDownload>,
    download_queue_bytes: u64,
    download_active_bytes: u64,
    open_cache_bytes: u64,
    open_cache_files: usize,
    download_active: bool,
    download_retry_at: Option<Instant>,
    active_export: std::sync::Arc<Mutex<ActiveExport>>,
    pending_exports: HashMap<String, VecDeque<(String, std::path::PathBuf)>>,
    staged_dialog_files: Vec<std::path::PathBuf>,
    pending_picker_purpose: std::sync::Arc<Mutex<Option<PendingPicker>>>,
    video_frames: HashMap<std::path::PathBuf, std::sync::Arc<Mutex<Option<mailcraft_ui_egui::VideoFrame>>>>,
    open_requests: Sender<(std::path::PathBuf, String)>,
    open_requests_rx: Receiver<(std::path::PathBuf, String)>,
    notifications: notifications::Queue,
    lanes: lanes::Lanes,
    zone: Mutex<CachedZone>,
    dark: Mutex<CachedDark>,
    phone_form_factor: Option<bool>,
    font_scale: Option<f32>,
    system_language: Option<String>,
    notification_language: Option<String>,
    notification_retry_at: Option<Instant>,
}

#[cfg(target_os = "android")]
impl Drop for AndroidServices {
    fn drop(&mut self) {
        if let Some(directory) = self.incoming_dir.as_deref() {
            for result in self.clipboard_files_rx.try_iter() {
                if let Ok(files) = result {
                    platform::discard_clipboard_files(directory, &files);
                }
            }
        }
        for path in std::mem::take(&mut self.staged_dialog_files) {
            self.discard_picked_file(&path);
        }
        for queue in std::mem::take(&mut self.pending_exports).into_values() {
            for (_, path) in queue {
                discard_android_export(&path);
            }
        }
        for download in std::mem::take(&mut self.download_queue) {
            if let AndroidDownload::File { path, .. } = download {
                discard_android_export(&path);
            }
        }
        *lock_active_export(&self.active_export) = ActiveExport::None;
        activity::set_document_result_sender(None);
    }
}

#[cfg(target_os = "android")]
enum ActiveExport {
    None,
    Copying,
}

#[cfg(target_os = "android")]
struct PendingSave {
    name: String,
    mime: String,
    bytes: Vec<u8>,
}

#[cfg(target_os = "android")]
#[derive(Clone)]
struct PendingPicker {
    purpose: String,
    started: Instant,
    shared_batch_open: bool,
    shared_batch_rejected: bool,
    shared_bytes: u64,
}

#[cfg(target_os = "android")]
const PENDING_PICKER_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[cfg(target_os = "android")]
const MAX_QUEUED_ANDROID_DOWNLOADS: usize = 64;

#[cfg(target_os = "android")]
#[cfg(target_os = "android")]
const MAX_QUEUED_ANDROID_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(target_os = "android")]
fn lock_active_export(active: &Mutex<ActiveExport>) -> std::sync::MutexGuard<'_, ActiveExport> {
    active.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(target_os = "android")]
fn lock_picker_purpose(pending: &Mutex<Option<PendingPicker>>) -> std::sync::MutexGuard<'_, Option<PendingPicker>> {
    pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(target_os = "android")]
fn clear_picker_purpose(pending: &Mutex<Option<PendingPicker>>, purpose: &str) {
    let mut current = lock_picker_purpose(pending);
    if current.as_ref().is_some_and(|current| current.purpose.as_str() == purpose) {
        *current = None;
    }
}

#[cfg(target_os = "android")]
enum AndroidDownload {
    Bytes { name: String, mime_type: Option<String>, bytes: Vec<u8>, open_after_saving: bool, save_as: bool },
    File { name: String, path: std::path::PathBuf },
}

#[cfg(target_os = "android")]
fn discard_android_export(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("MailCraft could not remove a downloaded Android export: {error}"),
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(target_os = "android")]
fn saved_file_mime_type(reported: Option<&str>, name: &str) -> String {
    const MAX_MIME_CHARS: usize = 256;
    let reported = reported.map(|mime| mime.chars().take(MAX_MIME_CHARS).collect::<String>());
    let reported = reported.as_deref().map(|mime| mime.split(';').next().unwrap_or_default().trim());
    let token = |value: &str| {
        !value.is_empty()
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~')
            })
    };
    let valid_reported = reported.filter(|mime| {
        !matches!(*mime, "application/octet-stream" | "binary/octet-stream" | "*/*")
            && mime.split_once('/').is_some_and(|(major, minor)| token(major) && token(minor) && !minor.contains('/'))
    });
    match valid_reported {
        Some(mime) => mime.to_owned(),
        None => mime_guess::from_path(name).first_or_octet_stream().to_string(),
    }
}

#[cfg(target_os = "android")]
impl AndroidServices {
    pub(super) fn new(
        activity: winit::platform::android::activity::AndroidApp,
        repaint: egui::Context,
        incoming_dir: Option<std::path::PathBuf>,
    ) -> Self {
        let (picked, picked_rx) = mpsc::channel();
        let (clipboard_files, clipboard_files_rx) = mpsc::channel();
        let (clipboard_picture, clipboard_picture_rx) = mpsc::channel();
        let (document_results, document_results_rx) = mpsc::channel();
        activity::set_document_result_sender(Some(document_results));
        let (errors, errors_rx) = mpsc::channel();
        let (downloaded, downloaded_rx) = mpsc::channel();
        let (download_finished, download_finished_rx) = mpsc::channel();
        let (open_requests, open_requests_rx) = mpsc::channel();
        let (open_cache_bytes, open_cache_files) = downloads::open_cache_usage();
        let zone = platform::timezone(&activity);
        let dark = platform::dark_mode(&activity);
        let phone_form_factor = platform::phone_form_factor(&activity);
        let font_scale = platform::font_scale(&activity);
        let system_language = platform::language_tag(&activity);
        Self {
            activity,
            repaint,
            incoming_dir,
            clipboard_files,
            clipboard_files_rx,
            clipboard_files_pending: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            clipboard_picture,
            clipboard_picture_rx,
            clipboard_picture_pending: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            document_results_rx,
            active_document_picker: None,
            pending_saves: HashMap::new(),
            picked,
            picked_rx,
            errors,
            errors_rx,
            downloaded,
            downloaded_rx,
            download_finished,
            download_finished_rx,
            download_queue: VecDeque::new(),
            download_queue_bytes: 0,
            download_active_bytes: 0,
            open_cache_bytes,
            open_cache_files,
            download_active: false,
            download_retry_at: None,
            active_export: std::sync::Arc::new(Mutex::new(ActiveExport::None)),
            pending_exports: HashMap::new(),
            staged_dialog_files: Vec::new(),
            pending_picker_purpose: std::sync::Arc::new(Mutex::new(None)),
            video_frames: HashMap::new(),
            open_requests,
            open_requests_rx,
            notifications: notifications::Queue::new(),
            lanes: lanes::Lanes::default(),
            zone: Mutex::new(CachedZone { name: zone, checked_at: Some(Instant::now()) }),
            dark: Mutex::new(CachedDark { value: dark, checked_at: Some(Instant::now()) }),
            phone_form_factor,
            font_scale,
            system_language,
            notification_language: None,
            notification_retry_at: None,
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn android_target_only() {}
