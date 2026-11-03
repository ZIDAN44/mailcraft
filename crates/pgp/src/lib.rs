//! MailCraft OpenPGP (RFC 9580): the cryptography under PGP/MIME (RFC 3156) and inline PGP mail,
//! with the MIME around it left to `mailcraft-mime`, on rPGP.
//!
//! * [`read_secret_key`] opens a secret key file (armored `.asc` or binary, with or without a
//!   passphrase) into an [`Unlocked`] key: the key without its passphrase, to be kept where the
//!   system keeps secrets, and its public part.
//! * [`public_keys`] reads correspondents' keys (a key file, an `application/pgp-keys` part,
//!   Autocrypt's `keydata`), and [`info`] says what a key is and until when it can be used
//!   ([`KeyInfo`]); its self-signatures are checked, so no one can add a subkey of theirs to
//!   someone else's key.
//! * [`sign`] makes a detached signature; [`verify`] checks one, and [`verify_cleartext`] a
//!   cleartext-signed text ([`Check`]).
//! * [`encrypt`] encrypts (AES-256, integrity protected) to keys' encryption subkeys, signing
//!   inside when asked; [`decrypt`] opens a message with the keys given, checking a signature
//!   inside against the keys that could have made it.
//! * [`autocrypt`] reads and writes the `Autocrypt:` header (Autocrypt Level 1), and [`inline`]
//!   finds a PGP block in a message's text.
//!
//! Everything read is hostile: sizes are bounded (what a message decompresses to included), and
//! nothing here panics, rPGP's own code run behind a guard. A failure to decrypt is one error
//! whatever its cause. Nothing here reads the clock (the caller passes the time; rPGP stamps
//! what it signs with it) or makes its own random numbers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod armor;
pub mod autocrypt;
mod keys;
mod message;

pub use armor::{Inline, inline};
pub use keys::{KeyInfo, Problem, Unlocked, info, minimal, public_keys, read_secret_key, same_key};
pub use message::{Check, Decrypted, decrypt, encrypt, is_encrypted, open, recipients, sign, verify, verify_cleartext};

/// Larger OpenPGP input than this isn't read, and nothing decrypts or decompresses to more.
pub const MAX: usize = 64 << 20;

/// The most keys read from one file or part.
pub const KEYS_MAX: usize = 64;

/// What went wrong, in words fit to show.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("it isn't valid OpenPGP ({0})")]
    Malformed(&'static str),
    #[error("the passphrase is wrong")]
    Passphrase,
    #[error("the file has no OpenPGP secret key")]
    NoSecretKey,
    #[error("there's no OpenPGP key in it")]
    NoKey,
    #[error("the key for {0} can't be used for encryption")]
    CantEncrypt(String),
    #[error("the key can't sign")]
    CantSign,
    /// One error for every way decrypting can fail.
    #[error("it can't be decrypted with your OpenPGP key")]
    Decrypt,
    #[error("it's too big")]
    TooBig,
    /// Making a signature or an encrypted message failed (never from what was read).
    #[error("{0}")]
    Failed(String),
}

/// `f`, with any panic in rPGP's code on hostile input taken as `err`.
pub(crate) fn guard<T>(err: Error, f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(err))
}

/// `t` as rPGP's timestamp (Unix seconds, clamped to what a packet holds).
pub(crate) fn stamp(t: i64) -> pgp::types::Timestamp {
    pgp::types::Timestamp::from_secs(u32::try_from(t.max(0)).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests;
