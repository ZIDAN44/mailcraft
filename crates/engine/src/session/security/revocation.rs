//! Whether a signer's certificate was withdrawn by its authority (OCSP, else its CRL), when File ›
//! Options › Safety says to ask: off at first, since asking tells the authority whose mail is
//! being read. The answers are kept a while by certificate, so a message shown again doesn't ask
//! again; an answer that can't be had is said as such (it's never taken for "withdrawn").

use mailcraft_model::Revocation;

use super::*;

/// The most certificates whose answers are kept, and the most withdrawn ones beyond them (each a
/// real authority's signed answer, so a ceiling against memory, not one ever met).
const KEPT_MAX: usize = 256;
const REVOKED_MAX: usize = 4096;
/// A "not withdrawn" answer is asked again after this long, or when it says it runs out.
const GOOD_FOR: i64 = 86_400;
/// No answer: asked again after this long.
const UNKNOWN_FOR: i64 = 3_600;

/// The authorities' answers, by certificate (DER), and when each was heard.
#[derive(Clone, Debug, Default)]
pub(crate) struct Revocations {
    heard: HashMap<Vec<u8>, (Revocation, i64)>,
    asking: HashSet<Vec<u8>>,
}

/// What's known about a signer's certificate being withdrawn (a withdrawn one is
/// [`smime::Problem::Revoked`] instead).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevocationCheck {
    /// Its authority is being asked.
    Asking,
    /// Its authority says it wasn't withdrawn.
    Good,
    /// No answer to go by, and why.
    Unknown(String),
}

impl Session {
    /// For a signature by `path` (the signer's certificate, then its issuers): what's known of
    /// its being withdrawn, asking its authority when nothing current is. `None`: not asked
    /// (the switch is off, or there's no issuer to ask about).
    pub(super) fn revocation_of(&mut self, path: &[Vec<u8>], problems: &mut Vec<smime::Problem>) -> Option<RevocationCheck> {
        if !self.prefs.check_revocation || path.len() < 2 {
            return None;
        }
        let cert = path.first()?;
        let now = self.clock.now;
        let current = self.revocation.heard.get(cert).filter(|(status, at)| match status {
            Revocation::Good { until } => now < at.saturating_add(GOOD_FOR) && until.is_none_or(|u| now < u),
            Revocation::Revoked { .. } => true,
            Revocation::Unknown { .. } => now < at.saturating_add(UNKNOWN_FOR),
        });
        match current.map(|(s, _)| s.clone()) {
            Some(Revocation::Good { .. }) => Some(RevocationCheck::Good),
            Some(Revocation::Revoked { at }) => {
                problems.push(smime::Problem::Revoked(at));
                None
            }
            Some(Revocation::Unknown { why }) => Some(RevocationCheck::Unknown(why)),
            None => {
                if self.revocation.asking.insert(cert.clone()) {
                    self.jobs.push(Job::CheckRevocation { chain: path.to_vec() });
                }
                Some(RevocationCheck::Asking)
            }
        }
    }

    /// An authority's answer: kept, and the messages waiting for it shown again.
    pub(super) fn heard_revocation(&mut self, cert: Vec<u8>, status: Revocation) {
        self.revocation.asking.remove(&cert);
        // Withdrawn stays withdrawn, whatever a later answer (or the lack of one) says.
        if matches!(self.revocation.heard.get(&cert), Some((Revocation::Revoked { .. }, _))) {
            return self.show_waiting();
        }
        // Full: the answers that would be asked again anyway go, never a withdrawn one (else a
        // flood of other signers, then a blocked request, would turn "withdrawn" into "unknown").
        if self.revocation.heard.len() >= KEPT_MAX {
            self.revocation.heard.retain(|_, (s, _)| matches!(s, Revocation::Revoked { .. }));
            if self.revocation.heard.len() >= REVOKED_MAX {
                self.revocation.heard.clear();
            }
        }
        self.revocation.heard.insert(cert, (status, self.clock.now));
        self.show_waiting();
    }

    /// The messages waiting for an authority's answer, shown again.
    fn show_waiting(&mut self) {
        let waiting =
            |r: &Reading| r.security.as_ref().and_then(|s| s.signed.as_ref()).is_some_and(|s| s.revocation == Some(RevocationCheck::Asking));
        let mut keys: Vec<MessageKey> = self.reading.iter().chain(self.windows.iter()).filter(|r| waiting(r)).map(|r| r.key.clone()).collect();
        keys.dedup();
        for key in keys {
            self.show_again(&key);
        }
    }
}
