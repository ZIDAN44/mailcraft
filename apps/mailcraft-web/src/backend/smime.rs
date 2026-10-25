//! S/MIME in the browser. The page has no keychain, so each certificate's `.p12` file itself is
//! what's kept (in the browser's storage, beside the account's mail): it's already encrypted with
//! its password (PBES2), which is never kept. As on the desktop, a renewal keeps the old
//! certificate to read mail sent to it ([`smime::own`]). Keys are opened with their password for
//! this page only; after a reload the certificates are locked until a password is given again
//! (File › Info › Security › Unlock…), which opens every one it's the password of. Signing,
//! encrypting and decrypting are the desktop's own code.

use mailcraft_engine::model::{Event, Job, Private};
use mailcraft_engine::smime::own::{Entry, Own, Taken};
use mailcraft_engine::{SMIME_DOC, SmimeSettings, mime, smime};
use serde::{Deserialize, Serialize};

use super::{Rcs, Task, emit, enqueue, notice, now, store};

/// The account document with the certificates and their `.p12` files.
const OWN_DOC: &str = "smime-own.json";
/// Where an earlier MailCraft kept the one `.p12` file (read until something new is kept).
const P12_DOC: &str = "smime-p12.json";

/// Why the key can't be used until its password is given again.
const LOCKED: &str = "your certificate is locked in this browser: unlock it with its password (File › Info › Security)";

#[derive(Default, Serialize, Deserialize)]
struct OwnDoc {
    #[serde(default)]
    current: Option<EntryDoc>,
    #[serde(default)]
    older: Vec<EntryDoc>,
}

#[derive(Serialize, Deserialize)]
struct EntryDoc {
    cert: Vec<u8>,
    p12: Vec<u8>,
}

impl From<EntryDoc> for Entry {
    fn from(e: EntryDoc) -> Entry {
        Entry { cert: e.cert, p12: e.p12 }
    }
}

/// The account's certificates as kept here. One kept by an earlier MailCraft (its file only) is
/// taken with the certificate the account's settings name.
fn load(sh: &Rcs, id: &str) -> Own {
    let st = store(sh);
    // Never kept: none (a document that isn't there reads as its default).
    if let Ok(Some(doc)) = st.load_account_doc::<Option<OwnDoc>>(id, OWN_DOC) {
        return Own { current: doc.current.map(Entry::from), older: doc.older.into_iter().map(Entry::from).collect() };
    }
    let p12: Vec<u8> = st.load_account_doc(id, P12_DOC).unwrap_or_default();
    let cert = st.load_account_doc::<SmimeSettings>(id, SMIME_DOC).ok().and_then(|s| s.chain.into_iter().next()).unwrap_or_default();
    Own { current: (!p12.is_empty() && !cert.is_empty()).then_some(Entry { cert, p12 }), older: Vec::new() }
}

/// Keep `own`; a certificate whose file had no password (its key itself) is used for this page
/// and never kept.
fn save(sh: &Rcs, id: &str, own: &Own) {
    let doc = |e: &Entry| (!e.p12.is_empty()).then(|| EntryDoc { cert: e.cert.clone(), p12: e.p12.clone() });
    let d = OwnDoc { current: own.current.as_ref().and_then(doc), older: own.older.iter().filter_map(doc).collect() };
    let st = store(sh);
    if let Err(e) = st.save_account_doc(id, OWN_DOC, &d) {
        notice(sh, id, format!("the certificates couldn't be kept in this browser: {e}"));
        return;
    }
    let _ = st.save_account_doc(id, P12_DOC, &Vec::<u8>::new());
}

/// The key of the certificate `cert`, when it's been unlocked in this page.
fn unlocked_for(sh: &Rcs, id: &str, cert: &[u8]) -> Option<smime::Identity> {
    sh.borrow().smime.get(id).and_then(|ids| ids.iter().find(|i| i.certificate() == cert).cloned())
}

/// The key that signs and encrypts.
fn current(sh: &Rcs, id: &str) -> Result<smime::Identity, String> {
    match load(sh, id).current {
        Some(e) => unlocked_for(sh, id, &e.cert).ok_or_else(|| LOCKED.to_string()),
        // One used for this page only (no password, so not kept) is the last one opened.
        None => sh.borrow().smime.get(id).and_then(|ids| ids.last().cloned()).ok_or_else(|| LOCKED.to_string()),
    }
}

/// Keys opened in this page, each replacing an earlier copy of itself.
fn unlock(sh: &Rcs, id: &str, opened: Vec<smime::Identity>) {
    let mut s = sh.borrow_mut();
    let ids = s.smime.entry(id.to_string()).or_default();
    for o in opened {
        ids.retain(|i| i.certificate() != o.certificate());
        ids.push(o);
    }
}

/// Keys forgotten in this page: those of the certificates `certs`.
fn lock(sh: &Rcs, id: &str, certs: &[Vec<u8>]) {
    if let Some(ids) = sh.borrow_mut().smime.get_mut(id) {
        ids.retain(|i| !certs.iter().any(|c| c.as_slice() == i.certificate()));
    }
}

/// What the account has now, for the engine.
fn identity(sh: &Rcs, id: &str, own: &Own, for_reading: bool) -> Event {
    let chain = match &own.current {
        Some(e) => unlocked_for(sh, id, &e.cert).map_or_else(|| vec![e.cert.clone()], |i| i.chain),
        None => sh.borrow().smime.get(id).and_then(|ids| ids.last().map(|i| i.chain.clone())).unwrap_or_default(),
    };
    Event::Identity { account: id.to_string(), chain, older: own.older_certs(), for_reading, error: None }
}

fn failed(id: &str, e: String) -> Event {
    Event::Identity { account: id.to_string(), chain: Vec::new(), older: Vec::new(), for_reading: false, error: Some(e) }
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

/// `cms` decrypted with the key it was sent to: the one in use or an older one. A key only
/// opened for this page (no password) is tried too.
fn decrypt(sh: &Rcs, id: &str, cms: &[u8]) -> Result<(Vec<u8>, bool), String> {
    let own = load(sh, id);
    let entry = own.for_message(cms).cloned();
    let me = match &entry {
        Some(e) => unlocked_for(sh, id, &e.cert),
        None => sh.borrow().smime.get(id).and_then(|ids| ids.iter().find(|i| smime::addressed_to(cms, i.certificate())).cloned()),
    };
    match (me, entry) {
        (Some(me), _) => smime::decrypt(cms, &me, &mut rand_core::OsRng).map_err(|e| e.to_string()),
        (None, Some(e)) if own.current.as_ref() == Some(&e) => Err(LOCKED.into()),
        (None, Some(e)) => {
            let until = smime::cert::info(&e.cert).map(|i| mailcraft_engine::model::dates::day_label(i.not_after.div_euclid(86_400))).unwrap_or_default();
            Err(format!(
                "it was sent to your older certificate (valid until {until}), which is locked in this browser: unlock it with its password (File › Info › Security)"
            ))
        }
        (None, None) => Err(
            "it was encrypted to a certificate this browser doesn't have: import that one too (File › Info › Security), and MailCraft keeps it to read such mail"
                .into(),
        ),
    }
}

/// An S/MIME job, for the account `id`.
pub(super) fn job(sh: &Rcs, id: &str, j: Job) {
    match j {
        // No file: those kept here, unlocked with the password (every one it opens).
        Job::ImportIdentity { data, password, .. } if data.is_empty() => {
            let own = load(sh, id);
            if own.current.is_none() && own.older.is_empty() {
                emit(sh, failed(id, "there's no certificate kept in this browser: import its .p12 file".into()));
                return;
            }
            let opened = own.unlock(&password.0);
            if opened.is_empty() {
                emit(sh, failed(id, "the password is wrong for every certificate kept in this browser".into()));
                return;
            }
            let locked = own.current.iter().chain(&own.older).filter(|e| !opened.iter().any(|o| o.certificate() == e.cert.as_slice())).count();
            unlock(sh, id, opened);
            if locked > 0 {
                notice(
                    sh,
                    id,
                    format!("{locked} of your certificates have another password and stay locked: give it with Unlock… to read mail sent to them"),
                );
            }
            emit(sh, identity(sh, id, &own, false));
        }
        Job::ImportIdentity { data, password, .. } => {
            let me = match smime::read_p12(&data, &password.0) {
                Ok(me) if !me.chain.is_empty() => me,
                Ok(_) => return emit(sh, failed(id, "the file holds no certificate".into())),
                Err(e) => return emit(sh, failed(id, e.to_string())),
            };
            // A file without a password is the key itself, readable by anyone with the browser's
            // storage: used for this page, never kept.
            let keep = !password.0.is_empty();
            if !keep {
                notice(sh, id, "the certificate has no password, so the browser uses it until the page is closed and doesn't keep it".into());
            }
            let mut own = load(sh, id);
            let entry = Entry { cert: me.certificate().to_vec(), p12: if keep { data } else { Vec::new() } };
            match own.take(entry, now()) {
                Ok(taken) => {
                    unlock(sh, id, vec![me]);
                    save(sh, id, &own);
                    emit(sh, identity(sh, id, &own, taken == Taken::ForReading));
                }
                Err(e) => emit(sh, failed(id, e.to_string())),
            }
        }
        Job::ForgetIdentity { .. } => {
            let mut own = load(sh, id);
            if let Some(cur) = own.current.take() {
                lock(sh, id, &[cur.cert]);
            }
            save(sh, id, &own);
        }
        Job::ForgetOlderIdentity { fingerprint, .. } => {
            let mut own = load(sh, id);
            let gone: Vec<Vec<u8>> =
                own.older.iter().filter(|e| e.fingerprint().eq_ignore_ascii_case(&fingerprint)).map(|e| e.cert.clone()).collect();
            own.forget_older(&fingerprint);
            lock(sh, id, &gone);
            save(sh, id, &own);
        }
        Job::UseOlderIdentity { fingerprint, .. } => {
            let mut own = load(sh, id);
            match own.use_older(&fingerprint) {
                Ok(()) => {
                    save(sh, id, &own);
                    emit(sh, identity(sh, id, &own, false));
                }
                Err(e) => emit(sh, failed(id, e.to_string())),
            }
        }
        Job::Protect { id: msg, raw, sign, encrypt_to, gcm, hide_subject, .. } => {
            let done = current(sh, id).and_then(|me| protect(&raw.0, &me, sign, &encrypt_to, gcm, hide_subject));
            let (raw, error) = match done {
                Ok(r) => (Some(Private(r)), None),
                Err(e) => (None, Some(e)),
            };
            emit(sh, Event::Protected { account: id.to_string(), id: msg, raw, error });
        }
        Job::Decrypt { key, cms, .. } => {
            let e = match decrypt(sh, id, &cms) {
                Ok((content, authenticated)) => Event::Decrypted { key, content: Some(Private(content)), authenticated, error: None },
                Err(e) => Event::Decrypted { key, content: None, authenticated: false, error: Some(e) },
            };
            emit(sh, e);
        }
        // Encrypted to the account's own certificate first, as on the desktop.
        Job::SealDraft { raw, folder, replaces, .. } => {
            let sealed = current(sh, id).and_then(|me| {
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
