//! The JNI bridge to the Android `Activity` owned by `android-activity`.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

use jni::objects::{JObject, JString};

static SYSTEM_BACK_REQUESTED: AtomicBool = AtomicBool::new(false);
static SYSTEM_BACK_CONTEXT: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();
static DOCUMENT_RESULT_SENDER: OnceLock<Mutex<Option<Sender<DocumentResult>>>> = OnceLock::new();
static NEXT_DOCUMENT_REQUEST: AtomicI32 = AtomicI32::new(1);

#[derive(Debug)]
pub(super) enum DocumentResult {
    Selected { request_id: i32, uri: String, name: Option<String>, mime: Option<String>, size: Option<u64> },
    Cancelled { request_id: i32 },
    Failed { request_id: i32, error: String },
}

pub(super) fn next_document_request_id() -> i32 {
    NEXT_DOCUMENT_REQUEST.fetch_add(1, Ordering::Relaxed).max(1)
}

pub(super) fn set_document_result_sender(sender: Option<Sender<DocumentResult>>) {
    let slot = DOCUMENT_RESULT_SENDER.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut active) => *active = sender,
        Err(poisoned) => *poisoned.into_inner() = sender,
    }
}

fn send_document_result(result: DocumentResult) {
    let Some(slot) = DOCUMENT_RESULT_SENDER.get() else { return };
    let sender = match slot.lock() {
        Ok(active) => active.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    if sender.is_some_and(|sender| sender.send(result).is_ok()) {
        let context = SYSTEM_BACK_CONTEXT.get().and_then(|slot| slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone());
        if let Some(context) = context {
            context.request_repaint();
        }
    }
}

pub(super) fn set_system_back_context(context: Option<egui::Context>) {
    let slot = SYSTEM_BACK_CONTEXT.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut active) => *active = context,
        Err(poisoned) => *poisoned.into_inner() = context,
    }
}

/// Whether Android's system back callback has queued a UI navigation event.
pub(super) fn take_system_back() -> bool {
    SYSTEM_BACK_REQUESTED.swap(false, Ordering::AcqRel)
}

/// Android 13+ dispatches system back through the Activity's registered callback.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_mailcraft_MailCraftActivity_nativeSystemBack(_env: jni::JNIEnv<'_>, _activity: jni::objects::JClass<'_>) {
    SYSTEM_BACK_REQUESTED.store(true, Ordering::Release);
    let context = SYSTEM_BACK_CONTEXT.get().and_then(|slot| slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone());
    if let Some(context) = context {
        context.request_repaint();
    }
}

/// Return one Storage Access Framework result to the Rust service waiting for it.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_mailcraft_MailCraftActivity_nativeDocumentResult(
    mut env: jni::JNIEnv<'_>,
    _activity: jni::objects::JClass<'_>,
    request_id: jni::sys::jint,
    result_code: jni::sys::jint,
    uri: JString<'_>,
    name: JString<'_>,
    mime: JString<'_>,
    size: jni::sys::jlong,
    error: JString<'_>,
) {
    let string = |env: &mut jni::JNIEnv<'_>, value: JString<'_>| {
        if value.is_null() { None } else { env.get_string(&value).ok().map(|text| text.to_string_lossy().into_owned()) }
    };
    let error = string(&mut env, error);
    let result = if let Some(error) = error.filter(|error| !error.is_empty()) {
        DocumentResult::Failed { request_id, error }
    } else if result_code != -1 {
        DocumentResult::Cancelled { request_id }
    } else if let Some(uri) = string(&mut env, uri).filter(|uri| !uri.is_empty()) {
        DocumentResult::Selected { request_id, uri, name: string(&mut env, name), mime: string(&mut env, mime), size: u64::try_from(size).ok() }
    } else {
        DocumentResult::Failed { request_id, error: "Android did not return the selected document".into() }
    };
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
        send_document_result(DocumentResult::Failed { request_id, error: "Android could not read the document selection".into() });
        return;
    }
    send_document_result(result);
}

/// Register the primary NativeActivity for middleware that needs an Android `Activity`.
pub(super) fn register_middleware_activity(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
    let result = robius_android_env::with_activity(|env, _application| {
        let raw: jni::sys::jobject = app.activity_as_ptr().cast();
        if raw.is_null() {
            return Err(jni::errors::Error::NullPtr("Android Activity"));
        }
        // SAFETY: `AndroidApp::activity_as_ptr` returns its live, unowned JNI global reference.
        // `app` remains borrowed for this call, and ManuallyDrop prevents JNI from releasing it.
        let activity = ManuallyDrop::new(unsafe { JObject::from_raw(raw) });
        robius_android_env::set_activity(env, &activity)
    });
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

/// Call an Android API with this window's Activity, not the process-wide Application context.
pub(super) fn with_activity<R>(
    app: &winit::platform::android::activity::AndroidApp,
    operation: impl FnOnce(&mut jni::JNIEnv<'_>, &JObject<'_>) -> R,
) -> robius_android_env::Result<R> {
    robius_android_env::with_activity(|env, _application| {
        let raw: jni::sys::jobject = app.activity_as_ptr().cast();
        if raw.is_null() {
            return Err(jni::errors::Error::NullPtr("Android Activity"));
        }
        // SAFETY: `AndroidApp::activity_as_ptr` returns its live, unowned JNI global reference.
        // `app` remains borrowed for this call, and ManuallyDrop ensures JNI does not treat the
        // global reference as a local reference or release it. The Activity borrow cannot escape
        // this closure because its result type `R` is independent of the closure's input lifetime.
        let activity = ManuallyDrop::new(unsafe { JObject::from_raw(raw) });
        Ok(operation(env, &activity))
    })?
}

pub(super) fn open_document_picker(app: &winit::platform::android::activity::AndroidApp, request_id: i32, mime: &str) -> Result<(), String> {
    let result = with_activity(app, |env, activity| {
        let operation = (|| -> Result<(), jni::errors::Error> {
            let mime = env.new_string(mime)?;
            env.call_method(
                activity,
                "startMailCraftOpenDocument",
                "(ILjava/lang/String;)V",
                &[jni::objects::JValue::Int(request_id), jni::objects::JValue::Object(mime.as_ref())],
            )?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    result.map_err(|error| error.to_string())?.map_err(|error| error.to_string())
}

pub(super) fn create_document(app: &winit::platform::android::activity::AndroidApp, request_id: i32, name: &str, mime: &str) -> Result<(), String> {
    let result = with_activity(app, |env, activity| {
        let operation = (|| -> Result<(), jni::errors::Error> {
            let name = env.new_string(name)?;
            let mime = env.new_string(mime)?;
            env.call_method(
                activity,
                "startMailCraftCreateDocument",
                "(ILjava/lang/String;Ljava/lang/String;)V",
                &[jni::objects::JValue::Int(request_id), jni::objects::JValue::Object(name.as_ref()), jni::objects::JValue::Object(mime.as_ref())],
            )?;
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    result.map_err(|error| error.to_string())?.map_err(|error| error.to_string())
}

/// Read the certificate roots trusted by Android's default TLS trust manager.
pub(super) fn system_trust_roots(app: &winit::platform::android::activity::AndroidApp) -> Result<Vec<Vec<u8>>, String> {
    with_activity(app, |env, _activity| system_trust_roots_from_env(env)).map_err(|error| error.to_string())?.map_err(|error| error.to_string())
}

fn system_trust_roots_from_env(env: &mut jni::JNIEnv<'_>) -> Result<Vec<Vec<u8>>, jni::errors::Error> {
    let operation = (|| -> Result<Vec<Vec<u8>>, jni::errors::Error> {
        let algorithm = env.call_static_method("javax/net/ssl/TrustManagerFactory", "getDefaultAlgorithm", "()Ljava/lang/String;", &[])?.l()?;
        let factory = env
            .call_static_method(
                "javax/net/ssl/TrustManagerFactory",
                "getInstance",
                "(Ljava/lang/String;)Ljavax/net/ssl/TrustManagerFactory;",
                &[jni::objects::JValue::Object(&algorithm)],
            )?
            .l()?;
        env.call_method(&factory, "init", "(Ljava/security/KeyStore;)V", &[jni::objects::JValue::Object(&jni::objects::JObject::null())])?;
        let managers = env.call_method(&factory, "getTrustManagers", "()[Ljavax/net/ssl/TrustManager;", &[])?.l()?;
        let managers = jni::objects::JObjectArray::from(managers);
        let count = env.get_array_length(&managers)?;
        if count < 0 || count > 32 {
            return Ok(Vec::new());
        }
        let mut certs = Vec::new();
        let mut total_bytes = 0usize;
        for index in 0..count {
            let manager = env.get_object_array_element(&managers, index)?;
            if !env.is_instance_of(&manager, "javax/net/ssl/X509TrustManager")? {
                continue;
            }
            let issuers = env.call_method(&manager, "getAcceptedIssuers", "()[Ljava/security/cert/X509Certificate;", &[])?.l()?;
            let issuers = jni::objects::JObjectArray::from(issuers);
            let issuer_count = env.get_array_length(&issuers)?;
            if issuer_count < 0 || issuer_count > 1024 {
                return Ok(Vec::new());
            }
            for issuer_index in 0..issuer_count {
                let issuer = env.get_object_array_element(&issuers, issuer_index)?;
                let encoded = env.call_method(&issuer, "getEncoded", "()[B", &[])?.l()?;
                let encoded = jni::objects::JByteArray::from(encoded);
                let encoded_len = env.get_array_length(&encoded)?;
                if encoded_len <= 0 || encoded_len > 1024 * 1024 {
                    continue;
                }
                let Some(next_total) = total_bytes.checked_add(encoded_len as usize) else {
                    return Ok(Vec::new());
                };
                if next_total > 8 * 1024 * 1024 {
                    return Ok(Vec::new());
                }
                certs.push(env.convert_byte_array(&encoded)?);
                total_bytes = next_total;
            }
        }
        Ok(certs)
    })();
    if operation.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    operation
}

pub(super) fn system_timezone_from_env(env: &mut jni::JNIEnv<'_>) -> Result<Option<String>, jni::errors::Error> {
    use jni::objects::JString;

    let operation = (|| -> Result<Option<String>, jni::errors::Error> {
        let timezone = env.call_static_method("java/util/TimeZone", "getDefault", "()Ljava/util/TimeZone;", &[])?.l()?;
        if timezone.is_null() {
            return Ok(None);
        }
        let id = env.call_method(&timezone, "getID", "()Ljava/lang/String;", &[])?.l()?;
        if id.is_null() {
            return Ok(None);
        }
        let mut id = env.get_string(&JString::from(id))?.to_string_lossy().into_owned();
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Ok(None);
        }
        // Android uses `GMT+05:30` for manually selected fixed-offset zones. MailCraft's
        // portable zone parser accepts the offset itself, but deliberately doesn't parse Java's
        // `GMT` prefix; normalize only this exact numeric form and leave IANA IDs intact.
        if let Some(offset) = id.strip_prefix("GMT") {
            let bytes = offset.as_bytes();
            let fixed_offset = bytes.len() == 6
                && matches!(bytes.first().copied(), Some(b'+') | Some(b'-'))
                && bytes.get(1..3).is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
                && bytes.get(3) == Some(&b':')
                && bytes.get(4..6).is_some_and(|digits| digits.iter().all(u8::is_ascii_digit));
            if fixed_offset {
                id = offset.to_owned();
            }
        }
        Ok(Some(id))
    })();
    if operation.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    operation
}

/// Read the locale configured for Android's application resources, including per-app language.
pub(super) fn system_language_from_context(env: &mut jni::JNIEnv<'_>, context: &JObject<'_>) -> Result<Option<String>, jni::errors::Error> {
    use jni::objects::JString;

    let operation = (|| -> Result<Option<String>, jni::errors::Error> {
        let resources = env.call_method(context, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
        if resources.is_null() {
            return Ok(None);
        }
        let configuration = env.call_method(&resources, "getConfiguration", "()Landroid/content/res/Configuration;", &[])?.l()?;
        if configuration.is_null() {
            return Ok(None);
        }
        let locales = env.call_method(&configuration, "getLocales", "()Landroid/os/LocaleList;", &[])?.l()?;
        if locales.is_null() {
            return Ok(None);
        }
        let locale = env.call_method(&locales, "get", "(I)Ljava/util/Locale;", &[jni::objects::JValue::Int(0)])?.l()?;
        if locale.is_null() {
            return Ok(None);
        }
        let tag = env.call_method(&locale, "toLanguageTag", "()Ljava/lang/String;", &[])?.l()?;
        if tag.is_null() {
            return Ok(None);
        }
        let tag = env.get_string(&JString::from(tag))?.to_string_lossy().into_owned();
        Ok((!tag.is_empty() && tag.len() <= 128 && !tag.chars().any(char::is_control)).then_some(tag))
    })();
    if operation.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    operation
}

/// WorkManager invokes the same Rust sync pool when the NativeActivity has released its cache.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_mailcraft_MailCraftSyncWorker_runNativeSync(
    mut env: jni::JNIEnv<'_>,
    worker: jni::objects::JObject<'_>,
) -> jni::sys::jint {
    let result = super::panic_boundary::catch_or(
        || run_native_sync(&mut env, &worker),
        Err("Android background sync stopped after an internal error".to_owned()),
    );
    match result {
        Ok(status) => status,
        Err(error) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            log::warn!("Android background sync will retry: {error}");
            1
        }
    }
}

fn run_native_sync(mut env: &mut jni::JNIEnv<'_>, worker: &jni::objects::JObject<'_>) -> Result<jni::sys::jint, String> {
    use std::sync::Arc;

    const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8 * 60);

    let input = env
        .call_method(&worker, "getInputData", "()Landroidx/work/Data;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    let reminders_only_key = env.new_string("remindersOnly").map_err(|error| error.to_string())?;
    let reminders_only = env
        .call_method(
            &input,
            "getBoolean",
            "(Ljava/lang/String;Z)Z",
            &[jni::objects::JValue::Object(reminders_only_key.as_ref()), jni::objects::JValue::Bool(0)],
        )
        .map_err(|error| error.to_string())?
        .z()
        .map_err(|error| error.to_string())?;
    if !reminders_only && super::is_work_offline() {
        return Ok(0);
    }
    static NEXT_REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    if !reminders_only && let Some(pool) = super::active_pool() {
        let request_id = NEXT_REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed).max(1);
        let waiter = pool.lock().unwrap_or_else(std::sync::PoisonError::into_inner).request_full_sync(request_id);
        let completed =
            waiter.wait_while(request_id, SYNC_TIMEOUT, || match env.call_method(&worker, "isStopped", "()Z", &[]).and_then(|value| value.z()) {
                Ok(stopped) => stopped,
                Err(_) => {
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    true
                }
            });
        return Ok(if completed == Some(true) { 0 } else { 1 });
    }

    let context = env
        .call_method(&worker, "getApplicationContext", "()Landroid/content/Context;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if context.is_null() {
        return Err("Android did not provide its application context".into());
    }
    let original_dir = context_directory_path(&mut env, &context, "getFilesDir")?;
    let no_backup_dir = context_directory_path(&mut env, &context, "getNoBackupFilesDir")?;
    let data_root = super::app_data_root_from_paths(original_dir, no_backup_dir)
        .ok_or_else(|| "Android did not provide MailCraft's private data directory".to_string())?;
    let cache = data_root.join("mailcraft");
    let system_language = match system_language_from_context(&mut env, &context) {
        Ok(language) => language,
        Err(error) => {
            log::warn!("Android background work could not read the app language: {error}");
            None
        }
    };
    let timezone = match system_timezone_from_env(&mut env) {
        Ok(timezone) => timezone,
        Err(error) => {
            log::warn!("Android background work could not read the system time zone: {error}");
            None
        }
    };
    let outcome = if reminders_only {
        super::background::reminders(cache, timezone.as_deref(), system_language.as_deref())
    } else {
        let roots = system_trust_roots_from_env(&mut env).map_err(|error| error.to_string())?;
        if let Err(error) = mailcraft_net::stream::set_android_system_roots(roots) {
            log::warn!("Android background sync could not load system TLS roots: {error}");
        }
        let keyring = android_native_keyring_store::Store::new().map_err(|error| format!("Android Keystore is unavailable: {error}"))?;
        keyring_core::set_default_store(keyring);
        let credentials: Arc<dyn mailcraft_net::worker::Credentials> = Arc::new(super::AndroidCredentials);
        super::background::sync(cache, credentials, SYNC_TIMEOUT, timezone.as_deref(), system_language.as_deref(), || {
            match env.call_method(&worker, "isStopped", "()Z", &[]).and_then(|value| value.z()) {
                Ok(stopped) => stopped,
                Err(_) => {
                    if env.exception_check().unwrap_or(false) {
                        let _ = env.exception_clear();
                    }
                    true
                }
            }
        })
    };
    let (status, new_mail, new_mail_key, reminders) = match outcome {
        super::background::SyncOutcome::Finished { new_mail, new_mail_key, reminders } => (0, new_mail, new_mail_key, reminders),
        super::background::SyncOutcome::Retry { new_mail, new_mail_key, reminders } => (1, new_mail, new_mail_key, reminders),
        super::background::SyncOutcome::CacheInUse => (1, 0, None, Vec::new()),
    };
    let should_notify = match env.call_method(&worker, "isStopped", "()Z", &[]).and_then(|value| value.z()) {
        Ok(stopped) => !stopped,
        Err(_) => {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            false
        }
    };
    if new_mail > 0 && should_notify {
        let title = env.new_string(mailcraft_ui_egui::i18n::t("New mail")).map_err(|error| error.to_string())?;
        let count = new_mail.to_string();
        let body = env.new_string(mailcraft_ui_egui::i18n::tfmt("{n} new messages", &[("n", &count)])).map_err(|error| error.to_string())?;
        let key = new_mail_key.filter(|key| key.len() <= 1024 && !key.chars().any(char::is_control)).unwrap_or_default();
        let key = env.new_string(key).map_err(|error| error.to_string())?;
        env.call_method(
            &worker,
            "postNewMail",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[jni::objects::JValue::Object(title.as_ref()), jni::objects::JValue::Object(body.as_ref()), jni::objects::JValue::Object(key.as_ref())],
        )
        .map_err(|error| error.to_string())?;
    }
    if should_notify {
        for reminder in reminders {
            if reminder.account.len() > 256
                || reminder.uid.len() > 1024
                || reminder.account.chars().any(char::is_control)
                || reminder.uid.chars().any(char::is_control)
            {
                continue;
            }
            let title = mailcraft_ui_egui::i18n::t(if reminder.task { "Tasks" } else { "Calendar" });
            let body = super::notifications::bounded_text(&reminder.summary);
            let account = env.new_string(&reminder.account).map_err(|error| error.to_string())?;
            let uid = env.new_string(&reminder.uid).map_err(|error| error.to_string())?;
            let title = env.new_string(title).map_err(|error| error.to_string())?;
            let body = env.new_string(body).map_err(|error| error.to_string())?;
            let identity = env
                .new_string(super::notifications::reminder_identity(
                    &reminder.account,
                    &reminder.uid,
                    reminder.occurrence,
                    reminder.due,
                    reminder.task,
                ))
                .map_err(|error| error.to_string())?;
            let _ = env
                .call_method(
                    &worker,
                    "postReminder",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZLjava/lang/String;)Z",
                    &[
                        jni::objects::JValue::Object(title.as_ref()),
                        jni::objects::JValue::Object(body.as_ref()),
                        jni::objects::JValue::Object(account.as_ref()),
                        jni::objects::JValue::Object(uid.as_ref()),
                        jni::objects::JValue::Long(reminder.occurrence),
                        jni::objects::JValue::Long(reminder.due),
                        jni::objects::JValue::Bool(reminder.task as u8),
                        jni::objects::JValue::Object(identity.as_ref()),
                    ],
                )
                .map_err(|error| error.to_string())?
                .z()
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(status)
}

fn context_directory_path(
    env: &mut jni::JNIEnv<'_>,
    context: &jni::objects::JObject<'_>,
    method: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    use jni::objects::JString;

    let directory =
        env.call_method(context, method, "()Ljava/io/File;", &[]).map_err(|error| error.to_string())?.l().map_err(|error| error.to_string())?;
    if directory.is_null() {
        return Ok(None);
    }
    let path = env
        .call_method(&directory, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if path.is_null() {
        return Ok(None);
    }
    let path = env.get_string(&JString::from(path)).map_err(|error| error.to_string())?.to_string_lossy().into_owned();
    Ok(Some(std::path::PathBuf::from(path)))
}
