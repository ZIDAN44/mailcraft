//! What the system's keychain keeps of an account's certificates: the one that signs and
//! encrypts (`smime`), and the older ones kept to read mail encrypted to them (issue #6: a
//! certificate renewed, mail sent to the last one still has to open). Each older one is an entry
//! of its own (`smime.older.<SHA-256>`), listed in `smime.older`, so none grows past what an entry
//! holds.

use super::*;

/// The keychain entry, next to the account's password.
pub(super) const SECRET: &str = "smime";
/// The list of older certificates' SHA-256s (JSON), each kept under [`older_entry`].
const OLDER: &str = "smime.older";
/// The most older certificates kept: a renewal a year for a long time.
pub(super) const OLDER_MAX: usize = 16;

fn older_entry(fingerprint: &str) -> String {
    format!("{OLDER}.{}", fingerprint.replace(':', "").to_lowercase())
}

/// A certificate's SHA-256, as [`smime::CertInfo::fingerprint`] has it (`AB:CD:…`).
pub(super) fn fingerprint(cert: &[u8]) -> String {
    smime::cert::info(cert).map(|i| i.fingerprint).unwrap_or_default()
}

/// What the keychain keeps: the key (PKCS #8) and the chain, as hex; or, for a key held
/// elsewhere, where it is and the card's PIN.
#[derive(Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    key: String,
    chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held: Option<KeyRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pin: Option<String>,
}

pub(super) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(super) fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok())).collect()
}

fn write(creds: &dyn Credentials, account: &AccountConfig, entry: &str, me: &Me) -> std::result::Result<(), String> {
    let chain = me.chain.iter().map(|c| hex(c)).collect();
    let kept = match &me.key {
        MyKey::Software(key) => Kept { key: hex(key), chain, held: None, pin: None },
        MyKey::Held { at, pin } => Kept { key: String::new(), chain, held: Some(at.clone()), pin: pin.clone() },
    };
    let json = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
    creds.set_secret(account, entry, &json)
}

fn read(creds: &dyn Credentials, account: &AccountConfig, entry: &str) -> std::result::Result<Me, String> {
    let gone = || "the certificate's key isn't in the keychain: import the certificate again (File › Info › Security)".to_string();
    let json = creds.get_secret(account, entry).ok_or_else(gone)?;
    let k: Kept = serde_json::from_str(&json).map_err(|_| gone())?;
    let chain = k.chain.iter().map(|c| unhex(c)).collect::<Option<Vec<_>>>().ok_or_else(gone)?;
    let key = match k.held {
        Some(at) => MyKey::Held { at, pin: k.pin },
        None => MyKey::Software(unhex(&k.key).ok_or_else(gone)?),
    };
    Ok(Me { chain, key })
}

/// Keep `me` as the account's certificate that signs and encrypts.
pub(super) fn keep(creds: &dyn Credentials, account: &AccountConfig, me: &Me) -> std::result::Result<(), String> {
    write(creds, account, SECRET, me)
}

/// The account's certificate that signs and encrypts, with its key.
pub(super) fn kept(creds: &dyn Credentials, account: &AccountConfig) -> std::result::Result<Me, String> {
    read(creds, account, SECRET)
}

/// The older certificates' SHA-256s, newest kept first.
pub(super) fn older_list(creds: &dyn Credentials, account: &AccountConfig) -> Vec<String> {
    let mut list: Vec<String> = creds.get_secret(account, OLDER).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    list.truncate(OLDER_MAX);
    list
}

fn set_older_list(creds: &dyn Credentials, account: &AccountConfig, list: &[String]) -> std::result::Result<(), String> {
    if list.is_empty() {
        creds.delete_secret(account, OLDER);
        return Ok(());
    }
    creds.set_secret(account, OLDER, &serde_json::to_string(list).map_err(|e| e.to_string())?)
}

/// An older certificate, with its key.
pub(super) fn kept_older(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<Me, String> {
    read(creds, account, &older_entry(fingerprint))
}

/// Keep `me` for reading only. At most [`OLDER_MAX`]: there's no room for more.
fn keep_older(creds: &dyn Credentials, account: &AccountConfig, me: &Me) -> std::result::Result<(), String> {
    let fp = fingerprint(me.certificate());
    if fp.is_empty() {
        return Err("the certificate can't be read".into());
    }
    let mut list = older_list(creds, account);
    list.retain(|f| *f != fp);
    if list.len() >= OLDER_MAX {
        return Err(format!("MailCraft keeps at most {OLDER_MAX} older certificates: remove one first (File › Info › Security)"));
    }
    write(creds, account, &older_entry(&fp), me)?;
    list.insert(0, fp);
    set_older_list(creds, account, &list)
}

/// Forget an older certificate and its key.
pub(super) fn forget_older(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) {
    creds.delete_secret(account, &older_entry(fingerprint));
    let mut list = older_list(creds, account);
    list.retain(|f| f != fingerprint);
    let _ = set_older_list(creds, account, &list);
}

/// Every certificate of the account's and its key, as the account goes.
pub(crate) fn forget_all(creds: &dyn Credentials, account: &AccountConfig) {
    for fp in older_list(creds, account) {
        creds.delete_secret(account, &older_entry(&fp));
    }
    creds.delete_secret(account, OLDER);
    creds.delete_secret(account, SECRET);
}

/// The older certificates (each one's own, DER), as the engine shows them.
pub(super) fn older_certs(creds: &dyn Credentials, account: &AccountConfig) -> Vec<Vec<u8>> {
    older_list(creds, account).iter().filter_map(|fp| kept_older(creds, account, fp).ok()).map(|m| m.certificate().to_vec()).collect()
}

/// Which of two certificates is the one to sign and encrypt with: one in date before one that
/// isn't, then the one issued later (a renewal), then the one that runs longer.
fn newer(a: &[u8], b: &[u8], now: i64) -> bool {
    let rank = |c: &[u8]| smime::cert::info(c).ok().map(|i| (i.not_before <= now && now <= i.not_after, i.not_before, i.not_after));
    rank(a) > rank(b)
}

/// Keep a certificate just imported (or found on a card or in the system's store): the newer of
/// it and the one in use signs and encrypts, the other is kept for reading. Gives back what the
/// account now has: the chain in use, and the older certificates.
pub(super) fn take(creds: &dyn Credentials, account: &AccountConfig, me: &Me, now: i64) -> std::result::Result<(Vec<Vec<u8>>, Vec<Vec<u8>>), String> {
    let fp = fingerprint(me.certificate());
    match kept(creds, account) {
        // The same one again (a new PIN, a key that moved): kept as it is now.
        Ok(cur) if fingerprint(cur.certificate()) == fp => keep(creds, account, me)?,
        Ok(cur) if !newer(me.certificate(), cur.certificate(), now) => keep_older(creds, account, me)?,
        Ok(cur) => {
            keep_older(creds, account, &cur)?;
            keep(creds, account, me)?;
            // One kept for reading that now signs and encrypts isn't kept twice.
            forget_older(creds, account, &fp);
        }
        Err(_) => {
            keep(creds, account, me)?;
            forget_older(creds, account, &fp);
        }
    }
    Ok((kept(creds, account).map(|m| m.chain).unwrap_or_default(), older_certs(creds, account)))
}

/// An older certificate made the one that signs and encrypts again; the one in use until now is
/// kept for reading.
pub(super) fn use_older(
    creds: &dyn Credentials,
    account: &AccountConfig,
    fingerprint: &str,
) -> std::result::Result<(Vec<Vec<u8>>, Vec<Vec<u8>>), String> {
    let older = kept_older(creds, account, fingerprint)?;
    if let Ok(cur) = kept(creds, account) {
        keep_older(creds, account, &cur)?;
    }
    keep(creds, account, &older)?;
    forget_older(creds, account, fingerprint);
    Ok((older.chain, older_certs(creds, account)))
}

/// After `done`, a kept PIN the card refused is forgotten (the reference to the key stays), in
/// the entry it came from: the one in use, or an older one (`older`, its SHA-256).
pub(super) fn forget_wrong_pin<T>(creds: &dyn Credentials, account: &AccountConfig, older: Option<&str>, done: &std::result::Result<T, String>) {
    if !matches!(done, Err(e) if e.contains(PIN_WRONG)) {
        return;
    }
    let entry = older.map(older_entry).unwrap_or_else(|| SECRET.to_string());
    let Some(json) = creds.get_secret(account, &entry) else { return };
    let Ok(mut k) = serde_json::from_str::<Kept>(&json) else { return };
    k.pin = None;
    if let Ok(json) = serde_json::to_string(&k) {
        let _ = creds.set_secret(account, &entry, &json);
    }
}

/// `cms` decrypted with the key it was encrypted to: the one in use, or an older one. Mail for a
/// certificate MailCraft doesn't have isn't tried with any (a card would ask for its PIN for
/// nothing).
pub(super) fn decrypt(creds: &dyn Credentials, account: &AccountConfig, cms: &[u8]) -> std::result::Result<(Vec<u8>, bool), String> {
    let cur = kept(creds, account);
    if let Ok(me) = &cur
        && smime::addressed_to(cms, me.certificate())
    {
        let done = me.with_key(|k| smime::decrypt_with(cms, me.certificate(), k));
        forget_wrong_pin(creds, account, None, &done);
        return done;
    }
    for fp in older_list(creds, account) {
        let Ok(me) = kept_older(creds, account, &fp) else { continue };
        if smime::addressed_to(cms, me.certificate()) {
            let done = me.with_key(|k| smime::decrypt_with(cms, me.certificate(), k));
            forget_wrong_pin(creds, account, Some(&fp), &done);
            return done;
        }
    }
    match cur {
        Ok(_) => Err("it was encrypted to a certificate MailCraft doesn't have: import that one too (File › Info › Security), and MailCraft keeps it to read such mail".into()),
        Err(e) if older_list(creds, account).is_empty() => Err(e),
        Err(_) => Err("it was encrypted to a certificate MailCraft doesn't have: import that one (File › Info › Security)".into()),
    }
}

#[cfg(test)]
mod tests;
