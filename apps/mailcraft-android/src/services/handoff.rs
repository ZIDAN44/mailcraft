//! Android launch intents, shared documents and in-process handoff.
use super::{activity, platform};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    Mutex, OnceLock,
    mpsc::{Receiver, Sender},
};
use std::time::Instant;

#[cfg(target_os = "android")]
fn safe_notification_id(value: &str) -> bool {
    value.len() <= 1024 && !value.chars().any(char::is_control)
}

#[cfg(target_os = "android")]
pub(crate) fn discard_staged_android_file(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(path) {
        eprintln!("MailCraft could not remove a staged Android file: {error}");
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(target_os = "android")]
pub(crate) fn cleanup_shared_attachment_staging(incoming_dir: Option<&std::path::Path>) {
    let Some(incoming_dir) = incoming_dir.and_then(|directory| directory.canonicalize().ok()) else { return };
    let shared_dir = incoming_dir.join("shared-attachments");
    let Ok(shared_dir) = shared_dir.canonicalize() else { return };
    if shared_dir.parent() != Some(incoming_dir.as_path()) {
        eprintln!("MailCraft skipped unexpected shared attachment storage outside its private import folder");
        return;
    }
    let Ok(entries) = std::fs::read_dir(&shared_dir) else { return };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_dir() || entry.path().parent() != Some(shared_dir.as_path()) {
            continue;
        }
        let folder = entry.path();
        let Ok(files) = std::fs::read_dir(&folder) else { continue };
        for file in files {
            let Ok(file) = file else { continue };
            let Ok(kind) = file.file_type() else { continue };
            if kind.is_file()
                && file.path().parent() == Some(folder.as_path())
                && let Err(error) = std::fs::remove_file(file.path())
            {
                eprintln!("MailCraft could not remove an interrupted shared attachment: {error}");
            }
        }
        if let Err(error) = std::fs::remove_dir(&folder) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("MailCraft could not remove an interrupted shared attachment folder: {error}");
            }
        }
    }
    if let Err(error) = std::fs::remove_dir(&shared_dir) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("MailCraft could not remove its empty shared attachment folder: {error}");
        }
    }
}

#[cfg(target_os = "android")]
pub(crate) fn cleanup_interrupted_picker_staging(incoming_dir: Option<&std::path::Path>, retained_paths: &[std::path::PathBuf]) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    let Some(incoming_dir) = incoming_dir.and_then(|directory| directory.canonicalize().ok()) else { return };
    let retained: std::collections::HashSet<_> = retained_paths.iter().filter_map(|path| path.canonicalize().ok()).collect();
    let Ok(entries) = std::fs::read_dir(&incoming_dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if entry.file_name() == "shared-attachments" {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_dir() {
            continue;
        }
        let Ok(folder) = path.canonicalize() else { continue };
        if folder.parent() != Some(incoming_dir.as_path()) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(&folder) else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        if now.duration_since(modified).map_or(true, |age| age < MAX_AGE) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&folder) else { continue };
        let mut staged_files = Vec::new();
        let mut safe_to_remove = true;
        for file in files {
            let Ok(file) = file else {
                safe_to_remove = false;
                break;
            };
            let Ok(file_type) = file.file_type() else {
                safe_to_remove = false;
                break;
            };
            if !file_type.is_file() {
                safe_to_remove = false;
                break;
            }
            let Ok(file_path) = file.path().canonicalize() else {
                safe_to_remove = false;
                break;
            };
            if file_path.parent() != Some(folder.as_path()) || retained.contains(&file_path) {
                safe_to_remove = false;
                break;
            }
            staged_files.push(file_path);
        }
        if !safe_to_remove {
            continue;
        }
        if staged_files.is_empty() {
            if let Err(error) = std::fs::remove_dir(&folder) {
                eprintln!("MailCraft could not remove an empty interrupted Android picker folder: {error}");
            }
            continue;
        }
        let mut removed_all = true;
        for file in staged_files {
            if let Err(error) = std::fs::remove_file(&file) {
                eprintln!("MailCraft could not remove an interrupted Android picker file: {error}");
                removed_all = false;
            }
        }
        if removed_all && let Err(error) = std::fs::remove_dir(&folder) {
            eprintln!("MailCraft could not remove an interrupted Android picker folder: {error}");
        }
    }
}

#[cfg(target_os = "android")]
pub(crate) struct CachedZone {
    pub(crate) name: Option<String>,
    pub(crate) checked_at: Option<Instant>,
}

#[cfg(target_os = "android")]
pub(crate) struct CachedDark {
    pub(crate) value: Option<bool>,
    pub(crate) checked_at: Option<Instant>,
}

#[cfg(target_os = "android")]
pub(crate) enum AndroidHandoff {
    Mailto(String),
    Notification(mailcraft_ui_egui::NotificationTarget),
    SharedText { subject: String, body: String, to: String, cc: String, bcc: String },
    Rejected(String),
    Documents { documents: Vec<AndroidDocument>, shared_text: Option<(String, String, String, String, String)> },
}

#[cfg(target_os = "android")]
pub(crate) enum AndroidOpen {
    Link(String),
    Notification(mailcraft_ui_egui::NotificationTarget),
    File { path: String, last_in_batch: bool },
    Attachment { path: String, last_in_batch: bool },
    SharedText { subject: String, body: String, to: String, cc: String, bcc: String },
}

#[cfg(target_os = "android")]
pub(crate) struct HandoffRoute {
    sender: Sender<Result<AndroidOpen, String>>,
    repaint: Mutex<Option<egui::Context>>,
    active: AtomicBool,
}

#[cfg(target_os = "android")]
static HANDOFF_ROUTE: OnceLock<Mutex<Option<std::sync::Arc<HandoffRoute>>>> = OnceLock::new();

#[cfg(target_os = "android")]
pub(crate) fn register_handoff_route() -> (std::sync::Arc<HandoffRoute>, Option<Receiver<Result<AndroidOpen, String>>>) {
    let registry = HANDOFF_ROUTE.get_or_init(|| Mutex::new(None));
    let mut current = registry.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(route) = current.as_ref().filter(|route| route.active.load(Ordering::Acquire)) {
        return (route.clone(), None);
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let route = std::sync::Arc::new(HandoffRoute { sender, repaint: Mutex::new(None), active: AtomicBool::new(true) });
    *current = Some(route.clone());
    (route, Some(receiver))
}

#[cfg(target_os = "android")]
impl HandoffRoute {
    pub(crate) fn set_repaint(&self, repaint: egui::Context) {
        if let Ok(mut current) = self.repaint.lock() {
            *current = Some(repaint);
        }
    }

    pub(crate) fn send(&self, message: Result<AndroidOpen, String>) {
        if !self.active.load(Ordering::Acquire) || self.sender.send(message).is_err() {
            return;
        }
        if let Ok(repaint) = self.repaint.lock()
            && let Some(repaint) = repaint.as_ref()
        {
            repaint.request_repaint();
        }
    }

    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }
}

/// Stage an Android launch/share intent and deliver its files to the active MailCraft window.
#[cfg(target_os = "android")]
pub(crate) fn route_android_handoff(
    app: winit::platform::android::activity::AndroidApp,
    handoff: Option<AndroidHandoff>,
    directory: Option<std::path::PathBuf>,
    route: std::sync::Arc<HandoffRoute>,
) -> (usize, Option<std::thread::JoinHandle<()>>) {
    let Some(handoff) = handoff else { return (0, None) };
    match handoff {
        AndroidHandoff::Notification(target) => {
            route.send(Ok(AndroidOpen::Notification(target)));
            (1, None)
        }
        AndroidHandoff::Mailto(url) => {
            route.send(Ok(AndroidOpen::Link(url)));
            (1, None)
        }
        AndroidHandoff::SharedText { subject, body, to, cc, bcc } => {
            route.send(Ok(AndroidOpen::SharedText { subject, body, to, cc, bcc }));
            (1, None)
        }
        AndroidHandoff::Rejected(error) => {
            route.send(Err(error));
            (1, None)
        }
        AndroidHandoff::Documents { documents, shared_text } => {
            let has_data_archive = documents.iter().any(is_data_archive);
            if has_data_archive && documents.len() != 1 {
                route.send(Err(mailcraft_ui_egui::i18n::t("Share a single .pst or .mailcraft data file at a time").to_owned()));
                return (1, None);
            }
            let include_shared_text =
                documents.iter().any(|document| document.shared && !is_openable_shared_file(&document.name, &document.mime_type));
            let shared_text = if include_shared_text { shared_text } else { None };
            let pending = documents.len().saturating_add(usize::from(shared_text.is_some()));
            let sender = route.clone();
            let activity = app.clone();
            let started = std::thread::Builder::new().name("android-file-handoff".into()).spawn(move || {
                if let Some((subject, body, to, cc, bcc)) = shared_text {
                    if !sender.active.load(Ordering::Acquire)
                        || sender.sender.send(Ok(AndroidOpen::SharedText { subject, body, to, cc, bcc })).is_err()
                    {
                        return;
                    }
                    if let Ok(repaint) = sender.repaint.lock()
                        && let Some(repaint) = repaint.as_ref()
                    {
                        repaint.request_repaint();
                    }
                }
                let mut remaining = if has_data_archive { mailcraft_engine::MAX_DATA_ARCHIVE_BYTES } else { 128_u64 << 20 };
                let mut remaining_attachments = mailcraft_engine::MAX_ATTACHMENT_BYTES as u64;
                let document_count = documents.len();
                for (index, document) in documents.into_iter().enumerate() {
                    let last_in_batch = index.saturating_add(1) == document_count;
                    let as_attachment = document.shared && !is_openable_shared_file(&document.name, &document.mime_type);
                    let limit = if as_attachment { remaining_attachments } else { remaining };
                    if limit == 0 {
                        let message = if as_attachment {
                            format!("Attachments shared to MailCraft are limited to {} MiB total", mailcraft_engine::MAX_ATTACHMENT_BYTES >> 20)
                        } else {
                            "MailCraft can import up to 128 MiB of shared files at a time".to_string()
                        };
                        sender.send(Err(message));
                        break;
                    }
                    let result = directory
                        .as_deref()
                        .ok_or_else(|| "MailCraft's private data folder is unavailable".to_string())
                        .and_then(|directory| {
                            let directory = if as_attachment { directory.join("shared-attachments") } else { directory.to_path_buf() };
                            platform::stage_android_document(
                                &activity,
                                &document.uri,
                                &document.mime_type,
                                &document.name,
                                &directory,
                                limit,
                                as_attachment,
                            )
                        })
                        .and_then(|path| {
                            let length = match std::fs::metadata(&path) {
                                Ok(metadata) => metadata.len(),
                                Err(error) => {
                                    discard_staged_android_file(&path);
                                    return Err(error.to_string());
                                }
                            };
                            if as_attachment {
                                remaining_attachments = remaining_attachments.saturating_sub(length);
                                Ok(AndroidOpen::Attachment { path: path.to_string_lossy().into_owned(), last_in_batch })
                            } else {
                                remaining = remaining.saturating_sub(length);
                                Ok(AndroidOpen::File { path: path.to_string_lossy().into_owned(), last_in_batch })
                            }
                        });
                    let failed = result.is_err();
                    if !sender.active.load(Ordering::Acquire) {
                        if let Ok(AndroidOpen::File { path, .. } | AndroidOpen::Attachment { path, .. }) = &result {
                            discard_staged_android_file(std::path::Path::new(path));
                        }
                        break;
                    }
                    if let Err(error) = sender.sender.send(result) {
                        if let Ok(AndroidOpen::File { path, .. } | AndroidOpen::Attachment { path, .. }) = error.0 {
                            discard_staged_android_file(std::path::Path::new(&path));
                        }
                        break;
                    }
                    if let Ok(repaint) = sender.repaint.lock()
                        && let Some(repaint) = repaint.as_ref()
                    {
                        repaint.request_repaint();
                    }
                    if failed {
                        break;
                    }
                }
            });
            let worker = match started {
                Ok(worker) => Some(worker),
                Err(error) => {
                    route.send(Err(format!("MailCraft couldn't start the file import: {error}")));
                    None
                }
            };
            (pending, worker)
        }
    }
}

#[cfg(target_os = "android")]
pub(crate) struct AndroidDocument {
    pub(crate) uri: String,
    pub(crate) mime_type: String,
    pub(crate) name: String,
    pub(crate) shared: bool,
}

/// Recipient arrays supplied by Android's standard email-sharing contract.
#[cfg(target_os = "android")]
fn shared_recipients(env: &mut jni::JNIEnv<'_>, intent: &jni::objects::JObject<'_>, key: &str) -> robius_android_env::Result<String> {
    use jni::objects::JValue;

    const MAX_RECIPIENTS: i32 = 50;
    const MAX_ADDRESS_CHARS: i32 = 512;
    const MAX_TOTAL_CHARS: usize = 8192;
    let key = env.new_string(key)?;
    let array = match env
        .call_method(intent, "getStringArrayExtra", "(Ljava/lang/String;)[Ljava/lang/String;", &[JValue::Object(key.as_ref())])
        .and_then(|value| value.l())
    {
        Ok(array) => array,
        Err(_) if env.exception_check().unwrap_or(false) => {
            let _ = env.exception_clear();
            return Ok(String::new());
        }
        Err(error) => return Err(error.into()),
    };
    if array.is_null() {
        return Ok(String::new());
    }
    let array = jni::objects::JObjectArray::from(array);
    let count = env.get_array_length(&array)?.min(MAX_RECIPIENTS);
    let mut addresses = Vec::new();
    let mut total_chars = 0usize;
    for index in 0..count {
        let value = env.get_object_array_element(&array, index)?;
        if value.is_null() {
            continue;
        }
        let length = platform::java_text_length(env, &value)?;
        if length == 0 || length > MAX_ADDRESS_CHARS {
            continue;
        }
        let address = platform::bounded_java_text(env, &value, MAX_ADDRESS_CHARS)?;
        let address = address.trim();
        if address.is_empty() || address.chars().any(char::is_control) {
            continue;
        }
        let extra = address.chars().count().saturating_add(2);
        if total_chars.saturating_add(extra) > MAX_TOTAL_CHARS {
            break;
        }
        total_chars += extra;
        addresses.push(address.to_string());
    }
    Ok(addresses.join(", "))
}

#[cfg(target_os = "android")]
fn is_data_archive(document: &AndroidDocument) -> bool {
    document.mime_type.split(';').next().unwrap_or_default().trim().eq_ignore_ascii_case("application/vnd.storytold.mailcraft")
        || std::path::Path::new(document.name.rsplit(':').next().unwrap_or(&document.name))
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "pst" | "mailcraft"))
}

#[cfg(target_os = "android")]
pub(crate) fn is_openable_shared_file(name: &str, mime_type: &str) -> bool {
    let extension = std::path::Path::new(name.rsplit(':').next().unwrap_or(name))
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "eml" | "msg" | "ics" | "ical" | "vcf" | "vcard" | "pst" | "mailcraft") {
        return true;
    }
    matches!(
        mime_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "message/rfc822"
            | "application/vnd.ms-outlook"
            | "application/vnd.storytold.mailcraft"
            | "text/calendar"
            | "text/vcalendar"
            | "text/x-vcalendar"
            | "text/vcard"
            | "text/x-vcard"
            | "application/ics"
    )
}

#[cfg(target_os = "android")]
pub(crate) fn incoming_android_intent(app: &winit::platform::android::activity::AndroidApp) -> Option<AndroidHandoff> {
    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<Option<AndroidHandoff>> {
            let intent = env.call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?.l()?;
            if intent.is_null() {
                return Ok(None);
            }
            let notification_key = env.new_string("mailcraft.notification")?;
            let notification_kind = env
                .call_method(
                    &intent,
                    "getStringExtra",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[jni::objects::JValue::Object(notification_key.as_ref())],
                )?
                .l()?;
            if !notification_kind.is_null() {
                let kind = platform::bounded_java_text(env, &notification_kind, 32)?;
                let target = match kind.as_str() {
                    "inbox" => Some(mailcraft_ui_egui::NotificationTarget::Inbox),
                    "calendar" => Some(mailcraft_ui_egui::NotificationTarget::Calendar),
                    "tasks" => Some(mailcraft_ui_egui::NotificationTarget::Tasks),
                    "message" => {
                        let key_name = env.new_string("mailcraft.key")?;
                        let key = env
                            .call_method(
                                &intent,
                                "getStringExtra",
                                "(Ljava/lang/String;)Ljava/lang/String;",
                                &[jni::objects::JValue::Object(key_name.as_ref())],
                            )?
                            .l()?;
                        if key.is_null() || platform::java_text_length(env, &key)? > 1024 {
                            Some(mailcraft_ui_egui::NotificationTarget::Inbox)
                        } else {
                            let key = platform::bounded_java_text(env, &key, 1024)?;
                            if safe_notification_id(&key) {
                                mailcraft_engine::model::MessageKey::from_id(&key)
                                    .map(|key| mailcraft_ui_egui::NotificationTarget::Message { key: key.to_id() })
                                    .or(Some(mailcraft_ui_egui::NotificationTarget::Inbox))
                            } else {
                                Some(mailcraft_ui_egui::NotificationTarget::Inbox)
                            }
                        }
                    }
                    "event" | "task" => {
                        let account_key = env.new_string("mailcraft.account")?;
                        let account = env
                            .call_method(
                                &intent,
                                "getStringExtra",
                                "(Ljava/lang/String;)Ljava/lang/String;",
                                &[jni::objects::JValue::Object(account_key.as_ref())],
                            )?
                            .l()?;
                        let uid_key = env.new_string("mailcraft.uid")?;
                        let uid = env
                            .call_method(
                                &intent,
                                "getStringExtra",
                                "(Ljava/lang/String;)Ljava/lang/String;",
                                &[jni::objects::JValue::Object(uid_key.as_ref())],
                            )?
                            .l()?;
                        if account.is_null()
                            || platform::java_text_length(env, &account)? > 256
                            || uid.is_null()
                            || platform::java_text_length(env, &uid)? > 1024
                        {
                            Some(if kind == "task" {
                                mailcraft_ui_egui::NotificationTarget::Tasks
                            } else {
                                mailcraft_ui_egui::NotificationTarget::Calendar
                            })
                        } else {
                            let account = platform::bounded_java_text(env, &account, 256)?;
                            let uid = platform::bounded_java_text(env, &uid, 1024)?;
                            let occurrence_key = env.new_string("mailcraft.occurrence")?;
                            let occurrence = env
                                .call_method(
                                    &intent,
                                    "getLongExtra",
                                    "(Ljava/lang/String;J)J",
                                    &[jni::objects::JValue::Object(occurrence_key.as_ref()), jni::objects::JValue::Long(0)],
                                )?
                                .j()?;
                            let due_key = env.new_string("mailcraft.due")?;
                            let due = env
                                .call_method(
                                    &intent,
                                    "getLongExtra",
                                    "(Ljava/lang/String;J)J",
                                    &[jni::objects::JValue::Object(due_key.as_ref()), jni::objects::JValue::Long(0)],
                                )?
                                .j()?;
                            if account.is_empty() || !safe_notification_id(&account) || uid.is_empty() || !safe_notification_id(&uid) {
                                Some(if kind == "task" {
                                    mailcraft_ui_egui::NotificationTarget::Tasks
                                } else {
                                    mailcraft_ui_egui::NotificationTarget::Calendar
                                })
                            } else {
                                Some(mailcraft_ui_egui::NotificationTarget::Reminder { account, uid, occurrence, due, task: kind == "task" })
                            }
                        }
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    return Ok(Some(AndroidHandoff::Notification(target)));
                }
            }
            let action = env.call_method(&intent, "getAction", "()Ljava/lang/String;", &[])?.l()?;
            if action.is_null() {
                return Ok(None);
            }
            let action = platform::bounded_java_text(env, &action, 64)?;
            if action == "android.intent.action.SENDTO" {
                let url = env.call_method(&intent, "getDataString", "()Ljava/lang/String;", &[])?.l()?;
                if url.is_null() {
                    return Ok(None);
                }
                if platform::java_text_length(env, &url)? > 8192 {
                    return Ok(Some(AndroidHandoff::Rejected("The shared mail link is too long to open".into())));
                }
                let url = platform::bounded_java_text(env, &url, 8192)?;
                return Ok(mailcraft_engine::model::mailto::is_mailto(&url).then_some(AndroidHandoff::Mailto(url)));
            }
            if action != "android.intent.action.VIEW" && action != "android.intent.action.SEND" && action != "android.intent.action.SEND_MULTIPLE" {
                return Ok(None);
            }
            let shared = action == "android.intent.action.SEND" || action == "android.intent.action.SEND_MULTIPLE";
            let mime_type = env.call_method(&intent, "getType", "()Ljava/lang/String;", &[])?.l()?;
            let mime_type = platform::bounded_java_text(env, &mime_type, 256)?;
            let resolver = env.call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?.l()?;
            let mut documents = Vec::new();
            let mut seen = std::collections::HashSet::new();
            macro_rules! collect_uri {
                ($uri:expr) => {{
                    let uri = $uri;
                    if !uri.is_null() {
                        let scheme = env.call_method(&uri, "getScheme", "()Ljava/lang/String;", &[])?.l()?;
                        let scheme = platform::bounded_java_text(env, &scheme, 32)?;
                        if matches!(scheme.as_str(), "content" | "file") {
                            let uri_text = env.call_method(&uri, "toString", "()Ljava/lang/String;", &[])?.l()?;
                            let fallback_name = env.call_method(&uri, "getLastPathSegment", "()Ljava/lang/String;", &[])?.l()?;
                            if platform::java_text_length(env, &uri_text)? > 8192 {
                                return Ok(Some(AndroidHandoff::Rejected("The shared document address is too long to open".into())));
                            }
                            let uri_text = platform::bounded_java_text(env, &uri_text, 8192)?;
                            let fallback_name = platform::bounded_java_text(env, &fallback_name, 512)?;
                            let name = platform::document_name(env, &resolver, &uri).unwrap_or(fallback_name);
                            if seen.insert(uri_text.clone()) {
                                documents.push(AndroidDocument { uri: uri_text, mime_type: mime_type.clone(), name, shared });
                            }
                        }
                    }
                }};
            }
            if action == "android.intent.action.VIEW" {
                collect_uri!(env.call_method(&intent, "getData", "()Landroid/net/Uri;", &[])?.l()?);
            } else {
                let key = env.new_string("android.intent.extra.STREAM")?;
                if action == "android.intent.action.SEND" {
                    collect_uri!(
                        env.call_method(
                            &intent,
                            "getParcelableExtra",
                            "(Ljava/lang/String;)Landroid/os/Parcelable;",
                            &[jni::objects::JValue::Object(key.as_ref())]
                        )?
                        .l()?
                    );
                } else {
                    let streams = env
                        .call_method(
                            &intent,
                            "getParcelableArrayListExtra",
                            "(Ljava/lang/String;)Ljava/util/ArrayList;",
                            &[jni::objects::JValue::Object(key.as_ref())],
                        )?
                        .l()?;
                    if !streams.is_null() {
                        let count = env.call_method(&streams, "size", "()I", &[])?.i()?.max(0);
                        if count > 50 {
                            return Ok(Some(AndroidHandoff::Rejected("MailCraft can open up to 50 shared files at a time".into())));
                        }
                        for index in 0..count {
                            collect_uri!(env.call_method(&streams, "get", "(I)Ljava/lang/Object;", &[jni::objects::JValue::Int(index)])?.l()?);
                        }
                    }
                }
            }
            let clip = env.call_method(&intent, "getClipData", "()Landroid/content/ClipData;", &[])?.l()?;
            let mut clip_shared_body = None;
            if !clip.is_null() {
                let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?.max(0);
                if count > 50 {
                    return Ok(Some(AndroidHandoff::Rejected("MailCraft can open up to 50 shared files at a time".into())));
                }
                for index in 0..count {
                    let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[jni::objects::JValue::Int(index)])?.l()?;
                    if !item.is_null() {
                        if shared && clip_shared_body.is_none() {
                            let html = env.call_method(&item, "getHtmlText", "()Ljava/lang/String;", &[])?.l()?;
                            if !html.is_null() {
                                let html = platform::bounded_java_text(env, &html, 1 << 20)?;
                                let body = mailcraft_engine::mime::pasted_html(&html).0;
                                clip_shared_body = (!body.trim().is_empty()).then_some(body);
                            }
                            if clip_shared_body.is_none() {
                                let text = env.call_method(&item, "getText", "()Ljava/lang/CharSequence;", &[])?.l()?;
                                if !text.is_null() {
                                    let body = platform::bounded_java_text(env, &text, 1 << 20)?;
                                    clip_shared_body = (!body.trim().is_empty()).then_some(body);
                                }
                            }
                        }
                        collect_uri!(env.call_method(&item, "getUri", "()Landroid/net/Uri;", &[])?.l()?);
                    }
                }
            }
            if documents.len() > 50 {
                return Ok(Some(AndroidHandoff::Rejected("MailCraft can open up to 50 shared files at a time".into())));
            }
            let shared_text = if shared {
                let text_key = env.new_string("android.intent.extra.TEXT")?;
                let text = env
                    .call_method(
                        &intent,
                        "getCharSequenceExtra",
                        "(Ljava/lang/String;)Ljava/lang/CharSequence;",
                        &[jni::objects::JValue::Object(text_key.as_ref())],
                    )?
                    .l()?;
                let body = if text.is_null() {
                    let html_key = env.new_string("android.intent.extra.HTML_TEXT")?;
                    let html = env
                        .call_method(
                            &intent,
                            "getStringExtra",
                            "(Ljava/lang/String;)Ljava/lang/String;",
                            &[jni::objects::JValue::Object(html_key.as_ref())],
                        )?
                        .l()?;
                    if !html.is_null() {
                        let html = platform::bounded_java_text(env, &html, 1 << 20)?;
                        let body = mailcraft_engine::mime::pasted_html(&html).0;
                        (!body.trim().is_empty()).then_some(body).or(clip_shared_body)
                    } else {
                        clip_shared_body
                    }
                } else {
                    let body = platform::bounded_java_text(env, &text, 1 << 20)?;
                    (!body.trim().is_empty()).then_some(body).or(clip_shared_body)
                };
                let to = shared_recipients(env, &intent, "android.intent.extra.EMAIL")?;
                let cc = shared_recipients(env, &intent, "android.intent.extra.CC")?;
                let bcc = shared_recipients(env, &intent, "android.intent.extra.BCC")?;
                let subject_key = env.new_string("android.intent.extra.SUBJECT")?;
                let subject = env
                    .call_method(
                        &intent,
                        "getStringExtra",
                        "(Ljava/lang/String;)Ljava/lang/String;",
                        &[jni::objects::JValue::Object(subject_key.as_ref())],
                    )?
                    .l()?;
                let subject = platform::bounded_java_text(env, &subject, 998)?;
                if body.is_some() || !subject.is_empty() || !to.is_empty() || !cc.is_empty() || !bcc.is_empty() {
                    Some((subject, body.unwrap_or_default(), to, cc, bcc))
                } else {
                    None
                }
            } else {
                None
            };
            if !documents.is_empty() {
                return Ok(Some(AndroidHandoff::Documents { documents, shared_text }));
            }
            if let Some((subject, body, to, cc, bcc)) = shared_text {
                return Ok(Some(AndroidHandoff::SharedText { subject, body, to, cc, bcc }));
            }
            Ok(None)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(handoff)) => handoff,
        Ok(Err(error)) | Err(error) => {
            eprintln!("MailCraft could not read the Android file handoff: {error}");
            None
        }
    }
}
