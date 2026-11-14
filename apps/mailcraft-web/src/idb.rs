//! The cache kept between visits, in the browser's IndexedDB: one object store of files by their
//! path in the cache (`a1/folders.json`, `a1/f…/3/12.eml`), mirrored from the in-memory cache's
//! changes. Passwords are never in the cache, so never here.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{IdbDatabase, IdbRequest, IdbTransactionMode};

use crate::js;

const DB: &str = "mailcraft";
const FILES: &str = "files";

/// A request's result, once it has one.
pub(crate) async fn done(req: &IdbRequest) -> Result<JsValue, String> {
    let r = req.clone();
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let ok_req = r.clone();
        let ok = Closure::once_into_js(move |_: web_sys::Event| {
            let _ = resolve.call1(&JsValue::NULL, &ok_req.result().unwrap_or(JsValue::UNDEFINED));
        });
        let fail = Closure::once_into_js(move |_: web_sys::Event| {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("the browser's storage refused"));
        });
        r.set_onsuccess(Some(ok.unchecked_ref()));
        r.set_onerror(Some(fail.unchecked_ref()));
    });
    JsFuture::from(promise).await.map_err(|e| js(&e))
}

/// Open the database (made on first use).
pub async fn open() -> Result<IdbDatabase, String> {
    open_named(DB, FILES).await
}

/// Open the database `name` with its one object store `store` (both made on first use).
pub(crate) async fn open_named(name: &str, store: &'static str) -> Result<IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(|e| js(&e))?.ok_or("no IndexedDB")?;
    let req = factory.open_with_u32(name, 1).map_err(|e| js(&e))?;
    let upgrade_req = req.clone();
    let upgrade = Closure::once_into_js(move |_: web_sys::Event| {
        if let Some(db) = upgrade_req.result().ok().and_then(|r| r.dyn_into::<IdbDatabase>().ok())
            && !db.object_store_names().contains(store)
            && let Err(e) = db.create_object_store(store)
        {
            log::error!("IndexedDB: {}", js(&e));
        }
    });
    req.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    done(&req).await?.dyn_into::<IdbDatabase>().map_err(|_| "not a database".to_string())
}

/// Every file kept.
pub async fn load(db: &IdbDatabase) -> Result<Vec<(String, Vec<u8>)>, String> {
    let tx = db.transaction_with_str(FILES).map_err(|e| js(&e))?;
    let store = tx.object_store(FILES).map_err(|e| js(&e))?;
    // Keys and values come in the same (key) order.
    let keys = done(&store.get_all_keys().map_err(|e| js(&e))?).await?;
    let values = done(&store.get_all().map_err(|e| js(&e))?).await?;
    let (keys, values) = (js_sys::Array::from(&keys), js_sys::Array::from(&values));
    Ok(keys.iter().zip(values.iter()).filter_map(|(k, v)| Some((k.as_string()?, v.dyn_into::<js_sys::Uint8Array>().ok()?.to_vec()))).collect())
}

/// Every file kept gives way to `files` (a backup's, issue #83), in one transaction: all of it,
/// or what was there stays. `then` is told how it went once the browser says: `Ok` when it has
/// them all, else why not.
pub fn replace(db: &IdbDatabase, files: &[(String, Vec<u8>)], then: impl Fn(Result<(), String>) + 'static) -> Result<(), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readwrite).map_err(|e| js(&e))?;
    let fill = || -> Result<(), String> {
        let store = tx.object_store(FILES).map_err(|e| js(&e))?;
        store.clear().map_err(|e| js(&e))?;
        for (path, bytes) in files {
            store.put_with_key(&js_sys::Uint8Array::from(bytes.as_slice()), &JsValue::from_str(path)).map_err(|e| js(&e))?;
        }
        Ok(())
    };
    if let Err(e) = fill() {
        // Left alone, it would finish with what it had: the files cleared.
        let _ = tx.abort();
        return Err(e);
    }
    let then = std::rc::Rc::new(then);
    let told = then.clone();
    let done = Closure::once_into_js(move |_: web_sys::Event| told(Ok(())));
    tx.set_oncomplete(Some(done.unchecked_ref()));
    // A failed write ends the transaction this way too, as does storage running out as it ends.
    let failed = Closure::once_into_js(move |_: web_sys::Event| then(Err("the browser's storage refused it (full?)".into())));
    tx.set_onabort(Some(failed.unchecked_ref()));
    Ok(())
}

/// Write changed files (`None`: removed), in one transaction that finishes on its own.
pub fn save(db: &IdbDatabase, changes: &[(String, Option<Vec<u8>>)]) -> Result<(), String> {
    let tx = db.transaction_with_str_and_mode(FILES, IdbTransactionMode::Readwrite).map_err(|e| js(&e))?;
    let store = tx.object_store(FILES).map_err(|e| js(&e))?;
    for (path, bytes) in changes {
        let key = JsValue::from_str(path);
        let r = match bytes {
            Some(b) => store.put_with_key(&js_sys::Uint8Array::from(b.as_slice()), &key),
            None => store.delete(&key),
        };
        r.map_err(|e| js(&e))?;
    }
    let failed = Closure::once_into_js(|_: web_sys::Event| log::warn!("the browser didn't keep the mail cache (storage full?)"));
    tx.set_onerror(Some(failed.unchecked_ref()));
    Ok(())
}
