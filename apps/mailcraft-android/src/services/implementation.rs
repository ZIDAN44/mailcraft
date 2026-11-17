use super::handoff::discard_staged_android_file;
use super::*;

#[cfg(target_os = "android")]
impl AndroidServices {
    fn process_document_results(&mut self) {
        let results: Vec<_> = self.document_results_rx.try_iter().collect();
        for result in results {
            let request_id = match &result {
                activity::DocumentResult::Selected { request_id, .. }
                | activity::DocumentResult::SelectedMany { request_id, .. }
                | activity::DocumentResult::Cancelled { request_id }
                | activity::DocumentResult::Failed { request_id, .. } => *request_id,
            };
            if self.active_document_picker.as_ref().is_some_and(|(active, _)| *active == request_id) {
                let Some((_, purpose)) = self.active_document_picker.take() else { continue };
                match result {
                    activity::DocumentResult::Selected { uri, name, mime, size, .. } => {
                        self.stage_picked_documents(purpose, vec![activity::PickedDocument { uri, name, mime, size }], false);
                    }
                    activity::DocumentResult::SelectedMany { documents, .. } => {
                        let is_batch = documents.len() > 1;
                        self.stage_picked_documents(purpose, documents, is_batch);
                    }
                    activity::DocumentResult::Cancelled { .. } => clear_picker_purpose(&self.pending_picker_purpose, &purpose),
                    activity::DocumentResult::Failed { error, .. } => {
                        clear_picker_purpose(&self.pending_picker_purpose, &purpose);
                        let _ = self.errors.send(picker_fallback());
                        log::warn!("Android document picker failed: {error}");
                        self.repaint.request_repaint();
                    }
                }
            } else {
                self.finish_user_save(result);
            }
        }
    }

    fn stage_picked_documents(&self, purpose: String, documents: Vec<activity::PickedDocument>, is_batch: bool) {
        if is_batch && purpose != "attach" {
            clear_picker_purpose(&self.pending_picker_purpose, &purpose);
            let _ = self.errors.send(picker_fallback());
            self.repaint.request_repaint();
            return;
        }
        let (worker_activity, worker_dir, worker_purpose) = (self.activity.clone(), self.incoming_dir.clone(), purpose.clone());
        let (worker_picked, worker_errors, worker_pending, worker_repaint) =
            (self.picked.clone(), self.errors.clone(), self.pending_picker_purpose.clone(), self.repaint.clone());
        let worker = std::thread::Builder::new().name("android-picked-document".into()).spawn(move || {
            let mut staged = Vec::with_capacity(documents.len());
            let mut total_bytes = 0_u64;
            let result = (|| -> Result<(), String> {
                for document in documents {
                    let (name_or_path, data) = read_picked_file(
                        &worker_activity,
                        worker_dir.as_deref(),
                        &worker_purpose,
                        matches!(worker_purpose.as_str(), "picture" | "signature-picture"),
                        &document.uri,
                        document.name.as_deref(),
                        document.mime.as_deref(),
                        document.size,
                    )?;
                    if worker_purpose == "attach" {
                        let path = std::path::Path::new(&name_or_path);
                        let size = match std::fs::metadata(path) {
                            Ok(metadata) => metadata.len(),
                            Err(error) => {
                                discard_staged_android_file(path);
                                return Err(error.to_string());
                            }
                        };
                        total_bytes = match total_bytes.checked_add(size).filter(|total| *total <= mailcraft_engine::MAX_ATTACHMENT_BYTES as u64) {
                            Some(total) => total,
                            None => {
                                discard_staged_android_file(path);
                                return Err(format!("Attachments are limited to {} MiB total", mailcraft_engine::MAX_ATTACHMENT_BYTES >> 20));
                            }
                        };
                    }
                    staged.push((name_or_path, data));
                }
                Ok(())
            })();
            if let Err(error) = result {
                for (path, _) in staged {
                    discard_staged_android_file(std::path::Path::new(&path));
                }
                clear_picker_purpose(&worker_pending, &worker_purpose);
                let _ = worker_errors.send(error);
                worker_repaint.request_repaint();
                return;
            }
            let staged_file = matches!(worker_purpose.as_str(), "data-file" | "account-data-file" | "attach");
            let purpose_for_ui = if worker_purpose == "attach" {
                if is_batch { "attach-path-batch" } else { "attach-path" }.to_owned()
            } else {
                worker_purpose.clone()
            };
            let mut staged_iter = staged.into_iter();
            while let Some((name_or_path, data)) = staged_iter.next() {
                if worker_picked.send((purpose_for_ui.clone(), name_or_path.clone(), data)).is_err() {
                    if staged_file {
                        discard_staged_android_file(std::path::Path::new(&name_or_path));
                        for (path, _) in staged_iter {
                            discard_staged_android_file(std::path::Path::new(&path));
                        }
                    }
                    break;
                }
            }
            clear_picker_purpose(&worker_pending, &worker_purpose);
            worker_repaint.request_repaint();
        });
        if let Err(error) = worker {
            clear_picker_purpose(&self.pending_picker_purpose, &purpose);
            log::warn!("Android selected-file work could not start: {error}");
            let _ = self.errors.send(picker_fallback());
            self.repaint.request_repaint();
        }
    }
}

#[cfg(target_os = "android")]
impl Services for AndroidServices {
    fn on_resume(&mut self) {
        self.refresh_display_configuration();
        self.notification_retry_at = None;
        if let Ok(mut zone) = self.zone.lock() {
            zone.checked_at = None;
        }
        if let Ok(mut dark) = self.dark.lock() {
            dark.checked_at = None;
        }
    }

    fn refresh_display_configuration(&mut self) {
        self.phone_form_factor = platform::phone_form_factor(&self.activity);
        self.font_scale = platform::font_scale(&self.activity);
        self.system_language = platform::language_tag(&self.activity);
    }

    fn phone_form_factor(&self) -> Option<bool> {
        self.phone_form_factor
    }

    fn system_font_scale(&self) -> Option<f32> {
        self.font_scale
    }

    fn system_language(&self) -> Option<&str> {
        self.system_language.as_deref()
    }

    fn language_changed(&mut self) {
        let language = mailcraft_ui_egui::i18n::current().code().to_owned();
        if self.notification_language.as_deref() == Some(language.as_str()) {
            return;
        }
        if self.notification_retry_at.is_some_and(|at| at > Instant::now()) {
            return;
        }
        match notifications::refresh_channels(&self.activity) {
            Ok(()) => {
                self.notification_language = Some(language);
                self.notification_retry_at = None;
            }
            Err(error) => {
                self.notification_retry_at = Instant::now().checked_add(std::time::Duration::from_secs(30));
                log::warn!("Android notification categories could not be refreshed: {error}");
            }
        }
    }

    fn can_open_notification_settings(&self) -> bool {
        true
    }

    fn open_notification_settings(&mut self) -> Result<(), String> {
        platform::open_notification_settings(&self.activity)
    }

    fn event(&mut self, event: &Event) {
        let Event::Exported { account, done: true, error, .. } = event else { return };
        let Some(queue) = self.pending_exports.get_mut(account) else { return };
        let Some((name, path)) = queue.pop_front() else { return };
        if queue.is_empty() {
            self.pending_exports.remove(account);
        }
        if error.is_some() {
            discard_android_export(&path);
        } else {
            self.queue_download_file(name, path);
        }
    }

    fn background(&mut self, lane: mailcraft_ui_egui::Lane, work: mailcraft_ui_egui::Work) -> Result<(), mailcraft_ui_egui::Work> {
        self.lanes.submit(lane, work)
    }

    fn has_background(&self) -> bool {
        self.lanes.available()
    }

    fn notify(&mut self, title: &str, body: &str) {
        self.notify_for(title, body, mailcraft_ui_egui::NotificationTarget::Inbox);
    }

    fn notify_for(&mut self, title: &str, body: &str, target: mailcraft_ui_egui::NotificationTarget) {
        self.notifications.push(title, body, target);
        self.notifications.try_show(&self.activity, &self.repaint, &self.errors);
    }

    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }

    fn zone(&self) -> Option<String> {
        self.cached_zone_name()
    }

    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        platform::clipboard_picture(&self.activity)
    }

    fn clipboard_picture_later(&mut self) -> Option<bool> {
        use std::sync::atomic::Ordering;

        let pending = &self.clipboard_picture_pending;
        if pending.load(Ordering::Acquire) {
            return Some(true);
        }
        match platform::clipboard_has_image(&self.activity) {
            Ok(false) => return Some(false),
            Err(error) => {
                log::debug!("Android clipboard image could not be checked: {error}");
                return None;
            }
            Ok(true) => {}
        }
        if pending.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Some(true);
        }
        let (activity, sender, repaint, pending) = (self.activity.clone(), self.clipboard_picture.clone(), self.repaint.clone(), pending.clone());
        let worker = std::thread::Builder::new().name("mailcraft-clipboard-picture".into()).spawn(move || {
            let picture = platform::clipboard_picture(&activity);
            pending.store(false, Ordering::Release);
            let _ = sender.send(picture);
            repaint.request_repaint();
        });
        if let Err(error) = worker {
            self.clipboard_picture_pending.store(false, Ordering::Release);
            log::warn!("Android clipboard image work could not start: {error}");
            return None;
        }
        Some(true)
    }

    fn take_clipboard_picture(&mut self) -> Option<Option<Vec<u8>>> {
        self.clipboard_picture_rx.try_recv().ok()
    }

    fn clipboard_files(&mut self) -> Vec<std::path::PathBuf> {
        let Some(directory) = self.incoming_dir.as_deref() else { return Vec::new() };
        platform::clipboard_files(&self.activity, directory)
    }

    fn clipboard_files_later(&mut self) -> Option<bool> {
        use std::sync::atomic::Ordering;

        let pending = &self.clipboard_files_pending;
        if pending.load(Ordering::Acquire) {
            return Some(true);
        }
        let directory = self.incoming_dir.clone()?;
        match platform::clipboard_has_files(&self.activity) {
            Ok(false) => return Some(false),
            Err(error) => {
                log::warn!("Android clipboard could not be checked: {error}");
                return Some(false);
            }
            Ok(true) => {}
        }
        if pending.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Some(true);
        }
        let (activity, sender, repaint, pending) = (self.activity.clone(), self.clipboard_files.clone(), self.repaint.clone(), pending.clone());
        let worker = std::thread::Builder::new().name("mailcraft-clipboard-files".into()).spawn(move || {
            let files = platform::clipboard_files(&activity, &directory);
            pending.store(false, Ordering::Release);
            if let Err(error) = sender.send(Ok(files))
                && let Ok(files) = error.0
            {
                platform::discard_clipboard_files(&directory, &files);
            }
            repaint.request_repaint();
        });
        if let Err(error) = worker {
            self.clipboard_files_pending.store(false, Ordering::Release);
            log::warn!("Android clipboard file copy could not start: {error}");
            return Some(false);
        }
        Some(true)
    }

    fn take_clipboard_files(&mut self) -> Option<Result<Vec<std::path::PathBuf>, String>> {
        self.clipboard_files_rx.try_recv().ok()
    }

    fn video_frame(&mut self, path: &std::path::Path) -> mailcraft_ui_egui::VideoFrame {
        self.video_frame_impl(path)
    }

    fn system_dark(&self) -> Option<bool> {
        self.cached_dark_mode()
    }

    fn cjk_font(&mut self, lang: &str) -> Option<(Vec<u8>, u32)> {
        platform::cjk_font(lang)
    }

    fn rtl_fonts(&mut self) -> Vec<Vec<u8>> {
        platform::rtl_fonts()
    }

    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn save_path(&mut self, _suggested: &str) -> Option<std::path::PathBuf> {
        None
    }

    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        platform::open_file(&self.activity, path)
    }

    fn can_open_path(&self) -> bool {
        false
    }

    fn open_url(&mut self, url: &str) -> Result<(), String> {
        platform::open_url(&self.activity, url)
    }

    fn can_dial_phone(&self) -> bool {
        true
    }

    fn dial_phone(&mut self, number: &str) -> Result<(), String> {
        platform::dial_phone(&self.activity, number)
    }

    fn can_open_map(&self) -> bool {
        true
    }

    fn open_map(&mut self, address: &str) -> Result<(), String> {
        platform::open_map(&self.activity, address)
    }

    fn can_share_contacts(&self) -> bool {
        true
    }

    fn can_share_files(&self) -> bool {
        true
    }

    fn can_export_contacts(&self) -> bool {
        true
    }

    fn can_add_to_system_contacts(&self) -> bool {
        true
    }

    fn can_share_calendar_events(&self) -> bool {
        true
    }

    fn can_export_calendar_events(&self) -> bool {
        true
    }

    fn can_add_to_system_calendar(&self) -> bool {
        true
    }

    fn share_contact(&mut self, title: &str, vcard: &str) -> Result<(), String> {
        platform::share_contact(&self.activity, title, vcard)
    }

    fn share_file(&mut self, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        platform::share_file(&self.activity, name, mime, bytes)
    }

    fn share_message(&mut self, name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        platform::share_message(&self.activity, name, mime, bytes)
    }

    fn share_files(&mut self, files: &[(&str, &str, &[u8])]) -> Result<(), String> {
        platform::share_files(&self.activity, files)
    }

    fn add_to_system_contacts(&mut self, contact: &mailcraft_engine::model::vcard::Card) -> Result<(), String> {
        platform::add_to_system_contacts(&self.activity, contact)
    }

    fn share_calendar_event(&mut self, title: &str, ical: &str) -> Result<(), String> {
        platform::share_calendar_event(&self.activity, title, ical)
    }

    fn add_to_system_calendar(
        &mut self,
        title: &str,
        location: &str,
        description: &str,
        start: i64,
        end: i64,
        all_day: bool,
        timezone: Option<&str>,
        rrule: Option<&str>,
        rdates: &[i64],
        exdates: &[i64],
        attendees: &[String],
    ) -> Result<(), String> {
        platform::add_to_system_calendar(
            &self.activity,
            title,
            location,
            description,
            start,
            end,
            all_day,
            timezone,
            rrule,
            rdates,
            exdates,
            attendees,
        )
    }

    fn back_key(&self) -> Option<egui::Key> {
        Some(egui::Key::BrowserBack)
    }

    fn pick_later(&mut self, purpose: &str) -> bool {
        let purpose = purpose.to_string();
        let pending_picker_purpose = self.pending_picker_purpose.clone();
        {
            let mut pending = lock_picker_purpose(&pending_picker_purpose);
            if pending.as_ref().is_some_and(|pending| pending.started.elapsed() <= PENDING_PICKER_MAX_AGE) {
                drop(pending);
                return true;
            }
            *pending = None;
            *pending = Some(PendingPicker {
                purpose: purpose.clone(),
                started: Instant::now(),
                shared_batch_open: false,
                shared_batch_rejected: false,
                shared_bytes: 0,
            });
        }
        let pick_image = matches!(purpose.as_str(), "picture" | "signature-picture");
        let request_id = activity::next_document_request_id();
        self.active_document_picker = Some((request_id, purpose.clone()));
        if let Err(error) =
            activity::open_document_picker(&self.activity, request_id, if pick_image { "image/*" } else { "*/*" }, purpose == "attach")
        {
            self.active_document_picker = None;
            clear_picker_purpose(&self.pending_picker_purpose, &purpose);
            log::warn!("Android document picker could not start: {error}");
            let _ = self.errors.send(picker_fallback());
            self.repaint.request_repaint();
        }
        true
    }

    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        self.process_document_results();
        let picked: Vec<_> = self.picked_rx.try_iter().collect();
        for (purpose, name, _) in &picked {
            let pending = lock_picker_purpose(&self.pending_picker_purpose);
            let shared_batch_open = pending.as_ref().is_some_and(|pending| pending.purpose == *purpose && pending.shared_batch_open);
            drop(pending);
            if !shared_batch_open {
                clear_picker_purpose(&self.pending_picker_purpose, purpose);
            }
            let is_data_archive = std::path::Path::new(name)
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "pst" | "mailcraft"));
            if purpose == "account-data-file" || (purpose == "data-file" && is_data_archive) {
                self.track_picked_file(std::path::Path::new(name));
            }
        }
        picked
    }

    fn receive_shared_picker_file(&mut self, path: &std::path::Path, last_in_batch: bool) -> Result<bool, String> {
        let Some(pending) = lock_picker_purpose(&self.pending_picker_purpose).clone() else { return Ok(false) };
        if pending.started.elapsed() > PENDING_PICKER_MAX_AGE {
            clear_picker_purpose(&self.pending_picker_purpose, &pending.purpose);
            return Ok(false);
        }
        if pending.shared_batch_rejected {
            self.discard_picked_file(path);
            if last_in_batch {
                clear_picker_purpose(&self.pending_picker_purpose, &pending.purpose);
            }
            return Ok(true);
        }
        let purpose = pending.purpose;
        macro_rules! reject_shared_batch {
            ($error:expr) => {{
                self.discard_picked_file(path);
                if last_in_batch {
                    clear_picker_purpose(&self.pending_picker_purpose, &purpose);
                } else if let Ok(mut pending_picker) = self.pending_picker_purpose.lock()
                    && let Some(pending_picker) = pending_picker.as_mut().filter(|pending| pending.purpose == purpose)
                {
                    pending_picker.shared_batch_rejected = true;
                }
                return Err($error);
            }};
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().chars().take(512).collect::<String>())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "attachment".into());
        let mime_type = mime_guess::from_path(&name).first_or_octet_stream().to_string();
        let max_bytes = match picker_file_limit(&purpose, &name, &mime_type) {
            Ok(limit) => limit,
            Err(error) => reject_shared_batch!(error),
        };
        let data_file = matches!(purpose.as_str(), "data-file" | "account-data-file");
        let staged_attachment = purpose == "attach";
        let (data, source_bytes) = if data_file || staged_attachment {
            let length = match std::fs::metadata(path) {
                Ok(metadata) => metadata.len(),
                Err(error) => reject_shared_batch!(error.to_string()),
            };
            if length > max_bytes {
                reject_shared_batch!(format!("The selected file is larger than {} MiB", max_bytes >> 20));
            }
            (Vec::new(), length)
        } else {
            let file = match std::fs::File::open(path) {
                Ok(file) => file,
                Err(error) => reject_shared_batch!(error.to_string()),
            };
            let mut reader = std::io::Read::take(file, max_bytes.saturating_add(1));
            let mut bytes = Vec::new();
            if let Err(error) = std::io::Read::read_to_end(&mut reader, &mut bytes) {
                reject_shared_batch!(error.to_string());
            }
            if bytes.len() as u64 > max_bytes {
                reject_shared_batch!(format!("The selected file is larger than {} MiB", max_bytes >> 20));
            }
            let length = bytes.len() as u64;
            (bytes, length)
        };
        if !data_file && !staged_attachment {
            self.discard_picked_file(path);
        }
        let next_shared_bytes = pending.shared_bytes.checked_add(if staged_attachment { source_bytes } else { 0 });
        if purpose == "attach" && next_shared_bytes.is_none_or(|total| total > mailcraft_engine::MAX_ATTACHMENT_BYTES as u64) {
            reject_shared_batch!(format!(
                "Attachments shared to MailCraft are limited to {} MiB total",
                mailcraft_engine::MAX_ATTACHMENT_BYTES >> 20
            ));
        }
        let picked_name = if data_file || staged_attachment { path.to_string_lossy().into_owned() } else { name };
        let picked_purpose = if staged_attachment { "attach-shared-path".to_owned() } else { purpose.clone() };
        if self.picked.send((picked_purpose, picked_name, data)).is_err() {
            reject_shared_batch!("MailCraft closed before it could receive the shared file".to_string());
        }
        if last_in_batch {
            clear_picker_purpose(&self.pending_picker_purpose, &purpose);
            self.active_document_picker = None;
        } else if let Ok(mut pending_picker) = self.pending_picker_purpose.lock()
            && let Some(pending_picker) = pending_picker.as_mut().filter(|pending| pending.purpose == purpose)
        {
            pending_picker.shared_batch_open = true;
            pending_picker.shared_bytes = next_shared_bytes.unwrap_or(pending_picker.shared_bytes);
        }
        self.repaint.request_repaint();
        Ok(true)
    }

    fn take_pick_errors(&mut self) -> Vec<String> {
        self.errors_rx.try_iter().collect()
    }

    fn cancel_pending_pick(&mut self) {
        *lock_picker_purpose(&self.pending_picker_purpose) = None;
        self.active_document_picker = None;
    }

    fn discard_picked_file(&mut self, path: &std::path::Path) {
        let Some(directory) = self.incoming_dir.as_deref().and_then(|directory| directory.canonicalize().ok()) else { return };
        let Ok(file) = path.canonicalize() else { return };
        let mut staging_roots = vec![directory.clone()];
        if let Some(shared) = self
            .incoming_dir
            .as_deref()
            .and_then(|directory| directory.join("shared-attachments").canonicalize().ok())
            .filter(|shared| shared.parent() == Some(directory.as_path()))
        {
            staging_roots.push(shared);
        }
        let Some(parent) = file.parent() else { return };
        if !staging_roots.iter().any(|root| parent.parent() == Some(root.as_path())) {
            return;
        }
        if let Err(error) = std::fs::remove_file(&file) {
            eprintln!("MailCraft could not remove a processed import file: {error}");
        } else if let Some(parent) = file.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }

    fn track_picked_file(&mut self, path: &std::path::Path) {
        if !self.staged_dialog_files.iter().any(|staged| staged == path) {
            self.staged_dialog_files.push(path.to_path_buf());
        }
    }

    fn keep_staged_files(&mut self, active: &[std::path::PathBuf]) {
        let mut index = 0;
        while index < self.staged_dialog_files.len() {
            let Some(path) = self.staged_dialog_files.get(index).cloned() else { break };
            if active.iter().any(|current| current == &path) {
                index += 1;
            } else {
                self.staged_dialog_files.remove(index);
                self.discard_picked_file(&path);
            }
        }
    }

    fn retain_picked_file(&mut self, path: &std::path::Path) {
        self.staged_dialog_files.retain(|staged| staged.as_path() != path);
    }

    fn take_downloaded(&mut self) -> Vec<String> {
        self.notifications.try_show(&self.activity, &self.repaint, &self.errors);
        let names = self.downloaded_rx.try_iter().collect();
        for (path, mime_type) in self.open_requests_rx.try_iter() {
            if let Err(error) = platform::open_file_with_mime(&self.activity, &path, &mime_type) {
                if let Err(remove_error) = std::fs::remove_file(&path) {
                    if remove_error.kind() != std::io::ErrorKind::NotFound {
                        log::debug!("MailCraft could not remove an attachment it failed to open: {remove_error}");
                    }
                }
                let _ = self.errors.send(error);
                self.repaint.request_repaint();
            } else {
                self.open_cache_bytes = self.open_cache_bytes.saturating_add(std::fs::metadata(&path).map_or(0, |file| file.len()));
                self.open_cache_files = self.open_cache_files.saturating_add(1);
            }
        }
        names
    }

    fn take_download_completions(&mut self) -> usize {
        let completed = self.download_finished_rx.try_iter().count();
        if completed > 0 {
            self.download_active = false;
            self.download_active_bytes = 0;
        }
        self.start_download();
        completed
    }

    fn downloads(&self) -> bool {
        true
    }

    fn has_files(&self) -> bool {
        true
    }

    fn opens_files_after_saving(&self) -> bool {
        true
    }

    fn download(&mut self, name: &str, bytes: &[u8]) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        if !self.download_queue_has_room(bytes.len() as u64) {
            self.download_queue_full();
            return;
        }
        self.queue_download_bytes(name, None, bytes.to_vec(), false);
    }

    fn download_owned(&mut self, name: &str, bytes: Vec<u8>) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        self.queue_download_bytes(name, None, bytes, false);
    }

    fn download_owned_with_mime(&mut self, name: &str, mime: &str, bytes: Vec<u8>) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        let mime = mime.chars().take(256).collect();
        self.queue_download_bytes(name, Some(mime), bytes, false);
    }

    fn save_as_owned(&mut self, name: &str, mime: &str, bytes: Vec<u8>) {
        self.queue_save_as(name, mime, bytes);
    }

    fn downloads_files(&self) -> bool {
        true
    }

    fn export_file(&mut self, account: &str, name: &str, path: std::path::PathBuf) -> Result<(), String> {
        if !self.download_queue_has_room(0) {
            return Err(
                mailcraft_ui_egui::i18n::t("MailCraft's save queue is full. Wait for current saves to finish before saving more files.").to_owned()
            );
        }
        let name = mailcraft_engine::mime::sanitize_filename(name);
        self.pending_exports.entry(account.to_owned()).or_default().push_back((name, path));
        Ok(())
    }

    fn open_file(&mut self, name: &str, bytes: &[u8]) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        if !self.download_queue_has_room(bytes.len() as u64) {
            self.download_queue_full();
            return;
        }
        self.queue_download_bytes(name, None, bytes.to_vec(), true);
    }

    fn open_file_owned(&mut self, name: &str, bytes: Vec<u8>) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        self.queue_download_bytes(name, None, bytes, true);
    }

    fn open_file_owned_with_mime(&mut self, name: &str, mime: &str, bytes: Vec<u8>) {
        let name = mailcraft_engine::mime::sanitize_filename(name);
        let mime = mime.chars().take(256).collect();
        self.queue_download_bytes(name, Some(mime), bytes, true);
    }
}
