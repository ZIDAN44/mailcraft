//! A scheduled Android pass reuses MailCraft's production session and network pool. A second
//! process may open the cache only after the NativeActivity has released its exclusive lease.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{Event, SyncStatus};
use mailcraft_engine::store::{Store, StoreError};
use mailcraft_net::worker::{Credentials, Pool};

pub(super) enum SyncOutcome {
    Finished { new_mail: u32, new_mail_key: Option<String>, reminders: Vec<mailcraft_engine::DueReminder> },
    Retry { new_mail: u32, new_mail_key: Option<String>, reminders: Vec<mailcraft_engine::DueReminder> },
    CacheInUse,
}

pub(super) fn sync(
    cache: std::path::PathBuf,
    credentials: Arc<dyn Credentials>,
    timeout: Duration,
    timezone: Option<&str>,
    system_language: Option<&str>,
    mut is_stopped: impl FnMut() -> bool,
) -> SyncOutcome {
    let saved_state = read_saved_ui_state(&cache);
    set_notification_language(saved_state.as_ref(), system_language);
    let store = match Store::open_alone(cache) {
        Ok(store) => store,
        Err(StoreError::Io { source, .. }) if source.to_string().contains("open in another copy of MailCraft") => {
            return SyncOutcome::CacheInUse;
        }
        Err(error) => {
            log::warn!("Android background sync could not open the offline cache: {error}");
            return SyncOutcome::Retry { new_mail: 0, new_mail_key: None, reminders: Vec::new() };
        }
    };

    let local = chrono::Local::now();
    let mut session = mailcraft_engine::Session::open(Some(store.clone()), Clock::new(local.timestamp(), local.offset().local_minus_utc()));
    session.set_zone(timezone);
    if let Some(prefs) = saved_state
        .as_ref()
        .and_then(|state| state.get("prefs"))
        .and_then(|prefs| serde_json::from_value::<mailcraft_engine::Prefs>(prefs.clone()).ok())
    {
        session.prefs = prefs;
        let defaults = mailcraft_engine::Prefs::default();
        if (session.prefs.keep_messages, session.prefs.keep_offline) != (defaults.keep_messages, defaults.keep_offline) {
            session.apply_sync_prefs();
        }
    }
    // Scheduled work must never override the user's persisted Send / Receive preference.
    if session.prefs.work_offline {
        return SyncOutcome::Finished { new_mail: 0, new_mail_key: None, reminders: due_reminders(&session) };
    }
    session.oauth_available = mailcraft_net::oauth::available();
    session.set_trust_roots(mailcraft_net::trust_roots());
    session.set_card_modules(mailcraft_net::card_modules());
    let mut pending: HashSet<String> =
        session.accounts.iter().filter(|account| !account.is_local()).map(|account| account.config.id.clone()).collect();
    if pending.is_empty() {
        return SyncOutcome::Finished { new_mail: 0, new_mail_key: None, reminders: due_reminders(&session) };
    }

    let mut pool = Pool::new(store, credentials);
    for job in session.take_jobs() {
        pool.submit(job);
    }

    let deadline = Instant::now().checked_add(timeout);
    let mut retry = false;
    let mut new_mail = 0u32;
    let mut new_mail_key = None;
    let mut newest_mail_when = i64::MIN;
    while !pending.is_empty() {
        if is_stopped() {
            retry = true;
            break;
        }
        let Some(deadline) = deadline else {
            retry = true;
            break;
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            retry = true;
            break;
        }
        let mut events = pool.poll();
        if events.is_empty() {
            if let Some(event) = pool.wait(remaining.min(Duration::from_millis(500))) {
                events.push(event);
                events.extend(pool.poll());
            }
        }
        for event in events {
            match &event {
                Event::SyncFinished { account, success, .. } => {
                    pending.remove(account);
                    retry |= !success;
                }
                Event::Status { account, status: SyncStatus::NeedsPassword { .. } } => {
                    // A user action is required before retrying this account; do not wake it every
                    // WorkManager interval while its credentials are absent or rejected.
                    pending.remove(account);
                }
                _ => {}
            }
            session.apply(event);
            let arrived = session.take_new_mail();
            if let Some(message) = arrived.iter().max_by_key(|message| message.when())
                && message.when() >= newest_mail_when
            {
                newest_mail_when = message.when();
                new_mail_key = Some(message.key.to_id());
            }
            new_mail = new_mail.saturating_add(arrived.len().min(50) as u32).min(999);
        }
        for job in session.take_jobs() {
            pool.submit(job);
        }
    }
    pool.shutdown();
    let reminders = due_reminders(&session);
    if retry { SyncOutcome::Retry { new_mail, new_mail_key, reminders } } else { SyncOutcome::Finished { new_mail, new_mail_key, reminders } }
}

/// Check cached calendar/task reminders without requiring a network connection.
pub(super) fn reminders(cache: std::path::PathBuf, timezone: Option<&str>, system_language: Option<&str>) -> SyncOutcome {
    let saved_state = read_saved_ui_state(&cache);
    set_notification_language(saved_state.as_ref(), system_language);
    let store = match Store::open_alone(cache) {
        Ok(store) => store,
        Err(StoreError::Io { source, .. }) if source.to_string().contains("open in another copy of MailCraft") => {
            return SyncOutcome::CacheInUse;
        }
        Err(error) => {
            log::warn!("Android could not open the cache for reminder checks: {error}");
            return SyncOutcome::Retry { new_mail: 0, new_mail_key: None, reminders: Vec::new() };
        }
    };
    let local = chrono::Local::now();
    let mut session = mailcraft_engine::Session::open(Some(store), Clock::new(local.timestamp(), local.offset().local_minus_utc()));
    session.set_zone(timezone);
    SyncOutcome::Finished { new_mail: 0, new_mail_key: None, reminders: due_reminders(&session) }
}

fn due_reminders(session: &mailcraft_engine::Session) -> Vec<mailcraft_engine::DueReminder> {
    session.due_reminders(chrono::Local::now().timestamp())
}

fn read_saved_ui_state(cache: &std::path::Path) -> Option<serde_json::Value> {
    use std::io::Read as _;

    const MAX_UI_STATE_BYTES: u64 = 4 << 20;
    let path = cache.join("ui.json");
    let Ok(file) = std::fs::File::open(path) else { return None };
    let mut bytes = Vec::new();
    let mut reader = file.take(MAX_UI_STATE_BYTES.saturating_add(1));
    if reader.read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_UI_STATE_BYTES {
        return None;
    }
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()
}

fn set_notification_language(state: Option<&serde_json::Value>, system_language: Option<&str>) {
    let language = state.and_then(|state| state.get("language")).and_then(serde_json::Value::as_str).unwrap_or("auto");
    let lang = if language == mailcraft_ui_egui::i18n::AUTO {
        system_language.and_then(mailcraft_ui_egui::i18n::lang_from_tag).unwrap_or_else(|| mailcraft_ui_egui::i18n::Lang::from_pref(language))
    } else {
        mailcraft_ui_egui::i18n::Lang::from_pref(language)
    };
    mailcraft_ui_egui::i18n::set_current(lang);
}
