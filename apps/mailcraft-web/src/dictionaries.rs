//! Browser Hunspell dictionary downloader (`fetch`) and IndexedDB cache (`mailcraft-dictionaries`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use mailcraft_jmap::Http as _;
use mailcraft_ui_egui::{DictionaryDownload, DownloadedDictionary};
use web_sys::IdbDatabase;

const DB_NAME: &str = "mailcraft-dictionaries";
const STORE_NAME: &str = "dictionaries";
/// Maximum size per Hunspell file fetched in the browser (32 MiB).
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct WebDictionaries {
    state: DictionaryDownload,
    ready: Vec<DownloadedDictionary>,
    db: Option<IdbDatabase>,
}

/// Shared handle held by [`super::web::WebServices`].
pub(crate) type Handle = Rc<RefCell<WebDictionaries>>;

/// Load any previously downloaded dictionaries cached in IndexedDB and install them into memory.
pub(crate) async fn init(handle: Handle, ctx: egui::Context) {
    let db = match crate::idb::open_named(DB_NAME, STORE_NAME).await {
        Ok(db) => db,
        Err(e) => {
            log::warn!("dictionary cache unavailable ({e})");
            return;
        }
    };
    let entries = crate::idb::load_from(&db, STORE_NAME).await.unwrap_or_default();
    handle.borrow_mut().db = Some(db);
    let mut pairs: BTreeMap<String, (Option<Vec<u8>>, Option<Vec<u8>>)> = BTreeMap::new();
    for (name, bytes) in entries {
        if let Some(locale) = name.strip_suffix(".aff") {
            pairs.entry(locale.to_string()).or_default().0 = Some(bytes);
        } else if let Some(locale) = name.strip_suffix(".dic") {
            pairs.entry(locale.to_string()).or_default().1 = Some(bytes);
        }
    }
    let mut any = false;
    for (locale, (aff, dic)) in pairs {
        if let (Some(aff), Some(dic)) = (aff, dic) {
            if let Err(e) = mailcraft_ui_egui::install_dictionary_bytes(&locale, &aff, &dic) {
                log::warn!("cached dictionary `{locale}` couldn't be loaded: {e}");
            } else {
                any = true;
            }
        }
    }
    if any {
        ctx.request_repaint();
    }
}

pub(crate) fn state(handle: &Handle) -> DictionaryDownload {
    handle.borrow().state.clone()
}

pub(crate) fn take_ready(handle: &Handle) -> Vec<DownloadedDictionary> {
    std::mem::take(&mut handle.borrow_mut().ready)
}

pub(crate) fn start(handle: &Handle, locale: &str, aff_url: &str, dic_url: &str, ctx: &egui::Context) {
    {
        let mut g = handle.borrow_mut();
        if matches!(g.state, DictionaryDownload::Downloading { .. }) {
            return;
        }
        g.state = DictionaryDownload::Downloading { locale: locale.to_string(), done: 0, total: 0 };
    }
    let handle = handle.clone();
    let locale = locale.to_string();
    let aff_url = aff_url.to_string();
    let dic_url = dic_url.to_string();
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let fetched = async {
            let aff = fetch_bytes(&aff_url).await?;
            let dic = fetch_bytes(&dic_url).await?;
            mailcraft_ui_egui::validate_hunspell(&locale, &aff, &dic)?;
            Ok::<_, String>((aff, dic))
        }
        .await;
        let mut g = handle.borrow_mut();
        match fetched {
            Ok((aff, dic)) => {
                if let Some(db) = &g.db {
                    let changes = [(format!("{locale}.aff"), Some(aff.clone())), (format!("{locale}.dic"), Some(dic.clone()))];
                    if let Err(e) = crate::idb::save_in(db, STORE_NAME, &changes) {
                        log::warn!("couldn't cache dictionary `{locale}` in IndexedDB: {e}");
                    }
                }
                g.state = DictionaryDownload::Idle;
                g.ready.push(DownloadedDictionary { locale, files: Some((aff, dic)) });
            }
            Err(message) => {
                g.state = DictionaryDownload::Error { locale, message };
            }
        }
        ctx.request_repaint();
    });
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = crate::fetch::Fetch.send(mailcraft_jmap::Request::get(url, MAX_FILE_BYTES)).await.map_err(|e| e.to_string())?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("HTTP {}", resp.status));
    }
    Ok(resp.body)
}
