//! The Security framework call `security-framework` has no safe wrapper for: making a `SecKey`
//! of a public EC point (`SecKeyCreateWithData`), the sender's one-time key of an EC recipient's
//! mail, so the Keychain's key can agree a secret with it (the safe `SecKey::key_exchange`).
//!
//! With `windows/cng.rs`, the only place in MailCraft outside vendored code that allows
//! `unsafe`: every block says why it holds.
#![allow(unsafe_code)]

use core_foundation::base::TCFType;
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::error::{CFError, CFErrorRef};
use core_foundation::string::CFString;
use security_framework::key::SecKey;
use security_framework_sys::item::{kSecAttrKeyClass, kSecAttrKeyClassPublic, kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom};
use security_framework_sys::key::SecKeyCreateWithData;

/// A public key of the uncompressed P-256 or P-384 point `point` (SEC 1: `04 ‖ X ‖ Y`).
pub(super) fn public_ec_key(point: &[u8]) -> Result<SecKey, String> {
    if !matches!(point.len(), 65 | 97) || point.first() != Some(&0x04) {
        return Err("the sender's key isn't a P-256 or P-384 point".into());
    }
    // SAFETY: the four attribute names and values are constant CFStrings the Security framework
    // exports for the life of the process; the get rule retains each for the wrapper.
    let (key_type, ec, key_class, public) = unsafe {
        (
            CFString::wrap_under_get_rule(kSecAttrKeyType),
            CFString::wrap_under_get_rule(kSecAttrKeyTypeECSECPrimeRandom),
            CFString::wrap_under_get_rule(kSecAttrKeyClass),
            CFString::wrap_under_get_rule(kSecAttrKeyClassPublic),
        )
    };
    let attributes = CFDictionary::from_CFType_pairs(&[(key_type, ec), (key_class, public)]);
    let data = CFData::from_buffer(point);
    let mut error: CFErrorRef = std::ptr::null_mut();
    // SAFETY: `data` and `attributes` are live CF objects for the call. On failure the framework
    // returns null and writes a new CFError (owned by the caller) to `error`; on success it
    // returns a new SecKey, also owned by the caller.
    let key = unsafe { SecKeyCreateWithData(data.as_concrete_TypeRef(), attributes.as_concrete_TypeRef(), &mut error) };
    if !error.is_null() {
        // SAFETY: a CFError the call created and handed over (the create rule releases it).
        let e = unsafe { CFError::wrap_under_create_rule(error) };
        return Err(format!("the sender's key can't be read: {}", e.description()));
    }
    if key.is_null() {
        return Err("the sender's key can't be read".into());
    }
    // SAFETY: a non-null SecKey the call created and handed over (the create rule releases it).
    Ok(unsafe { SecKey::wrap_under_create_rule(key) })
}
