//! MailCraft S/MIME (RFC 8551): the cryptography under signed and encrypted mail, with the MIME
//! around it left to `mailcraft-mime`.
//!
//! * `read_p12` (desktop only) opens a `.p12` / `.pfx` file (PBES2 with AES, and the older 3DES and RC2-40
//!   ones Windows and Firefox still write) into an [`Identity`]: the private key and its chain.
//! * [`sign`] makes a detached `SignedData` (SHA-256; RSA PKCS #1 v1.5 or ECDSA) with the
//!   attributes RFC 8551 §2.5 asks for; [`verify`] checks one, detached or with its content, and
//!   says who signed and what they can read ([`Signed`]).
//! * [`encrypt`] makes `EnvelopedData` (AES-256-CBC, what every mail program reads) or
//!   `AuthEnvelopedData` (AES-256-GCM, authenticated: for recipients who said they read it), to
//!   RSA certificates; [`decrypt`] opens both, with AES-CBC/GCM or 3DES, PKCS #1 v1.5 or OAEP.
//! * [`cert::info`] says what a certificate is, and [`cert::check`] whether to trust it for a
//!   sender (chain to a root, in date, for email, for that address).
//!
//! Everything read is hostile: BER is rewritten as DER first ([`ber`]) with every size bounded,
//! and nothing here panics. A failure to decrypt is one error whatever its cause, so a reply
//! can't tell an attacker which step failed. Nothing here reads the clock or makes random
//! numbers itself: the caller passes the time and a cryptographic random number generator.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod ber;
pub mod cert;
mod envelope;
pub mod oids;
#[cfg(not(target_arch = "wasm32"))]
mod p12;
mod sign;

pub use cert::{CertInfo, Problem};
pub use envelope::{Cipher, decrypt, encrypt};
#[cfg(not(target_arch = "wasm32"))]
pub use p12::read_p12;
pub use sign::{Signed, sign, verify};

/// Larger CMS than this isn't read (a message's size limit is far below it).
pub const CMS_MAX: usize = 64 << 20;

/// What went wrong, in words fit to show.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("it isn't valid S/MIME ({0})")]
    Malformed(&'static str),
    #[error("the password is wrong, or it isn't a certificate file")]
    Password,
    #[error("the file has no private key with its certificate")]
    NoKey,
    #[error("{0} isn't supported")]
    Unsupported(String),
    /// The signature doesn't match the message: it was changed after it was signed, or the
    /// signature isn't the signer's.
    #[error("the signature doesn't match the message")]
    BadSignature,
    /// One error for every way decrypting can fail.
    #[error("it can't be decrypted with your certificate")]
    Decrypt,
}

/// A certificate with its private key, as a `.p12` file holds it.
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    /// The private key, PKCS #8 DER.
    pub key: Vec<u8>,
    /// The certificate first, then the ones that issued it (DER).
    pub chain: Vec<Vec<u8>>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("key", &"<redacted>").field("chain", &self.chain.len()).finish()
    }
}

impl Identity {
    /// The certificate itself.
    pub fn certificate(&self) -> &[u8] {
        self.chain.first().map(Vec::as_slice).unwrap_or_default()
    }
}

/// DER of `input` (BER or DER), within [`CMS_MAX`].
pub(crate) fn der_of(input: &[u8]) -> Result<Vec<u8>, Error> {
    if input.len() > CMS_MAX {
        return Err(Error::Malformed("too big"));
    }
    ber::to_der(input)
}

#[cfg(test)]
mod tests;
