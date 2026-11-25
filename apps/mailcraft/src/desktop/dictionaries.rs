//! Asynchronous Hunspell dictionary downloading for the desktop app: fetches `.aff` and `.dic`
//! off the UI thread, validates them with `craft-proof`, writes them atomically into
//! `<app_data_dir>/dictionaries/`, and notifies the UI thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mailcraft_ui_egui::{DictionaryDownload, DownloadedDictionary};

#[derive(Default)]
struct Inner {
    active_locale: Option<String>,
    error: Option<(String, String)>,
    ready: Vec<DownloadedDictionary>,
}

/// Manages background Hunspell dictionary downloads for the desktop window.
#[derive(Default)]
pub(crate) struct Downloader {
    dir: Option<PathBuf>,
    ctx: Option<egui::Context>,
    state: Arc<Mutex<Inner>>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
}

impl Downloader {
    pub(crate) fn new(dir: Option<PathBuf>, ctx: egui::Context) -> Self {
        Self {
            dir,
            ctx: Some(ctx),
            state: Arc::new(Mutex::new(Inner::default())),
            done: Arc::new(AtomicU64::new(0)),
            total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn has_dir(&self) -> bool {
        self.dir.is_some()
    }

    pub(crate) fn state(&self) -> DictionaryDownload {
        let g = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(locale) = &g.active_locale {
            return DictionaryDownload::Downloading {
                locale: locale.clone(),
                done: self.done.load(Ordering::Relaxed),
                total: self.total.load(Ordering::Relaxed),
            };
        }
        if let Some((locale, message)) = &g.error {
            return DictionaryDownload::Error { locale: locale.clone(), message: message.clone() };
        }
        DictionaryDownload::Idle
    }

    pub(crate) fn take_ready(&mut self) -> Vec<DownloadedDictionary> {
        let mut g = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut g.ready)
    }

    pub(crate) fn start(&mut self, locale: &str, aff_url: &str, dic_url: &str) {
        let Some(dir) = self.dir.clone() else {
            let mut g = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.error = Some((locale.to_string(), "no dictionaries folder configured".into()));
            return;
        };
        {
            let mut g = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if g.active_locale.is_some() {
                return;
            }
            g.active_locale = Some(locale.to_string());
            g.error = None;
        }
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);

        let fallback_locale = locale.to_string();
        let locale = fallback_locale.clone();
        let aff_url = aff_url.to_string();
        let dic_url = dic_url.to_string();
        let state = Arc::clone(&self.state);
        let done = Arc::clone(&self.done);
        let total = Arc::clone(&self.total);
        let ctx = self.ctx.clone();
        if !super::off_thread(std::thread::Builder::new().name("dict-download".into()), "dictionary download", move || {
            let outcome =
                mailcraft_net::dictionaries::fetch_pair(&aff_url, &dic_url, &done, &total).map_err(|e| e.to_string()).and_then(|(aff, dic)| {
                    mailcraft_ui_egui::validate_hunspell(&locale, &aff, &dic)?;
                    mailcraft_net::dictionaries::save_pair_atomic(&dir, &locale, &aff, &dic).map_err(|e| e.to_string())?;
                    Ok(())
                });
            {
                let mut g = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.active_locale = None;
                match outcome {
                    Ok(()) => g.ready.push(DownloadedDictionary { locale, files: None }),
                    Err(message) => g.error = Some((locale, message)),
                }
            }
            if let Some(c) = ctx {
                c.request_repaint();
            }
        }) {
            let mut g = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.active_locale = None;
            g.error = Some((fallback_locale, "couldn't start background download thread".into()));
        }
    }
}
