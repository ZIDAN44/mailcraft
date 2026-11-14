//! The session: S/MIME mail as it's read. Its protection taken off for showing (signatures
//! checked, the signers' certificates remembered, encrypted mail asked to be decrypted), and
//! as it reads for a reply, printing or an export, with nothing checked again; what was
//! decrypted forgotten when its folder's keys name other messages.

use super::*;

impl Session {
    /// What `raw` shows once its protection is taken off, and what that protection was: a
    /// signature checked, the content inside an encryption (when it's been decrypted; if not, it's
    /// asked for and the message stays as it is until it comes). `key` is the message. One whose
    /// header says it isn't protected is given back as it is, neither copied nor parsed.
    pub(crate) fn unwrap<'a>(&mut self, key: &MessageKey, raw: &'a [u8]) -> (std::borrow::Cow<'a, [u8]>, Option<Security>) {
        if !mime::smime::is_protected(raw) {
            return (std::borrow::Cow::Borrowed(raw), None);
        }
        let mut cur = raw.to_vec();
        let mut sec = Security::default();
        let mut any = false;
        // The keys it brings (Autocrypt, attached), first: its own signature may be by one.
        self.learn_keys(&key.account, raw);
        for _ in 0..4 {
            match mime::smime::protection(&cur) {
                mime::smime::Protection::None => match self.unwrap_pgp(key, &cur, &mut sec) {
                    pgp::Step::Not => break,
                    pgp::Step::Next(next) => {
                        any = true;
                        cur = next;
                    }
                    pgp::Step::Stop => {
                        any = true;
                        break;
                    }
                },
                mime::smime::Protection::Signed { content, signature } => {
                    any = true;
                    // Checked against the From that's shown: the one inside when the signed
                    // part carries its own (a message from one address, signed by its real
                    // sender, read as signed by the other it named inside otherwise).
                    let (next, headers) = mime::smime::with_entity_protected(&cur, &content);
                    if sec.signed.is_none() {
                        sec.signed = Some(self.check_signature(&key.account, &signature, Some(&content), &sender(&next)));
                    }
                    sec.headers |= headers;
                    cur = next;
                }
                mime::smime::Protection::Opaque(cms) => {
                    any = true;
                    let inside = smime::carried(&cms).ok().map(|content| mime::smime::with_entity_protected(&cur, &content));
                    if sec.signed.is_none() {
                        let from = inside.as_ref().map_or_else(|| sender(&cur), |(next, _)| sender(next));
                        sec.signed = Some(self.check_signature(&key.account, &cms, None, &from));
                    }
                    match inside {
                        Some((next, headers)) => {
                            sec.headers |= headers;
                            cur = next;
                        }
                        None => break,
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
                        let (next, headers) = mime::smime::with_entity_protected(&cur, entity);
                        sec.headers |= headers;
                        cur = next;
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
        // Keys attached inside what was protected.
        if any {
            self.learn_keys(&key.account, &cur);
        }
        (std::borrow::Cow::Owned(cur), any.then_some(sec))
    }

    /// A message as it reads: what [`Session::unwrap`] shows, without checking anything again.
    pub fn readable_raw(&self, key: &MessageKey) -> Option<Vec<u8>> {
        Some(self.as_written(key, self.body_raw(key)?).0)
    }

    /// `raw` (message `key`) with its protection taken off as far as it can be (an encryption
    /// not decrypted yet stays), and whether a signature and an encryption were.
    pub(crate) fn as_written(&self, key: &MessageKey, raw: Vec<u8>) -> (Vec<u8>, bool, bool) {
        let mut cur = raw;
        let (mut signed, mut encrypted) = (false, false);
        for _ in 0..4 {
            cur = match mime::smime::protection(&cur) {
                mime::smime::Protection::None => match self.readable_pgp(key, &cur, mime::pgp::protection(&cur)) {
                    Some((next, sealed)) => {
                        encrypted |= sealed;
                        signed |= !sealed;
                        next
                    }
                    None => break,
                },
                mime::smime::Protection::Signed { content, .. } => {
                    signed = true;
                    mime::smime::with_entity(&cur, &content)
                }
                mime::smime::Protection::Opaque(cms) => match smime::carried(&cms) {
                    Ok(c) => {
                        signed = true;
                        mime::smime::with_entity(&cur, &c)
                    }
                    Err(_) => break,
                },
                mime::smime::Protection::Encrypted(_) => match self.opened.get(key) {
                    Some((entity, _)) => {
                        encrypted = true;
                        mime::smime::with_entity(&cur, entity)
                    }
                    None => break,
                },
            };
        }
        (cur, signed, encrypted)
    }

    /// Whether what was decrypted for `key` is to be taken: asked for, and not an answer to
    /// an asking from before its folder's keys moved on (those come first, in order).
    pub(crate) fn decrypt_awaited(&mut self, key: &MessageKey) -> bool {
        if let Some(n) = self.stale_decrypts.get_mut(key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.stale_decrypts.remove(key);
            }
            return false;
        }
        self.opening.remove(key) || key.folder != crate::session::OUTBOX
    }

    /// What was decrypted, and what's being, of the messages in `folder` is forgotten: its keys
    /// name other messages now (the Outbox's count its rows). An answer still to come for one
    /// asked before is that message's, not the one its key names now: it's let pass.
    pub(crate) fn forget_opened_in(&mut self, folder: &str) {
        if self.stale_decrypts.len() > 256 {
            self.stale_decrypts.clear();
        }
        for k in self.opening.iter().filter(|k| k.folder == folder) {
            *self.stale_decrypts.entry(k.clone()).or_default() += 1;
        }
        self.opened.retain(|k| k.folder != folder);
        self.opening.retain(|k| k.folder != folder);
        self.decrypt_errors.retain(|k, _| k.folder != folder);
        self.pgp.forget_folder(folder);
    }

    fn ask_decrypt(&mut self, key: &MessageKey, cms: Vec<u8>) -> Lock {
        let Some(config) = self.account(&key.account).map(|a| a.config.clone()) else { return Lock::Failed("its account is gone".into()) };
        if self.identity(&key.account).is_none() && self.smime.get(&key.account).is_none_or(|s| s.older.is_empty()) {
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
                let (mut problems, path) = smime::cert::check_with_path(&signed.signer, &signed.certs, &roots, self.clock.now, from)
                    .unwrap_or_else(|_| (vec![smime::Problem::UnknownIssuer], Vec::new()));
                // Its authority is asked only when it's one this computer trusts: where to ask
                // is written in the certificate, and one anyone made names a server of theirs
                // (which then heard that, and when, the message was read).
                let known_authority = !problems.contains(&smime::Problem::UnknownIssuer);
                let revocation = if known_authority { self.revocation_of(&path, &mut problems) } else { None };
                // Remembered to encrypt to them: one that's theirs and in date. One from no
                // authority this computer trusts only where none is kept yet.
                let theirs = problems.iter().all(|p| matches!(p, smime::Problem::UnknownIssuer));
                if theirs && !from.is_empty() {
                    self.remember_cert(account, from, &signed, problems.is_empty());
                }
                SignedBy {
                    seal: if problems.is_empty() { Seal::Trusted } else { Seal::Untrusted(problems) },
                    cert: smime::cert::info(&signed.signer).ok(),
                    key: None,
                    at: signed.signed_at,
                    weak: signed.weak,
                    revocation,
                }
            }
            Err(e) => SignedBy {
                seal: Seal::Broken(match e {
                    smime::Error::BadSignature => "the message was changed after it was signed, or the signature isn't the sender's".into(),
                    other => other.to_string(),
                }),
                cert: None,
                key: None,
                at: None,
                weak: false,
                revocation: None,
            },
        }
    }

    /// The certificate `signed` came with is the one to encrypt to `from` with. It doesn't
    /// take the place of another kept for them when it's the lesser one: any mail naming them
    /// did (one signed with a certificate its sender made, and the next message to them was
    /// encrypted to that sender; an old message read again, and to the certificate they had
    /// before). A trusted one (`trusted`: nothing is wrong with it) that can still be
    /// encrypted to gives way only to a trusted one that isn't older. One that isn't trusted
    /// gives way to whichever came last, as ever: by their dates, one made today in someone's
    /// name stayed for good over the one they really sign with.
    fn remember_cert(&mut self, account: &str, from: &str, signed: &smime::Signed, trusted: bool) {
        let now = self.clock.now;
        // The one kept, when it's trusted and still good to encrypt to: in date, not withdrawn.
        let good = |cert: &[u8]| smime::cert::info(cert).is_ok_and(|i| i.not_before <= now && i.not_after >= now) && !self.heard_revoked(cert);
        let kept_holds = self.smime.get(account).and_then(|s| s.known.get(from)).is_some_and(|k| k.trusted && good(&k.cert));
        let s = self.smime.entry(account.to_string()).or_default();
        // The same one seen again where its authority isn't known stays the one that was.
        let trusted = trusted || s.known.get(from).is_some_and(|k| k.trusted && k.cert == signed.signer);
        let known = KnownCert { cert: signed.signer.clone(), gcm: signed.reads_gcm(), seen: now, trusted };
        if let Some(kept) = s.known.get(from).filter(|k| k.cert != known.cert) {
            let since = |cert: &[u8]| smime::cert::info(cert).map_or(i64::MIN, |i| i.not_before);
            let older = since(&known.cert) < since(&kept.cert);
            if kept_holds && (!trusted || older) {
                return;
            }
        }
        let changed = s.known.get(from).is_none_or(|k| k.cert != known.cert || k.gcm != known.gcm || k.trusted != known.trusted);
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
}
