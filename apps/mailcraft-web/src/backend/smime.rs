//! S/MIME in the browser. The page has no keychain, so the certificate's `.p12` file itself is
//! what's kept (in the browser's storage, beside the account's mail): it's already encrypted with
//! its password (PBES2), which is never kept. The key is opened with the password for this page
//! only; after a reload the certificate is locked until its password is given again (File ›
//! Info › Security › Unlock…). Signing, encrypting and decrypting are the desktop's own code.

use mailcraft_engine::model::{Event, Job, Private};
use mailcraft_engine::{mime, smime};

use super::{Rcs, Task, emit, enqueue, notice, now, store};

/// The account document with the `.p12` file.
const P12_DOC: &str = "smime-p12.json";

/// Why the key can't be used until its password is given again.
const LOCKED: &str = "your certificate is locked in this browser: unlock it with its password (File › Info › Security)";

fn unlocked(sh: &Rcs, id: &str) -> Result<smime::Identity, String> {
    sh.borrow().smime.get(id).cloned().ok_or_else(|| LOCKED.to_string())
}

/// `raw` signed and/or encrypted with `me`, as [`Job::Protect`] asks.
fn protect(raw: &[u8], me: &smime::Identity, sign: bool, encrypt_to: &[Vec<u8>], gcm: bool, hide_subject: bool) -> Result<Vec<u8>, String> {
    use rand_core::RngCore as _;
    let boundary = format!("=_mcs{:016x}", rand_core::OsRng.next_u64());
    let mut sign_with = |entity: &[u8]| smime::sign(entity, me, now(), &mut rand_core::OsRng).map_err(|e| e.to_string());
    let cipher = if gcm { smime::Cipher::Aes256Gcm } else { smime::Cipher::Aes256Cbc };
    let mut encrypt_with = |entity: &[u8]| smime::encrypt(entity, encrypt_to, cipher, &mut rand_core::OsRng).map_err(|e| e.to_string());
    let signer: Option<mime::smime::Crypto<'_>> = if sign { Some(&mut sign_with) } else { None };
    let encrypter: Option<mime::smime::Crypto<'_>> = if encrypt_to.is_empty() { None } else { Some(&mut encrypt_with) };
    mime::smime::protect(raw, signer, encrypter, gcm, hide_subject, &boundary)
}

/// An S/MIME job, for the account `id`.
pub(super) fn job(sh: &Rcs, id: &str, j: Job) {
    match j {
        Job::ImportIdentity { data, password, .. } => {
            // No file: the one kept here, unlocked.
            let kept = data.is_empty();
            let p12 = if kept { store(sh).load_account_doc::<Vec<u8>>(id, P12_DOC).unwrap_or_default() } else { data };
            let done = if p12.is_empty() {
                Err("there's no certificate kept in this browser: import its .p12 file".to_string())
            } else {
                smime::read_p12(&p12, &password.0).map_err(|e| e.to_string())
            };
            let (chain, error) = match done {
                Ok(me) => {
                    // A file without a password is the key itself, readable by anyone with the
                    // browser's storage: used for this page, never kept.
                    if !kept && password.0.is_empty() {
                        notice(sh, id, "the certificate has no password, so the browser uses it until the page is closed and doesn't keep it".into());
                    } else if !kept && let Err(e) = store(sh).save_account_doc(id, P12_DOC, &p12) {
                        notice(sh, id, format!("the certificate is used now, but couldn't be kept for next time: {e}"));
                    }
                    let chain = me.chain.clone();
                    sh.borrow_mut().smime.insert(id.to_string(), me);
                    (chain, None)
                }
                Err(e) => (Vec::new(), Some(e)),
            };
            emit(sh, Event::Identity { account: id.to_string(), chain, older: Vec::new(), error });
        }
        Job::ForgetIdentity { .. } => {
            sh.borrow_mut().smime.remove(id);
            if let Err(e) = store(sh).save_account_doc(id, P12_DOC, &Vec::<u8>::new()) {
                notice(sh, id, format!("the certificate couldn't be removed from this browser: {e}"));
            }
        }
        Job::Protect { id: msg, raw, sign, encrypt_to, gcm, hide_subject, .. } => {
            let done = unlocked(sh, id).and_then(|me| protect(&raw.0, &me, sign, &encrypt_to, gcm, hide_subject));
            let (raw, error) = match done {
                Ok(r) => (Some(Private(r)), None),
                Err(e) => (None, Some(e)),
            };
            emit(sh, Event::Protected { account: id.to_string(), id: msg, raw, error });
        }
        Job::Decrypt { key, cms, .. } => {
            let done = unlocked(sh, id).and_then(|me| smime::decrypt(&cms, &me, &mut rand_core::OsRng).map_err(|e| e.to_string()));
            let e = match done {
                Ok((content, authenticated)) => Event::Decrypted { key, content: Some(Private(content)), authenticated, error: None },
                Err(e) => Event::Decrypted { key, content: None, authenticated: false, error: Some(e) },
            };
            emit(sh, e);
        }
        // Encrypted to the account's own certificate first, as on the desktop.
        Job::SealDraft { raw, folder, replaces, .. } => {
            let sealed = unlocked(sh, id).and_then(|me| {
                let own = me.chain.first().cloned().ok_or_else(|| "the certificate is missing".to_string())?;
                protect(&raw.0, &me, false, &[own], true, false)
            });
            let raw = match sealed {
                Ok(s) => s,
                Err(e) => {
                    notice(sh, id, format!("the draft couldn't be encrypted, so it's saved as it is: {e}"));
                    raw.0
                }
            };
            enqueue(sh, id, Task::Job(Job::SaveDraft { account: id.to_string(), raw, folder, replaces }));
        }
        _ => {}
    }
}
