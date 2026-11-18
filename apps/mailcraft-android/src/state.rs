use super::*;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const MAX_STATE_BYTES: u64 = 4 << 20;
const DRAFT_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(5);
const DRAFT_EDIT_QUIET_TIME: Duration = Duration::from_secs(2);

#[cfg(target_os = "android")]
pub(super) fn read_saved_state(path: &std::path::Path) -> Option<serde_json::Value> {
    let read = std::fs::File::open(path).and_then(|file| {
        let mut bytes = Vec::new();
        let mut reader = std::io::Read::take(file, MAX_STATE_BYTES.saturating_add(1));
        std::io::Read::read_to_end(&mut reader, &mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(std::io::Error::other("saved Android preferences are too large"));
        }
        Ok(bytes)
    });
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("MailCraft could not read Android preferences: {error}");
            set_aside_saved_state(path);
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(error) => {
            eprintln!("MailCraft could not restore Android preferences: {error}");
            set_aside_saved_state(path);
            None
        }
    }
}

#[cfg(target_os = "android")]
fn saved_state_bytes(mail: &mailcraft_ui_egui::MailApp) -> Result<Vec<u8>, String> {
    let mut state = mail.saved_state();
    if let Some(object) = state.as_object_mut()
        && let Some(key) = mail.session.compose.as_ref().and_then(|compose| compose.draft.as_ref())
    {
        object.insert("activeDraft".into(), serde_json::Value::String(key.to_id()));
    }
    if let Some(object) = state.as_object_mut()
        && let Some(compose) = mail.session.compose.as_ref()
        && compose.draft_id.len() <= 1024
        && !compose.draft_id.chars().any(char::is_control)
        && !compose.draft_id.is_empty()
    {
        object.insert("activeDraftAccount".into(), serde_json::Value::String(compose.account.clone()));
        object.insert("activeDraftMessageId".into(), serde_json::Value::String(compose.draft_id.clone()));
    }
    let bytes = serde_json::to_vec(&state).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err("saved Android preferences are larger than 4 MiB".into());
    }
    Ok(bytes)
}

#[cfg(target_os = "android")]
fn set_aside_saved_state(path: &std::path::Path) {
    static NEXT_BAD_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
    let index = NEXT_BAD_STATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let aside = path.with_extension(format!("bad-{stamp}-{index}"));
    if let Err(error) = std::fs::rename(path, &aside) {
        eprintln!("MailCraft could not set aside damaged Android preferences: {error}");
    }
}

#[cfg(target_os = "android")]
fn write_saved_state(path: &std::path::Path, bytes: &[u8]) -> bool {
    match mailcraft_engine::store::write_atomic(path, bytes) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("MailCraft could not save Android preferences: {error}");
            false
        }
    }
}

#[cfg(target_os = "android")]
pub(super) struct AndroidApp {
    pub(super) mail: mailcraft_ui_egui::MailApp,
    pub(super) pending_draft_restore: Option<PendingDraftRestore>,
    pub(super) activity: winit::platform::android::activity::AndroidApp,
    pub(super) state_writer: AndroidStateWriter,
    pub(super) handoff_rx: Receiver<Result<AndroidOpen, String>>,
    pub(super) handoff_route: std::sync::Arc<HandoffRoute>,
    pub(super) handoff_remaining: usize,
    pub(super) cache_warning: bool,
    pub(super) system_insets: Option<platform::SystemInsets>,
    pub(super) system_bar_colors: Option<(egui::Color32, egui::Color32)>,
    pub(super) insets_checked_at: Instant,
    pub(super) network_available: Option<bool>,
    pub(super) network_checked_at: Instant,
    pub(super) network_sync_pending: bool,
    pub(super) resume_sync_pending: bool,
    pub(super) was_work_offline: bool,
    pub(super) was_focused: Option<bool>,
    pub(super) last_resume_sync: Option<Instant>,
    pub(super) draft_checkpoint_at: Instant,
    pub(super) last_input_at: Instant,
    pub(super) display_checked_at: Instant,
}

pub(super) struct PendingDraftRestore {
    key: Option<mailcraft_engine::model::MessageKey>,
    account: Option<String>,
    message_id: Option<String>,
}

impl PendingDraftRestore {
    pub(super) fn from_saved_state(state: &serde_json::Value) -> Option<Self> {
        let key = state.get("activeDraft").and_then(serde_json::Value::as_str).and_then(mailcraft_engine::model::MessageKey::from_id).filter(|key| {
            key.account.len() <= 256
                && !key.account.chars().any(char::is_control)
                && key.folder.len() <= 4096
                && !key.folder.chars().any(char::is_control)
        });
        let account = state
            .get("activeDraftAccount")
            .and_then(serde_json::Value::as_str)
            .filter(|account| account.len() <= 256 && !account.chars().any(char::is_control))
            .map(str::to_owned);
        let message_id = state
            .get("activeDraftMessageId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 1024 && !id.chars().any(char::is_control))
            .map(str::to_owned);
        (key.is_some() || (account.is_some() && message_id.is_some())).then_some(Self { key, account, message_id })
    }
}

#[cfg(target_os = "android")]
pub(super) struct AndroidStateWriter {
    path: Option<std::path::PathBuf>,
    sender: Option<Sender<StateMessage>>,
    thread: Option<std::thread::JoinHandle<()>>,
    last_bytes: Option<Vec<u8>>,
    checked_at: Instant,
}

#[cfg(target_os = "android")]
impl Drop for AndroidApp {
    fn drop(&mut self) {
        self.handoff_route.deactivate();
    }
}

#[cfg(target_os = "android")]
enum StateMessage {
    Save(Vec<u8>),
    Flush(Sender<bool>),
    Stop,
}

#[cfg(target_os = "android")]
impl AndroidStateWriter {
    pub(super) fn new(path: Option<std::path::PathBuf>) -> Self {
        let (sender, thread) = if let Some(path) = &path {
            let (sender, receiver) = mpsc::channel::<StateMessage>();
            let path = path.clone();
            match std::thread::Builder::new().name("android-settings".into()).spawn(move || {
                let mut last_write_succeeded = true;
                let mut retry: Option<Vec<u8>> = None;
                loop {
                    let message = match receiver.recv_timeout(Duration::from_secs(30)) {
                        Ok(message) => message,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Some(bytes) = retry.take() {
                                if write_saved_state(&path, &bytes) {
                                    last_write_succeeded = true;
                                } else {
                                    retry = Some(bytes);
                                }
                            }
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    match message {
                        StateMessage::Save(bytes) => {
                            last_write_succeeded = write_saved_state(&path, &bytes);
                            retry = (!last_write_succeeded).then_some(bytes);
                        }
                        StateMessage::Flush(acknowledge) => {
                            if let Some(bytes) = retry.take() {
                                if write_saved_state(&path, &bytes) {
                                    last_write_succeeded = true;
                                } else {
                                    retry = Some(bytes);
                                }
                            }
                            let _ = acknowledge.send(last_write_succeeded);
                        }
                        StateMessage::Stop => {
                            if let Some(bytes) = retry.take() {
                                let _ = write_saved_state(&path, &bytes);
                            }
                            break;
                        }
                    }
                }
            }) {
                Ok(thread) => (Some(sender), Some(thread)),
                Err(error) => {
                    eprintln!("MailCraft could not start Android settings writer: {error}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };
        Self { path, sender, thread, last_bytes: None, checked_at: Instant::now() }
    }

    fn save_if_due(&mut self, mail: &mailcraft_ui_egui::MailApp) {
        if self.checked_at.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.checked_at = Instant::now();
        self.save(mail);
    }

    fn save(&mut self, mail: &mailcraft_ui_egui::MailApp) {
        let bytes = match saved_state_bytes(mail) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("MailCraft could not serialize Android preferences: {error}");
                return;
            }
        };
        if self.last_bytes.as_ref() == Some(&bytes) {
            return;
        }
        if let Some(sender) = &self.sender {
            if sender.send(StateMessage::Save(bytes.clone())).is_ok() {
                self.last_bytes = Some(bytes.clone());
                return;
            }
            self.sender = None;
        }
        if let Some(path) = &self.path {
            if let Err(error) = mailcraft_engine::store::write_atomic(path, &bytes) {
                eprintln!("MailCraft could not save Android preferences: {error}");
                return;
            }
            self.last_bytes = Some(bytes);
        }
    }

    /// Ensure the newest preferences queued before Android backgrounds the app have reached disk.
    fn save_before_background(&mut self, mail: &mailcraft_ui_egui::MailApp) {
        const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
        let bytes = match saved_state_bytes(mail) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("MailCraft could not serialize Android preferences: {error}");
                return;
            }
        };
        if self.last_bytes.as_ref() != Some(&bytes) {
            let queued = self.sender.as_ref().is_some_and(|sender| sender.send(StateMessage::Save(bytes.clone())).is_ok());
            if queued {
                self.last_bytes = Some(bytes.clone());
            } else if let Some(path) = &self.path {
                self.sender = None;
                if let Err(error) = mailcraft_engine::store::write_atomic(path, &bytes) {
                    eprintln!("MailCraft could not save Android preferences before suspending: {error}");
                    return;
                }
                self.last_bytes = Some(bytes);
                return;
            } else {
                return;
            }
        }
        let Some(sender) = &self.sender else { return };
        let (acknowledge, flushed) = mpsc::channel();
        if sender.send(StateMessage::Flush(acknowledge)).is_err() {
            self.sender = None;
            if let Some(path) = &self.path
                && let Some(bytes) = &self.last_bytes
                && let Err(error) = mailcraft_engine::store::write_atomic(path, bytes)
            {
                eprintln!("MailCraft could not save Android preferences before suspending: {error}");
            }
            return;
        }
        match flushed.recv_timeout(FLUSH_TIMEOUT) {
            Ok(true) => {}
            Ok(false) => {
                if let Some(path) = &self.path
                    && let Some(bytes) = &self.last_bytes
                    && let Err(error) = mailcraft_engine::store::write_atomic(path, bytes)
                {
                    eprintln!("MailCraft could not save Android preferences before suspending: {error}");
                }
            }
            Err(_) => {
                eprintln!("MailCraft's Android preferences were still being written after the flush timeout; writing a final copy synchronously");
                if let Some(path) = &self.path
                    && let Err(error) = mailcraft_engine::store::write_atomic(path, &bytes)
                {
                    eprintln!("MailCraft could not save Android preferences before suspending: {error}");
                }
            }
        }
    }

    fn close(&mut self, mail: &mailcraft_ui_egui::MailApp) {
        self.save(mail);
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(StateMessage::Stop);
        }
        if self.thread.take().is_some_and(|thread| thread.join().is_err()) {
            eprintln!("MailCraft's Android settings writer did not close cleanly");
        }
    }
}

#[cfg(target_os = "android")]
impl AndroidApp {
    fn checkpointed_draft_key(&self) -> Option<mailcraft_engine::model::MessageKey> {
        let pending = self.pending_draft_restore.as_ref()?;
        if let Some(key) = pending.key.as_ref()
            && self
                .mail
                .session
                .account(&key.account)
                .and_then(|account| account.folder(mailcraft_engine::model::FolderRole::Drafts))
                .is_some_and(|folder| folder.path.as_str() == key.folder.as_str())
            && let Some(store) = self.mail.session.store.as_ref()
            && let Ok(cached) = store.load_folder(&key.account, &key.folder)
            && cached.messages.iter().any(|summary| &summary.key == key)
        {
            return Some(key.clone());
        }
        let account_id = pending.account.as_deref()?;
        let wanted = mailcraft_engine::model::thread::norm_id(pending.message_id.as_deref()?);
        if wanted.is_empty() {
            return None;
        }
        let account = self.mail.session.account(account_id)?;
        let folder = account.folder(mailcraft_engine::model::FolderRole::Drafts)?;
        let store = self.mail.session.store.as_ref()?;
        let cached = store.load_folder(account_id, &folder.path).ok()?;
        cached
            .messages
            .iter()
            .find(|summary| mailcraft_engine::model::thread::norm_id(&summary.message_id) == wanted)
            .map(|summary| summary.key.clone())
    }

    fn restore_checkpointed_draft(&mut self) -> bool {
        if self.pending_draft_restore.is_none() {
            return false;
        }
        if self.mail.session.compose.is_some() {
            self.pending_draft_restore = None;
            return false;
        }
        let target_account = self
            .pending_draft_restore
            .as_ref()
            .and_then(|pending| pending.key.as_ref().map(|key| key.account.as_str()).or(pending.account.as_deref()));
        if target_account.is_some_and(|account| self.mail.session.account(account).is_none()) {
            self.pending_draft_restore = None;
            return false;
        }
        let Some(key) = self.checkpointed_draft_key() else { return false };
        self.mail.session.module = mailcraft_engine::Module::Mail;
        if self.mail.session.run("draft.edit", &serde_json::json!({"key": key.to_id()})).is_ok() {
            self.pending_draft_restore = None;
            true
        } else {
            false
        }
    }
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let Some(insets) = self.system_insets else { return };
        let scale = ctx.pixels_per_point().max(1.0);
        input.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
            left: insets.left.max(0) as f32 / scale,
            right: insets.right.max(0) as f32 / scale,
            top: insets.top.max(0) as f32 / scale,
            bottom: insets.bottom.max(0) as f32 / scale,
        }));
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| !input.events.is_empty()) {
            self.last_input_at = Instant::now();
        }
        let focused = ctx.input(|input| input.viewport().focused);
        let lost_focus = self.was_focused == Some(true) && focused == Some(false);
        let gained_focus = self.was_focused != Some(true) && focused == Some(true);
        if focused.is_some() {
            self.was_focused = focused;
        }
        if gained_focus {
            self.mail.services.on_resume();
            self.display_checked_at = Instant::now();
        } else if self.display_checked_at.elapsed() >= Duration::from_secs(5) {
            self.mail.services.refresh_display_configuration();
            self.display_checked_at = Instant::now();
        }
        if let Some(scale) = self.mail.services.system_font_scale() {
            ctx.set_zoom_factor(scale);
        }
        if lost_focus {
            let note_saved = self.mail.checkpoint_note_edit();
            let saved = self.mail.checkpoint_drafts();
            self.draft_checkpoint_at = Instant::now();
            if !self.mail.flush_changes(Duration::from_secs(2)) {
                log::warn!("Android lost focus before all offline mail changes reached the cache");
            }
            self.state_writer.save_before_background(&self.mail);
            if saved > 0 {
                log::info!("Android moved to the background; checkpointed {saved} draft(s)");
            }
            if note_saved {
                log::info!("Android moved to the background; saved the note being edited");
            }
        }
        if self.draft_checkpoint_at.elapsed() >= DRAFT_CHECKPOINT_INTERVAL && self.last_input_at.elapsed() >= DRAFT_EDIT_QUIET_TIME {
            self.draft_checkpoint_at = Instant::now();
            let note_saved = self.mail.checkpoint_note_edit();
            let saved = self.mail.checkpoint_drafts();
            if saved > 0 {
                log::info!("Android checkpointed {saved} changed draft(s) while open");
            }
            if note_saved {
                log::info!("Android checkpointed the note being edited while open");
            }
        }
        let sync_due = self.last_resume_sync.is_none_or(|at| at.elapsed() >= Duration::from_secs(30));
        let network_restored = if self.network_checked_at.elapsed() >= Duration::from_secs(15) {
            self.network_checked_at = Instant::now();
            if let Some(available) = platform::network_available(&self.activity) {
                let restored = self.network_available != Some(true) && available;
                self.network_available = Some(available);
                restored
            } else {
                false
            }
        } else {
            false
        };
        self.resume_sync_pending |= gained_focus;
        self.network_sync_pending |= network_restored;
        // A resume or network recovery must not silently consume its sync request while the
        // user has explicitly chosen to work offline. Leave it pending so switching back online
        // while this window is open queues the catch-up sync then.
        let work_offline = self.mail.session.prefs.work_offline;
        super::set_work_offline(work_offline);
        let resumed_online = self.was_work_offline && !work_offline;
        self.was_work_offline = work_offline;
        if resumed_online {
            // Turning Work offline off restarts each account worker, whose connection path
            // already performs a full sync. Treat that as the pending catch-up pass.
            self.resume_sync_pending = false;
            self.network_sync_pending = false;
            self.last_resume_sync = Some(Instant::now());
        }
        let sync_for_network = self.network_sync_pending && self.network_available == Some(true);
        if (self.resume_sync_pending || sync_for_network) && sync_due && !work_offline && self.network_available != Some(false) {
            let accounts = self.mail.sync_now();
            self.last_resume_sync = Some(Instant::now());
            self.resume_sync_pending = false;
            if self.network_available == Some(true) {
                self.network_sync_pending = false;
            }
            if accounts > 0 {
                let reason = if sync_for_network { "network restored" } else { "app gained focus" };
                log::info!("Android {reason}; queued sync for {accounts} account(s)");
            }
        }
        let insets_poll = if ctx.egui_wants_keyboard_input() {
            if self.system_insets.is_some_and(|insets| insets.ime_visible) {
                Duration::from_millis(250)
            } else if self.system_insets.is_some_and(|insets| !insets.ime_supported) {
                Duration::from_millis(500)
            } else {
                Duration::from_millis(100)
            }
        } else {
            // At rest, refresh alongside the foreground network check instead of waking the
            // renderer on a separate, shorter cadence just to poll unchanged window insets.
            Duration::from_secs(15)
        };
        let elapsed = self.insets_checked_at.elapsed();
        if elapsed >= insets_poll {
            self.system_insets = platform::system_insets(&self.activity);
            self.insets_checked_at = Instant::now();
        }
        ctx.request_repaint_after(insets_poll.saturating_sub(self.insets_checked_at.elapsed()));
        loop {
            match self.handoff_rx.try_recv() {
                Ok(Ok(AndroidOpen::Link(link))) => {
                    self.handoff_remaining = self.handoff_remaining.saturating_sub(1);
                    self.mail.open_handed(&link);
                }
                Ok(Ok(AndroidOpen::Notification(target))) => {
                    self.handoff_remaining = self.handoff_remaining.saturating_sub(1);
                    self.mail.open_notification_target(target);
                }
                Ok(Ok(AndroidOpen::File { path, last_in_batch })) => {
                    self.handoff_remaining = self.handoff_remaining.saturating_sub(1);
                    match self.mail.receive_shared_picker_file(std::path::Path::new(&path), last_in_batch) {
                        Ok(true) => {}
                        Ok(false) => self.mail.open_handed_from_picker(&path),
                        Err(error) => self.mail.session.notice(error, true),
                    }
                }
                Ok(Ok(AndroidOpen::Attachment { path, last_in_batch })) => {
                    self.handoff_remaining = self.handoff_remaining.saturating_sub(1);
                    match self.mail.receive_shared_picker_file(std::path::Path::new(&path), last_in_batch) {
                        Ok(true) => {}
                        Ok(false) => self.mail.open_shared_attachment_from_picker(&path),
                        Err(error) => self.mail.session.notice(error, true),
                    }
                }
                Ok(Ok(AndroidOpen::SharedText { subject, body, to, cc, bcc })) => {
                    self.handoff_remaining = self.handoff_remaining.saturating_sub(1);
                    self.mail.open_shared_text(&subject, &body, &to, &cc, &bcc);
                }
                Ok(Err(error)) => {
                    // A failed file stops the batch worker, so later documents won't produce
                    // completion messages. End the handoff instead of leaving the UI waiting.
                    self.handoff_remaining = 0;
                    self.mail.services.cancel_pending_pick();
                    self.mail.session.notice(error, true);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.handoff_remaining = 0;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if self.handoff_remaining > 0 {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let control_v = ctx.input(|input| input.key_pressed(egui::Key::V) && (input.modifiers.ctrl || input.modifiers.command));
        // A native Paste event already carries the clipboard text, so only fetch the full text
        // when the key press arrived without it. A quick MIME check keeps image pastes intact.
        let native_text_paste = ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Paste(_))));
        let text_edit_focused = ctx.text_edit_focused();
        if text_edit_focused && control_v && native_text_paste {
            match platform::clipboard_has_image(&self.activity) {
                Ok(true) => ctx.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Paste(_)))),
                Ok(false) => {}
                Err(error) => log::debug!("MailCraft couldn't check Android's clipboard image: {error}"),
            }
        }
        let needs_clipboard_read = text_edit_focused && control_v && !native_text_paste;
        if needs_clipboard_read {
            match platform::clipboard_text(&self.activity) {
                Ok(platform::ClipboardText::Text(text)) => ctx.input_mut(|input| input.events.push(egui::Event::Paste(text))),
                Ok(platform::ClipboardText::Image) if control_v => {
                    ctx.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Paste(_))))
                }
                Ok(platform::ClipboardText::Image) => {}
                Ok(platform::ClipboardText::TooLarge) => {
                    ctx.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Paste(_))));
                    self.mail.session.notice(mailcraft_ui_egui::i18n::t("The Android clipboard text is too large to paste.").to_owned(), true);
                    ctx.request_repaint();
                }
                Ok(platform::ClipboardText::Empty) => {}
                Err(error) => log::debug!("MailCraft couldn't read text from Android's clipboard: {error}"),
            }
        }
        if activity::take_system_back() {
            ctx.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: egui::Key::BrowserBack,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            });
        }
        self.mail.logic(ctx);
        if std::mem::take(&mut self.cache_warning) {
            self.mail
                .session
                .notice(mailcraft_ui_egui::i18n::t("MailCraft couldn't open its data files, so it runs without its mail cache this time."), true);
            ctx.request_repaint();
        }
        self.state_writer.save_if_due(&self.mail);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let pixels_per_point = ui.ctx().pixels_per_point().max(1.0);
        let status_color = self.mail.ui.tokens.bar;
        let navigation_color = self.mail.ui.tokens.chrome;
        let insets = self.system_insets.unwrap_or(platform::SystemInsets { top: 28, ..Default::default() });
        let top = insets.top.max(0) as f32 / pixels_per_point;
        egui::Panel::top("android_status_bar_inset").frame(egui::Frame::new().fill(status_color)).show(ui, |ui| ui.set_height(top));
        let bottom = insets.bottom.max(0) as f32 / pixels_per_point;
        if bottom > 0.0 {
            egui::Panel::bottom("android_navigation_bar_inset").frame(egui::Frame::new().fill(navigation_color)).show(ui, |ui| ui.set_height(bottom));
        }
        if insets.left > 0 {
            egui::Panel::left("android_left_system_inset").show(ui, |ui| ui.set_width(insets.left as f32 / pixels_per_point));
        }
        if insets.right > 0 {
            egui::Panel::right("android_right_system_inset").show(ui, |ui| ui.set_width(insets.right as f32 / pixels_per_point));
        }
        self.mail.ui(ui);
        let bar_colors = (self.mail.ui.tokens.bar, self.mail.ui.tokens.chrome);
        if self.system_bar_colors != Some(bar_colors) {
            self.system_bar_colors = Some(bar_colors);
            if let Err(error) = platform::system_bar_appearance(&self.activity, bar_colors.0, bar_colors.1) {
                log::debug!("MailCraft could not update Android's system bar appearance: {error}");
            }
        }
        if self.restore_checkpointed_draft() {
            ctx.request_repaint();
        } else if self.pending_draft_restore.is_some() {
            // A just-checkpointed server draft can still be moving from the worker's durable
            // pending queue into the cached Drafts folder. Retry after the next sync events.
            ctx.request_repaint_after(Duration::from_secs(15));
        }
        super::set_work_offline(self.mail.session.prefs.work_offline);
        // egui-winit keeps an in-app clipboard on Android for keyboard paste; leave its command
        // intact while also copying the same text to Android's system clipboard.
        let copied_text = ctx.output(|output| {
            let clipboard_command =
                output.commands.iter().rposition(|command| matches!(command, egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_)));
            let command = clipboard_command.and_then(|index| output.commands.get(index));
            match command {
                Some(egui::OutputCommand::CopyText(text)) if text.len() <= 4 << 20 => Some(Ok(text.clone())),
                Some(egui::OutputCommand::CopyText(_)) => Some(Err(())),
                _ => None,
            }
        });
        if let Some(copied_text) = copied_text {
            let notice = match copied_text {
                Err(()) => Some(mailcraft_ui_egui::i18n::t("The selected text is too large for Android's clipboard")),
                Ok(text) => platform::copy_text(&self.activity, &text).map_or_else(
                    |error| {
                        log::debug!("MailCraft could not copy text to Android's clipboard: {error}");
                        Some(mailcraft_ui_egui::i18n::t("MailCraft couldn't copy text to Android's clipboard."))
                    },
                    |_| None,
                ),
            };
            if let Some(notice) = notice {
                self.mail.session.notice(notice.to_owned(), true);
                ctx.request_repaint();
            }
        }
    }

    fn on_exit(&mut self) {
        self.handoff_route.deactivate();
        self.mail.checkpoint_note_edit();
        let saved = self.mail.checkpoint_drafts();
        if saved > 0 {
            log::info!("Android is closing; checkpointed {saved} draft(s)");
        }
        if !self.mail.flush_changes(Duration::from_secs(2)) {
            log::warn!("Android closed before all queued offline mail changes reached the cache");
        }
        self.state_writer.close(&self.mail);
        if let Err(error) = self.mail.shutdown() {
            eprintln!("MailCraft could not seal its offline cache: {error}");
        }
    }
}
