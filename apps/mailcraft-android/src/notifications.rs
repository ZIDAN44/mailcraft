//! Native Android notifications for new mail and calendar reminders.

use super::activity;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant};

static PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);
static NEXT_NOTIFICATION_ID: AtomicI32 = AtomicI32::new(1);
const MAX_TARGET_BYTES: usize = 1024;

pub(super) enum ShowResult {
    Posted,
    WaitingForPermission,
    Denied,
    ChannelDisabled,
}

pub(super) struct Queue {
    pending: VecDeque<(String, String, mailcraft_ui_egui::NotificationTarget)>,
    checked_at: Option<Instant>,
    retry_at: Option<Instant>,
    permission_denied_notified: bool,
}

impl Queue {
    const CAPACITY: usize = 16;

    pub(super) fn new() -> Self {
        Self { pending: VecDeque::new(), checked_at: None, retry_at: None, permission_denied_notified: false }
    }

    pub(super) fn push(&mut self, title: &str, body: &str, target: mailcraft_ui_egui::NotificationTarget) {
        if self.pending.len() >= Self::CAPACITY {
            self.pending.pop_front();
        }
        self.pending.push_back((bounded_text(title), bounded_text(body), target));
    }

    pub(super) fn try_show(
        &mut self,
        app: &winit::platform::android::activity::AndroidApp,
        repaint: &egui::Context,
        errors: &std::sync::mpsc::Sender<String>,
    ) {
        if self.pending.is_empty() {
            return;
        }
        if let Some(retry_at) = self.retry_at {
            let remaining = retry_at.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                repaint.request_repaint_after(remaining);
                return;
            }
            self.retry_at = None;
        }
        if self.checked_at.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
            repaint.request_repaint_after(Duration::from_secs(1));
            return;
        }
        self.checked_at = Some(Instant::now());
        let Some((title, body, target)) = self.pending.front().cloned() else { return };
        match show(app, &title, &body, &target) {
            Ok(ShowResult::Posted) => {
                self.pending.pop_front();
                self.retry_at = None;
                self.permission_denied_notified = false;
            }
            Ok(ShowResult::WaitingForPermission) => {
                // The system permission sheet is open; retry when Android returns to the app.
                let retry_after = Duration::from_secs(15);
                self.retry_at = Instant::now().checked_add(retry_after);
                repaint.request_repaint_after(retry_after);
            }
            Ok(ShowResult::Denied) => {
                // A request was already shown and Android still denies notifications. Forget
                // these old alerts; a later one checks permission again without retaining them.
                self.pending.clear();
                self.retry_at = None;
                if !self.permission_denied_notified {
                    self.permission_denied_notified = true;
                    let message =
                        mailcraft_ui_egui::i18n::t("Android notifications are off. Turn them on in Settings for new mail and calendar reminders.");
                    let _ = errors.send(message.to_owned());
                    repaint.request_repaint();
                }
            }
            Ok(ShowResult::ChannelDisabled) => {
                // Respect the user's choice for this category without dropping a queued
                // notification from a different category, such as new mail.
                self.pending.pop_front();
                self.retry_at = None;
            }
            Err(error) => {
                eprintln!("MailCraft could not show an Android notification: {error}");
                self.pending.pop_front();
                self.retry_at = None;
            }
        }
        if self.retry_at.is_none() && !self.pending.is_empty() {
            repaint.request_repaint_after(Duration::from_secs(1));
        }
    }
}

pub(super) fn bounded_text(text: &str) -> String {
    const MAX_BYTES: usize = 512;
    let mut bounded = String::with_capacity(text.len().min(MAX_BYTES));
    for character in text.chars() {
        let bidi_control = matches!(character as u32, 0x061C | 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069);
        let character = if character.is_control() || bidi_control { ' ' } else { character };
        if bounded.len().saturating_add(character.len_utf8()) > MAX_BYTES {
            break;
        }
        bounded.push(character);
    }
    bounded.trim().to_owned()
}

fn safe_target_id(value: &str) -> bool {
    value.len() <= MAX_TARGET_BYTES && !value.chars().any(char::is_control)
}

pub(super) fn reminder_identity(account: &str, uid: &str, occurrence: i64, due: i64, task: bool) -> String {
    format!("{}:{account}{}:{uid}:{occurrence}:{due}:{}", account.len(), uid.len(), u8::from(task))
}

/// Keep all notification categories available and named in the UI's active language.
pub(super) fn refresh_channels(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<()> {
            let service_name = env.new_string("notification")?;
            let manager = env
                .call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service_name.as_ref())])?
                .l()?;
            if manager.is_null() {
                return Err(jni::errors::Error::NullPtr("notification manager"));
            }
            for (channel_id, channel_name) in [
                ("ai.storyteller.mailcraft.mail", mailcraft_ui_egui::i18n::t("New mail")),
                ("ai.storyteller.mailcraft.calendar", mailcraft_ui_egui::i18n::t("Calendar")),
                ("ai.storyteller.mailcraft.tasks", mailcraft_ui_egui::i18n::t("Tasks")),
            ] {
                let channel_id = env.new_string(channel_id)?;
                let channel_name = env.new_string(channel_name)?;
                let channel = env
                    .call_method(
                        &manager,
                        "getNotificationChannel",
                        "(Ljava/lang/String;)Landroid/app/NotificationChannel;",
                        &[JValue::Object(channel_id.as_ref())],
                    )?
                    .l()?;
                if channel.is_null() {
                    let channel = env.new_object(
                        "android/app/NotificationChannel",
                        "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
                        &[JValue::Object(channel_id.as_ref()), JValue::Object(channel_name.as_ref()), JValue::Int(3)],
                    )?;
                    env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&channel)])?;
                } else {
                    env.call_method(&channel, "setName", "(Ljava/lang/CharSequence;)V", &[JValue::Object(channel_name.as_ref())])?;
                    env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&channel)])?;
                }
            }
            Ok(())
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}

/// Posts a notification, or requests Android 13+'s permission when needed.
pub(super) fn show(
    app: &winit::platform::android::activity::AndroidApp,
    title: &str,
    body: &str,
    target: &mailcraft_ui_egui::NotificationTarget,
) -> Result<ShowResult, String> {
    use jni::objects::JValue;

    let result = activity::with_activity(app, |env, activity| {
        let operation = (|| -> robius_android_env::Result<ShowResult> {
            let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
            let permission = env.new_string("android.permission.POST_NOTIFICATIONS")?;
            if sdk >= 33 {
                let denied =
                    env.call_method(activity, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(permission.as_ref())])?.i()? != 0;
                if denied {
                    if PERMISSION_REQUESTED.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                        let request = env.call_method(activity, "requestMailCraftNotificationPermission", "()V", &[]);
                        if request.is_err() {
                            PERMISSION_REQUESTED.store(false, Ordering::Relaxed);
                        }
                        request?;
                        return Ok(ShowResult::WaitingForPermission);
                    }
                    let pending = env.call_method(activity, "isMailCraftNotificationPermissionPending", "()Z", &[])?.z()?;
                    if pending {
                        return Ok(ShowResult::WaitingForPermission);
                    }
                    // Android has returned from the prompt without granting permission. Let a
                    // later notification ask again; the system can still show it on request.
                    PERMISSION_REQUESTED.store(false, Ordering::Relaxed);
                    return Ok(ShowResult::Denied);
                }
                PERMISSION_REQUESTED.store(false, Ordering::Relaxed);
            }

            let (channel_id, channel_name) = match target {
                mailcraft_ui_egui::NotificationTarget::Reminder { task: true, .. } | mailcraft_ui_egui::NotificationTarget::Tasks => {
                    ("ai.storyteller.mailcraft.tasks", mailcraft_ui_egui::i18n::t("Tasks"))
                }
                mailcraft_ui_egui::NotificationTarget::Reminder { task: false, .. } | mailcraft_ui_egui::NotificationTarget::Calendar => {
                    ("ai.storyteller.mailcraft.calendar", mailcraft_ui_egui::i18n::t("Calendar"))
                }
                mailcraft_ui_egui::NotificationTarget::Inbox | mailcraft_ui_egui::NotificationTarget::Message { .. } => {
                    ("ai.storyteller.mailcraft.mail", mailcraft_ui_egui::i18n::t("New mail"))
                }
            };
            let channel_id = env.new_string(channel_id)?;
            let channel_name = env.new_string(channel_name)?;
            let service_name = env.new_string("notification")?;
            let manager = env
                .call_method(activity, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(service_name.as_ref())])?
                .l()?;
            if manager.is_null() {
                return Err(jni::errors::Error::NullPtr("notification manager"));
            }
            if !env.call_method(&manager, "areNotificationsEnabled", "()Z", &[])?.z()? {
                return Ok(ShowResult::Denied);
            }

            let channel = env
                .call_method(
                    &manager,
                    "getNotificationChannel",
                    "(Ljava/lang/String;)Landroid/app/NotificationChannel;",
                    &[JValue::Object(channel_id.as_ref())],
                )?
                .l()?;
            if channel.is_null() {
                let new_channel = env.new_object(
                    "android/app/NotificationChannel",
                    "(Ljava/lang/String;Ljava/lang/CharSequence;I)V",
                    &[JValue::Object(channel_id.as_ref()), JValue::Object(channel_name.as_ref()), JValue::Int(3)],
                )?;
                env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&new_channel)])?;
            } else {
                // Android keeps channel names across launches. Refresh the localized name when
                // the Activity's configured language changes, while leaving importance alone.
                env.call_method(&channel, "setName", "(Ljava/lang/CharSequence;)V", &[JValue::Object(channel_name.as_ref())])?;
                env.call_method(&manager, "createNotificationChannel", "(Landroid/app/NotificationChannel;)V", &[JValue::Object(&channel)])?;
                if env.call_method(&channel, "getImportance", "()I", &[])?.i()? <= 0 {
                    return Ok(ShowResult::ChannelDisabled);
                }
            }

            if let mailcraft_ui_egui::NotificationTarget::Reminder { account, uid, occurrence, due, task } = target
                && safe_target_id(account)
                && safe_target_id(uid)
            {
                let identity = env.new_string(&reminder_identity(account, uid, *occurrence, *due, *task))?;
                let already_posted = env
                    .call_static_method(
                        "ai/storyteller/mailcraft/MailCraftSyncWorker",
                        "wasReminderPosted",
                        "(Landroid/content/Context;Ljava/lang/String;)Z",
                        &[JValue::Object(activity), JValue::Object(identity.as_ref())],
                    )?
                    .z()?;
                if already_posted {
                    return Ok(ShowResult::Posted);
                }
            }

            let builder = env.new_object(
                "android/app/Notification$Builder",
                "(Landroid/content/Context;Ljava/lang/String;)V",
                &[JValue::Object(activity), JValue::Object(channel_id.as_ref())],
            )?;
            let resources = env.call_method(activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
            let icon_name = env.new_string("mailcraft")?;
            let drawable_type = env.new_string("drawable")?;
            let icon_package = env.new_string("ai.storyteller.mailcraft")?;
            let app_icon = env
                .call_method(
                    &resources,
                    "getIdentifier",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)I",
                    &[JValue::Object(icon_name.as_ref()), JValue::Object(drawable_type.as_ref()), JValue::Object(icon_package.as_ref())],
                )?
                .i()?;
            // Keep notifications usable if an OEM packages resources differently.
            let icon = if app_icon > 0 { app_icon } else { env.get_static_field("android/R$drawable", "ic_dialog_email", "I")?.i()? };
            env.call_method(&builder, "setSmallIcon", "(I)Landroid/app/Notification$Builder;", &[JValue::Int(icon)])?;
            let title = env.new_string(title)?;
            env.call_method(
                &builder,
                "setContentTitle",
                "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
                &[JValue::Object(title.as_ref())],
            )?;
            let body = env.new_string(body)?;
            env.call_method(
                &builder,
                "setContentText",
                "(Ljava/lang/CharSequence;)Landroid/app/Notification$Builder;",
                &[JValue::Object(body.as_ref())],
            )?;
            env.call_method(&builder, "setAutoCancel", "(Z)Landroid/app/Notification$Builder;", &[JValue::Bool(1)])?;
            // Keep sender and subject off the lock screen; Android shows only the app name there.
            env.call_method(&builder, "setVisibility", "(I)Landroid/app/Notification$Builder;", &[JValue::Int(0)])?;

            let activity_class = env.call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?.l()?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Landroid/content/Context;Ljava/lang/Class;)V",
                &[JValue::Object(activity), JValue::Object(&activity_class)],
            )?;
            // Launch a fresh NativeActivity so android_main can read this notification's target.
            // android_native_app_glue doesn't forward onNewIntent to the native event loop; reusing
            // the existing Activity would leave it on the notification or share that opened it.
            let notification_id = NEXT_NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed).max(1);
            let kind = match target {
                mailcraft_ui_egui::NotificationTarget::Inbox => "inbox",
                mailcraft_ui_egui::NotificationTarget::Message { key } if safe_target_id(key) => "message",
                mailcraft_ui_egui::NotificationTarget::Message { .. } => "inbox",
                mailcraft_ui_egui::NotificationTarget::Reminder { uid, task: true, .. } if safe_target_id(uid) => "task",
                mailcraft_ui_egui::NotificationTarget::Reminder { uid, task: false, .. } if safe_target_id(uid) => "event",
                mailcraft_ui_egui::NotificationTarget::Reminder { task: true, .. } => "tasks",
                mailcraft_ui_egui::NotificationTarget::Reminder { task: false, .. } => "calendar",
                mailcraft_ui_egui::NotificationTarget::Calendar => "calendar",
                mailcraft_ui_egui::NotificationTarget::Tasks => "tasks",
            };
            let target_key = env.new_string("mailcraft.notification")?;
            let kind_value = env.new_string(kind)?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(target_key.as_ref()), JValue::Object(kind_value.as_ref())],
            )?;
            if let mailcraft_ui_egui::NotificationTarget::Reminder { account, uid, occurrence, due, task: _ } = target
                && safe_target_id(account)
                && safe_target_id(uid)
            {
                let account_key = env.new_string("mailcraft.account")?;
                let account_value = env.new_string(account)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(account_key.as_ref()), JValue::Object(account_value.as_ref())],
                )?;
                let uid_key = env.new_string("mailcraft.uid")?;
                let uid_value = env.new_string(uid)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(uid_key.as_ref()), JValue::Object(uid_value.as_ref())],
                )?;
                let occurrence_key = env.new_string("mailcraft.occurrence")?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;J)Landroid/content/Intent;",
                    &[JValue::Object(occurrence_key.as_ref()), JValue::Long(*occurrence)],
                )?;
                let due_key = env.new_string("mailcraft.due")?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;J)Landroid/content/Intent;",
                    &[JValue::Object(due_key.as_ref()), JValue::Long(*due)],
                )?;
            }
            if let mailcraft_ui_egui::NotificationTarget::Message { key } = target
                && safe_target_id(key)
            {
                let key_name = env.new_string("mailcraft.key")?;
                let key_value = env.new_string(key)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(key_name.as_ref()), JValue::Object(key_value.as_ref())],
                )?;
            }
            let pending = env
                .call_static_method(
                    "android/app/PendingIntent",
                    "getActivity",
                    "(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;",
                    &[JValue::Object(activity), JValue::Int(notification_id), JValue::Object(&intent), JValue::Int(0x0c000000)],
                )?
                .l()?;
            env.call_method(
                &builder,
                "setContentIntent",
                "(Landroid/app/PendingIntent;)Landroid/app/Notification$Builder;",
                &[JValue::Object(&pending)],
            )?;
            let notification = env.call_method(&builder, "build", "()Landroid/app/Notification;", &[])?.l()?;
            env.call_method(&manager, "notify", "(ILandroid/app/Notification;)V", &[JValue::Int(notification_id), JValue::Object(&notification)])?;
            if let mailcraft_ui_egui::NotificationTarget::Reminder { account, uid, occurrence, due, task } = target
                && safe_target_id(account)
                && safe_target_id(uid)
            {
                let identity = env.new_string(&reminder_identity(account, uid, *occurrence, *due, *task))?;
                env.call_static_method(
                    "ai/storyteller/mailcraft/MailCraftSyncWorker",
                    "markReminderPosted",
                    "(Landroid/content/Context;Ljava/lang/String;)V",
                    &[JValue::Object(activity), JValue::Object(identity.as_ref())],
                )?;
            }
            Ok(ShowResult::Posted)
        })();
        if operation.is_err() && env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        operation
    });
    match result {
        Ok(Ok(posted)) => Ok(posted),
        Ok(Err(error)) | Err(error) => Err(error.to_string()),
    }
}
