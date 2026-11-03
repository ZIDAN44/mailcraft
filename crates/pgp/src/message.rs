//! Messages: signed (detached, cleartext, or inside), encrypted and decrypted.

use std::io::Read as _;

use pgp::composed::{
    ArmorOptions, CleartextSignedMessage, Deserializable as _, DetachedSignature, Esk, Message, MessageBuilder, SignedSecretKey, SubpacketConfig,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{Signature, Subpacket, SubpacketData};
use pgp::types::{KeyDetails, Password, SigningKey, VerifyingKey};
use rand_core::{CryptoRng, RngCore};

use crate::keys::{Part, analyse, parse_public};
use crate::{Error, MAX, guard, stamp};

/// What a signature says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// It holds, made by `keys[key]` (of the keys given to check it with) at `at`; `weak`: over
    /// SHA-1 or older, which no longer proves much.
    Good { key: usize, at: Option<i64>, weak: bool },
    /// It was made by one of the keys given, and it doesn't hold: what it covers was changed.
    Bad,
    /// It was made by a key not given (its key ID, hex).
    Unknown(String),
}

/// What [`decrypt`] or [`open`] found inside.
#[derive(Clone, PartialEq, Eq)]
pub struct Decrypted {
    pub data: Vec<u8>,
    /// Signed inside: what the signature says (`None`: not signed).
    pub check: Option<Check>,
}

impl std::fmt::Debug for Decrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decrypted").field("data", &self.data.len()).field("check", &self.check).finish()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn weak(s: &Signature) -> bool {
    matches!(s.hash_alg(), Some(HashAlgorithm::Sha1 | HashAlgorithm::Md5 | HashAlgorithm::Ripemd160))
}

fn when(s: &Signature) -> Option<i64> {
    s.created().map(|t| i64::from(t.as_secs()))
}

/// The key ID a signature names as its maker.
fn issuer(s: &Signature) -> String {
    if let Some(fp) = s.issuer_fingerprint().first() {
        let h = hex(fp.as_bytes());
        return h.get(h.len().saturating_sub(16)..).unwrap_or_default().to_string();
    }
    s.issuer_key_id().first().map(|id| hex(id.as_ref())).unwrap_or_default()
}

/// A part of a key that may have signed: the primary key or a subkey.
#[derive(Clone, Copy)]
enum Component<'a> {
    Primary(&'a pgp::packet::PublicKey),
    Sub(&'a pgp::packet::PublicSubkey),
}

impl Component<'_> {
    fn as_dyn(&self) -> &dyn VerifyingKey {
        match self {
            Component::Primary(k) => *k,
            Component::Sub(k) => *k,
        }
    }

    /// `s` checked over `data` with this part.
    fn verifies(&self, s: &Signature, data: &[u8]) -> bool {
        match self {
            Component::Primary(k) => s.verify(*k, data).is_ok(),
            Component::Sub(k) => s.verify(*k, data).is_ok(),
        }
    }
}

/// Whether `part` is the key `s` names as its maker.
fn names(s: &Signature, part: &dyn VerifyingKey) -> bool {
    let fp = part.fingerprint();
    let id = part.legacy_key_id();
    s.issuer_fingerprint().iter().any(|f| **f == fp) || s.issuer_key_id().iter().any(|k| **k == id)
}

/// `s` checked by `verify` with each of `keys` (binary public keys) that could have made it.
fn check(s: &Signature, keys: &[&[u8]], verify: &dyn Fn(Component<'_>) -> bool) -> Check {
    let mut named = false;
    for (i, data) in keys.iter().enumerate() {
        let Ok(key) = parse_public(data) else { continue };
        for (part, _, _) in analyse(&key).sign {
            let c = match part {
                Part::Primary => Component::Primary(&key.primary_key),
                Part::Sub(n) => match key.public_subkeys.get(n) {
                    Some(sub) => Component::Sub(&sub.key),
                    None => continue,
                },
            };
            if !names(s, c.as_dyn()) {
                continue;
            }
            named = true;
            if verify(c) {
                return Check::Good { key: i, at: when(s), weak: weak(s) };
            }
        }
    }
    if named { Check::Bad } else { Check::Unknown(issuer(s)) }
}

fn detached(signature: &[u8]) -> Result<DetachedSignature, Error> {
    if signature.len() > MAX {
        return Err(Error::TooBig);
    }
    DetachedSignature::from_reader_single(signature).map(|(s, _)| s).map_err(|_| Error::Malformed("the signature can't be read"))
}

/// The detached `signature` (armored or binary) over `data`, checked with `keys` (binary public
/// keys: those that could have made it).
pub fn verify(data: &[u8], signature: &[u8], keys: &[&[u8]]) -> Result<Check, Error> {
    guard(Error::Malformed("the signature can't be read"), || {
        let sig = detached(signature)?;
        Ok(check(&sig.signature, keys, &|c| c.verifies(&sig.signature, data)))
    })
}

/// A cleartext-signed text (`-----BEGIN PGP SIGNED MESSAGE-----`): the text that was signed
/// (CRLF line ends), and what its signature says.
pub fn verify_cleartext(armored: &[u8], keys: &[&[u8]]) -> Result<(String, Check), Error> {
    if armored.len() > MAX {
        return Err(Error::TooBig);
    }
    guard(Error::Malformed("the signed text can't be read"), || {
        let text = String::from_utf8_lossy(armored);
        let (csf, _) = CleartextSignedMessage::from_string(&text).map_err(|_| Error::Malformed("the signed text can't be read"))?;
        let signed = csf.signed_text();
        let sig = csf.signatures().first().ok_or(Error::Malformed("the signed text has no signature"))?;
        let found = check(sig, keys, &|c| c.verifies(sig, signed.as_bytes()));
        Ok((signed, found))
    })
}

/// Whether `data` (armored or binary) is an encrypted message.
pub fn is_encrypted(data: &[u8]) -> bool {
    data.len() <= MAX && guard(Error::Decrypt, || Ok(Message::from_reader(data).is_ok_and(|(m, _)| m.is_encrypted()))).unwrap_or(false)
}

/// The key IDs (hex) an encrypted message is addressed to (a hidden recipient isn't named).
pub fn recipients(data: &[u8]) -> Vec<String> {
    if data.len() > MAX {
        return Vec::new();
    }
    guard(Error::Decrypt, || {
        let Ok((Message::Encrypted { esk, .. }, _)) = Message::from_reader(data) else { return Ok(Vec::new()) };
        Ok(esk
            .iter()
            .filter_map(|e| match e {
                Esk::PublicKeyEncryptedSessionKey(p) => p.id().ok().map(|id| hex(id.as_ref())).filter(|h| h != "0000000000000000"),
                Esk::SymKeyEncryptedSessionKey(_) => None,
            })
            .take(64)
            .collect())
    })
    .unwrap_or_default()
}

/// What a message that isn't encrypted (any more) holds: decompressed, read to its end (no more
/// than [`MAX`]), and its signature checked with `keys`.
fn unpack(msg: Message<'_>, keys: &[&[u8]], fail: Error) -> Result<Decrypted, Error> {
    // Compressed, or signed over something compressed (a few layers deep at most).
    let mut msg = msg;
    for _ in 0..4 {
        msg = msg.decompress().map_err(|_| fail.clone())?;
    }
    if msg.is_compressed() || msg.is_encrypted() {
        return Err(fail);
    }
    let mut data = Vec::new();
    (&mut msg).take(MAX as u64 + 1).read_to_end(&mut data).map_err(|_| fail.clone())?;
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    let check = match &msg {
        Message::Signed { reader, .. } => {
            // The first signature that holds, else what the first says.
            let mut first = None;
            for i in 0..reader.num_signatures().min(16) {
                let Some(sig) = reader.signature(i) else { continue };
                let c = self::check(sig, keys, &|c| msg.verify_nested_explicit(i, c.as_dyn()).is_ok());
                if matches!(c, Check::Good { .. }) {
                    first = Some(c);
                    break;
                }
                first.get_or_insert(c);
            }
            first
        }
        _ => None,
    };
    Ok(Decrypted { data, check })
}

/// A signed (not encrypted) message, armored or binary: what it holds and its signature checked.
pub fn open(data: &[u8], keys: &[&[u8]]) -> Result<Decrypted, Error> {
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    let bad = Error::Malformed("the signed message can't be read");
    guard(bad.clone(), || {
        let (msg, _) = Message::from_reader(data).map_err(|_| bad.clone())?;
        if msg.is_encrypted() {
            return Err(Error::Decrypt);
        }
        unpack(msg, keys, bad)
    })
}

fn secret(data: &[u8]) -> Result<SignedSecretKey, Error> {
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    SignedSecretKey::from_bytes(data).map_err(|_| Error::Malformed("the secret key can't be read"))
}

/// `data` (armored or binary) decrypted with one of `secrets` (binary secret keys, unlocked), and
/// a signature inside checked with `keys`. A message integrity check that fails is a failure to
/// decrypt: nothing of what was inside comes out.
pub fn decrypt(data: &[u8], secrets: &[&[u8]], keys: &[&[u8]]) -> Result<Decrypted, Error> {
    if data.len() > MAX {
        return Err(Error::TooBig);
    }
    guard(Error::Decrypt, || {
        let mine: Vec<SignedSecretKey> = secrets.iter().filter_map(|s| secret(s).ok()).collect();
        if mine.is_empty() {
            return Err(Error::Decrypt);
        }
        let (msg, _) = Message::from_reader(data).map_err(|_| Error::Malformed("the encrypted message can't be read"))?;
        if !msg.is_encrypted() {
            return Err(Error::Malformed("it isn't encrypted"));
        }
        let none = Password::empty();
        let msg = msg.decrypt_with_keys(mine.iter().map(|_| &none).collect(), mine.iter().collect()).map_err(|_| Error::Decrypt)?;
        if msg.is_encrypted() {
            return Err(Error::Decrypt);
        }
        unpack(msg, keys, Error::Decrypt)
    })
}

/// What a signature made now by `key` says of itself: its maker and time (`now`).
fn subpackets(key: &dyn SigningKey, now: i64) -> Result<SubpacketConfig, Error> {
    let fail = |_| Error::Failed("the signature can't be made".into());
    let hashed = vec![
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint())).map_err(fail)?,
        Subpacket::regular(SubpacketData::SignatureCreationTime(stamp(now))).map_err(fail)?,
    ];
    let unhashed = vec![Subpacket::regular(SubpacketData::IssuerKeyId(key.legacy_key_id())).map_err(fail)?];
    Ok(SubpacketConfig::UserDefined { hashed, unhashed })
}

/// The part of `key` that signs (the newest that may), as rPGP signs with it.
enum Signer<'a> {
    Primary(&'a pgp::packet::SecretKey),
    Sub(&'a pgp::packet::SecretSubkey),
}

fn signer(key: &SignedSecretKey, now: i64) -> Result<Signer<'_>, Error> {
    let public = key.to_public_key();
    let a = analyse(&public);
    let usable = a.sign.iter().filter(|(_, made, until)| *made <= now.saturating_add(86_400) && until.is_none_or(|u| u > now));
    let Some((part, _, _)) = usable.max_by_key(|(p, made, _)| (matches!(p, Part::Sub(_)), *made)) else { return Err(Error::CantSign) };
    match part {
        Part::Primary => Ok(Signer::Primary(&key.primary_key)),
        Part::Sub(n) => {
            let fp = public.public_subkeys.get(*n).map(|s| s.key.fingerprint()).ok_or(Error::CantSign)?;
            key.secret_subkeys.iter().find(|s| s.key.fingerprint() == fp).map(|s| Signer::Sub(&s.key)).ok_or(Error::CantSign)
        }
    }
}

/// A detached signature (armored) over `data`, made at `now` with `key` (a binary secret key,
/// unlocked): SHA-256, binary, as PGP/MIME signs the part exactly as sent.
pub fn sign(data: &[u8], key: &[u8], now: i64, rng: &mut (impl RngCore + CryptoRng)) -> Result<String, Error> {
    let key = secret(key)?;
    let fail = |_| Error::Failed("the signature can't be made".into());
    let sig = match signer(&key, now)? {
        Signer::Primary(k) => {
            DetachedSignature::sign_binary_data_with_subpackets(&mut *rng, k, &Password::empty(), HashAlgorithm::Sha256, data, subpackets(k, now)?)
        }
        Signer::Sub(k) => {
            DetachedSignature::sign_binary_data_with_subpackets(&mut *rng, k, &Password::empty(), HashAlgorithm::Sha256, data, subpackets(k, now)?)
        }
    }
    .map_err(fail)?;
    sig.to_armored_string(ArmorOptions::default()).map_err(fail)
}

/// `data` encrypted (AES-256, integrity protected, armored) to each of `recipients` (binary public
/// keys: each one's newest key that encrypts and is valid at `now`), signed inside with `signer`
/// (a binary secret key, unlocked) when given. A recipient whose key can't be encrypted to is an
/// error that names them.
pub fn encrypt(
    data: &[u8],
    recipients: &[&[u8]],
    signer_key: Option<&[u8]>,
    now: i64,
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<String, Error> {
    let fail = |_| Error::Failed("it can't be encrypted".into());
    let keys = recipients.iter().map(|r| parse_public(r)).collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err(Error::Failed("there's no one to encrypt it to".into()));
    }
    let signing = signer_key.map(secret).transpose()?;
    let mut builder = MessageBuilder::from_bytes("", data.to_vec()).seipd_v1(&mut *rng, SymmetricKeyAlgorithm::AES256);
    for key in &keys {
        let a = analyse(key);
        let who = || a.info.emails.first().cloned().unwrap_or_else(|| a.info.key_id.clone());
        if a.info.encryptable(now).is_err() {
            return Err(Error::CantEncrypt(who()));
        }
        let usable = a.encrypt.iter().filter(|(_, made, until)| *made <= now.saturating_add(86_400) && until.is_none_or(|u| u > now));
        let Some((part, _, _)) = usable.max_by_key(|(_, made, _)| *made) else { return Err(Error::CantEncrypt(who())) };
        match part {
            Part::Primary => builder.encrypt_to_key(&mut *rng, &key.primary_key).map_err(fail)?,
            Part::Sub(n) => builder.encrypt_to_key(&mut *rng, &key.public_subkeys.get(*n).ok_or(Error::CantEncrypt(who()))?.key).map_err(fail)?,
        };
    }
    if let Some(key) = &signing {
        match signer(key, now)? {
            Signer::Primary(k) => builder.sign_with_subpackets(k, Password::empty(), HashAlgorithm::Sha256, subpackets(k, now)?),
            Signer::Sub(k) => builder.sign_with_subpackets(k, Password::empty(), HashAlgorithm::Sha256, subpackets(k, now)?),
        };
    }
    builder.to_armored_string(&mut *rng, ArmorOptions::default()).map_err(fail)
}
