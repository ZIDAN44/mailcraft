//! What the page keeps sealed between visits (gap 7: a certificate's password, when the user asks
//! for its certificates to stay unlocked): AES-256-GCM with a key the browser makes itself and
//! won't let out (WebCrypto, `extractable: false`), kept in IndexedDB as the key object. Script on
//! this page can use it; nothing can read it, copy it to another browser or profile, or open
//! what it sealed anywhere else. Forgetting it makes everything it sealed unreadable.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CryptoKey, IdbDatabase, IdbTransactionMode};

const DB: &str = "mailcraft-vault";
const KEYS: &str = "keys";
const KEY: &str = "seal";
/// AES-GCM's nonce, before the sealed bytes.
const IV: usize = 12;

fn js(e: &JsValue) -> String {
    e.as_string().or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| format!("{e:?}"))
}

fn crypto() -> Result<web_sys::Crypto, String> {
    web_sys::window().ok_or("no window")?.crypto().map_err(|e| js(&e))
}

async fn open_db() -> Result<IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(|e| js(&e))?.ok_or("no IndexedDB")?;
    let req = factory.open_with_u32(DB, 1).map_err(|e| js(&e))?;
    let upgrade_req = req.clone();
    let upgrade = Closure::once_into_js(move |_: web_sys::Event| {
        if let Some(db) = upgrade_req.result().ok().and_then(|r| r.dyn_into::<IdbDatabase>().ok())
            && !db.object_store_names().contains(KEYS)
            && let Err(e) = db.create_object_store(KEYS)
        {
            log::error!("IndexedDB: {}", js(&e));
        }
    });
    req.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    crate::idb::done(&req).await?.dyn_into::<IdbDatabase>().map_err(|_| "not a database".to_string())
}

/// The sealing key, made the first time (`create`) and kept as it is.
async fn key(create: bool) -> Result<Option<CryptoKey>, String> {
    let db = open_db().await?;
    let store = db.transaction_with_str(KEYS).and_then(|t| t.object_store(KEYS)).map_err(|e| js(&e))?;
    let found = crate::idb::done(&store.get(&JsValue::from_str(KEY)).map_err(|e| js(&e))?).await?;
    if let Ok(k) = found.dyn_into::<CryptoKey>() {
        return Ok(Some(k));
    }
    if !create {
        return Ok(None);
    }
    let usages = js_sys::Array::of2(&JsValue::from_str("encrypt"), &JsValue::from_str("decrypt"));
    let params = web_sys::AesKeyGenParams::new("AES-GCM", 256);
    let made = crypto()?.subtle().generate_key_with_object(&params, false, &usages).map_err(|e| js(&e))?;
    let k: CryptoKey = JsFuture::from(made).await.map_err(|e| js(&e))?.dyn_into().map_err(|_| "not a key".to_string())?;
    let store = db.transaction_with_str_and_mode(KEYS, IdbTransactionMode::Readwrite).and_then(|t| t.object_store(KEYS)).map_err(|e| js(&e))?;
    crate::idb::done(&store.put_with_key(&k, &JsValue::from_str(KEY)).map_err(|e| js(&e))?).await?;
    Ok(Some(k))
}

/// `plain` sealed with this browser's key (made if there's none yet).
pub async fn seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    let k = key(true).await?.ok_or("no key")?;
    let mut iv = [0u8; IV];
    crypto()?.get_random_values_with_u8_array(&mut iv).map_err(|e| js(&e))?;
    // The nonce copied into the page's own memory: a view into ours (`new_with_u8_slice` on a
    // temporary) was read after the temporary was gone, and the stored nonce didn't open it.
    let params = web_sys::AesGcmParams::new_with_u8_array("AES-GCM", &js_sys::Uint8Array::from(iv.as_slice()));
    let sealed = crypto()?.subtle().encrypt_with_object_and_u8_array(&params, &k, plain).map_err(|e| js(&e))?;
    let sealed = js_sys::Uint8Array::new(&JsFuture::from(sealed).await.map_err(|e| js(&e))?).to_vec();
    Ok([iv.as_slice(), &sealed].concat())
}

/// What [`seal`] sealed, or why not: no key (forgotten, another browser), or not sealed by it.
pub async fn open(sealed: &[u8]) -> Result<Vec<u8>, String> {
    let k = key(false).await?.ok_or("this browser no longer has the key it was kept with")?;
    let (iv, body) = (sealed.get(..IV).ok_or("too short")?, sealed.get(IV..).ok_or("too short")?);
    let params = web_sys::AesGcmParams::new_with_u8_array("AES-GCM", &js_sys::Uint8Array::from(iv));
    let opened = crypto()?.subtle().decrypt_with_object_and_u8_array(&params, &k, body).map_err(|e| js(&e))?;
    let opened = JsFuture::from(opened).await.map_err(|_| "it wasn't kept with this browser's key".to_string())?;
    Ok(js_sys::Uint8Array::new(&opened).to_vec())
}

/// Forget the key: everything sealed with it can't be opened again.
pub async fn forget() -> Result<(), String> {
    let db = open_db().await?;
    let store = db.transaction_with_str_and_mode(KEYS, IdbTransactionMode::Readwrite).and_then(|t| t.object_store(KEYS)).map_err(|e| js(&e))?;
    crate::idb::done(&store.delete(&JsValue::from_str(KEY)).map_err(|e| js(&e))?).await.map(|_| ())
}
