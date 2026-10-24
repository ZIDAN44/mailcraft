//! The CNG calls `rustls-cng` has no safe wrapper for: decrypting an RSA recipient's content key
//! (`NCryptDecrypt`, PKCS #1 v1.5 or OAEP) and an EC recipient's Diffie-Hellman secret (the
//! sender's one-time public key imported beside the private key, `NCryptSecretAgreement`, and
//! the raw secret read with `NCryptDeriveKey`'s `TRUNCATE`, the X9.63 derivation done in Rust).
//!
//! The one place in MailCraft outside vendored code that allows `unsafe`: every block says why
//! it holds, every handle is freed by [`Object`] whatever happens, and every size is checked
//! before it's used.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use mailcraft_smime::cert::Hash;
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDH_PUBLIC_P256_MAGIC, BCRYPT_ECDH_PUBLIC_P384_MAGIC, BCRYPT_ECDSA_PUBLIC_P256_MAGIC,
    BCRYPT_ECDSA_PUBLIC_P384_MAGIC, BCRYPT_KDF_RAW_SECRET, BCRYPT_OAEP_PADDING_INFO, BCRYPT_SHA1_ALGORITHM, BCRYPT_SHA256_ALGORITHM,
    BCRYPT_SHA384_ALGORITHM, BCRYPT_SHA512_ALGORITHM, NCRYPT_HANDLE, NCRYPT_KEY_HANDLE, NCRYPT_PAD_OAEP_FLAG, NCRYPT_PAD_PKCS1_FLAG,
    NCRYPT_PROV_HANDLE, NCRYPT_PROVIDER_HANDLE_PROPERTY, NCRYPT_SECRET_HANDLE, NCryptDecrypt, NCryptDeriveKey, NCryptFreeObject, NCryptGetProperty,
    NCryptImportKey, NCryptSecretAgreement,
};

/// An HRESULT (winerror.h).
pub(super) type HResult = i32;

const NTE_BAD_DATA: HResult = 0x8009_0005_u32 as HResult;
const NTE_BUFFER_TOO_SMALL: HResult = 0x8009_0028_u32 as HResult;

/// The longest content key a recipient's RSA key decrypts to (an AES-256 key is 32 bytes; RSA
/// keys run to 16 384 bits).
const DECRYPTED_MAX: u32 = 2048;

/// A CNG object (a provider, a key, a secret) freed when it goes.
struct Object(NCRYPT_HANDLE);

impl Drop for Object {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: `self.0` is a handle CNG gave this module (`NCryptGetProperty`'s provider,
            // `NCryptImportKey`'s key, `NCryptSecretAgreement`'s secret), owned by this `Object`
            // only and freed exactly once, here.
            let _ = unsafe { NCryptFreeObject(self.0) };
        }
    }
}

fn ok(hr: HResult) -> Result<(), HResult> {
    if hr == 0 { Ok(()) } else { Err(hr) }
}

fn length(b: &[u8]) -> Result<u32, HResult> {
    u32::try_from(b.len()).map_err(|_| NTE_BAD_DATA)
}

/// `wrapped` decrypted with the RSA private key `key` (a handle the caller keeps open for the
/// call): PKCS #1 v1.5, or OAEP with `oaep` (and MGF1 with the same hash, as CNG has it).
pub(super) fn decrypt(key: NCRYPT_KEY_HANDLE, wrapped: &[u8], oaep: Option<Hash>) -> Result<Vec<u8>, HResult> {
    let len = length(wrapped)?;
    let info = oaep.map(|h| BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: match h {
            Hash::Sha1 => BCRYPT_SHA1_ALGORITHM,
            Hash::Sha256 => BCRYPT_SHA256_ALGORITHM,
            Hash::Sha384 => BCRYPT_SHA384_ALGORITHM,
            Hash::Sha512 => BCRYPT_SHA512_ALGORITHM,
        },
        pbLabel: null_mut(),
        cbLabel: 0,
    });
    let (padding, flags) = match &info {
        Some(i) => (i as *const BCRYPT_OAEP_PADDING_INFO as *const c_void, NCRYPT_PAD_OAEP_FLAG),
        None => (null(), NCRYPT_PAD_PKCS1_FLAG),
    };
    let mut size = 0u32;
    // SAFETY: `key` is a live CNG key handle (the caller's `NCryptKey` outlives this call);
    // `wrapped` is readable for `len` bytes; `padding` is null or points to `info`, which lives
    // to the end of this function and whose algorithm name is a static wide string; a null
    // output buffer of 0 bytes asks only for the size, which CNG writes to `size`.
    ok(unsafe { NCryptDecrypt(key, wrapped.as_ptr(), len, padding, null_mut(), 0, &mut size, flags) })?;
    if size == 0 || size > DECRYPTED_MAX {
        return Err(NTE_BAD_DATA);
    }
    let mut out = vec![0u8; size as usize];
    let mut written = 0u32;
    // SAFETY: as above, with `out` writable for `size` bytes: CNG writes at most `size` and says
    // how many in `written`.
    ok(unsafe { NCryptDecrypt(key, wrapped.as_ptr(), len, padding, out.as_mut_ptr(), size, &mut written, flags) })?;
    if written > size {
        return Err(NTE_BUFFER_TOO_SMALL);
    }
    out.truncate(written as usize);
    Ok(out)
}

/// The EC private key `key`'s Diffie-Hellman secret with the public point `point` (SEC 1,
/// uncompressed): the shared point's x, big-endian, the curve's size.
pub(super) fn agree(key: NCRYPT_KEY_HANDLE, point: &[u8]) -> Result<Vec<u8>, HResult> {
    let (size, magics) = match point {
        [0x04, rest @ ..] if rest.len() == 64 => (32usize, [BCRYPT_ECDH_PUBLIC_P256_MAGIC, BCRYPT_ECDSA_PUBLIC_P256_MAGIC]),
        [0x04, rest @ ..] if rest.len() == 96 => (48usize, [BCRYPT_ECDH_PUBLIC_P384_MAGIC, BCRYPT_ECDSA_PUBLIC_P384_MAGIC]),
        _ => return Err(NTE_BAD_DATA),
    };
    let xy = point.get(1..).ok_or(NTE_BAD_DATA)?;
    let mut provider: NCRYPT_PROV_HANDLE = 0;
    let mut written = 0u32;
    // SAFETY: `key` is a live key handle; the output is `provider`, a handle-sized integer whose
    // exact size is passed, written whole by CNG (and a new handle, freed by `Object` below).
    ok(unsafe {
        NCryptGetProperty(
            key,
            NCRYPT_PROVIDER_HANDLE_PROPERTY,
            (&mut provider as *mut NCRYPT_PROV_HANDLE).cast::<u8>(),
            u32::try_from(std::mem::size_of::<NCRYPT_PROV_HANDLE>()).map_err(|_| NTE_BAD_DATA)?,
            &mut written,
            0,
        )
    })?;
    let provider = Object(provider);
    // The key's provider keeps EC keys as ECDH ones or as ECDSA ones: the sender's point is
    // imported as either, the first that agrees wins.
    let mut last = NTE_BAD_DATA;
    for magic in magics {
        // BCRYPT_ECCKEY_BLOB: the magic and the coordinate size (little-endian), then X and Y.
        let mut blob = Vec::with_capacity(8 + xy.len());
        blob.extend_from_slice(&magic.to_le_bytes());
        blob.extend_from_slice(&u32::try_from(size).map_err(|_| NTE_BAD_DATA)?.to_le_bytes());
        blob.extend_from_slice(xy);
        let blob_len = length(&blob)?;
        let mut public: NCRYPT_KEY_HANDLE = 0;
        // SAFETY: `provider.0` is a live provider handle; the blob type is a static wide string;
        // no parameter list; `blob` is readable for `blob_len` bytes; on success CNG writes a new
        // key handle to `public`, freed by `Object`.
        let hr = unsafe { NCryptImportKey(provider.0, 0, BCRYPT_ECCPUBLIC_BLOB, null(), &mut public, blob.as_ptr(), blob_len, 0) };
        if hr != 0 {
            last = hr;
            continue;
        }
        let public = Object(public);
        let mut secret: NCRYPT_SECRET_HANDLE = 0;
        // SAFETY: both key handles are live; on success CNG writes a new secret handle to
        // `secret`, freed by `Object`.
        let hr = unsafe { NCryptSecretAgreement(key, public.0, &mut secret, 0) };
        if hr != 0 {
            last = hr;
            continue;
        }
        let secret = Object(secret);
        let mut z = vec![0u8; size];
        let mut got = 0u32;
        // SAFETY: `secret.0` is a live secret handle; the KDF name is a static wide string; no
        // parameter list; `z` is writable for `size` bytes, CNG writes at most that and says how
        // many in `got`.
        ok(unsafe {
            NCryptDeriveKey(secret.0, BCRYPT_KDF_RAW_SECRET, null(), z.as_mut_ptr(), u32::try_from(size).map_err(|_| NTE_BAD_DATA)?, &mut got, 0)
        })?;
        if got as usize != size {
            return Err(NTE_BAD_DATA);
        }
        // CNG gives the raw secret little-endian.
        z.reverse();
        return Ok(z);
    }
    Err(last)
}

/// Delete `key` from its key store (the tests' imported keys).
#[cfg(test)]
pub(super) fn delete(key: rustls_cng::key::NCryptKey) {
    use windows_sys::Win32::Security::Cryptography::NCryptDeleteKey;
    // SAFETY: `key.inner()` is a live key handle that `key` owns; `NCryptDeleteKey` deletes the
    // key and frees the handle, so `key` is forgotten rather than dropped (which would free it a
    // second time).
    let _ = unsafe { NCryptDeleteKey(key.inner(), 0) };
    std::mem::forget(key);
}
