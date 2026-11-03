//! Keys: secret key files unlocked, public keys read, and what a key is and can do, from its
//! self-signatures (each checked: a subkey or user ID someone else bound to a key isn't its).

use pgp::composed::{PublicOrSecret, SignedPublicKey, SignedPublicSubKey};
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{Signature, SignatureType};
use pgp::ser::Serialize as _;
use pgp::types::{KeyDetails, Password, Tag};
use serde::{Deserialize, Serialize};

use crate::{Error, KEYS_MAX, MAX, guard};

/// The most user IDs and subkeys looked at in one key.
const PARTS_MAX: usize = 64;

/// What a key is, as its self-signatures say.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyInfo {
    /// The primary key's fingerprint, upper-case hex.
    pub fingerprint: String,
    /// Its last 16 hex digits.
    pub key_id: String,
    /// Whose it is: the first user ID's name, else its address.
    pub name: String,
    /// The user IDs bound to it (`Name <address>`), revoked ones left out.
    pub user_ids: Vec<String>,
    /// Their addresses, lower-case.
    pub emails: Vec<String>,
    /// When it was made, and until when it's valid (Unix seconds; `None`: no end).
    pub created: i64,
    pub expires: Option<i64>,
    /// When its owner withdrew it.
    pub revoked: Option<i64>,
    /// It can be encrypted to: until `encrypt_until` (`None`: no end).
    pub encrypts: bool,
    pub encrypt_until: Option<i64>,
    /// It can sign.
    pub signs: bool,
    /// `RSA`, `Ed25519`…
    pub algorithm: String,
}

/// Why a key can't be used now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Its owner withdrew it, at this time.
    Revoked(i64),
    /// It ran out at this time.
    Expired(i64),
    /// It isn't valid until this time.
    NotYetValid(i64),
    /// It has no key that can be encrypted to.
    NoEncryption,
}

impl KeyInfo {
    /// Why the key can't be trusted at `now`: withdrawn, out of date or not yet valid.
    pub fn problems(&self, now: i64) -> Vec<Problem> {
        let mut out = Vec::new();
        if let Some(at) = self.revoked {
            out.push(Problem::Revoked(at));
        }
        if let Some(at) = self.expires.filter(|at| *at <= now) {
            out.push(Problem::Expired(at));
        }
        if self.created > now.saturating_add(86_400) {
            out.push(Problem::NotYetValid(self.created));
        }
        out
    }

    /// Whether mail can be encrypted to it at `now`, and if not why.
    pub fn encryptable(&self, now: i64) -> Result<(), Problem> {
        if let Some(p) = self.problems(now).into_iter().find(|p| !matches!(p, Problem::NotYetValid(_))) {
            return Err(p);
        }
        if !self.encrypts {
            return Err(Problem::NoEncryption);
        }
        match self.encrypt_until {
            Some(at) if at <= now => Err(Problem::Expired(at)),
            _ => Ok(()),
        }
    }

    /// Whether it's a key for `email`.
    pub fn is_for(&self, email: &str) -> bool {
        let e = email.trim();
        !e.is_empty() && self.emails.iter().any(|m| m.eq_ignore_ascii_case(e))
    }

    /// The fingerprint in groups of four, as people compare them.
    pub fn spaced(&self) -> String {
        let chars: Vec<char> = self.fingerprint.chars().collect();
        chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
    }
}

/// A secret key without its passphrase, and its public part.
#[derive(Clone, PartialEq, Eq)]
pub struct Unlocked {
    /// The transferable secret key, binary, its secret parts unprotected.
    pub secret: Vec<u8>,
    /// The transferable public key, binary.
    pub public: Vec<u8>,
    pub info: KeyInfo,
}

impl std::fmt::Debug for Unlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unlocked").field("secret", &"<redacted>").field("info", &self.info.fingerprint).finish()
    }
}

/// The armored blocks in `text` whose type (`PUBLIC KEY BLOCK`…) `want` accepts, at most
/// [`KEYS_MAX`].
pub(crate) fn armored(text: &[u8], want: impl Fn(&str) -> bool) -> Vec<&[u8]> {
    const BEGIN: &[u8] = b"-----BEGIN PGP ";
    let mut out = Vec::new();
    let mut from = 0;
    let mut unended: Vec<String> = Vec::new();
    // Every start line is looked at once, and only so many (a text of them all costs no more).
    for _ in 0..KEYS_MAX * 4 {
        if out.len() >= KEYS_MAX {
            break;
        }
        let Some(at) = find(text, BEGIN, from) else { break };
        from = at + BEGIN.len();
        let rest = text.get(at + BEGIN.len()..).unwrap_or_default();
        let Some(end_of_type) = rest.iter().take(64).position(|b| *b == b'-') else { continue };
        let kind = String::from_utf8_lossy(rest.get(..end_of_type).unwrap_or_default()).to_string();
        let line_start = at == 0 || matches!(text.get(at - 1), Some(b'\n' | b'\r'));
        // Only the blocks wanted are followed to their end, once per kind that has none: a text
        // made of start lines costs no more than reading it a few times.
        if !line_start || !want(&kind) || unended.contains(&kind) {
            continue;
        }
        // A cleartext-signed text ends with its signature's block.
        let end_marker = format!("-----END PGP {}-----", if kind == "SIGNED MESSAGE" { "SIGNATURE" } else { kind.as_str() });
        let Some(end) = find(text, end_marker.as_bytes(), at) else {
            unended.push(kind);
            continue;
        };
        let stop = end + end_marker.len();
        out.push(text.get(at..stop).unwrap_or_default());
        from = stop;
    }
    out
}

pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Every key in `data`: armored blocks (any number, text around them ignored) or binary packets.
fn parse_all(data: &[u8]) -> Result<Vec<PublicOrSecret>, Error> {
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    guard(Error::Malformed("a key can't be read"), || {
        let blocks = armored(data, |k| k.ends_with("KEY BLOCK"));
        let mut out = Vec::new();
        let sources: Vec<&[u8]> = if blocks.is_empty() { vec![data] } else { blocks };
        for src in sources {
            let Ok((keys, _)) = PublicOrSecret::from_reader_many(src) else { continue };
            for k in keys.take(KEYS_MAX * 4).flatten() {
                if out.len() < KEYS_MAX {
                    out.push(k);
                }
            }
        }
        Ok(out)
    })
}

/// The public keys in `data` (a key file or part, armored or binary, public or secret keys), each
/// as binary transferable public key. None: [`Error::NoKey`].
pub fn public_keys(data: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let keys = parse_all(data)?;
    let out: Vec<Vec<u8>> = keys
        .into_iter()
        .filter_map(|k| {
            let public = match k {
                PublicOrSecret::Public(p) => p,
                PublicOrSecret::Secret(s) => s.to_public_key(),
            };
            public.to_bytes().ok()
        })
        .collect();
    if out.is_empty() { Err(Error::NoKey) } else { Ok(out) }
}

/// One transferable public key (binary), parsed.
pub(crate) fn parse_public(data: &[u8]) -> Result<SignedPublicKey, Error> {
    use pgp::composed::Deserializable as _;
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    guard(Error::Malformed("a key can't be read"), || SignedPublicKey::from_bytes(data).map_err(|_| Error::Malformed("a key can't be read")))
}

/// What the public key `data` (binary) is.
pub fn info(data: &[u8]) -> Result<KeyInfo, Error> {
    let key = parse_public(data)?;
    guard(Error::Malformed("a key can't be read"), || Ok(analyse(&key).info))
}

/// Whether two public keys (binary) are the same key (their primary keys' fingerprints).
pub fn same_key(a: &[u8], b: &[u8]) -> bool {
    match (info(a), info(b)) {
        (Ok(x), Ok(y)) => x.fingerprint == y.fingerprint,
        _ => false,
    }
}

/// The secret key in the file `data`, unlocked with `passphrase` (ignored for a key without one).
pub fn read_secret_key(data: &[u8], passphrase: &str) -> Result<Unlocked, Error> {
    let secret = parse_all(data)?
        .into_iter()
        .find_map(|k| match k {
            PublicOrSecret::Secret(s) => Some(s),
            PublicOrSecret::Public(_) => None,
        })
        .ok_or(Error::NoSecretKey)?;
    guard(Error::Malformed("the key can't be read"), || {
        let mut key = secret;
        let pw = Password::from(passphrase);
        // Each part with the passphrase. A part that stays locked is one the file doesn't hold
        // (GnuPG exports a primary key kept elsewhere as a stub) or one with another passphrase:
        // the parts that opened are what's used.
        let mut opened = usize::from(key.primary_key.remove_password(&pw).is_ok());
        for sub in key.secret_subkeys.iter_mut() {
            opened += usize::from(sub.key.remove_password(&pw).is_ok());
        }
        if opened == 0 {
            return Err(Error::Passphrase);
        }
        let public = key.to_public_key();
        let info = analyse(&public).info;
        let secret = key.to_bytes().map_err(|_| Error::Malformed("the key can't be read"))?;
        let public = public.to_bytes().map_err(|_| Error::Malformed("the key can't be read"))?;
        Ok(Unlocked { secret, public, info })
    })
}

/// A part of a key: the primary key, or subkey `n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Primary,
    Sub(usize),
}

/// What [`analyse`] found: the key's [`KeyInfo`], and its parts that encrypt and sign (each
/// with when it was made and when it runs out).
pub(crate) struct Analysis {
    pub info: KeyInfo,
    pub encrypt: Vec<(Part, i64, Option<i64>)>,
    pub sign: Vec<(Part, i64, Option<i64>)>,
}

fn created(s: &Signature) -> i64 {
    s.created().map(|t| i64::from(t.as_secs())).unwrap_or(0)
}

/// The end of a key made at `made` that a signature gives it (`None`: no end).
fn end(made: i64, s: &Signature) -> Option<i64> {
    let secs = s.key_expiration_time().map(|d| std::time::Duration::from(d).as_secs()).unwrap_or(0);
    (secs > 0).then(|| made.saturating_add(i64::try_from(secs).unwrap_or(i64::MAX)))
}

fn can_encrypt_alg(a: PublicKeyAlgorithm) -> bool {
    matches!(
        a,
        PublicKeyAlgorithm::RSA
            | PublicKeyAlgorithm::RSAEncrypt
            | PublicKeyAlgorithm::ECDH
            | PublicKeyAlgorithm::X25519
            | PublicKeyAlgorithm::X448
            | PublicKeyAlgorithm::Elgamal
            | PublicKeyAlgorithm::ElgamalEncrypt
    )
}

fn can_sign_alg(a: PublicKeyAlgorithm) -> bool {
    matches!(
        a,
        PublicKeyAlgorithm::RSA
            | PublicKeyAlgorithm::RSASign
            | PublicKeyAlgorithm::DSA
            | PublicKeyAlgorithm::ECDSA
            | PublicKeyAlgorithm::EdDSALegacy
            | PublicKeyAlgorithm::Ed25519
            | PublicKeyAlgorithm::Ed448
    )
}

/// The address in a user ID (`Name <a@b>`, or an address alone), lower-case.
pub(crate) fn email_of(uid: &str) -> Option<String> {
    let inner = match (uid.rfind('<'), uid.rfind('>')) {
        (Some(a), Some(b)) if a < b => uid.get(a + 1..b).unwrap_or_default(),
        _ => uid,
    };
    let e = inner.trim();
    (e.contains('@') && !e.contains(char::is_whitespace)).then(|| e.to_lowercase())
}

fn name_of(uid: &str) -> String {
    match uid.rfind('<') {
        Some(at) => uid.get(..at).unwrap_or_default().trim().trim_matches('"').trim().to_string(),
        None => String::new(),
    }
}

/// The key read from its self-signatures, each checked.
pub(crate) fn analyse(key: &SignedPublicKey) -> Analysis {
    let primary = &key.primary_key;
    let made = i64::from(primary.created_at().as_secs());
    let fingerprint = format!("{:X}", primary.fingerprint());
    let key_id = fingerprint.get(fingerprint.len().saturating_sub(16)..).unwrap_or_default().to_string();
    let algorithm = match primary.algorithm() {
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign | PublicKeyAlgorithm::RSAEncrypt => "RSA".to_string(),
        PublicKeyAlgorithm::EdDSALegacy | PublicKeyAlgorithm::Ed25519 => "Ed25519".to_string(),
        other => format!("{other:?}"),
    };
    let mut info = KeyInfo { fingerprint, key_id, created: made, algorithm, ..Default::default() };
    // The user IDs, and the self-signature that speaks for the key (the primary user ID's newest).
    let mut speaks: Option<(bool, &Signature)> = None;
    for u in key.details.users.iter().take(PARTS_MAX) {
        let good: Vec<&Signature> =
            u.signatures.iter().take(PARTS_MAX).filter(|s| s.verify_certification(primary, Tag::UserId, &u.id).is_ok()).collect();
        let Some(latest) = good.iter().max_by_key(|s| created(s)).copied() else { continue };
        if latest.typ() == Some(SignatureType::CertRevocation) {
            continue;
        }
        let uid = String::from_utf8_lossy(u.id.id()).trim().to_string();
        if let Some(e) = email_of(&uid)
            && !info.emails.contains(&e)
        {
            info.emails.push(e);
        }
        if info.name.is_empty() {
            let n = name_of(&uid);
            info.name = if n.is_empty() { email_of(&uid).unwrap_or_else(|| uid.clone()) } else { n };
        }
        info.user_ids.push(uid);
        let primary_uid = latest.is_primary();
        let better = match speaks {
            None => true,
            Some((was_primary, s)) => (primary_uid && !was_primary) || (primary_uid == was_primary && created(latest) > created(s)),
        };
        if better {
            speaks = Some((primary_uid, latest));
        }
    }
    let direct = key.details.direct_signatures.iter().take(PARTS_MAX).filter(|s| s.verify_key(primary).is_ok()).max_by_key(|s| created(s));
    let speaking = speaks.map(|(_, s)| s).or(direct);
    info.revoked = key
        .details
        .revocation_signatures
        .iter()
        .take(PARTS_MAX)
        .filter(|s| s.typ() == Some(SignatureType::KeyRevocation) && s.verify_key(primary).is_ok())
        .map(created)
        .min();
    info.expires = speaking.and_then(|s| end(made, s)).or_else(|| direct.and_then(|s| end(made, s)));
    let mut encrypt = Vec::new();
    let mut sign = Vec::new();
    if let Some(s) = speaking {
        let flags = s.key_flags();
        let any = flags.sign() || flags.certify() || flags.encrypt_comms() || flags.encrypt_storage() || flags.authentication();
        if (flags.sign() || !any) && can_sign_alg(primary.algorithm()) {
            sign.push((Part::Primary, made, info.expires));
        }
        if (flags.encrypt_comms() || flags.encrypt_storage() || (!any && primary.algorithm() == PublicKeyAlgorithm::RSA))
            && can_encrypt_alg(primary.algorithm())
        {
            encrypt.push((Part::Primary, made, info.expires));
        }
    }
    for (n, sub) in key.public_subkeys.iter().enumerate().take(PARTS_MAX) {
        subkey(key, n, sub, info.expires, &mut encrypt, &mut sign);
    }
    info.encrypts = !encrypt.is_empty();
    info.encrypt_until = if encrypt.iter().any(|(_, _, until)| until.is_none()) { None } else { encrypt.iter().filter_map(|(_, _, u)| *u).max() };
    info.signs = !sign.is_empty();
    Analysis { info, encrypt, sign }
}

/// Subkey `n`, when it's bound to the key and not withdrawn: what it does, until when (never past
/// `key_end`, the primary key's).
fn subkey(
    key: &SignedPublicKey,
    n: usize,
    sub: &SignedPublicSubKey,
    key_end: Option<i64>,
    encrypt: &mut Vec<(Part, i64, Option<i64>)>,
    sign: &mut Vec<(Part, i64, Option<i64>)>,
) {
    let primary = &key.primary_key;
    let made = i64::from(sub.key.created_at().as_secs());
    let bound: Vec<&Signature> = sub.signatures.iter().take(PARTS_MAX).filter(|s| s.verify_subkey_binding(primary, &sub.key).is_ok()).collect();
    if bound.iter().any(|s| s.typ() == Some(SignatureType::SubkeyRevocation)) {
        return;
    }
    let Some(binding) = bound.iter().filter(|s| s.typ() == Some(SignatureType::SubkeyBinding)).max_by_key(|s| created(s)).copied() else { return };
    let until = match (end(made, binding), key_end) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let flags = binding.key_flags();
    let any = flags.sign() || flags.certify() || flags.encrypt_comms() || flags.encrypt_storage() || flags.authentication();
    let alg = sub.key.algorithm();
    if (flags.encrypt_comms() || flags.encrypt_storage() || !any) && can_encrypt_alg(alg) {
        encrypt.push((Part::Sub(n), made, until));
    }
    // A signing subkey counts only with its own signature back over the primary key: else
    // anyone could claim someone else's signing key as theirs.
    let backed = binding.embedded_signature().is_some_and(|b| b.verify_primary_key_binding(&sub.key, primary).is_ok());
    if flags.sign() && backed && can_sign_alg(alg) {
        sign.push((Part::Sub(n), made, until));
    }
}

/// `data` (a binary public key) with only what Autocrypt sends (Level 1 §2.1.1): the primary
/// key, the user IDs for `email` (all of them, when none is), and the newest subkey that
/// encrypts, each with its self-signatures.
pub fn minimal(data: &[u8], email: &str) -> Result<Vec<u8>, Error> {
    let key = parse_public(data)?;
    guard(Error::Malformed("a key can't be read"), || {
        let a = analyse(&key);
        let mut small = key.clone();
        let wanted: Vec<_> = small
            .details
            .users
            .iter()
            .filter(|u| email_of(&String::from_utf8_lossy(u.id.id())).is_some_and(|e| e.eq_ignore_ascii_case(email.trim())))
            .cloned()
            .collect();
        if !wanted.is_empty() {
            small.details.users = wanted;
        }
        small.details.user_attributes.clear();
        let newest = a.encrypt.iter().filter_map(|(p, made, _)| if let Part::Sub(n) = p { Some((*n, *made)) } else { None }).max_by_key(|(_, m)| *m);
        small.public_subkeys = newest.and_then(|(n, _)| key.public_subkeys.get(n).cloned()).into_iter().collect();
        small.to_bytes().map_err(|_| Error::Malformed("a key can't be written"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_come_out_of_user_ids() {
        assert_eq!(email_of("Ana Lima <Ana@Posteo.example>").as_deref(), Some("ana@posteo.example"));
        assert_eq!(email_of("ana@posteo.example").as_deref(), Some("ana@posteo.example"));
        assert_eq!(email_of("Ana Lima"), None);
        assert_eq!(email_of("<>"), None);
        assert_eq!(name_of("\"Ana Lima\" <ana@posteo.example>"), "Ana Lima");
    }

    #[test]
    fn armored_blocks_are_found_whole_and_bounded() {
        let text = b"Hi\n-----BEGIN PGP PUBLIC KEY BLOCK-----\nabc\n-----END PGP PUBLIC KEY BLOCK-----\ntext -----BEGIN PGP PUBLIC KEY BLOCK----- inside a line\n";
        let found = armored(text, |k| k.ends_with("KEY BLOCK"));
        assert_eq!(found.len(), 1);
        assert!(found[0].starts_with(b"-----BEGIN") && found[0].ends_with(b"BLOCK-----"));
        assert!(armored(b"-----BEGIN PGP PUBLIC KEY BLOCK-----\nno end", |_| true).is_empty());
        let many = "-----BEGIN PGP X-----\n-----END PGP X-----\n".repeat(200);
        assert_eq!(armored(many.as_bytes(), |_| true).len(), KEYS_MAX);
    }
}
