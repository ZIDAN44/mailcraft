//! Reading OpenPGP mail: a step of [`Session::unwrap`] for PGP/MIME and inline PGP, signatures
//! checked as S/MIME's are, encrypted mail decrypted by the network layer with the account's keys.

use super::*;

/// What a step of taking a message's protection off came to.
pub(crate) enum Step {
    /// It isn't OpenPGP.
    Not,
    /// What it shows with this layer taken off.
    Next(Vec<u8>),
    /// It's OpenPGP, and nothing more can be taken off (yet).
    Stop,
}

/// What a signature's check says, as the network layer passes it.
fn as_signature(c: openpgp::Check) -> PgpSignature {
    match c {
        openpgp::Check::Good { key, at, weak } => PgpSignature::Good { key, at, weak },
        openpgp::Check::Bad => PgpSignature::Bad,
        openpgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
    }
}

fn broken(why: String) -> SignedBy {
    SignedBy { seal: Seal::Broken(why), cert: None, key: None, at: None, weak: false, revocation: None }
}

impl Session {
    /// What a signature says, as the reading pane shows it: it holds by a key for the sender
    /// that's valid now, or with what's wrong with the key; or it doesn't hold, or can't be
    /// checked. `keys` are those it was checked with.
    pub(super) fn pgp_signed_by(&self, sig: &PgpSignature, keys: &[Vec<u8>], from: &str) -> SignedBy {
        match sig {
            PgpSignature::Good { key, at, weak } => {
                let info = keys.get(*key).and_then(|k| openpgp::info(k).ok());
                let mut problems = Vec::new();
                if let Some(i) = &info {
                    for p in i.problems(self.clock.now) {
                        problems.push(match p {
                            openpgp::Problem::Revoked(t) => smime::Problem::Revoked(t),
                            openpgp::Problem::Expired(t) => smime::Problem::Expired(t),
                            openpgp::Problem::NotYetValid(t) => smime::Problem::NotYetValid(t),
                            openpgp::Problem::NoEncryption => continue,
                        });
                    }
                    if !from.trim().is_empty() && !i.is_for(from) {
                        problems.push(smime::Problem::OtherAddress(i.emails.clone()));
                    }
                }
                SignedBy {
                    seal: if problems.is_empty() { Seal::Trusted } else { Seal::Untrusted(problems) },
                    cert: None,
                    key: info,
                    at: *at,
                    weak: *weak,
                    revocation: None,
                }
            }
            PgpSignature::Bad => broken("the message was changed after it was signed, or the signature isn't the sender's".into()),
            PgpSignature::Unknown { key_id } => {
                SignedBy { seal: Seal::UnknownKey(key_id.clone()), cert: None, key: None, at: None, weak: false, revocation: None }
            }
        }
    }

    /// A check made here (a detached or cleartext signature, a signed message) as shown.
    fn checked(&self, done: Result<openpgp::Check, openpgp::Error>, keys: &[Vec<u8>], from: &str) -> SignedBy {
        match done {
            Ok(c) => self.pgp_signed_by(&as_signature(c), keys, from),
            Err(e) => broken(e.to_string()),
        }
    }

    /// One layer of OpenPGP taken off `cur` (message `key`), what it was said in `sec`.
    pub(crate) fn unwrap_pgp(&mut self, key: &MessageKey, cur: &[u8], sec: &mut Security) -> Step {
        let from = sender(cur);
        match mime::pgp::protection(cur) {
            mime::pgp::Protection::None => Step::Not,
            mime::pgp::Protection::Signed { content, signature } => {
                sec.pgp = true;
                if sec.signed.is_none() {
                    let keys = self.keys_for(&key.account, &from);
                    let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
                    sec.signed = Some(self.checked(openpgp::verify(&content, &signature, &refs), &keys, &from));
                }
                let (next, headers) = mime::smime::with_entity_protected(cur, &content);
                sec.headers |= headers;
                Step::Next(next)
            }
            mime::pgp::Protection::Encrypted(data) => {
                sec.pgp = true;
                self.open_encrypted(key, cur, data, false, "", &from, sec)
            }
            mime::pgp::Protection::Inline { body, charset } => {
                let keys = self.keys_for(&key.account, &from);
                let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
                match openpgp::inline(&body) {
                    None => Step::Not,
                    Some(openpgp::Inline::Signed(block)) => {
                        sec.pgp = true;
                        match openpgp::verify_cleartext(block, &refs) {
                            Ok((text, check)) => {
                                if sec.signed.is_none() {
                                    sec.signed = Some(self.checked(Ok(check), &keys, &from));
                                }
                                Step::Next(mime::smime::with_entity(cur, &mime::pgp::text_entity(text.as_bytes(), &charset)))
                            }
                            Err(e) => {
                                sec.signed.get_or_insert_with(|| broken(e.to_string()));
                                Step::Stop
                            }
                        }
                    }
                    Some(openpgp::Inline::Message(block)) if openpgp::is_encrypted(block) => {
                        sec.pgp = true;
                        self.open_encrypted(key, cur, block.to_vec(), true, &charset, &from, sec)
                    }
                    Some(openpgp::Inline::Message(block)) => {
                        sec.pgp = true;
                        match openpgp::open(block, &refs) {
                            Ok(d) => {
                                if sec.signed.is_none()
                                    && let Some(c) = d.check
                                {
                                    sec.signed = Some(self.checked(Ok(c), &keys, &from));
                                }
                                Step::Next(mime::smime::with_entity(cur, &mime::pgp::text_entity(&d.data, &charset)))
                            }
                            Err(e) => {
                                sec.signed.get_or_insert_with(|| broken(e.to_string()));
                                Step::Stop
                            }
                        }
                    }
                }
            }
        }
    }

    /// An encrypted message: what it held when it's been decrypted, else asked for.
    #[allow(clippy::too_many_arguments)]
    fn open_encrypted(&mut self, key: &MessageKey, cur: &[u8], data: Vec<u8>, inline: bool, charset: &str, from: &str, sec: &mut Security) -> Step {
        // One encryption is decrypted; another inside it stays as it is.
        if sec.lock.is_some() {
            return Step::Stop;
        }
        if let Some((entity, _)) = self.opened.get(key) {
            sec.lock = Some(Lock::Open { authenticated: true });
            if sec.signed.is_none() {
                sec.signed = self.pgp.signed.get(key).cloned();
            }
            let (next, headers) = mime::smime::with_entity_protected(cur, entity);
            sec.headers |= headers;
            return Step::Next(next);
        }
        sec.lock = Some(match self.decrypt_errors.get(key) {
            Some(e) => Lock::Failed(e.clone()),
            None => self.ask_pgp_decrypt(key, data, inline, charset, from),
        });
        Step::Stop
    }

    fn ask_pgp_decrypt(&mut self, key: &MessageKey, data: Vec<u8>, inline: bool, charset: &str, from: &str) -> Lock {
        let Some(config) = self.account(&key.account).map(|a| a.config.clone()) else { return Lock::Failed("its account is gone".into()) };
        if self.pgp.settings.get(&key.account).is_none_or(|s| s.keys.is_empty()) {
            return Lock::Failed("this account has no OpenPGP key to decrypt it with (File › Info › Security)".into());
        }
        if self.opening.insert(key.clone()) {
            let verify_with = self.keys_for(&key.account, from);
            self.pgp.asked.insert(key.clone(), (verify_with.clone(), from.to_string()));
            let work = PgpWork::Decrypt { key: key.clone(), data, inline, charset: charset.to_string(), verify_with };
            self.jobs.push(Job::Pgp { account: config, work });
        }
        Lock::Opening
    }

    /// A layer of OpenPGP taken off as [`Session::readable_raw`] reads: nothing checked again.
    pub(crate) fn readable_pgp(&self, key: &MessageKey, cur: &[u8]) -> Option<Vec<u8>> {
        match mime::pgp::protection(cur) {
            mime::pgp::Protection::None => None,
            mime::pgp::Protection::Signed { content, .. } => Some(mime::smime::with_entity(cur, &content)),
            mime::pgp::Protection::Encrypted(_) => self.opened.get(key).map(|(entity, _)| mime::smime::with_entity(cur, entity)),
            mime::pgp::Protection::Inline { body, charset } => match openpgp::inline(&body)? {
                openpgp::Inline::Signed(block) => {
                    let (text, _) = openpgp::verify_cleartext(block, &[]).ok()?;
                    Some(mime::smime::with_entity(cur, &mime::pgp::text_entity(text.as_bytes(), &charset)))
                }
                openpgp::Inline::Message(block) if openpgp::is_encrypted(block) => {
                    self.opened.get(key).map(|(entity, _)| mime::smime::with_entity(cur, entity))
                }
                openpgp::Inline::Message(block) => {
                    let d = openpgp::open(block, &[]).ok()?;
                    Some(mime::smime::with_entity(cur, &mime::pgp::text_entity(&d.data, &charset)))
                }
            },
        }
    }
}
