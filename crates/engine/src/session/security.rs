//! The session: S/MIME (issue #6). An account's certificate, imported from a `.p12` file (its
//! private key kept by the network layer in the system's keychain, only the certificate here);
//! signed mail checked as it's shown, encrypted mail decrypted for showing and kept in memory
//! only; mail signed and encrypted on its way out; and the certificates of the people who sign
//! their mail remembered, to encrypt to them.
//!
//! As EFAIL taught: only a message protected as a whole counts as signed or encrypted, a
//! decrypted message never loads anything from the web, and encryption that isn't authenticated
//! (CBC, unsigned) says so.

use mailcraft_model::Private;
use mailcraft_smime as smime;

use super::*;

mod addresses;
mod cards;
mod opened;
mod pgp;
mod reading;
mod revocation;
pub(crate) use addresses::OwnCerts;
pub use cards::CardSearch;
pub(crate) use opened::Opened;
pub(crate) use pgp::PgpState;
pub use pgp::{KeySource, KnownKey, PGP_DOC, PgpSettings, Reach, Scheme};
pub use revocation::RevocationCheck;
pub(crate) use revocation::Revocations;

/// An account's S/MIME settings (the account document `smime.json`).
pub const SMIME_DOC: &str = "smime.json";

/// An account's S/MIME: its certificate, what new messages do, whom it can encrypt to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SmimeSettings {
    /// The account's certificate and the ones that issued it (DER); its key is in the keychain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<Vec<u8>>,
    /// Older certificates (DER), kept to read mail encrypted to them after a renewal; their keys
    /// are in the keychain too. Newest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub older: Vec<Vec<u8>>,
    /// Sign new messages.
    pub sign: bool,
    /// Encrypt new messages (when every recipient's certificate is known).
    pub encrypt: bool,
    /// Encrypted mail shows its subject on the way (header protection's `[...]` left out).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_subject: bool,
    /// New messages are encrypted by themselves while every recipient can receive it (S/MIME
    /// or OpenPGP), until the user chooses (issue #6).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub auto_encrypt: bool,
    /// The browser (gap 7): the certificates stay unlocked between visits, their password
    /// encrypted with a key the browser won't let out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_unlocked: bool,
    /// Certificates of people who sent signed mail, by lower-case address.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known: BTreeMap<String, KnownCert>,
}

/// The S/MIME settings of an account with none kept: [`SmimeSettings::default`].
static NO_SMIME: SmimeSettings = SmimeSettings {
    chain: Vec::new(),
    older: Vec::new(),
    sign: false,
    encrypt: false,
    keep_subject: false,
    auto_encrypt: false,
    keep_unlocked: false,
    known: BTreeMap::new(),
};

/// Someone's certificate, from a signed message of theirs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KnownCert {
    pub cert: Vec<u8>,
    /// They said they read AES-GCM.
    pub gcm: bool,
    /// When it was last seen (Unix seconds).
    pub seen: i64,
    /// It came with nothing wrong with it: from an authority this computer trusts, in date.
    pub trusted: bool,
}

/// How a message shown was protected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Security {
    pub lock: Option<Lock>,
    pub signed: Option<SignedBy>,
    /// Its subject and addresses came from inside, where they were protected (RFC 9788).
    pub headers: bool,
    /// It was protected with OpenPGP, not S/MIME.
    pub pgp: bool,
}

impl Security {
    /// Encrypted without anything to tell whether it was changed on the way: CBC, not signed.
    pub fn unauthenticated(&self) -> bool {
        matches!(self.lock, Some(Lock::Open { authenticated: false }))
            && !matches!(self.signed.as_ref().map(|s| &s.seal), Some(Seal::Trusted | Seal::Untrusted(_)))
    }
}

/// An encrypted message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lock {
    /// Being decrypted.
    Opening,
    /// Decrypted; `authenticated`: with AES-GCM.
    Open {
        authenticated: bool,
    },
    Failed(String),
}

/// A signed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedBy {
    pub seal: Seal,
    /// The signer's certificate, when the signature could be read.
    pub cert: Option<smime::CertInfo>,
    /// Or, signed with OpenPGP, the signer's key.
    pub key: Option<mailcraft_pgp::KeyInfo>,
    pub at: Option<i64>,
    /// Made with SHA-1.
    pub weak: bool,
    /// Whether its authority withdrew its certificate, when asked (File › Options › Safety).
    pub revocation: Option<RevocationCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seal {
    /// It holds, by a certificate trusted for the sender.
    Trusted,
    /// It holds, but the certificate can't be trusted for this message, for these reasons.
    Untrusted(Vec<smime::Problem>),
    /// It doesn't hold (the message changed after it was signed), or can't be read: why.
    Broken(String),
    /// OpenPGP: made with a key MailCraft doesn't have (its key ID), so it can't be checked.
    UnknownKey(String),
}

/// A message waiting for [`Event::Protected`] before it goes.
#[derive(Clone, Debug)]
pub(super) struct Protecting {
    compose: Compose,
    from: String,
    recipients: Vec<String>,
}

/// The sender's address, from a message's headers.
fn sender(raw: &[u8]) -> String {
    mime::parse(raw).map(|p| p.from.email.trim().to_lowercase()).unwrap_or_default()
}

/// Whether `raw` is encrypted whole (S/MIME, or OpenPGP in a part of its own), as MailCraft
/// keeps a draft and sends a message: not text that has an encrypted block in it, which is
/// what its writer put there (a draft with one pasted in never opened again otherwise).
pub(crate) fn is_sealed(raw: &[u8]) -> bool {
    matches!(mime::smime::protection(raw), mime::smime::Protection::Encrypted(_))
        || matches!(mime::pgp::protection(raw), mime::pgp::Protection::Encrypted(_))
}

impl Session {
    pub(crate) fn load_smime(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let s: SmimeSettings = store.load_account_doc(&a.config.id, SMIME_DOC).unwrap_or_default();
            self.smime.insert(a.config.id.clone(), s);
        }
        let ids: Vec<String> = self.smime.keys().cloned().collect();
        for id in ids {
            self.read_own_certs(&id);
        }
        self.load_pgp();
    }

    fn save_smime(&mut self, account: &str) {
        // A certificate replaced keeps their count, which the reach shown was told apart by.
        self.forget_reach();
        // Every change to its certificates is kept: what they are is read again with it.
        self.read_own_certs(account);
        let (Some(store), Some(s)) = (&self.store, self.smime.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, SMIME_DOC, s) {
            self.notice(format!("the S/MIME settings could not be saved: {e}"), true);
        }
    }

    /// An account's S/MIME settings, as kept (the defaults for one with none), borrowed: they
    /// hold up to 5000 people's certificates, and most ask only whether it signs.
    pub fn smime_settings(&self, account: &str) -> &SmimeSettings {
        self.smime.get(account).unwrap_or(&NO_SMIME)
    }

    /// Forget an older certificate (`fingerprint`, its [`smime::CertInfo::fingerprint`]) and its
    /// key: mail encrypted to it can't be read any more.
    pub fn forget_older_identity(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.config_of(account)?;
        let s = self.smime.entry(account.to_string()).or_default();
        let before = s.older.len();
        s.older.retain(|c| smime::cert::info(c).map(|i| !i.fingerprint.eq_ignore_ascii_case(fingerprint.trim())).unwrap_or(true));
        if s.older.len() == before {
            return Err(CmdError::Params(format!("no older certificate `{fingerprint}`")));
        }
        self.jobs.push(Job::ForgetOlderIdentity { account: config, fingerprint: fingerprint.trim().to_uppercase() });
        self.save_smime(account);
        Ok(serde_json::json!({ "forgotten": fingerprint }))
    }

    /// Sign and encrypt with an older certificate again; the one in use is kept for reading.
    pub fn use_older_identity(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.config_of(account)?;
        if !self.older_identities(account).iter().any(|i| i.fingerprint.eq_ignore_ascii_case(fingerprint.trim())) {
            return Err(CmdError::Params(format!("no older certificate `{fingerprint}`")));
        }
        self.jobs.push(Job::UseOlderIdentity { account: config, fingerprint: fingerprint.trim().to_uppercase() });
        Ok(serde_json::json!({ "using": fingerprint }))
    }

    /// The roots this computer trusts for email (the system's, from the app).
    pub fn set_trust_roots(&mut self, roots: Vec<Vec<u8>>) {
        self.trust_roots = roots;
    }

    /// The roots a signer's chain may reach to be trusted: this computer's, and the root of each
    /// certificate the user imported (a `.p12` from an authority this computer doesn't know, a
    /// company's own one, brings it: importing it is trusting it, as other mail programs ask).
    /// Borrowed, not copied: they're asked for each signed message shown.
    fn anchors(&self) -> Vec<&[u8]> {
        let mut roots: Vec<&[u8]> = self.trust_roots.iter().map(Vec::as_slice).collect();
        for s in self.smime.values() {
            for c in s.chain.iter().skip(1).filter(|c| smime::cert::self_signed(c)) {
                if !roots.contains(&c.as_slice()) {
                    roots.push(c);
                }
            }
        }
        roots
    }

    /// Keep the certificate and key in the `.p12` file at `path` for `account`.
    pub fn import_identity(&mut self, account: &str, path: &str, password: &str) -> CmdResult {
        let data = crate::cmd::read_capped(path, 1 << 20)?;
        self.import_identity_data(account, data, password)
    }

    /// [`Session::import_identity`] from the file's bytes (the browser's picker gives no path).
    /// Empty: the one the browser kept, unlocked with `password`.
    pub fn import_identity_data(&mut self, account: &str, data: Vec<u8>, password: &str) -> CmdResult {
        let config = self.config_of(account)?;
        if data.len() > 1 << 20 {
            return Err(CmdError::Params("that file is too big for a certificate".into()));
        }
        self.jobs.push(Job::ImportIdentity { account: config, data, password: mailcraft_model::Secret(password.to_string()) });
        Ok(serde_json::json!({ "importing": account }))
    }

    /// Forget the account's certificate (and the keychain's key); older ones kept for reading stay.
    pub fn forget_identity(&mut self, account: &str) -> CmdResult {
        let config = self.config_of(account)?;
        self.jobs.push(Job::ForgetIdentity { account: config });
        let keeps_pgp = self.pgp_key(account).is_some();
        let s = self.smime.entry(account.to_string()).or_default();
        s.chain.clear();
        // With an OpenPGP key left, new messages are still signed and encrypted as chosen.
        if !keeps_pgp {
            s.sign = false;
            s.encrypt = false;
            s.auto_encrypt = false;
        }
        self.save_smime(account);
        Ok(serde_json::json!({ "forgotten": account }))
    }

    /// The browser (gap 7): keep `account`'s certificates unlocked between visits, or stop (their
    /// kept password forgotten). The desktop keeps keys in the system's keychain already.
    pub fn keep_unlocked(&mut self, account: &str, on: bool) -> CmdResult {
        let config = self.config_of(account)?;
        if !self.jmap_only {
            return Err(CmdError::Disabled("the desktop app keeps certificates in the system's keychain: they stay unlocked".into()));
        }
        self.smime.entry(account.to_string()).or_default().keep_unlocked = on;
        self.save_smime(account);
        self.jobs.push(Job::KeepUnlocked { account: config, on });
        Ok(serde_json::json!({ "keepUnlocked": on }))
    }

    /// Whether new messages from `account` are signed or encrypted.
    pub fn set_smime_defaults(
        &mut self,
        account: &str,
        sign: Option<bool>,
        encrypt: Option<bool>,
        keep_subject: Option<bool>,
        auto_encrypt: Option<bool>,
    ) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let has = self.can_protect(account);
        let s = self.smime.entry(account.to_string()).or_default();
        if (sign == Some(true) || encrypt == Some(true) || auto_encrypt == Some(true)) && !has {
            return Err(CmdError::Disabled("import a certificate or an OpenPGP key for the account first".into()));
        }
        if let Some(v) = auto_encrypt {
            s.auto_encrypt = v;
        }
        if let Some(v) = sign {
            s.sign = v;
        }
        if let Some(v) = encrypt {
            s.encrypt = v;
        }
        if let Some(v) = keep_subject {
            s.keep_subject = v;
        }
        let out = serde_json::json!({ "sign": s.sign, "encrypt": s.encrypt, "keepSubject": s.keep_subject, "autoEncrypt": s.auto_encrypt });
        self.save_smime(account);
        Ok(out)
    }

    /// Whether the account can sign and encrypt at all: it has a certificate or an OpenPGP key.
    pub fn can_protect(&self, account: &str) -> bool {
        self.identity(account).is_some() || self.pgp_key(account).is_some()
    }

    /// A new message takes its account's settings: signed only from an address a certificate
    /// or a key is for (issue #81).
    pub(super) fn smime_defaults(&self, c: &mut Compose) {
        let s = self.smime_settings(&c.account);
        if self.can_protect(&c.account) {
            c.sign |= s.sign && self.can_sign_from(&c.account, &c.from);
            c.encrypt |= s.encrypt;
            // Encryption follows the recipients, unless it's on already.
            c.encrypt_auto = s.auto_encrypt && !c.encrypt;
        }
    }

    /// `account`'s messages being signed or encrypted are let go (the account is gone).
    pub(crate) fn forget_protecting(&mut self, account: &str) {
        self.protecting.retain(|_, p| p.compose.account != account);
    }

    /// The certificate to encrypt to `email` with, and whether they read AES-GCM.
    fn cert_for(&self, account: &str, email: &str) -> Option<(Vec<u8>, bool)> {
        let email = email.trim().to_lowercase();
        let s = self.smime.get(account)?;
        // One of the account's own addresses: its own certificate, that address's when it
        // has one.
        if self.account(account).is_some_and(|a| a.config.has_address(&email)) {
            return self.own_cert_for(account, &email).map(|(c, _)| c).or_else(|| s.chain.first().map(Vec::as_slice)).map(|c| (c.to_vec(), true));
        }
        let k = s.known.get(&email)?;
        // Out of date certificates aren't encrypted to, nor one its authority withdrew.
        let info = smime::cert::info(&k.cert).ok()?;
        let now = self.clock.now;
        (info.not_before <= now && info.not_after >= now && !self.heard_revoked(&k.cert)).then(|| (k.cert.clone(), k.gcm))
    }

    /// Before `c` goes: what signing and encrypting it takes. `None` when it's neither; else the
    /// job to sign and encrypt `raw`.
    pub(super) fn protect_job(&mut self, c: &Compose, raw: &[u8], recipients: &[String], from: &str) -> Result<Option<Job>, CmdError> {
        if !c.sign && !c.encrypt {
            return Ok(None);
        }
        if self.scheme_for(c, recipients) == Scheme::Pgp {
            return self.pgp_protect_job(c, raw, recipients, from).map(Some);
        }
        let s = self.smime_settings(&c.account);
        let config = self.account(&c.account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        // The certificate of the address it's from (issue #81): one that names another address
        // would sign as someone who isn't the sender. Only encrypted, the account's own does
        // for the copy it keeps.
        let own = match self.own_cert_for(&c.account, from).map(|(c, _)| c.to_vec()) {
            Some(own) => own,
            None if s.chain.is_empty() && s.older.is_empty() => {
                return Err(CmdError::Params(
                    "there's no certificate for this account to sign or encrypt with: import one in File › Info › Security".into(),
                ));
            }
            None if c.sign => {
                return Err(CmdError::Params(format!(
                    "there's no certificate for {from} to sign with: import one that names it in File › Info › Security, or send without signing"
                )));
            }
            None => s.chain.first().or(s.older.first()).cloned().unwrap_or_default(),
        };
        // A certificate out of date signs nothing anyone would trust.
        if let Ok(info) = smime::cert::info(&own)
            && (info.not_after < self.clock.now || info.not_before > self.clock.now)
        {
            return Err(CmdError::Params(format!(
                "your certificate is valid from {} to {}: replace it in File › Info › Security, or send without signing or encryption",
                mailcraft_model::dates::full_time(info.not_before, &self.clock),
                mailcraft_model::dates::full_time(info.not_after, &self.clock)
            )));
        }
        let mut encrypt_to = Vec::new();
        let mut gcm = true;
        if c.encrypt {
            let mut missing = Vec::new();
            for r in recipients {
                match self.cert_for(&c.account, r) {
                    Some((cert, reads_gcm)) => {
                        gcm &= reads_gcm;
                        encrypt_to.push(cert);
                    }
                    None => missing.push(r.clone()),
                }
            }
            if !missing.is_empty() {
                return Err(CmdError::Params(format!(
                    "there's no certificate to encrypt to {}: a signed message from them brings one, or send it without encryption",
                    missing.join(", ")
                )));
            }
            // The Sent copy reads too.
            encrypt_to.push(own.clone());
        }
        let job = Job::Protect {
            account: config,
            id: c.id,
            raw: Private(raw.to_vec()),
            sign: c.sign,
            encrypt_to,
            gcm: gcm && c.encrypt,
            hide_subject: !s.keep_subject,
            signer: Some(own),
        };
        self.protecting.insert(c.id, Protecting { compose: c.clone(), from: from.to_string(), recipients: recipients.to_vec() });
        Ok(Some(job))
    }

    pub(super) fn apply_smime(&mut self, e: Event) {
        match e {
            Event::Identity { account, chain, older, for_reading, error } => match error {
                Some(e) => self.notice(format!("The certificate couldn't be kept: {e}"), true),
                None => {
                    let s = self.smime.entry(account.clone()).or_default();
                    // The one just kept for reading is put first among the older ones.
                    let reading = older.first().filter(|_| for_reading).cloned();
                    let had = !s.chain.is_empty();
                    s.older = older;
                    if !chain.is_empty() {
                        s.chain = chain;
                    }
                    // A first certificate is imported to be used: new messages are signed from
                    // now on. A renewal or a swap keeps what the user chose.
                    if !had && reading.is_none() {
                        s.sign = true;
                    }
                    let signing = s.sign;
                    self.save_smime(&account);
                    self.retry_failed_decrypts();
                    let who = |i: smime::CertInfo| i.emails.first().cloned().unwrap_or(i.name);
                    match reading.and_then(|c| smime::cert::info(&c).ok()) {
                        Some(old) => {
                            let until = mailcraft_model::dates::full_time(old.not_after, &self.clock);
                            self.notice(
                                format!(
                                    "The certificate for {} (valid until {until}) is older than the one in use, so it's kept to read mail encrypted to it",
                                    who(old)
                                ),
                                false,
                            );
                        }
                        None => {
                            let who = self.identity(&account).cloned().map(who).unwrap_or_default();
                            let then = if signing { "new messages are signed" } else { "it signs and encrypts from now on" };
                            self.notice(format!("Your certificate for {who} is ready: {then}"), false);
                        }
                    }
                }
            },
            Event::Protected { account: _, id, raw, error } => {
                let Some(p) = self.protecting.remove(&id) else { return };
                match (raw, error) {
                    (Some(Private(raw)), None) => {
                        if let Err(e) = self.deliver(&p.compose, raw, p.from, p.recipients) {
                            self.send_problem(id, e.to_string());
                        }
                    }
                    (_, e) => self.send_problem(id, format!("it couldn't be signed or encrypted: {}", e.unwrap_or_default())),
                }
            }
            Event::Decrypted { key, content, authenticated, error } => {
                // One no longer asked for (the Outbox's rows moved meanwhile) is another's now.
                if !self.decrypt_awaited(&key) {
                    return;
                }
                match (content, error) {
                    (Some(Private(entity)), None) => self.keep_opened(&key, entity, authenticated),
                    (_, e) => {
                        self.decrypt_errors.insert(key.clone(), e.unwrap_or_else(|| "it can't be decrypted".into()));
                    }
                }
                // Shown again, now with what was inside.
                self.show_again(&key);
            }
            Event::Revocation { cert, status } => self.heard_revocation(cert, status),
            Event::HeldKeys { keys, error } => self.heard_held_keys(keys, error),
            e @ (Event::PgpKeys { .. } | Event::PgpDecrypted { .. }) => self.apply_pgp(e),
            _ => {}
        }
    }

    /// Keep what was decrypted of `key` (the MIME entity inside, and whether it was
    /// authenticated) in memory. Past [`opened::OPENED_MAX`] the oldest goes, with what its
    /// signature said: never one shown, which would ask for its key (a keychain or card prompt)
    /// again.
    fn keep_opened(&mut self, key: &MessageKey, entity: Vec<u8>, authenticated: bool) {
        let (reading, windows) = (&self.reading, &self.windows);
        let shown = |k: &MessageKey| reading.as_ref().is_some_and(|r| r.key == *k) || windows.iter().any(|w| w.key == *k);
        if let Some(gone) = self.opened.keep(key, entity, authenticated, shown) {
            self.pgp.forget_message(&gone);
        }
    }

    /// Mail that couldn't be read for want of a key is tried again, now there's one (the one
    /// shown at once).
    fn retry_failed_decrypts(&mut self) {
        let failed: Vec<MessageKey> = self.decrypt_errors.drain().map(|(k, _)| k).collect();
        for k in failed {
            self.show_again(&k);
        }
    }

    /// A message shown (in the reading pane or a window of its own) shown again, its protection
    /// taken off and checked anew.
    pub(crate) fn show_again(&mut self, key: &MessageKey) {
        let shown = self.reading.as_ref().is_some_and(|r| r.key == *key) || self.windows.iter().any(|w| w.key == *key);
        if shown && let Some(raw) = self.body_raw(key) {
            for w in self.windows.iter_mut().filter(|w| w.key == *key) {
                w.parsed = None;
            }
            self.show_body(key, &raw);
        }
    }

    /// The message being written (or set aside) `id` couldn't go: said over it.
    fn send_problem(&mut self, id: u64, error: String) {
        // Closed while it was being signed or encrypted: open again, to be sent or kept (as one
        // the server refused is). Only said, its text was nowhere any more.
        if let Some(at) = self.sent_away.iter().position(|c| c.id == id) {
            let mut c = self.sent_away.remove(at);
            c.window = true;
            self.open_compose(c);
        }
        let mut found = false;
        for c in self.compose.iter_mut().chain(self.composes.iter_mut()).filter(|c| c.id == id) {
            c.sending = false;
            c.error = Some(error.clone());
            found = true;
        }
        if !found {
            self.notice(format!("A message couldn't be sent: {error}"), true);
        }
    }

    /// Whether the message shown is one that was encrypted (nothing from the web loads in it).
    pub(super) fn reading_encrypted(&self) -> bool {
        self.reading.as_ref().and_then(|r| r.security.as_ref()).is_some_and(|s| s.lock.is_some())
    }
}
