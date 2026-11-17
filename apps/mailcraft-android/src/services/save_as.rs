//! Saving files to a user-selected Android document-provider URI.

use super::*;

fn save_error() -> String {
    mailcraft_ui_egui::i18n::t("MailCraft couldn't save the file. Try again.").to_owned()
}

impl AndroidServices {
    pub(crate) fn queue_save_as(&mut self, name: &str, mime: &str, bytes: Vec<u8>) {
        let byte_len = bytes.len() as u64;
        if byte_len > MAX_QUEUED_ANDROID_DOWNLOAD_BYTES || !self.download_queue_has_room(byte_len) {
            self.download_queue_full();
            return;
        }
        self.download_queue_bytes = self.download_queue_bytes.saturating_add(byte_len);
        let name = mailcraft_engine::mime::sanitize_filename(name);
        let mime = mime.chars().take(256).collect();
        self.download_queue.push_back(AndroidDownload::Bytes { name, mime_type: Some(mime), bytes, open_after_saving: false, save_as: true });
        self.start_download();
    }

    pub(crate) fn save_to_user_location(&mut self, name: String, mime: Option<String>, bytes: Vec<u8>) {
        self.download_active = true;
        let name = safe_download_name(&name);
        let mime = saved_file_mime_type(mime.as_deref(), &name);
        let request_id = activity::next_document_request_id();
        self.pending_saves.insert(request_id, PendingSave { name: name.clone(), mime: mime.clone(), bytes });
        if let Err(error) = activity::create_document(&self.activity, request_id, &name, &mime) {
            self.pending_saves.remove(&request_id);
            log::warn!("Android save dialog could not start: {error}");
            let _ = self.errors.send(save_error());
            let _ = self.download_finished.send(());
            self.repaint.request_repaint();
        }
    }

    pub(crate) fn finish_user_save(&mut self, result: activity::DocumentResult) {
        let request_id = match &result {
            activity::DocumentResult::Selected { request_id, .. }
            | activity::DocumentResult::SelectedMany { request_id, .. }
            | activity::DocumentResult::Cancelled { request_id }
            | activity::DocumentResult::Failed { request_id, .. } => *request_id,
        };
        let Some(pending) = self.pending_saves.remove(&request_id) else { return };
        match result {
            activity::DocumentResult::Selected { uri, .. } => {
                let (activity, saved, errors, finished, repaint) =
                    (self.activity.clone(), self.downloaded.clone(), self.errors.clone(), self.download_finished.clone(), self.repaint.clone());
                let name = pending.name;
                let mime = pending.mime;
                let worker = std::thread::Builder::new().name("android-save-as".into()).spawn(move || {
                    match platform::write_bytes_to_uri(&activity, &uri, &pending.bytes) {
                        Ok(()) => {
                            let _ = saved.send(name);
                        }
                        Err(error) => {
                            log::warn!("Android could not save {mime} through the document provider: {error}");
                            let _ = errors.send(save_error());
                        }
                    }
                    let _ = finished.send(());
                    repaint.request_repaint();
                });
                if let Err(error) = worker {
                    log::warn!("Android save work could not start: {error}");
                    let _ = self.errors.send(save_error());
                    let _ = self.download_finished.send(());
                    self.repaint.request_repaint();
                }
            }
            activity::DocumentResult::SelectedMany { .. } => {
                let _ = self.errors.send(save_error());
                let _ = self.download_finished.send(());
                self.repaint.request_repaint();
            }
            activity::DocumentResult::Cancelled { .. } => {
                let _ = self.download_finished.send(());
                self.repaint.request_repaint();
            }
            activity::DocumentResult::Failed { error, .. } => {
                log::warn!("Android save dialog failed: {error}");
                let _ = self.errors.send(save_error());
                let _ = self.download_finished.send(());
                self.repaint.request_repaint();
            }
        }
    }
}
