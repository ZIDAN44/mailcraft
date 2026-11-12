//! The session: OpenPGP (issue #36), beside S/MIME and shown the same way. An account's own keys
//! (imported from a key file; their secret parts kept by the network layer in the system's
//! keychain, only the public parts here; the one in use first, older ones kept to decrypt), and
//! its correspondents' keys, learned from the mail they send (Autocrypt headers, attached keys
//! for their own address) or imported from a file. Reading ([`reading`]): PGP/MIME and inline
//! PGP, signatures checked as the message is shown, encrypted mail decrypted for showing and
//! kept in memory only. Writing ([`writing`]): PGP/MIME signed and encrypted (to the sender too,
//! so Sent Items reads), the account's key sent along in an `Autocrypt:` header, and whether the
//! message being written can go encrypted to everyone in it (issue #6).

use mailcraft_model::{PgpSignature, PgpWork, Private};
use mailcraft_pgp as openpgp;

use super::*;

mod reading;
mod writing;
pub(crate) use reading::Step;

/// An account's OpenPGP settings (the account document `pgp.json`).
pub const PGP_DOC: &str = "pgp.json";

/// The most correspondents' keys kept for an account; the one seen longest ago goes first.
const KNOWN_MAX: usize = 5000;

/// An account's OpenPGP: its own keys and its correspondents'.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PgpSettings {
    /// The account's own keys' public parts (binary), the one that signs and decrypts first,
    /// then older ones kept to decrypt; their secret parts are in the keychain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Vec<u8>>,
    /// Correspondents' keys, by lower-case address.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known: BTreeMap<String, KnownKey>,
    /// The account's key isn't sent with its mail (an `Autocrypt:` header).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_autocrypt: bool,
}

/// Someone's key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KnownKey {
    /// The key (binary, as received).
    pub key: Vec<u8>,
    pub info: openpgp::KeyInfo,
    pub from: KeySource,
    /// When the message that brought it was written, or when it was imported (Unix seconds).
    pub seen: i64,
    /// They'd rather have mail encrypted (Autocrypt's `prefer-encrypt=mutual`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mutual: bool,
}

/// Where a correspondent's key came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    /// Their mail's `Autocrypt:` header (a newer message's replaces it).
    #[default]
    Autocrypt,
    /// A key attached to their mail, for their own address.
    Attached,
    /// Imported by the user from a file (never replaced by one from mail).
    Imported,
}

/// How a message is signed and encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scheme {
    Smime,
    Pgp,
}

/// Whether the message being written can go encrypted to everyone in it (issue #6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The account has neither a certificate nor an OpenPGP key.
    NotSetUp,
    /// It has no recipients yet.
    NoOne,
    /// Every recipient can receive it, with this.
    Everyone(Scheme),
    /// These recipients can't (with what reaches the most of them).
    Missing { scheme: Scheme, who: Vec<String> },
}

/// The session's OpenPGP state.
#[derive(Debug, Default)]
pub(crate) struct PgpState {
    settings: HashMap<String, PgpSettings>,
    /// Messages being decrypted: the keys their signature is checked with, and their sender.
    asked: HashMap<MessageKey, (Vec<Vec<u8>>, String)>,
    /// What signatures inside decrypted messages say (in memory, as what they held is).
    signed: HashMap<MessageKey, SignedBy>,
    /// The last [`Reach`] worked out, and a digest of what it was worked out from.
    reach: Option<(u64, Reach)>,
    /// What each account's own keys are (read once, as the window asks every frame).
    own: HashMap<String, Vec<openpgp::KeyInfo>>,
}

impl Session {
    pub(crate) fn load_pgp(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        for id in ids {
            let s: PgpSettings = store.load_account_doc(&id, PGP_DOC).unwrap_or_default();
            self.pgp.settings.insert(id.clone(), s);
            self.read_own(&id);
        }
    }

    fn read_own(&mut self, account: &str) {
        let own = self.pgp.settings.get(account).map(|s| s.keys.iter().filter_map(|k| openpgp::info(k).ok()).collect()).unwrap_or_default();
        self.pgp.own.insert(account.to_string(), own);
    }

    fn save_pgp(&mut self, account: &str) {
        self.pgp.reach = None;
        self.read_own(account);
        let (Some(store), Some(s)) = (&self.store, self.pgp.settings.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PGP_DOC, s) {
            self.notice(format!("the OpenPGP settings could not be saved: {e}"), true);
        }
    }

    /// An account's OpenPGP settings.
    pub fn pgp_settings(&self, account: &str) -> PgpSettings {
        self.pgp.settings.get(account).cloned().unwrap_or_default()
    }

    /// The account's key in use, when it has one.
    pub fn pgp_key(&self, account: &str) -> Option<openpgp::KeyInfo> {
        self.pgp.own.get(account).and_then(|o| o.first()).cloned()
    }

    /// The account's older keys, kept to decrypt mail sent to them.
    pub fn pgp_older_keys(&self, account: &str) -> Vec<openpgp::KeyInfo> {
        self.pgp.own.get(account).map(|o| o.iter().skip(1).cloned().collect()).unwrap_or_default()
    }

    fn config_of(&self, account: &str) -> Result<AccountConfig, CmdError> {
        self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))
    }

    /// Keep the secret key in the file at `path` (opened with `passphrase`) for `account`.
    pub fn import_pgp_key(&mut self, account: &str, path: &str, passphrase: &str) -> CmdResult {
        let data = crate::cmd::read_capped(path, 1 << 20)?;
        self.import_pgp_key_data(account, data, passphrase)
    }

    /// [`Session::import_pgp_key`] from the file's bytes.
    pub fn import_pgp_key_data(&mut self, account: &str, data: Vec<u8>, passphrase: &str) -> CmdResult {
        let config = self.config_of(account)?;
        if data.len() > 1 << 20 {
            return Err(CmdError::Params("that file is too big for an OpenPGP key".into()));
        }
        let work = PgpWork::Import { file: Private(data), passphrase: mailcraft_model::Secret(passphrase.to_string()) };
        self.jobs.push(Job::Pgp { account: config, work });
        Ok(serde_json::json!({ "importing": account }))
    }

    /// Find one of the account's own keys by fingerprint (spaces and case don't matter).
    fn own_fingerprint(&self, account: &str, fingerprint: &str) -> Result<String, CmdError> {
        let wanted: String = fingerprint.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
        let s = self.pgp.settings.get(account);
        s.into_iter()
            .flat_map(|s| s.keys.iter())
            .filter_map(|k| openpgp::info(k).ok())
            .find(|i| !wanted.is_empty() && (i.fingerprint == wanted || i.key_id == wanted))
            .map(|i| i.fingerprint)
            .ok_or_else(|| CmdError::Params(format!("no OpenPGP key `{fingerprint}` for this account")))
    }

    /// Forget one of the account's keys (its secret part goes from the keychain too).
    pub fn forget_pgp_key(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.config_of(account)?;
        let fp = self.own_fingerprint(account, fingerprint)?;
        let s = self.pgp.settings.entry(account.to_string()).or_default();
        s.keys.retain(|k| openpgp::info(k).map(|i| i.fingerprint != fp).unwrap_or(true));
        let none_left = s.keys.is_empty() && self.identity(account).is_none();
        if none_left {
            let m = self.smime.entry(account.to_string()).or_default();
            m.sign = false;
            m.encrypt = false;
            m.auto_encrypt = false;
            self.save_smime(account);
        }
        self.jobs.push(Job::Pgp { account: config, work: PgpWork::Forget { fingerprint: fp.clone() } });
        self.save_pgp(account);
        Ok(serde_json::json!({ "forgotten": fp }))
    }

    /// Sign and decrypt with one of the account's older keys again.
    pub fn use_pgp_key(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.config_of(account)?;
        let fp = self.own_fingerprint(account, fingerprint)?;
        self.jobs.push(Job::Pgp { account: config, work: PgpWork::Use { fingerprint: fp.clone() } });
        Ok(serde_json::json!({ "using": fp }))
    }

    /// Whether `account`'s mail may send its key along (`autocrypt`).
    pub fn set_pgp_options(&mut self, account: &str, autocrypt: Option<bool>) -> CmdResult {
        self.config_of(account)?;
        let s = self.pgp.settings.entry(account.to_string()).or_default();
        if let Some(on) = autocrypt {
            s.no_autocrypt = !on;
        }
        let out = serde_json::json!({ "autocrypt": !s.no_autocrypt });
        self.save_pgp(account);
        Ok(out)
    }

    /// Keep the correspondents' keys in the file at `path`.
    pub fn import_public_keys(&mut self, account: &str, path: &str) -> CmdResult {
        let data = crate::cmd::read_capped(path, 8 << 20)?;
        self.import_public_keys_data(account, &data)
    }

    /// Keep the correspondents' keys in `data` (a key file or an attached key, armored or
    /// binary), each for the addresses its user IDs name. Importing is choosing: a key imported
    /// isn't replaced by one that comes in mail.
    pub fn import_public_keys_data(&mut self, account: &str, data: &[u8]) -> CmdResult {
        self.config_of(account)?;
        let keys = openpgp::public_keys(data).map_err(|e| CmdError::Params(e.to_string()))?;
        let mut added = Vec::new();
        for k in keys {
            let Ok(info) = openpgp::info(&k) else { continue };
            for email in info.emails.clone() {
                self.remember_key(account, &email, &k, &info, KeySource::Imported, self.clock.now, false);
                added.push(email);
            }
        }
        if added.is_empty() {
            return Err(CmdError::Params("there's no key in it for an address".into()));
        }
        self.save_pgp(account);
        self.notice(format!("OpenPGP keys kept for {}", added.join(", ")), false);
        Ok(serde_json::json!({ "added": added }))
    }

    /// Forget the key MailCraft has for `email`.
    pub fn forget_public_key(&mut self, account: &str, email: &str) -> CmdResult {
        self.config_of(account)?;
        let email = email.trim().to_lowercase();
        let gone = self.pgp.settings.get_mut(account).and_then(|s| s.known.remove(&email)).is_some();
        if !gone {
            return Err(CmdError::Params(format!("there's no OpenPGP key for `{email}`")));
        }
        self.save_pgp(account);
        Ok(serde_json::json!({ "forgotten": email }))
    }

    /// Remember `key` for `email`; true when something changed (the caller saves). One from mail
    /// replaces another from mail only when its message is no older (Autocrypt Level 1 §2.3), and
    /// never one the user imported.
    fn remember_key(&mut self, account: &str, email: &str, key: &[u8], info: &openpgp::KeyInfo, from: KeySource, seen: i64, mutual: bool) -> bool {
        let email = email.trim().to_lowercase();
        if email.is_empty() || key.len() > 1 << 20 {
            return false;
        }
        let s = self.pgp.settings.entry(account.to_string()).or_default();
        if let Some(k) = s.known.get_mut(&email) {
            let by_hand = from == KeySource::Imported;
            if k.from == KeySource::Imported && !by_hand {
                return false;
            }
            if k.info.fingerprint == info.fingerprint {
                // The same key, perhaps with newer self-signatures (a later expiry).
                if seen < k.seen && !by_hand {
                    return false;
                }
                let changed = k.key != key || k.mutual != mutual || k.from != from;
                k.key = key.to_vec();
                k.info = info.clone();
                k.seen = seen.max(k.seen);
                k.mutual = mutual;
                k.from = if by_hand { from } else { k.from };
                return changed;
            }
            if seen < k.seen && !by_hand {
                return false;
            }
        } else if s.known.len() >= KNOWN_MAX
            && let Some(oldest) = s.known.iter().min_by_key(|(_, k)| k.seen).map(|(e, _)| e.clone())
        {
            s.known.remove(&oldest);
        }
        s.known.insert(email, KnownKey { key: key.to_vec(), info: info.clone(), from, seen, mutual });
        true
    }

    /// The keys a message brings for its sender: its `Autocrypt:` header (for the address it
    /// comes from), and attached keys with a user ID for that address. Mail from the account's
    /// own addresses brings nothing.
    pub(crate) fn learn_keys(&mut self, account: &str, raw: &[u8]) {
        let headers = mime::pgp::autocrypt(raw);
        let parts = mime::pgp::key_parts(raw);
        if headers.is_empty() && parts.is_empty() {
            return;
        }
        let Some(p) = mime::parse(raw) else { return };
        let from = p.from.email.trim().to_lowercase();
        if from.is_empty() || self.accounts.iter().any(|a| a.config.has_address(&from)) {
            return;
        }
        let seen = p.date;
        let mut changed = false;
        for h in headers {
            let Some(a) = openpgp::autocrypt::parse(&h) else { continue };
            if a.addr != from {
                continue;
            }
            if let Ok(info) = openpgp::info(&a.keydata) {
                changed |= self.remember_key(account, &from, &a.keydata, &info, KeySource::Autocrypt, seen, a.mutual);
            }
        }
        for part in parts {
            for k in openpgp::public_keys(&part).unwrap_or_default().into_iter().take(4) {
                if let Ok(info) = openpgp::info(&k)
                    && info.is_for(&from)
                {
                    changed |= self.remember_key(account, &from, &k, &info, KeySource::Attached, seen, false);
                }
            }
        }
        if changed {
            self.save_pgp(account);
        }
    }

    /// The keys a signature from `from` may be checked with: theirs, and the account's own for
    /// its own mail.
    fn keys_for(&self, account: &str, from: &str) -> Vec<Vec<u8>> {
        let from = from.trim().to_lowercase();
        let Some(s) = self.pgp.settings.get(account) else { return Vec::new() };
        let mut out: Vec<Vec<u8>> = s.known.get(&from).map(|k| k.key.clone()).into_iter().collect();
        if self.accounts.iter().any(|a| a.config.email.trim().eq_ignore_ascii_case(&from)) {
            out.extend(s.keys.iter().cloned());
        }
        out
    }

    pub(super) fn apply_pgp(&mut self, e: Event) {
        match e {
            Event::PgpKeys { account, keys, imported, error } => {
                if error.is_none() || !keys.is_empty() {
                    let had = self.can_protect(&account);
                    self.pgp.settings.entry(account.clone()).or_default().keys = keys;
                    self.save_pgp(&account);
                    // A first key is imported to be used: new messages are signed from now on.
                    if !had && imported.is_some() {
                        self.smime.entry(account.clone()).or_default().sign = true;
                        self.save_smime(&account);
                    }
                }
                if let Some(e) = error {
                    self.notice(format!("The OpenPGP key couldn't be kept: {e}"), true);
                    return;
                }
                if imported.is_some() {
                    let who = self.pgp_key(&account).map(|i| i.emails.first().cloned().unwrap_or(i.name)).unwrap_or_default();
                    let then = if self.smime_settings(&account).sign { "new messages are signed" } else { "it signs and decrypts from now on" };
                    self.notice(format!("Your OpenPGP key for {who} is ready: {then}"), false);
                }
                // Mail that couldn't be read for want of a key is tried again.
                let failed: Vec<MessageKey> = self.decrypt_errors.drain().map(|(k, _)| k).collect();
                for k in failed {
                    self.show_again(&k);
                }
            }
            Event::PgpDecrypted { key, content, signature, error } => {
                self.opening.remove(&key);
                let (keys, from) = self.pgp.asked.remove(&key).unwrap_or_default();
                match (content, error) {
                    (Some(Private(entity)), None) => {
                        if self.opened.len() >= OPENED_MAX
                            && let Some(k) = self.opened.keys().next().cloned()
                        {
                            self.opened.remove(&k);
                            self.pgp.signed.remove(&k);
                        }
                        self.opened.insert(key.clone(), (entity, true));
                        if let Some(sig) = signature {
                            let by = self.pgp_signed_by(&sig, &keys, &from);
                            self.pgp.signed.insert(key.clone(), by);
                        }
                    }
                    (_, e) => {
                        self.decrypt_errors.insert(key.clone(), e.unwrap_or_else(|| "it can't be decrypted".into()));
                    }
                }
                self.show_again(&key);
            }
            _ => {}
        }
    }
}
