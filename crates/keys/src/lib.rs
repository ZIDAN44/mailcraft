//! MailCraft's private keys held where it can't read them, signing and decrypting S/MIME mail
//! there through [`mailcraft_smime::PrivateKey`]:
//!
//! * [`pkcs11`]: a smart card or token (a PIV card, a YubiKey, a national ID card, a company's
//!   badge), through the PKCS #11 module its maker ships (OpenSC's, for most). The key never
//!   leaves the card; the card asks for its PIN.
//! * [`macos`]: an identity in the macOS Keychain (one imported there, or a card macOS reads
//!   itself), signing and decrypting through the Security framework.
//! * [`windows`]: an identity in the Windows certificate store, its key in CNG.
//!
//! The Keychain's and CNG's decrypting have no safe binding: `macos/sec.rs` and `windows/cng.rs`
//! are the only files that allow `unsafe` (the crate denies it; every other crate forbids it),
//! each block with its `SAFETY:` reasoning, as PhotoCraft's tablet crate does.
//!
//! What a key is, is said by [`KeyRef`], which MailCraft keeps instead of the key: where to find
//! it again. Native only (they load the system's code).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
pub mod macos;
pub mod pkcs11;
#[cfg(windows)]
pub mod windows;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The card's module couldn't be loaded, or said something went wrong.
    #[error("the smart card: {0}")]
    Card(String),
    /// The PIN was wrong (or the card is locked).
    #[error("the PIN is wrong, or the card is locked")]
    Pin,
    /// The certificate (or its key) isn't there any more.
    #[error("the certificate isn't on the card or in the keychain any more")]
    Missing,
    #[error("the keychain: {0}")]
    Keychain(String),
}

/// Where a key is, kept in place of the key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeyRef {
    /// On a smart card: the module's path, the token's serial number and the key's `CKA_ID`
    /// (hex). Its PIN is kept in the system's keychain, or asked for.
    #[serde(rename_all = "camelCase")]
    Card { module: String, token: String, id: String },
    /// In the macOS Keychain: the certificate's SHA-1 (hex), what the Keychain finds it by.
    #[serde(rename_all = "camelCase")]
    Keychain { sha1: String },
    /// In the Windows certificate store (Current User › Personal): the certificate's SHA-256
    /// (hex). The key stays in CNG.
    #[serde(rename_all = "camelCase")]
    WindowsStore { sha256: String },
}

/// A certificate found with a private key beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub key: KeyRef,
    /// What it's called there (the token's label, the Keychain's name).
    pub label: String,
    /// The certificate (DER).
    pub cert: Vec<u8>,
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok())).collect()
}
