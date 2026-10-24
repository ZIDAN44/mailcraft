//! Against a real PKCS #11 module: SoftHSM 2 with a token holding an RSA and an EC certificate
//! and their keys (`crates/keys/softhsm.sh` sets it up, in Docker). Run with
//! `MAILCRAFT_PKCS11_MODULE=/usr/lib/softhsm/libsofthsm2.so cargo test -p mailcraft-keys -- --ignored`.

use super::*;

const PIN: &str = "123456";

fn module() -> String {
    std::env::var("MAILCRAFT_PKCS11_MODULE").unwrap_or_else(|_| "/usr/lib/softhsm/libsofthsm2.so".into())
}

/// A chain for a certificate on the card: itself, then the CA the script left beside the token.
fn chain(cert: &[u8]) -> Vec<Vec<u8>> {
    let ca = std::env::var("MAILCRAFT_PKCS11_CA").ok().and_then(|p| std::fs::read(p).ok()).and_then(|t| der_pem(&t));
    let mut c = vec![cert.to_vec()];
    c.extend(ca);
    c
}

fn der_pem(text: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(text).ok()?;
    let body: String = s.lines().filter(|l| !l.starts_with("-----")).collect();
    base64_decode(&body)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()) {
        acc = (acc << 6) | T.iter().position(|t| *t == c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[test]
#[ignore = "needs SoftHSM 2 (crates/keys/softhsm.sh)"]
fn a_card_signs_and_decrypts_what_openssl_and_mailcraft_read() {
    if std::env::var_os("MAILCRAFT_PKCS11_SECOND").is_some() {
        return;
    }
    let found = certificates(&module(), Some(PIN)).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    // One card in: a wrong PIN is refused. The card now counts its tries down: a kept PIN isn't
    // tried while it does, a typed one is, and a good login sets it right again.
    assert!(matches!(certificates(&module(), Some("000000")), Err(Error::Pin)));
    assert!(matches!(open(&found[0].key, Some(PIN), PinFrom::Kept), Err(Error::PinAtRisk)), "a kept PIN waits");
    assert!(open(&found[0].key, Some(PIN), PinFrom::Typed).is_ok(), "a typed one may try");
    let now = 1_792_900_000;
    let mut rng = rand_core::OsRng;
    for f in &found {
        let mut k = open(&f.key, Some(PIN), PinFrom::Kept).unwrap();
        // Signed on the card, checked as any reader would.
        let content = b"Content-Type: text/plain\r\n\r\nSigned on the card.\r\n";
        let sig = mailcraft_smime::sign_with(content, &chain(&f.cert), &mut k, now).unwrap_or_else(|e| panic!("{}: {e}", f.label));
        let (signed, _) = mailcraft_smime::verify(&sig, Some(content)).unwrap();
        assert_eq!(signed.signer, f.cert, "{}", f.label);
        // Encrypted to the card's certificate, CBC and GCM, decrypted on the card.
        for cipher in [mailcraft_smime::Cipher::Aes256Cbc, mailcraft_smime::Cipher::Aes256Gcm] {
            let sealed = mailcraft_smime::encrypt(b"for the card only", std::slice::from_ref(&f.cert), cipher, &mut rng).unwrap();
            let (open_text, _) = mailcraft_smime::decrypt_with(&sealed, &f.cert, &mut k).unwrap_or_else(|e| panic!("{}: {e}", f.label));
            assert_eq!(open_text, b"for the card only", "{}", f.label);
        }
        // Written out for OpenSSL to check (softhsm.sh reads them).
        if let Ok(dir) = std::env::var("MAILCRAFT_PKCS11_OUT") {
            let name = f.label.split_whitespace().next().unwrap_or("key").to_string();
            std::fs::write(format!("{dir}/{name}.sig.der"), &sig).unwrap();
        }
    }
    // A wrong PIN is refused; after it, a kept one waits; with none, the card says it needs one
    // (its keys show only once logged in); a good typed login sets it right again.
    let first = &found[0].key;
    assert!(matches!(open(first, Some("000000"), PinFrom::Kept), Err(Error::Pin)));
    assert!(matches!(open(first, Some("000000"), PinFrom::Kept), Err(Error::PinAtRisk)), "never a second wrong try");
    assert!(matches!(open(first, None, PinFrom::Kept), Err(Error::NeedPin)));
    assert!(open(first, Some(PIN), PinFrom::Typed).is_ok());
    assert!(open(first, Some(PIN), PinFrom::Kept).is_ok());
    // A key that isn't there.
    let gone = KeyRef::Card { module: module(), token: "no such token".into(), id: "01".into() };
    assert!(matches!(open(&gone, Some(PIN), PinFrom::Kept), Err(Error::Missing)));
}

#[test]
fn modules_that_arent_there_arent_offered_and_dont_load() {
    for m in known_modules() {
        assert!(std::path::Path::new(&m).exists());
    }
    assert!(matches!(certificates("/no/such/module.so", None), Err(Error::Card(_))));
    let r = KeyRef::Card { module: "/no/such/module.so".into(), token: "t".into(), id: "zz".into() };
    assert!(open(&r, None, PinFrom::Kept).is_err());
}

/// Two cards in (`softhsm.sh` adds the second): a PIN typed for the list isn't tried on either,
/// so a wrong one can't use up the other card's tries; the certificates are still listed, and
/// the card chosen opens with its own PIN.
#[test]
#[ignore = "needs SoftHSM 2 with two tokens (crates/keys/softhsm.sh)"]
fn two_cards_dont_share_a_pin() {
    if std::env::var_os("MAILCRAFT_PKCS11_SECOND").is_none() {
        return;
    }
    let found = certificates(&module(), Some("000000")).unwrap();
    assert_eq!(found.len(), 2, "listed without logging in: {found:?}");
    assert!(open(&found[0].key, Some(PIN), PinFrom::Typed).is_ok());
}
