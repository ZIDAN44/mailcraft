//! Against Windows' own CNG: the smime crate's test identities (Riley's RSA key, Zoe's P-256 one)
//! imported into a certificate store held in memory (the user's own store isn't touched), their
//! keys in the Microsoft Software Key Storage Provider, deleted again after each test.
//! What CNG signs and decrypts is checked against the smime crate and OpenSSL's mail.

use rustls_cng::store::Pkcs12Flags;

use super::*;

const NOW: i64 = 1_792_900_000;
const BODY: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello Riley, the proof is approved.\r\n";

fn data(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The identity in `p12`, its key imported into CNG (the Software Key Storage Provider keeps it
/// in a key container of its own, as for any `.pfx`), deleted again when this goes, as PdfCraft's
/// store tests clean up after themselves.
struct InCng {
    _store: CertStore,
    key: StoreKey,
    // Importing the same `.p12` reuses its key container, so one test's clean-up would delete
    // another's key: the tests that hold one take turns.
    _turn: std::sync::MutexGuard<'static, ()>,
}

static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Drop for InCng {
    fn drop(&mut self) {
        if let Ok(k) = self.key.context.acquire_key(true) {
            cng::delete(k);
        }
    }
}

fn in_cng(p12: &str) -> InCng {
    let turn = TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = CertStore::from_pkcs12(&data(p12), "secret", Pkcs12Flags::ALWAYS_CNG_KSP).unwrap_or_else(|e| panic!("{p12}: {}", describe(&e)));
    let context = store.find_all().unwrap().into_iter().find(has_key).unwrap_or_else(|| panic!("{p12}: no key"));
    InCng { _store: store, key: StoreKey::new(context), _turn: turn }
}

#[test]
fn cng_signs_what_anyone_can_check() {
    for (p12, ec) in [("riley-aes.p12", false), ("zoe-aes.p12", true)] {
        let mut held = in_cng(p12);
        let k = &mut held.key;
        assert_eq!(k.ec, ec, "{p12}");
        let chain = vec![k.context.as_der().to_vec()];
        let sig = mailcraft_smime::sign_with(BODY, &chain, k, NOW).unwrap_or_else(|e| panic!("{p12}: {e}"));
        let (signed, _) = mailcraft_smime::verify(&sig, Some(BODY)).unwrap_or_else(|e| panic!("{p12}: {e}"));
        assert_eq!(signed.signer, chain[0], "{p12}");
        // Something else signed doesn't pass for it.
        assert!(mailcraft_smime::verify(&sig, Some(b"Content-Type: text/plain\r\n\r\nOther.\r\n")).is_err(), "{p12}");
    }
}

#[test]
fn cng_decrypts_mail_to_an_rsa_key() {
    let mut k_held = in_cng("riley-aes.p12");
    let k = &mut k_held.key;
    let cert = k.context.as_der().to_vec();
    // OpenSSL's: AES-CBC, BER, 3DES, AES-GCM, and OAEP (SHA-1, its default).
    for (name, authenticated) in
        [("enc-cbc.der", false), ("enc-cbc-ber.der", false), ("enc-3des.der", false), ("enc-gcm.der", true), ("enc-oaep.der", false)]
    {
        let (plain, auth) = mailcraft_smime::decrypt_with(&data(name), &cert, k).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((plain.as_slice(), auth), (BODY, authenticated), "{name}");
    }
    // MailCraft's own, both kinds.
    let mut rng = rand_core::OsRng;
    for cipher in [mailcraft_smime::Cipher::Aes256Cbc, mailcraft_smime::Cipher::Aes256Gcm] {
        let sealed = mailcraft_smime::encrypt(BODY, std::slice::from_ref(&cert), cipher, &mut rng).unwrap();
        assert_eq!(mailcraft_smime::decrypt_with(&sealed, &cert, k).unwrap().0, BODY);
    }
    // Mail for someone else: the one error, whatever the cause.
    assert_eq!(mailcraft_smime::decrypt_with(&data("enc-not-mine.der"), &cert, k), Err(mailcraft_smime::Error::Decrypt));
}

#[test]
fn cng_decrypts_mail_to_an_ec_key() {
    let mut k_held = in_cng("zoe-aes.p12");
    let k = &mut k_held.key;
    let cert = k.context.as_der().to_vec();
    // OpenSSL's: SHA-1 and SHA-256 derivation, and one to an RSA and an EC recipient (GCM).
    for name in ["enc-ec-sha1.der", "enc-ec-sha256.der", "enc-mixed.der"] {
        let (plain, _) = mailcraft_smime::decrypt_with(&data(name), &cert, k).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(plain, BODY, "{name}");
    }
    let sealed = mailcraft_smime::encrypt(BODY, std::slice::from_ref(&cert), mailcraft_smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
    assert_eq!(mailcraft_smime::decrypt_with(&sealed, &cert, k).unwrap().0, BODY);
}

/// What the CNG calls are given straight, as a hostile message could make them: an error each
/// time, never a panic or a crash.
#[test]
fn cng_refuses_what_isnt_right() {
    let mut rsa_held = in_cng("riley-aes.p12");
    let rsa = &mut rsa_held.key;
    for wrapped in [&[][..], &[0u8; 1], &[0xff; 256], &[0u8; 255], &[0x41; 257], &[7u8; 4096]] {
        for oaep in [None, Some(Hash::Sha1), Some(Hash::Sha256), Some(Hash::Sha512)] {
            assert!(rsa.rsa_decrypt(wrapped, oaep).is_err(), "{} bytes, {oaep:?}", wrapped.len());
        }
    }
    // An RSA key does no Diffie-Hellman.
    let mut p256 = vec![4u8];
    p256.extend_from_slice(&[1; 64]);
    assert!(rsa.ecdh(&p256).is_err());
    drop(rsa_held);

    let mut ec_held = in_cng("zoe-aes.p12");
    let ec = &mut ec_held.key;
    // The wrong lengths and forms, a point that isn't on the curve, all zeroes, the curve's
    // other size: refused.
    let mut off_curve = vec![4u8];
    off_curve.extend_from_slice(&[0x11; 64]);
    let mut compressed = vec![2u8];
    compressed.extend_from_slice(&[0x11; 32]);
    let mut zero = vec![4u8];
    zero.extend_from_slice(&[0; 64]);
    let mut p384_sized = vec![4u8];
    p384_sized.extend_from_slice(&[0x11; 96]);
    for point in [&[][..], &[4u8][..], &off_curve, &compressed, &zero, &p384_sized, &[4u8; 66]] {
        assert!(ec.ecdh(point).is_err(), "{} bytes", point.len());
    }
    // An EC key decrypts no RSA.
    assert!(ec.rsa_decrypt(&[1; 256], None).is_err());
    // Every byte of real mail changed in turn: an error or (CBC) other bytes, never a panic.
    let cert = ec.context.as_der().to_vec();
    let m = data("enc-ec-sha256.der");
    for at in (0..m.len()).step_by(3) {
        let mut b = m.clone();
        b[at] ^= 0x41;
        let _ = mailcraft_smime::decrypt_with(&b, &cert, ec);
    }
}

#[test]
fn a_certificate_thats_not_in_the_store_isnt_found() {
    assert_eq!(open(&KeyRef::WindowsStore { sha256: "00".repeat(32) }).err(), Some(Error::Missing));
    assert_eq!(open(&KeyRef::WindowsStore { sha256: "not hex".into() }).err(), Some(Error::Missing));
    assert_eq!(open(&KeyRef::Keychain { sha1: "00".into() }).err(), Some(Error::Missing));
    // Listing the user's own store works, whatever it holds.
    assert!(certificates().is_ok());
}

#[test]
fn errors_say_what_they_mean() {
    assert_eq!(describe(&CngError::WindowsError(NTE_BAD_KEYSET)), "the key is missing from its key store (Windows error 0x80090016)");
    assert_eq!(describe(&CngError::WindowsError(0x8010_000C)), "the smart card isn't available (Windows error 0x8010000C)");
    assert_eq!(describe(&CngError::WindowsError(0x57)), "Windows error 0x00000057");
}

/// The user's own store, as a `.pfx` double-clicked into it leaves it: Riley's and Zoe's test
/// identities imported with keys that can't be exported (`Import-PfxCertificate` without
/// `-Exportable`), and a certificate with no key beside them. Found by the listing, used by
/// reference, as the app does. Run by hand around the import and the clean-up:
///   Import-PfxCertificate -FilePath crates\smime\testdata\riley-aes.p12 -CertStoreLocation Cert:\CurrentUser\My -Password (ConvertTo-SecureString secret -AsPlainText -Force)
///   cargo test -p mailcraft-keys the_users_store -- --ignored
#[test]
#[ignore = "needs the test identities imported into Current User › Personal"]
fn the_users_store_signs_and_decrypts_with_keys_that_cant_leave_it() {
    let found = certificates().unwrap();
    let find = |who: &str| found.iter().find(|f| f.label.contains(who)).unwrap_or_else(|| panic!("{who} isn't listed: {found:?}")).clone();
    let (riley, zoe) = (find("riley@studio.example"), find("zoe"));
    // A certificate with no key (Ana's, imported alone) isn't offered.
    assert!(!found.iter().any(|f| f.label.contains("ana@posteo.example")), "{found:?}");
    for f in [&riley, &zoe] {
        let mut k = open(&f.key).unwrap();
        let sig = mailcraft_smime::sign_with(BODY, std::slice::from_ref(&f.cert), &mut k, NOW).unwrap_or_else(|e| panic!("{}: {e}", f.label));
        assert_eq!(mailcraft_smime::verify(&sig, Some(BODY)).unwrap().0.signer, f.cert);
        let sealed =
            mailcraft_smime::encrypt(BODY, std::slice::from_ref(&f.cert), mailcraft_smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
        assert_eq!(mailcraft_smime::decrypt_with(&sealed, &f.cert, &mut k).unwrap_or_else(|e| panic!("{}: {e}", f.label)).0, BODY);
        if let Ok(dir) = std::env::var("MAILCRAFT_STORE_OUT") {
            let name = if f.label.contains("zoe") { "zoe" } else { "riley" };
            std::fs::write(format!("{dir}/store-{name}.sig.der"), &sig).unwrap();
        }
    }
    let mut k = open(&riley.key).unwrap();
    for name in ["enc-cbc.der", "enc-gcm.der", "enc-oaep.der", "enc-3des.der"] {
        assert_eq!(mailcraft_smime::decrypt_with(&data(name), &riley.cert, &mut k).unwrap_or_else(|e| panic!("{name}: {e}")).0, BODY, "{name}");
    }
    let mut k = open(&zoe.key).unwrap();
    for name in ["enc-ec-sha1.der", "enc-ec-sha256.der", "enc-mixed.der"] {
        assert_eq!(mailcraft_smime::decrypt_with(&data(name), &zoe.cert, &mut k).unwrap_or_else(|e| panic!("{name}: {e}")).0, BODY, "{name}");
    }
    // The chain Windows knows goes with it (the test CA isn't trusted here: just the certificate).
    assert!(issuers(&riley.key).iter().all(|c| *c != riley.cert));
}
