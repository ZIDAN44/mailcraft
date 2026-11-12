//! Which of an account's certificates and OpenPGP keys signs mail from which of its addresses
//! (issue #81). An account keeps several of each with their keys: the one in use, and the
//! others (a renewed one's predecessors, kept to read old mail; and those of its other
//! addresses). A message is signed with one that names the address it's from, so whoever reads
//! it sees a signer who is the sender; the account's own address falls back on the one in use,
//! whatever it names, as it always did.

use super::*;

impl Session {
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
    fn own_certs(&self, account: &str) -> Vec<(Vec<u8>, smime::CertInfo)> {
        let Some(s) = self.smime.get(account) else { return Vec::new() };
        s.chain.first().into_iter().chain(&s.older).filter_map(|c| smime::cert::info(c).ok().map(|i| (c.clone(), i))).collect()
    }

    /// The certificate mail from `from` (one of the account's addresses; none: its own) is
    /// signed with: the first, the one in use before the others, that names the address and is
    /// valid now. The account's own address makes do with the one in use when none does.
    pub(crate) fn own_cert_for(&self, account: &str, from: &str) -> Option<(Vec<u8>, smime::CertInfo)> {
        let address = self.address_from(account, from);
        let now = self.clock.now;
        let mut certs = self.own_certs(account);
        let named =
            certs.iter().position(|(_, i)| i.not_before <= now && i.not_after >= now && i.emails.iter().any(|e| e.eq_ignore_ascii_case(&address)));
        match named {
            Some(at) => Some(certs.swap_remove(at)),
            None if self.is_own_address(account, &address) => certs.into_iter().next(),
            None => None,
        }
    }

    /// [`Session::identity`] for mail from `from`: the certificate that signs it.
    pub fn identity_for(&self, account: &str, from: &str) -> Option<smime::CertInfo> {
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
