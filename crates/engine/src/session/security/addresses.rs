//! Which of an account's certificates and OpenPGP keys signs mail from which of its addresses
//! (issue #81). An account keeps several of each with their keys: the one in use, and the
//! others (a renewed one's predecessors, kept to read old mail; and those of its other
//! addresses). A message is signed with one that names the address it's from, so whoever reads
//! it sees a signer who is the sender; the account's own address falls back on the one in use,
//! whatever it names, as it always did.

use super::*;

/// What an account's own certificates are (read once each time they change, as the window asks
/// every frame and every message written does): one that can't be read is left out.
#[derive(Debug, Default)]
pub(crate) struct OwnCerts {
    /// The one in use, [`SmimeSettings::chain`]'s first.
    in_use: Option<smime::CertInfo>,
    /// The older ones, newest first, each with where it is in [`SmimeSettings::older`].
    older: Vec<smime::CertInfo>,
    older_at: Vec<usize>,
}

impl Session {
    /// Read the account's own certificates again (they changed, or were loaded).
    pub(super) fn read_own_certs(&mut self, account: &str) {
        let mut own = OwnCerts::default();
        if let Some(s) = self.smime.get(account) {
            own.in_use = s.chain.first().and_then(|c| smime::cert::info(c).ok());
            for (at, c) in s.older.iter().enumerate() {
                if let Ok(i) = smime::cert::info(c) {
                    own.older.push(i);
                    own.older_at.push(at);
                }
            }
        }
        self.smime_own.insert(account.to_string(), own);
    }

    /// The account's certificate, when it has one.
    pub fn identity(&self, account: &str) -> Option<&smime::CertInfo> {
        self.smime_own.get(account).and_then(|o| o.in_use.as_ref())
    }

    /// The account's older certificates, kept to read mail encrypted to them, newest first.
    pub fn older_identities(&self, account: &str) -> &[smime::CertInfo] {
        self.smime_own.get(account).map_or(&[], |o| o.older.as_slice())
    }

    /// The address mail from `from` is from: `from`, or the account's own for none.
    pub(crate) fn address_from(&self, account: &str, from: &str) -> String {
        let own = self.account(account).map(|a| a.config.email.trim().to_lowercase()).unwrap_or_default();
        let from = from.trim().to_lowercase();
        if from.is_empty() { own } else { from }
    }

    /// Whether `address` is the account's own (not one of its other addresses).
    pub(crate) fn is_own_address(&self, account: &str, address: &str) -> bool {
        self.account(account).is_some_and(|a| a.config.email.trim().eq_ignore_ascii_case(address.trim()))
    }

    /// The account's certificates with keys here: the one in use first, then the others.
    fn own_certs<'a>(&'a self, account: &str) -> impl Iterator<Item = (&'a [u8], &'a smime::CertInfo)> + 'a {
        let (s, own) = (self.smime.get(account), self.smime_own.get(account));
        let in_use = s.and_then(|s| s.chain.first()).zip(own.and_then(|o| o.in_use.as_ref()));
        let older = s.zip(own).into_iter().flat_map(|(s, o)| o.older_at.iter().zip(&o.older).filter_map(|(at, i)| s.older.get(*at).map(|c| (c, i))));
        in_use.into_iter().chain(older).map(|(c, i)| (c.as_slice(), i))
    }

    /// The certificate mail from `from` (one of the account's addresses; none: its own) is
    /// signed with: the first, the one in use before the others, that names the address and is
    /// valid now. The account's own address makes do with the one in use when none does.
    pub(crate) fn own_cert_for(&self, account: &str, from: &str) -> Option<(&[u8], &smime::CertInfo)> {
        let address = self.address_from(account, from);
        let now = self.clock.now;
        let named = self
            .own_certs(account)
            .find(|(_, i)| i.not_before <= now && i.not_after >= now && i.emails.iter().any(|e| e.eq_ignore_ascii_case(&address)));
        match named {
            Some(found) => Some(found),
            None if self.is_own_address(account, &address) => self.own_certs(account).next(),
            None => None,
        }
    }

    /// [`Session::identity`] for mail from `from`: the certificate that signs it.
    pub fn identity_for(&self, account: &str, from: &str) -> Option<&smime::CertInfo> {
        self.own_cert_for(account, from).map(|(_, i)| i)
    }

    /// Whether mail from `from` can be signed: a certificate or a key is its address's.
    pub fn can_sign_from(&self, account: &str, from: &str) -> bool {
        self.own_cert_for(account, from).is_some() || self.own_pgp_for(account, from).is_some()
    }

    /// The account's addresses whose mail the certificate `fingerprint` signs, its own first.
    pub fn signed_by_certificate(&self, account: &str, fingerprint: &str) -> Vec<String> {
        self.addresses_signed(account, |s, address| {
            s.own_cert_for(account, address).is_some_and(|(_, i)| i.fingerprint.eq_ignore_ascii_case(fingerprint))
        })
    }

    pub(crate) fn addresses_signed(&self, account: &str, signs: impl Fn(&Session, &str) -> bool) -> Vec<String> {
        let Some(a) = self.account(account) else { return Vec::new() };
        a.config.addresses().filter(|address| signs(self, address)).map(str::to_string).collect()
    }

    /// From changed (`was` the account, `was_from` the address): signing follows it. A message
    /// that was to be signed because its account signs its mail isn't from an address nothing
    /// signs for; one from an address that can be signed is, when the account signs its mail.
    /// What was chosen for this message otherwise stays.
    pub(crate) fn signing_follows_from(&mut self, was: &str, was_from: &str) {
        let Some(c) = self.compose.as_ref() else { return };
        let (account, from, sign) = (c.account.clone(), c.from.clone(), c.sign);
        let wants = |s: &Session, account: &str, from: &str| s.smime_settings(account).sign && s.can_sign_from(account, from);
        // As its account's settings had it: it follows. Changed for this message: it stays,
        // unless nothing can sign it now.
        let followed = sign == wants(self, was, was_from);
        let now = if followed { wants(self, &account, &from) } else { sign && self.can_sign_from(&account, &from) };
        if let Some(c) = self.compose.as_mut() {
            c.sign = now;
        }
    }
}
