//! Bounded Android Downloads queue and streaming exports.

use super::*;

pub(crate) fn cleanup_stale_open_files() {
    const RETENTION: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let Some(directory) = mailcraft_engine::store::scratch_dir("open") else { return };
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(RETENTION) else { return };
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            log::debug!("MailCraft could not inspect its temporary attachment folder: {error}");
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale_regular_file = entry.file_type().is_ok_and(|kind| kind.is_file())
            && entry.metadata().ok().and_then(|metadata| metadata.modified().ok()).is_some_and(|modified| modified < cutoff);
        if stale_regular_file && let Err(error) = std::fs::remove_file(&path) {
            log::debug!("MailCraft could not remove an old temporary attachment: {error}");
        }
    }
}

pub(crate) fn open_cache_usage() -> (u64, usize) {
    let Some(directory) = mailcraft_engine::store::scratch_dir("open") else { return (0, 0) };
    std::fs::read_dir(directory).map_or((0, 0), |entries| {
        entries.flatten().fold((0_u64, 0_usize), |(bytes, count), entry| {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                return (bytes, count);
            }
            let length = entry.metadata().map_or(0, |metadata| metadata.len());
            (bytes.saturating_add(length), count.saturating_add(1))
        })
    })
}

enum DownloadResult {
    Open(std::path::PathBuf),
    Saved(String),
}

fn stage_file_for_open(name: &str, source_path: Option<&std::path::Path>, data: &[u8]) -> Result<std::path::PathBuf, String> {
    let directory = mailcraft_engine::store::scratch_dir("open")
        .ok_or_else(|| mailcraft_ui_egui::i18n::t("There's no folder to put the attachment in for another app").to_owned())?;
    let name = safe_download_name(name);
    let extension_start = name.rfind('.').filter(|index| *index > 0);
    let (stem, extension) = extension_start.map_or((name.as_str(), ""), |index| name.split_at(index));
    for suffix in 0..128_u16 {
        let candidate = if suffix == 0 { name.clone() } else { format!("{stem} ({suffix}){extension}") };
        let path = directory.join(candidate);
        let mut destination = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        let result = if let Some(source) = source_path {
            std::fs::File::open(source).and_then(|mut source| std::io::copy(&mut source, &mut destination).map(|_| ()))
        } else {
            std::io::Write::write_all(&mut destination, data)
        };
        if let Err(error) = result {
            let _ = std::fs::remove_file(&path);
            return Err(error.to_string());
        }
        return Ok(path);
    }
    Err(mailcraft_ui_egui::i18n::t("MailCraft couldn't save the file. Try again.").to_owned())
}

impl AndroidServices {
    pub(crate) fn start_download(&mut self) {
        if self.download_active {
            return;
        }
        if let Some(retry_at) = self.download_retry_at {
            let remaining = retry_at.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                self.repaint.request_repaint_after(remaining);
                return;
            }
            self.download_retry_at = None;
        }
        let Some(download) = self.download_queue.pop_front() else { return };
        let (name, mime_type, open_after_saving, source_path, data) = match download {
            AndroidDownload::Bytes { name, mime_type, bytes, open_after_saving, save_as } => {
                self.download_queue_bytes = self.download_queue_bytes.saturating_sub(bytes.len() as u64);
                self.download_active_bytes = bytes.len() as u64;
                if save_as {
                    self.save_to_user_location(name, mime_type, bytes);
                    return;
                }
                (name, mime_type, open_after_saving, None, bytes)
            }
            AndroidDownload::File { name, path } => (name, None, false, Some(path), Vec::new()),
        };
        let name = safe_download_name(&name);
        self.start_download_to_downloads(name, mime_type, open_after_saving, source_path, data);
    }

    pub(crate) fn download_queue_has_room(&self, bytes: u64) -> bool {
        let pending_exports = self.pending_exports.values().fold(0usize, |total, queue| total.saturating_add(queue.len()));
        let queued = self.download_queue.len().saturating_add(pending_exports).saturating_add(self.open_cache_files);
        if queued >= MAX_QUEUED_ANDROID_DOWNLOADS {
            return false;
        }
        self.download_active_bytes
            .checked_add(self.download_queue_bytes)
            .and_then(|total| total.checked_add(self.open_cache_bytes))
            .and_then(|total| total.checked_add(bytes))
            .is_some_and(|total| total <= MAX_QUEUED_ANDROID_DOWNLOAD_BYTES)
            || (!self.download_active && self.download_queue.is_empty())
    }

    pub(crate) fn download_queue_full(&mut self) {
        let _ = self.errors.send("MailCraft's save queue is full. Wait for current saves to finish before saving more files.".into());
        self.repaint.request_repaint();
    }

    pub(crate) fn queue_download_bytes(&mut self, name: String, mime_type: Option<String>, bytes: Vec<u8>, open_after_saving: bool) {
        let byte_len = bytes.len() as u64;
        if !self.download_queue_has_room(byte_len) {
            self.download_queue_full();
            return;
        }
        self.download_queue_bytes = self.download_queue_bytes.saturating_add(byte_len);
        self.download_queue.push_back(AndroidDownload::Bytes { name, mime_type, bytes, open_after_saving, save_as: false });
        self.start_download();
    }

    pub(crate) fn queue_download_file(&mut self, name: String, path: std::path::PathBuf) {
        if !self.download_queue_has_room(0) {
            discard_android_export(&path);
            self.download_queue_full();
            return;
        }
        self.download_queue.push_back(AndroidDownload::File { name, path });
        self.start_download();
    }

    fn start_download_to_downloads(
        &mut self,
        name: String,
        mime_type: Option<String>,
        open_after_saving: bool,
        source_path: Option<std::path::PathBuf>,
        data: Vec<u8>,
    ) {
        self.download_active = true;
        if source_path.is_some() {
            *lock_active_export(&self.active_export) = ActiveExport::Copying;
        }
        let active_export = self.active_export.clone();
        let (saved_tx, error_tx, finished_tx, open_tx) =
            (self.downloaded.clone(), self.errors.clone(), self.download_finished.clone(), self.open_requests.clone());
        let repaint = self.repaint.clone();
        let activity = self.activity.clone();
        let suggested = name.clone();
        let source_to_remove = source_path.clone();
        let worker_source_to_remove = source_to_remove.clone();
        let worker_active_export = active_export.clone();
        let worker = std::thread::Builder::new().name("android-download".into()).spawn(move || {
            let mime_type = saved_file_mime_type(mime_type.as_deref(), &suggested);
            let result = if open_after_saving {
                stage_file_for_open(&suggested, worker_source_to_remove.as_deref(), &data).map(DownloadResult::Open)
            } else {
                platform::create_download_uri(&activity, &suggested, &mime_type).and_then(|uri| {
                    let write_result = if let Some(path) = &worker_source_to_remove {
                        platform::write_file_to_uri(&activity, &uri, path)
                    } else {
                        platform::write_bytes_to_uri(&activity, &uri, &data)
                    };
                    let publish_result = write_result.and_then(|()| platform::publish_download_uri(&activity, &uri));
                    match publish_result {
                        Ok(()) => Ok(DownloadResult::Saved(uri)),
                        Err(error) => {
                            platform::delete_uri(&activity, &uri);
                            Err(error)
                        }
                    }
                })
            };
            match result {
                Ok(DownloadResult::Open(path)) => {
                    let _ = open_tx.send((path, mime_type));
                }
                Ok(DownloadResult::Saved(_uri)) => {
                    let _ = saved_tx.send(suggested.clone());
                }
                Err(error) => {
                    let _ = error_tx.send(error);
                }
            }
            if let Some(path) = &worker_source_to_remove {
                discard_android_export(path);
            }
            *lock_active_export(&worker_active_export) = ActiveExport::None;
            let _ = finished_tx.send(());
            repaint.request_repaint();
        });
        if let Err(error) = worker {
            if let Some(path) = source_to_remove.as_ref() {
                discard_android_export(path);
            }
            *lock_active_export(&active_export) = ActiveExport::None;
            log::warn!("Android download worker could not start: {error}");
            let _ = self.errors.send(mailcraft_ui_egui::i18n::t("MailCraft couldn't save the file. Try again.").to_owned());
            let _ = self.download_finished.send(());
            self.repaint.request_repaint();
        }
    }
}
