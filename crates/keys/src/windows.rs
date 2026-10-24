//! Identities in the Windows certificate store (Current User › Personal): certificates with their
//! private key in CNG, as Windows keeps the ones a `.pfx` was imported into, a company deployed,
//! or a smart card's driver shows. The key stays in CNG; Windows may ask permission (or a smart
//! card its PIN) before it's used.
//!
//! Listing and signing go through `rustls-cng`'s safe wrappers, as PdfCraft signs with the store.
//! Decrypting (an RSA recipient's key, `NCryptDecrypt`) and an EC recipient's secret
//! (`NCryptSecretAgreement`) have no safe wrapper, so they're in [`cng`], the one file of this
//! crate that allows `unsafe` there (each block with its `SAFETY:` reasoning).

use mailcraft_smime::cert::Hash;
use mailcraft_smime::{PrivateKey, key};
use rustls_cng::cert::CertContext;
use rustls_cng::error::CngError;
use rustls_cng::key::{AlgorithmGroup, SignaturePadding};
use rustls_cng::store::{CertStore, CertStoreType};

use crate::{Error, Found, KeyRef, hex};

mod cng;

/// The most certificates looked at in the store.
const CERTS_MAX: usize = 512;

fn store_error(e: &CngError) -> Error {
    Error::Keychain(describe(e))
}

fn sha256(der: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(der).to_vec()
}

fn personal() -> Result<CertStore, Error> {
    CertStore::open(CertStoreType::CurrentUser, "My").map_err(|e| store_error(&e))
}

fn contexts() -> Result<Vec<CertContext>, Error> {
    let mut all = personal()?.find_all().map_err(|e| store_error(&e))?;
    all.truncate(CERTS_MAX);
    Ok(all)
}

/// Whether the store has a private key for `context` that MailCraft can use: one that opens
/// without asking, or one that asks (a smart card's PIN, strong key protection) when it's used.
fn has_key(context: &CertContext) -> bool {
    match context.acquire_key(true) {
        Ok(_) => true,
        Err(CngError::WindowsError(NTE_SILENT_CONTEXT | NTE_UI_REQUIRED)) => true,
        Err(_) => false,
    }
}

/// The store's certificates for email (an address in them) with a private key beside them.
pub fn certificates() -> Result<Vec<Found>, Error> {
    let mut found = Vec::new();
    for c in contexts()? {
        let der = c.as_der().to_vec();
        let Ok(info) = mailcraft_smime::cert::info(&der) else { continue };
        if info.emails.is_empty() || !has_key(&c) {
            continue;
        }
        let label = match info.emails.first() {
            Some(e) if !info.name.eq_ignore_ascii_case(e) => format!("{} <{e}>", info.name),
            _ => info.name.clone(),
        };
        found.push(Found { key: KeyRef::WindowsStore { sha256: hex(&sha256(&der)) }, label, cert: der });
    }
    Ok(found)
}

/// The issuers Windows chains `context` to, without the certificate itself (best effort).
pub fn issuers(key: &KeyRef) -> Vec<Vec<u8>> {
    let Ok(StoreKey { context, .. }) = open(key) else { return Vec::new() };
    let own = context.as_der().to_vec();
    context.as_chain_der().map(|chain| chain.into_iter().filter(|c| *c != own).collect()).unwrap_or_default()
}

/// A key in the store.
pub struct StoreKey {
    context: CertContext,
    ec: bool,
}

/// Find the key `key` refers to (a [`KeyRef::WindowsStore`]).
pub fn open(key: &KeyRef) -> Result<StoreKey, Error> {
    let KeyRef::WindowsStore { sha256: want } = key else { return Err(Error::Missing) };
    let want = crate::unhex(want).ok_or(Error::Missing)?;
    let found = personal()?.find_by_sha256(&want).map_err(|e| store_error(&e))?;
    // The hash is the certificate's: anything else found isn't it.
    let context = found.into_iter().find(|c| sha256(c.as_der()) == want).ok_or(Error::Missing)?;
    Ok(StoreKey::new(context))
}

fn failed(e: &CngError) -> mailcraft_smime::Error {
    mailcraft_smime::Error::Unsupported(format!("the Windows certificate store: {}", describe(e)))
}

impl StoreKey {
    pub(crate) fn new(context: CertContext) -> StoreKey {
        let ec = mailcraft_smime::cert::info(context.as_der()).map(|i| i.key.starts_with("EC")).unwrap_or(false);
        StoreKey { context, ec }
    }

    /// The key, opened so its provider may ask (for a PIN, or permission) when it must.
    fn key(&self) -> Result<rustls_cng::key::NCryptKey, mailcraft_smime::Error> {
        self.context.acquire_key(false).map_err(|e| failed(&e))
    }
}

impl PrivateKey for StoreKey {
    fn sign_sha256(&mut self, digest: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        let k = self.key()?;
        if self.ec {
            // CNG gives `r ‖ s`.
            let raw = k.sign(digest, SignaturePadding::None).map_err(|e| failed(&e))?;
            return key::ecdsa_der(&raw);
        }
        // With PKCS #1 padding CNG adds the SHA-256 DigestInfo itself.
        k.sign(digest, SignaturePadding::Pkcs1).map_err(|e| failed(&e))
    }

    fn rsa_decrypt(&mut self, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, mailcraft_smime::Error> {
        let k = self.key()?;
        if !matches!(k.algorithm_group(), Ok(AlgorithmGroup::Rsa)) {
            return Err(mailcraft_smime::Error::Decrypt);
        }
        cng::decrypt(k.inner(), wrapped, oaep).map_err(|_| mailcraft_smime::Error::Decrypt)
    }

    fn ecdh(&mut self, point: &[u8]) -> Result<Vec<u8>, mailcraft_smime::Error> {
        let k = self.key()?;
        cng::agree(k.inner(), point).map_err(|code| {
            if code == NTE_NOT_SUPPORTED as i32 || code == NTE_BAD_KEY_STATE as i32 {
                mailcraft_smime::Error::Unsupported("Windows can't agree a key with this EC key (its provider may keep it for signing only)".into())
            } else {
                mailcraft_smime::Error::Decrypt
            }
        })
    }
}

// Error codes (winerror.h) that explain why a key can't be used.
const NTE_NO_KEY: u32 = 0x8009_000D;
const NTE_BAD_KEY_STATE: u32 = 0x8009_000B;
const NTE_PERM: u32 = 0x8009_0010;
const NTE_NOT_FOUND: u32 = 0x8009_0011;
const NTE_BAD_KEYSET: u32 = 0x8009_0016;
const NTE_PROVIDER_DLL_FAIL: u32 = 0x8009_001D;
const NTE_SILENT_CONTEXT: u32 = 0x8009_0022;
const NTE_NOT_SUPPORTED: u32 = 0x8009_0029;
const NTE_UI_REQUIRED: u32 = 0x8009_002E;
const NTE_DEVICE_NOT_READY: u32 = 0x8009_0030;
const NTE_DEVICE_NOT_FOUND: u32 = 0x8009_0035;
const NTE_USER_CANCELLED: u32 = 0x8009_0036;
const CRYPT_E_NO_KEY_PROPERTY: u32 = 0x8009_200B;

/// A CNG error with the Windows code and, for the usual ones, what it means.
fn describe(e: &CngError) -> String {
    let CngError::WindowsError(code) = e else { return e.to_string() };
    let meaning = match *code {
        NTE_NO_KEY | NTE_BAD_KEYSET | NTE_NOT_FOUND => "the key is missing from its key store",
        NTE_PERM => "access to the key was denied",
        NTE_PROVIDER_DLL_FAIL => "its key provider's software failed to load (a smart card driver that is missing or broken)",
        NTE_NOT_SUPPORTED => "its key provider doesn't do that",
        NTE_DEVICE_NOT_READY | NTE_DEVICE_NOT_FOUND => "the smart card or token isn't available",
        NTE_USER_CANCELLED => "the request was cancelled",
        NTE_SILENT_CONTEXT | NTE_UI_REQUIRED => "the key needs a prompt to open",
        CRYPT_E_NO_KEY_PROPERTY => "the certificate has no private key",
        0x8010_0000..=0x8010_FFFF => "the smart card isn't available",
        _ => return format!("Windows error 0x{code:08X}"),
    };
    format!("{meaning} (Windows error 0x{code:08X})")
}

#[cfg(test)]
mod tests;
