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

/// An account's S/MIME settings (the account document `smime.json`).
pub const SMIME_DOC: &str = "smime.json";

/// The most decrypted messages held in memory at once.
const OPENED_MAX: usize = 64;

/// An account's S/MIME: its certificate, what new messages do, whom it can encrypt to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SmimeSettings {
    /// The account's certificate and the ones that issued it (DER); its key is in the keychain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<Vec<u8>>,
    /// Sign new messages.
    pub sign: bool,
    /// Encrypt new messages (when every recipient's certificate is known).
    pub encrypt: bool,
    /// Certificates of people who sent signed mail, by lower-case address.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known: BTreeMap<String, KnownCert>,
}

/// Someone's certificate, from a signed message of theirs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KnownCert {
    pub cert: Vec<u8>,
    /// They said they read AES-GCM.
    pub gcm: bool,
    /// When it was last seen (Unix seconds).
    pub seen: i64,
}

/// How a message shown was protected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Security {
    pub lock: Option<Lock>,
    pub signed: Option<SignedBy>,
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
    pub at: Option<i64>,
    /// Made with SHA-1.
    pub weak: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seal {
    /// It holds, by a certificate trusted for the sender.
    Trusted,
    /// It holds, but the certificate can't be trusted for this message, for these reasons.
    Untrusted(Vec<smime::Problem>),
    /// It doesn't hold (the message changed after it was signed), or can't be read: why.
    Broken(String),
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

impl Session {
    pub(crate) fn load_smime(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let s: SmimeSettings = store.load_account_doc(&a.config.id, SMIME_DOC).unwrap_or_default();
            self.smime.insert(a.config.id.clone(), s);
        }
    }

    fn save_smime(&mut self, account: &str) {
        let (Some(store), Some(s)) = (&self.store, self.smime.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, SMIME_DOC, s) {
            self.notice(format!("the S/MIME settings could not be saved: {e}"), true);
        }
    }

    /// An account's S/MIME settings.
    pub fn smime_settings(&self, account: &str) -> SmimeSettings {
        self.smime.get(account).cloned().unwrap_or_default()
    }

    /// The account's certificate, when it has one.
    pub fn identity(&self, account: &str) -> Option<smime::CertInfo> {
        self.smime.get(account).and_then(|s| s.chain.first()).and_then(|c| smime::cert::info(c).ok())
    }

    /// The roots this computer trusts for email (the system's, from the app).
    pub fn set_trust_roots(&mut self, roots: Vec<Vec<u8>>) {
        self.trust_roots = roots;
    }

    /// The roots a signer's chain may reach to be trusted: this computer's, and the root of each
    /// certificate the user imported (a `.p12` from an authority this computer doesn't know, a
    /// company's own one, brings it: importing it is trusting it, as other mail programs ask).
    fn anchors(&self) -> Vec<Vec<u8>> {
        let mut roots = self.trust_roots.clone();
        for s in self.smime.values() {
            for c in s.chain.iter().skip(1).filter(|c| smime::cert::self_signed(c)) {
                if !roots.contains(c) {
                    roots.push(c.clone());
                }
            }
        }
        roots
    }

    /// Keep the certificate and key in the `.p12` file at `path` for `account`.
    pub fn import_identity(&mut self, account: &str, path: &str, password: &str) -> CmdResult {
        let config = self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        let data = crate::cmd::read_capped(path, 1 << 20)?;
        self.jobs.push(Job::ImportIdentity { account: config, data, password: mailcraft_model::Secret(password.to_string()) });
        Ok(serde_json::json!({ "importing": account }))
    }

    /// Forget the account's certificate (and the keychain's key).
    pub fn forget_identity(&mut self, account: &str) -> CmdResult {
        let config = self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        self.jobs.push(Job::ForgetIdentity { account: config });
        let s = self.smime.entry(account.to_string()).or_default();
        s.chain.clear();
        s.sign = false;
        s.encrypt = false;
        self.save_smime(account);
        Ok(serde_json::json!({ "forgotten": account }))
    }

    /// Whether new messages from `account` are signed or encrypted.
    pub fn set_smime_defaults(&mut self, account: &str, sign: Option<bool>, encrypt: Option<bool>) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let has = self.identity(account).is_some();
        let s = self.smime.entry(account.to_string()).or_default();
        if (sign == Some(true) || encrypt == Some(true)) && !has {
            return Err(CmdError::Disabled("import a certificate for the account first".into()));
        }
        if let Some(v) = sign {
            s.sign = v;
        }
        if let Some(v) = encrypt {
            s.encrypt = v;
        }
        let out = serde_json::json!({ "sign": s.sign, "encrypt": s.encrypt });
        self.save_smime(account);
        Ok(out)
    }

    /// A new message takes its account's settings.
    pub(super) fn smime_defaults(&self, c: &mut Compose) {
        let s = self.smime_settings(&c.account);
        if !s.chain.is_empty() {
            c.sign |= s.sign;
            c.encrypt |= s.encrypt;
        }
    }

    /// What `raw` shows once its protection is taken off, and what that protection was: a
    /// signature checked, the content inside an encryption (when it's been decrypted; if not, it's
    /// asked for and the message stays as it is until it comes). `key` is the message.
    pub(super) fn unwrap(&mut self, key: &MessageKey, raw: &[u8]) -> (Vec<u8>, Option<Security>) {
        let mut cur = raw.to_vec();
        let mut sec = Security::default();
        let mut any = false;
        for _ in 0..4 {
            match mime::smime::protection(&cur) {
                mime::smime::Protection::None => break,
                mime::smime::Protection::Signed { content, signature } => {
                    any = true;
                    if sec.signed.is_none() {
                        sec.signed = Some(self.check_signature(&key.account, &signature, Some(&content), &sender(&cur)));
                    }
                    cur = mime::smime::with_entity(&cur, &content);
                }
                mime::smime::Protection::Opaque(cms) => {
                    any = true;
                    if sec.signed.is_none() {
                        sec.signed = Some(self.check_signature(&key.account, &cms, None, &sender(&cur)));
                    }
                    match smime::carried(&cms) {
                        Ok(content) => cur = mime::smime::with_entity(&cur, &content),
                        Err(_) => break,
                    }
                }
                mime::smime::Protection::Encrypted(cms) => {
                    any = true;
                    // One encryption is decrypted; another inside it stays as it is.
                    if sec.lock.is_some() {
                        break;
                    }
                    if let Some((entity, authenticated)) = self.opened.get(key) {
                        sec.lock = Some(Lock::Open { authenticated: *authenticated });
                        cur = mime::smime::with_entity(&cur, entity);
                        continue;
                    }
                    sec.lock = Some(match self.decrypt_errors.get(key) {
                        Some(e) => Lock::Failed(e.clone()),
                        None => self.ask_decrypt(key, cms),
                    });
                    break;
                }
            }
        }
        (cur, any.then_some(sec))
    }

    /// A message as it reads: what [`Session::unwrap`] shows, without checking anything again.
    pub fn readable_raw(&self, key: &MessageKey) -> Option<Vec<u8>> {
        let mut cur = self.store.as_ref()?.read_body(key)?;
        for _ in 0..4 {
            cur = match mime::smime::protection(&cur) {
                mime::smime::Protection::None => break,
                mime::smime::Protection::Signed { content, .. } => mime::smime::with_entity(&cur, &content),
                mime::smime::Protection::Opaque(cms) => match smime::carried(&cms) {
                    Ok(c) => mime::smime::with_entity(&cur, &c),
                    Err(_) => break,
                },
                mime::smime::Protection::Encrypted(_) => match self.opened.get(key) {
                    Some((entity, _)) => mime::smime::with_entity(&cur, entity),
                    None => break,
                },
            };
        }
        Some(cur)
    }

    fn ask_decrypt(&mut self, key: &MessageKey, cms: Vec<u8>) -> Lock {
        let Some(config) = self.account(&key.account).map(|a| a.config.clone()) else { return Lock::Failed("its account is gone".into()) };
        if self.identity(&key.account).is_none() {
            return Lock::Failed("this account has no certificate to decrypt it with (File › Info › Security)".into());
        }
        if self.opening.insert(key.clone()) {
            self.jobs.push(Job::Decrypt { account: config, key: key.clone(), cms });
        }
        Lock::Opening
    }

    /// A signature checked: whether it holds, and whether its certificate is to be trusted for
    /// mail from `from`. A certificate that holds for its sender is remembered, to encrypt to them.
    fn check_signature(&mut self, account: &str, cms: &[u8], content: Option<&[u8]>, from: &str) -> SignedBy {
        match smime::verify(cms, content) {
            Ok((signed, _)) => {
                let roots = self.anchors();
                let problems = smime::cert::check(&signed.signer, &signed.certs, &roots, self.clock.now, from)
                    .unwrap_or_else(|_| vec![smime::Problem::UnknownIssuer]);
                let for_sender = !problems.iter().any(|p| matches!(p, smime::Problem::OtherAddress(_) | smime::Problem::NotForEmail));
                if for_sender && !from.is_empty() {
                    self.remember_cert(account, from, &signed);
                }
                SignedBy {
                    seal: if problems.is_empty() { Seal::Trusted } else { Seal::Untrusted(problems) },
                    cert: smime::cert::info(&signed.signer).ok(),
                    at: signed.signed_at,
                    weak: signed.weak,
                }
            }
            Err(e) => SignedBy {
                seal: Seal::Broken(match e {
                    smime::Error::BadSignature => "the message was changed after it was signed, or the signature isn't the sender's".into(),
                    other => other.to_string(),
                }),
                cert: None,
                at: None,
                weak: false,
            },
        }
    }

    fn remember_cert(&mut self, account: &str, from: &str, signed: &smime::Signed) {
        let now = self.clock.now;
        let s = self.smime.entry(account.to_string()).or_default();
        let known = KnownCert { cert: signed.signer.clone(), gcm: signed.reads_gcm(), seen: now };
        let changed = s.known.get(from).is_none_or(|k| k.cert != known.cert || k.gcm != known.gcm);
        // At most a few thousand people; the oldest seen goes first.
        if s.known.len() >= 5000
            && !s.known.contains_key(from)
            && let Some(oldest) = s.known.iter().min_by_key(|(_, k)| k.seen).map(|(e, _)| e.clone())
        {
            s.known.remove(&oldest);
        }
        s.known.insert(from.to_string(), known);
        if changed {
            self.save_smime(account);
        }
    }

    /// The certificate to encrypt to `email` with, and whether they read AES-GCM.
    fn cert_for(&self, account: &str, email: &str) -> Option<(Vec<u8>, bool)> {
        let email = email.trim().to_lowercase();
        let s = self.smime.get(account)?;
        // The account's own address: its own certificate.
        if self.account(account).is_some_and(|a| a.config.email.trim().eq_ignore_ascii_case(&email)) {
            return s.chain.first().map(|c| (c.clone(), true));
        }
        let k = s.known.get(&email)?;
        // Out of date certificates aren't encrypted to.
        let info = smime::cert::info(&k.cert).ok()?;
        (info.not_after >= self.clock.now).then(|| (k.cert.clone(), k.gcm))
    }

    /// Before `c` goes: what signing and encrypting it takes. `None` when it's neither; else the
    /// job to sign and encrypt `raw`.
    pub(super) fn protect_job(&mut self, c: &Compose, raw: &[u8], recipients: &[String], from: &str) -> Result<Option<Job>, CmdError> {
        if !c.sign && !c.encrypt {
            return Ok(None);
        }
        let s = self.smime_settings(&c.account);
        let config = self.account(&c.account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let Some(own) = s.chain.first().cloned() else {
            return Err(CmdError::Params(
                "there's no certificate for this account to sign or encrypt with: import one in File › Info › Security".into(),
            ));
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
            encrypt_to.push(own);
        }
        let job = Job::Protect { account: config, id: c.id, raw: Private(raw.to_vec()), sign: c.sign, encrypt_to, gcm: gcm && c.encrypt };
        self.protecting.insert(c.id, Protecting { compose: c.clone(), from: from.to_string(), recipients: recipients.to_vec() });
        Ok(Some(job))
    }

    pub(super) fn apply_smime(&mut self, e: Event) {
        match e {
            Event::Identity { account, chain, error } => match error {
                Some(e) => self.notice(format!("The certificate couldn't be kept: {e}"), true),
                None => {
                    let s = self.smime.entry(account.clone()).or_default();
                    s.chain = chain;
                    // A certificate is imported to be used: new messages are signed from now on.
                    s.sign = true;
                    self.save_smime(&account);
                    let who = self.identity(&account).map(|i| i.emails.first().cloned().unwrap_or(i.name)).unwrap_or_default();
                    self.notice(format!("Your certificate for {who} is ready: new messages are signed"), false);
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
                self.opening.remove(&key);
                match (content, error) {
                    (Some(Private(entity)), None) => {
                        if self.opened.len() >= OPENED_MAX
                            && let Some(k) = self.opened.keys().next().cloned()
                        {
                            self.opened.remove(&k);
                        }
                        self.opened.insert(key.clone(), (entity, authenticated));
                    }
                    (_, e) => {
                        self.decrypt_errors.insert(key.clone(), e.unwrap_or_else(|| "it can't be decrypted".into()));
                    }
                }
                // Shown again, now with what was inside.
                let shown = self.reading.as_ref().is_some_and(|r| r.key == key) || self.windows.iter().any(|w| w.key == key);
                if shown && let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&key)) {
                    for w in self.windows.iter_mut().filter(|w| w.key == key) {
                        w.parsed = None;
                    }
                    self.show_body(&key, &raw);
                }
            }
            _ => {}
        }
    }

    /// The message being written (or set aside) `id` couldn't go: said over it.
    fn send_problem(&mut self, id: u64, error: String) {
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
