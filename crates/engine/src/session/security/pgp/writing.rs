//! Writing with OpenPGP: which protection a message gets, the job that signs and encrypts it as
//! PGP/MIME, the account's key sent along (Autocrypt), and whether the message being written can
//! go encrypted to everyone in it (issue #6).

use super::*;

/// A digest of what a [`Reach`] is worked out from, to work it out again only when that changes.
fn digest(parts: &[&[u8]]) -> u64 {
    parts.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
        let h = p.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
        (h ^ 0xff).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Session {
    /// The addresses a message being written goes to (its To, Cc and Bcc, as far as they read).
    pub(crate) fn compose_recipients(c: &Compose) -> Vec<String> {
        [&c.to, &c.cc, &c.bcc]
            .iter()
            .flat_map(|f| f.split([',', ';']).filter_map(|p| addr::parse_list(p).ok()).flatten().collect::<Vec<_>>())
            .map(|a| a.email.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect()
    }

    /// The key to encrypt to `email` with: the account's own for its own address, else theirs,
    /// when it can be encrypted to now.
    fn pgp_key_for(&self, account: &str, email: &str) -> Option<Vec<u8>> {
        let email = email.trim().to_lowercase();
        let s = self.pgp.settings.get(account)?;
        if self.account(account).is_some_and(|a| a.config.email.trim().eq_ignore_ascii_case(&email)) {
            return s.keys.first().cloned();
        }
        let k = s.known.get(&email)?;
        k.info.encryptable(self.clock.now).is_ok().then(|| k.key.clone())
    }

    /// The recipients of `recipients` that can't receive mail encrypted with `scheme`.
    fn unreachable(&self, account: &str, recipients: &[String], scheme: Scheme) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in recipients {
            let reached = match scheme {
                Scheme::Smime => self.cert_for(account, r).is_some(),
                Scheme::Pgp => self.pgp_key_for(account, r).is_some(),
            };
            if !reached && !out.contains(r) {
                out.push(r.clone());
            }
        }
        out
    }

    /// Which protection `c` gets: the one chosen, else the one the account has, else (it has
    /// both) the one every recipient can receive, S/MIME when both or neither reach them all.
    pub(crate) fn scheme_for(&self, c: &Compose, recipients: &[String]) -> Scheme {
        let smime = self.identity(&c.account).is_some();
        let pgp = self.pgp_key(&c.account).is_some();
        match (c.scheme, smime, pgp) {
            (Some(Scheme::Pgp), _, true) | (_, false, true) => Scheme::Pgp,
            (Some(Scheme::Smime), true, _) | (_, true, false) | (_, false, false) => Scheme::Smime,
            (_, true, true) => {
                let s = self.unreachable(&c.account, recipients, Scheme::Smime).len();
                let p = self.unreachable(&c.account, recipients, Scheme::Pgp).len();
                if s > 0 && p < s { Scheme::Pgp } else { Scheme::Smime }
            }
        }
    }

    /// Before `c` goes with OpenPGP: the job to sign and/or encrypt `raw`.
    pub(crate) fn pgp_protect_job(&mut self, c: &Compose, raw: &[u8], recipients: &[String], from: &str) -> Result<Job, CmdError> {
        let config = self.account(&c.account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let s = self.pgp_settings(&c.account);
        let Some(own) = s.keys.first().cloned() else {
            return Err(CmdError::Params(
                "there's no OpenPGP key for this account to sign or encrypt with: import one in File › Info › Security".into(),
            ));
        };
        let info = openpgp::info(&own).map_err(|e| CmdError::Failed(e.to_string()))?;
        if let Some(p) = info.problems(self.clock.now).first() {
            let why = match p {
                openpgp::Problem::Revoked(_) => "was withdrawn".to_string(),
                openpgp::Problem::Expired(at) => format!("ran out {}", mailcraft_model::dates::full_time(*at, &self.clock)),
                openpgp::Problem::NotYetValid(at) => format!("isn't valid until {}", mailcraft_model::dates::full_time(*at, &self.clock)),
                openpgp::Problem::NoEncryption => String::new(),
            };
            return Err(CmdError::Params(format!(
                "your OpenPGP key {why}: replace it in File › Info › Security, or send without signing or encryption"
            )));
        }
        if c.sign && !info.signs {
            return Err(CmdError::Params("your OpenPGP key can't sign: send it without signing, or import a key that can".into()));
        }
        let mut encrypt_to = Vec::new();
        if c.encrypt {
            let missing = self.unreachable(&c.account, recipients, Scheme::Pgp);
            if !missing.is_empty() {
                return Err(CmdError::Params(format!(
                    "there's no OpenPGP key to encrypt to {}: their key comes with their mail (Autocrypt) or can be imported in File › Info › Security, or send it without encryption",
                    missing.join(", ")
                )));
            }
            for r in recipients {
                if let Some(k) = self.pgp_key_for(&c.account, r)
                    && !encrypt_to.contains(&k)
                {
                    encrypt_to.push(k);
                }
            }
            // The Sent copy reads too.
            if info.encrypts && !encrypt_to.contains(&own) {
                encrypt_to.push(own);
            }
        }
        let hide_subject = !self.smime_settings(&c.account).keep_subject;
        let work = PgpWork::Protect { id: c.id, raw: Private(raw.to_vec()), sign: c.sign, encrypt_to, hide_subject };
        self.protecting.insert(c.id, Protecting { compose: c.clone(), from: from.to_string(), recipients: recipients.to_vec() });
        Ok(Job::Pgp { account: config, work })
    }

    /// Whether `c`'s draft is encrypted with the account's OpenPGP key (it's to be encrypted
    /// with OpenPGP).
    pub(crate) fn seals_with_pgp(&self, c: &Compose) -> bool {
        c.encrypt && self.pgp_key(&c.account).is_some() && self.scheme_for(c, &Self::compose_recipients(c)) == Scheme::Pgp
    }

    /// `raw` (a message about to go from `account`) with the account's key in an `Autocrypt:`
    /// header, so whoever answers can encrypt to it: when it has a key it can be encrypted to,
    /// and hasn't turned that off.
    pub(crate) fn with_autocrypt(&self, account: &str, raw: Vec<u8>) -> Vec<u8> {
        let s = self.pgp_settings(account);
        let (Some(own), Some(a)) = (s.keys.first(), self.account(account)) else { return raw };
        let email = a.config.email.trim().to_string();
        if s.no_autocrypt || email.is_empty() || openpgp::info(own).map(|i| i.encryptable(self.clock.now).is_err()).unwrap_or(true) {
            return raw;
        }
        let Ok(small) = openpgp::minimal(own, &email) else { return raw };
        let mutual = self.smime_settings(account).auto_encrypt;
        [openpgp::autocrypt::header(&email, &small, mutual).as_bytes(), &raw].concat()
    }

    /// Whether the message being written can go encrypted to everyone in it, and if not who
    /// can't (issue #6); worked out again only when its recipients or the keys known change.
    /// While the account's option has encryption follow the recipients, it's turned on and off
    /// to match.
    pub fn encryption_reach(&mut self) -> Option<Reach> {
        let c = self.compose.as_ref()?;
        let known = |m: usize| (m as u64).to_le_bytes();
        let (smime_known, pgp_known) = (
            self.smime.get(&c.account).map_or(0, |s| s.known.len() + s.chain.len()),
            self.pgp.settings.get(&c.account).map_or(0, |s| s.known.len() + s.keys.len()),
        );
        let scheme = format!("{:?}", c.scheme);
        let day = self.clock.now.div_euclid(3600).to_le_bytes();
        let d = digest(&[
            &c.id.to_le_bytes(),
            c.account.as_bytes(),
            c.to.as_bytes(),
            c.cc.as_bytes(),
            c.bcc.as_bytes(),
            scheme.as_bytes(),
            &known(smime_known),
            &known(pgp_known),
            &day,
        ]);
        let reach = match &self.pgp.reach {
            Some((was, r)) if *was == d => r.clone(),
            _ => {
                let r = self.work_out_reach(c);
                self.pgp.reach = Some((d, r.clone()));
                r
            }
        };
        if let Some(c) = self.compose.as_mut()
            && c.encrypt_auto
        {
            c.encrypt = matches!(reach, Reach::Everyone(_));
        }
        Some(reach)
    }

    fn work_out_reach(&self, c: &Compose) -> Reach {
        let (smime, pgp) = (self.identity(&c.account).is_some(), self.pgp_key(&c.account).is_some());
        if !smime && !pgp {
            return Reach::NotSetUp;
        }
        let recipients = Self::compose_recipients(c);
        if recipients.is_empty() {
            return Reach::NoOne;
        }
        let missing = |scheme: Scheme| self.unreachable(&c.account, &recipients, scheme);
        let only = match (c.scheme, smime, pgp) {
            (Some(Scheme::Pgp), _, true) | (_, false, true) => Some(Scheme::Pgp),
            (Some(Scheme::Smime), true, _) | (_, true, false) => Some(Scheme::Smime),
            _ => None,
        };
        let one = |scheme: Scheme| {
            let who = missing(scheme);
            if who.is_empty() { Reach::Everyone(scheme) } else { Reach::Missing { scheme, who } }
        };
        match only {
            Some(s) => one(s),
            None => match (one(Scheme::Smime), one(Scheme::Pgp)) {
                (r @ Reach::Everyone(_), _) | (_, r @ Reach::Everyone(_)) => r,
                (Reach::Missing { scheme: s1, who: w1 }, Reach::Missing { scheme: s2, who: w2 }) => {
                    if w2.len() < w1.len() {
                        Reach::Missing { scheme: s2, who: w2 }
                    } else {
                        Reach::Missing { scheme: s1, who: w1 }
                    }
                }
                (r, _) => r,
            },
        }
    }
}
