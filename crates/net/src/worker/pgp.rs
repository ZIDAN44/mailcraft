//! The pool: OpenPGP's work with an account's own keys (issue #36), each job on a thread of its
//! own. A key's secret part, its passphrase taken off as it's imported, lives in the system's
//! keychain only (`Credentials::set_secret`, one entry `pgp.<fingerprint>` per key, listed in
//! `pgp`, the one in use first); the engine sees the public parts, never a secret one. As with
//! S/MIME certificates, older keys stay to read mail encrypted to them.

use mailcraft_model::{PgpSignature, PgpWork, Private};
use mailcraft_pgp as pgp;

use super::*;

/// The list of the account's keys' fingerprints (JSON), the one in use first.
const LIST: &str = "pgp";
/// The most keys kept for an account.
pub(super) const KEYS_MAX: usize = 16;

/// Held while the entries change (each job runs on a thread of its own).
static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn changing() -> std::sync::MutexGuard<'static, ()> {
    CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn entry(fingerprint: &str) -> String {
    format!("{LIST}.{}", fingerprint.trim().to_lowercase())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok())).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0)
}

/// The fingerprints of the account's keys, the one in use first.
pub(super) fn list(creds: &dyn Credentials, account: &AccountConfig) -> Vec<String> {
    let mut l: Vec<String> = creds.get_secret(account, LIST).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    l.truncate(KEYS_MAX);
    l
}

fn set_list(creds: &dyn Credentials, account: &AccountConfig, list: &[String]) -> std::result::Result<(), String> {
    if list.is_empty() {
        creds.delete_secret(account, LIST);
        return Ok(());
    }
    creds.set_secret(account, LIST, &serde_json::to_string(list).map_err(|e| e.to_string())?)
}

/// Key `fingerprint`'s secret part (binary, unlocked).
fn secret(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<Vec<u8>, String> {
    let gone = || "the key's secret part isn't in the keychain: import the key again (File › Info › Security)".to_string();
    creds.get_secret(account, &entry(fingerprint)).and_then(|h| unhex(&h)).ok_or_else(gone)
}

/// The secret part of the key a message is signed with: `signer` (its fingerprint; issue #81:
/// the one that names the address the message is from) when it's one of the account's, else
/// the one in use.
fn signing_key(creds: &dyn Credentials, account: &AccountConfig, signer: Option<&str>) -> std::result::Result<Vec<u8>, String> {
    let keys = list(creds, account);
    let fp = match signer {
        // The one named and no other: signed with the key in use instead, mail from one
        // address went out signed as another (as with a certificate, it's said instead).
        Some(named) => keys
            .iter()
            .find(|k| k.eq_ignore_ascii_case(named))
            .ok_or_else(|| "the key for the address it's from isn't in the keychain: import the key again (File › Info › Security)".to_string())?,
        None => keys.first().ok_or_else(|| "there's no OpenPGP key for this account: import one in File › Info › Security".to_string())?,
    };
    secret(creds, account, fp)
}

/// Every key's secret part, the one in use first.
fn secrets(creds: &dyn Credentials, account: &AccountConfig) -> Vec<Vec<u8>> {
    list(creds, account).iter().filter_map(|fp| secret(creds, account, fp).ok()).collect()
}

/// The keys' public parts, the one in use first.
fn publics(creds: &dyn Credentials, account: &AccountConfig) -> Vec<Vec<u8>> {
    secrets(creds, account).iter().filter_map(|s| pgp::read_secret_key(s, "").ok()).map(|k| k.public).collect()
}

/// Keep `key`, which signs and decrypts from now on.
fn take(creds: &dyn Credentials, account: &AccountConfig, key: &pgp::Unlocked) -> std::result::Result<(), String> {
    let _changing = changing();
    let fp = key.info.fingerprint.clone();
    let mut l = list(creds, account);
    l.retain(|f| !f.eq_ignore_ascii_case(&fp));
    if l.len() >= KEYS_MAX {
        return Err(format!("MailCraft keeps at most {KEYS_MAX} OpenPGP keys for an account: remove one first (File › Info › Security)"));
    }
    creds.set_secret(account, &entry(&fp), &hex(&key.secret))?;
    l.insert(0, fp);
    set_list(creds, account, &l)
}

/// Key `fingerprint` signs and decrypts again.
fn use_key(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<(), String> {
    let _changing = changing();
    let mut l = list(creds, account);
    let Some(at) = l.iter().position(|f| f.eq_ignore_ascii_case(fingerprint.trim())) else { return Err("that key isn't kept".into()) };
    let fp = l.remove(at);
    l.insert(0, fp);
    set_list(creds, account, &l)
}

fn forget(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) {
    let _changing = changing();
    creds.delete_secret(account, &entry(fingerprint));
    let mut l = list(creds, account);
    l.retain(|f| !f.eq_ignore_ascii_case(fingerprint.trim()));
    let _ = set_list(creds, account, &l);
}

/// Every key of the account's, as the account goes.
pub(crate) fn forget_all(creds: &dyn Credentials, account: &AccountConfig) {
    let _changing = changing();
    for fp in list(creds, account) {
        creds.delete_secret(account, &entry(&fp));
    }
    creds.delete_secret(account, LIST);
}

/// `raw` signed with `secret` (a binary secret key) and/or encrypted to `encrypt_to`, as PGP/MIME.
pub(crate) fn protect(raw: &[u8], secret: &[u8], sign: bool, encrypt_to: &[Vec<u8>], hide_subject: bool) -> std::result::Result<Vec<u8>, String> {
    use rand_core::RngCore as _;
    let boundary = format!("=_mcp{:016x}", rand_core::OsRng.next_u64());
    let at = now();
    let keys: Vec<&[u8]> = encrypt_to.iter().map(Vec::as_slice).collect();
    let mut sign_with = |entity: &[u8]| pgp::sign(entity, secret, at, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let mut encrypt_with =
        |entity: &[u8]| pgp::encrypt(entity, &keys, None, at, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let signer: Option<mailcraft_mime::smime::Crypto<'_>> = if sign { Some(&mut sign_with) } else { None };
    let encrypter: Option<mailcraft_mime::smime::Crypto<'_>> = if keys.is_empty() { None } else { Some(&mut encrypt_with) };
    mailcraft_mime::pgp::protect(raw, signer, encrypter, hide_subject, &boundary)
}

/// `data` decrypted with the account's keys: the MIME entity inside (a text made one, for inline
/// PGP), and what a signature in it says.
pub(crate) fn decrypt(
    creds: &dyn Credentials,
    account: &AccountConfig,
    data: &[u8],
    inline: bool,
    charset: &str,
    verify_with: &[Vec<u8>],
) -> std::result::Result<(Vec<u8>, Option<PgpSignature>), String> {
    let mine = secrets(creds, account);
    if mine.is_empty() {
        return Err("this account has no OpenPGP key to decrypt it with (File › Info › Security)".into());
    }
    let secrets: Vec<&[u8]> = mine.iter().map(Vec::as_slice).collect();
    let keys: Vec<&[u8]> = verify_with.iter().map(Vec::as_slice).collect();
    let d = pgp::decrypt(data, &secrets, &keys).map_err(|e| match e {
        pgp::Error::Decrypt => {
            let to = pgp::recipients(data);
            if to.is_empty() {
                e.to_string()
            } else {
                format!("it was encrypted to a key MailCraft doesn't have ({}): import that key too (File › Info › Security)", to.join(", "))
            }
        }
        other => other.to_string(),
    })?;
    let signature = d.check.map(|c| match c {
        pgp::Check::Good { key, at, weak } => PgpSignature::Good { key, at, weak },
        pgp::Check::Bad => PgpSignature::Bad,
        pgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
    });
    let entity = if inline { mailcraft_mime::pgp::text_entity(&d.data, charset) } else { d.data };
    Ok((entity, signature))
}

/// What the account has now, or why it couldn't be done.
fn keys_now(creds: &dyn Credentials, account: &AccountConfig, imported: Option<String>, done: std::result::Result<(), String>) -> Event {
    let keys = publics(creds, account);
    match done {
        Ok(()) => Event::PgpKeys { account: account.id.clone(), keys, imported, error: None },
        Err(e) => Event::PgpKeys { account: account.id.clone(), keys, imported: None, error: Some(e) },
    }
}

impl Pool {
    pub(super) fn pgp(&mut self, account: AccountConfig, work: PgpWork) {
        // A draft is sealed at once (it may be the last thing before MailCraft closes), then saved.
        if let PgpWork::SealDraft { raw, folder, replaces, keep } = work {
            let sealed = list(self.creds.as_ref(), &account)
                .first()
                .ok_or_else(|| "there's no OpenPGP key".to_string())
                .and_then(|fp| secret(self.creds.as_ref(), &account, fp))
                .and_then(|s| {
                    let me = pgp::read_secret_key(&s, "").map_err(|e| e.to_string())?;
                    protect(&raw.0, &s, false, &[me.public], false)
                });
            let raw = match sealed {
                Ok(s) => s,
                Err(e) => {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the draft couldn't be encrypted, so it's saved as it is: {e}"),
                    });
                    raw.0
                }
            };
            let account = account.id;
            self.submit(if keep { Job::KeepDraft { account, raw, folder, replaces } } else { Job::SaveDraft { account, raw, folder, replaces } });
            return;
        }
        let creds = self.creds.clone();
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-pgp".into()).spawn(move || {
            let creds = creds.as_ref();
            let event = match work {
                PgpWork::Import { file, passphrase } => match pgp::read_secret_key(&file.0, &passphrase.0) {
                    Ok(key) => {
                        let fp = key.info.fingerprint.clone();
                        keys_now(creds, &account, Some(fp), take(creds, &account, &key))
                    }
                    Err(e) => keys_now(creds, &account, None, Err(e.to_string())),
                },
                PgpWork::Use { fingerprint } => keys_now(creds, &account, None, use_key(creds, &account, &fingerprint)),
                PgpWork::Forget { fingerprint } => {
                    forget(creds, &account, &fingerprint);
                    keys_now(creds, &account, None, Ok(()))
                }
                PgpWork::Decrypt { key, data, inline, charset, verify_with } => match decrypt(creds, &account, &data, inline, &charset, &verify_with)
                {
                    Ok((entity, signature)) => Event::PgpDecrypted { key, content: Some(Private(entity)), signature, error: None },
                    Err(e) => Event::PgpDecrypted { key, content: None, signature: None, error: Some(e) },
                },
                PgpWork::Protect { id, raw, sign, encrypt_to, hide_subject, signer } => {
                    let done = signing_key(creds, &account, signer.as_deref()).and_then(|s| protect(&raw.0, &s, sign, &encrypt_to, hide_subject));
                    match done {
                        Ok(r) => Event::Protected { account: account.id.clone(), id, raw: Some(Private(r)), error: None },
                        Err(e) => Event::Protected { account: account.id.clone(), id, raw: None, error: Some(e) },
                    }
                }
                PgpWork::SealDraft { .. } => return,
            };
            sink.emit(event);
        });
        if let Err(e) = spawned {
            log::warn!("OpenPGP: {e}");
        }
    }
}

#[cfg(test)]
mod tests;
