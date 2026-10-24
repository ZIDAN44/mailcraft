//! Smart cards and tokens through PKCS #11 (OASIS PKCS #11 v2.40 / 3.0): the card's own module
//! is loaded, the certificates on it that have a private key beside them (the same `CKA_ID`) are
//! offered, and signing and decrypting happen on the card:
//!
//! * signing: `CKM_RSA_PKCS` over the SHA-256 DigestInfo, or `CKM_ECDSA` over the digest (its
//!   `r ‖ s` written as DER);
//! * an RSA recipient's key: `CKM_RSA_PKCS`, or `CKM_RSA_PKCS_OAEP` with the mail's hash;
//! * an EC recipient's key: `CKM_ECDH1_DERIVE` with no KDF, the shared secret read back (a
//!   session object, never kept on the card) for the X9.63 derivation done here.

use cryptoki::context::{CInitializeArgs, CInitializeFlags, Pkcs11};
use cryptoki::error::RvError;
use cryptoki::mechanism::elliptic_curve::{EcKdf, Ecdh1DeriveParams};
use cryptoki::mechanism::rsa::{PkcsMgfType, PkcsOaepParams, PkcsOaepSource};
use cryptoki::mechanism::{Mechanism, MechanismType};
use cryptoki::object::{Attribute, AttributeType, KeyType, ObjectClass, ObjectHandle};
use cryptoki::session::{Session, UserType};
use cryptoki::types::{AuthPin, Ulong};
use mailcraft_smime::cert::Hash;
use mailcraft_smime::{PrivateKey, key};

use crate::{Error, Found, KeyRef, hex, unhex};

/// The most certificates looked at on one card.
const OBJECTS_MAX: usize = 64;

/// Where the usual modules are, by system: OpenSC's (most cards), then the ones a card's maker
/// installs. The first that exists is offered.
pub fn known_modules() -> Vec<String> {
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Windows\System32\opensc-pkcs11.dll",
            r"C:\Program Files\OpenSC Project\OpenSC\pkcs11\opensc-pkcs11.dll",
            r"C:\Windows\System32\eps2003csp11.dll",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/Library/OpenSC/lib/opensc-pkcs11.so",
            "/opt/homebrew/lib/opensc-pkcs11.so",
            "/usr/local/lib/opensc-pkcs11.so",
            "/usr/local/lib/libykcs11.dylib",
        ]
    } else {
        &[
            "/usr/lib/x86_64-linux-gnu/opensc-pkcs11.so",
            "/usr/lib64/opensc-pkcs11.so",
            "/usr/lib/opensc-pkcs11.so",
            "/usr/lib/aarch64-linux-gnu/opensc-pkcs11.so",
            "/usr/lib/x86_64-linux-gnu/libykcs11.so",
        ]
    };
    candidates.iter().filter(|p| std::path::Path::new(p).exists()).map(|p| p.to_string()).collect()
}

fn card(e: cryptoki::error::Error) -> Error {
    match e {
        cryptoki::error::Error::Pkcs11(RvError::PinIncorrect | RvError::PinLocked | RvError::PinLenRange | RvError::PinInvalid, _) => Error::Pin,
        cryptoki::error::Error::LibraryLoading(e) => Error::Card(format!("its module can't be loaded: {e}")),
        other => Error::Card(other.to_string()),
    }
}

fn load(module: &str) -> Result<Pkcs11, Error> {
    let p = Pkcs11::new(module).map_err(card)?;
    match p.initialize(CInitializeArgs::new(CInitializeFlags::OS_LOCKING_OK)) {
        Ok(()) | Err(cryptoki::error::Error::Pkcs11(RvError::CryptokiAlreadyInitialized, _)) => Ok(p),
        Err(e) => Err(card(e)),
    }
}

/// Whose PIN is given: one the user just typed, or one kept from before (which, if the card
/// changed it, would fail every time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinFrom {
    Typed,
    Kept,
}

/// Log in with `pin` (none: not at all). A locked card isn't tried, and a kept PIN isn't tried on
/// a card down to its last tries: one that fails there can lock it for good (a PIV card or a
/// YubiKey, until its PUK). A PIN just typed is the user's to try.
fn login(session: &Session, token: &cryptoki::slot::TokenInfo, pin: Option<&str>, from: PinFrom) -> Result<(), Error> {
    let Some(pin) = pin else { return Ok(()) };
    if token.user_pin_locked() {
        return Err(Error::Pin);
    }
    if from == PinFrom::Kept && (token.user_pin_final_try() || token.user_pin_count_low()) {
        return Err(Error::PinAtRisk);
    }
    match session.login(UserType::User, Some(&AuthPin::from(pin.to_string()))) {
        Ok(()) | Err(cryptoki::error::Error::Pkcs11(RvError::UserAlreadyLoggedIn, _)) => Ok(()),
        Err(e) => Err(card(e)),
    }
}

fn value(session: &Session, object: ObjectHandle, kind: AttributeType) -> Option<Vec<u8>> {
    session.get_attributes(object, &[kind]).ok()?.into_iter().find_map(|a| match a {
        Attribute::Value(v) | Attribute::Id(v) | Attribute::Label(v) | Attribute::EcPoint(v) => Some(v),
        _ => None,
    })
}

/// The certificates on every card the module at `module` sees that have their private key with
/// them, each with where to find it again. `pin` logs in first (some cards show their
/// certificates only then), but only with one card in: a PIN is one card's, and trying it on
/// another uses up that card's tries. With several, they're listed without it, and the PIN is
/// given to the one chosen.
pub fn certificates(module: &str, pin: Option<&str>) -> Result<Vec<Found>, Error> {
    let p = load(module)?;
    let mut found = Vec::new();
    // The cards in: a reader's empty, never-initialised token holds nothing to count.
    let slots: Vec<_> = p
        .get_slots_with_token()
        .map_err(card)?
        .into_iter()
        .filter_map(|slot| p.get_token_info(slot).ok().filter(|t| t.token_initialized()).map(|t| (slot, t)))
        .collect();
    let pin = if slots.len() == 1 { pin } else { None };
    for (slot, token) in slots {
        let Ok(session) = p.open_ro_session(slot) else { continue };
        login(&session, &token, pin, PinFrom::Typed)?;
        let certs = session.find_objects(&[Attribute::Class(ObjectClass::CERTIFICATE)]).map_err(card)?;
        for c in certs.into_iter().take(OBJECTS_MAX) {
            let (Some(der), Some(id)) = (value(&session, c, AttributeType::Value), value(&session, c, AttributeType::Id)) else { continue };
            let with_key = session.find_objects(&[Attribute::Class(ObjectClass::PRIVATE_KEY), Attribute::Id(id.clone())]).map_err(card)?;
            // Without a PIN, a card's private keys are often hidden: offered all the same.
            if with_key.is_empty() && pin.is_some() {
                continue;
            }
            let label = value(&session, c, AttributeType::Label).map(|l| String::from_utf8_lossy(&l).trim().to_string()).unwrap_or_default();
            let token_label = token.label().trim().to_string();
            found.push(Found {
                key: KeyRef::Card { module: module.to_string(), token: token.serial_number().trim().to_string(), id: hex(&id) },
                label: if label.is_empty() { token_label } else { format!("{label} ({token_label})") },
                cert: der,
            });
        }
    }
    Ok(found)
}

/// A key on a card, logged in to: signs and decrypts there.
pub struct CardKey {
    session: Session,
    key: ObjectHandle,
}

/// Open the key `key` refers to (a [`KeyRef::Card`]), logging in with `pin`.
pub fn open(key: &KeyRef, pin: Option<&str>, from: PinFrom) -> Result<CardKey, Error> {
    let KeyRef::Card { module, token, id } = key else { return Err(Error::Missing) };
    let id = unhex(id).ok_or(Error::Missing)?;
    let p = load(module)?;
    for slot in p.get_slots_with_token().map_err(card)? {
        let Ok(info) = p.get_token_info(slot) else { continue };
        if info.serial_number().trim() != token {
            continue;
        }
        let session = p.open_ro_session(slot).map_err(card)?;
        login(&session, &info, pin, from)?;
        let keys = session.find_objects(&[Attribute::Class(ObjectClass::PRIVATE_KEY), Attribute::Id(id.clone())]).map_err(card)?;
        // A card's private keys often show only once logged in.
        let key = keys.first().copied().ok_or(if pin.is_none() { Error::NeedPin } else { Error::Missing })?;
        return Ok(CardKey { session, key });
    }
    Err(Error::Missing)
}

fn failed(e: cryptoki::error::Error) -> mailcraft_smime::Error {
    mailcraft_smime::Error::Unsupported(format!("the smart card: {}", card(e)))
}

impl CardKey {
    fn is_ec(&self) -> bool {
        self.session
            .get_attributes(self.key, &[AttributeType::KeyType])
            .ok()
            .into_iter()
            .flatten()
            .any(|a| matches!(a, Attribute::KeyType(k) if k == KeyType::EC))
    }
}

impl PrivateKey for CardKey {
    fn sign_sha256(&mut self, digest: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        if self.is_ec() {
            let raw = self.session.sign(&Mechanism::Ecdsa, self.key, digest).map_err(failed)?;
            return key::ecdsa_der(&raw);
        }
        self.session.sign(&Mechanism::RsaPkcs, self.key, &key::sha256_digest_info(digest)).map_err(failed)
    }

    fn rsa_decrypt(&mut self, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, mailcraft_smime::Error> {
        let mechanism = match oaep {
            None => Mechanism::RsaPkcs,
            Some(h) => {
                let (hash, mgf) = match h {
                    Hash::Sha1 => (MechanismType::SHA1, PkcsMgfType::MGF1_SHA1),
                    Hash::Sha256 => (MechanismType::SHA256, PkcsMgfType::MGF1_SHA256),
                    Hash::Sha384 => (MechanismType::SHA384, PkcsMgfType::MGF1_SHA384),
                    Hash::Sha512 => (MechanismType::SHA512, PkcsMgfType::MGF1_SHA512),
                };
                Mechanism::RsaPkcsOaep(PkcsOaepParams::new(hash, mgf, PkcsOaepSource::empty()))
            }
        };
        self.session.decrypt(&mechanism, self.key, wrapped).map_err(|_| mailcraft_smime::Error::Decrypt)
    }

    fn ecdh(&mut self, point: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        // The secret is the curve's size: P-256's 32 bytes, P-384's 48.
        let len = match point.len() {
            65 => 32,
            97 => 48,
            _ => return Err(mailcraft_smime::Error::Decrypt),
        };
        let template = [
            Attribute::Class(ObjectClass::SECRET_KEY),
            Attribute::KeyType(KeyType::GENERIC_SECRET),
            Attribute::ValueLen(Ulong::try_from(len).map_err(|_| mailcraft_smime::Error::Decrypt)?),
            Attribute::Token(false),
            Attribute::Sensitive(false),
            Attribute::Extractable(true),
        ];
        // The point as it is, or as a DER OCTET STRING, which some cards want instead.
        let mut wrapped = vec![0x04, u8::try_from(point.len()).map_err(|_| mailcraft_smime::Error::Decrypt)?];
        wrapped.extend_from_slice(point);
        for public in [point, wrapped.as_slice()] {
            let mechanism = Mechanism::Ecdh1Derive(Ecdh1DeriveParams::new(EcKdf::null(), public));
            if let Ok(secret) = self.session.derive_key(&mechanism, self.key, &template) {
                let z = value(&self.session, secret, AttributeType::Value);
                let _ = self.session.destroy_object(secret);
                if let Some(z) = z.filter(|z| z.len() == len) {
                    return Ok(z);
                }
            }
        }
        Err(mailcraft_smime::Error::Decrypt)
    }
}

#[cfg(test)]
mod tests;
