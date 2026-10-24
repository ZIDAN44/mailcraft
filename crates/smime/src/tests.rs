//! Against OpenSSL 3's output (`testdata/`, made with `openssl cms`: a test CA, Ana and Riley
//! with RSA 2048 keys, Zoe with EC P-256), and round trips of MailCraft's own.

use super::*;

/// The fixtures were made on 2026-10-24; their certificates run for two years.
const NOW: i64 = 1_792_900_000;

fn data(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn pem(name: &str) -> Vec<u8> {
    let text = String::from_utf8(data(name)).unwrap();
    let (_, der) = der::pem::decode_vec(text.as_bytes()).unwrap();
    der
}

fn rng() -> rand_core::OsRng {
    rand_core::OsRng
}

fn riley() -> Identity {
    read_p12(&data("riley-aes.p12"), "secret").unwrap()
}

const BODY: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello Riley, the proof is approved.\r\n";

#[test]
fn every_kind_of_p12_opens_and_a_wrong_password_doesnt() {
    for name in ["riley-aes.p12", "riley-legacy.p12", "riley-3des.p12", "zoe-aes.p12"] {
        let id = read_p12(&data(name), "secret").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(id.chain.len(), 2, "{name}: the certificate and its CA");
        let me = cert::info(id.certificate()).unwrap();
        assert!(me.emails.iter().any(|e| e.ends_with(".example")), "{name}: the certificate with the key comes first");
        assert_eq!(cert::info(&id.chain[1]).unwrap().name, "MailCraft Test CA");
    }
    assert_eq!(read_p12(&data("riley-aes.p12"), "wrong"), Err(Error::Password));
    assert_eq!(read_p12(b"not a p12", "secret"), Err(Error::Password));
    let info = cert::info(riley().certificate()).unwrap();
    assert_eq!(
        (info.name.as_str(), info.emails.as_slice(), info.issuer.as_str(), info.key.as_str()),
        ("riley", &["riley@studio.example".to_string()][..], "MailCraft Test CA", "RSA 2048")
    );
    assert!(info.not_before < NOW && NOW < info.not_after);
    assert_eq!(info.fingerprint.len(), 32 * 3 - 1);
    assert!(format!("{:?}", riley()).contains("<redacted>"), "the key never prints");
}

#[test]
fn openssl_signatures_verify() {
    for name in ["sig-rsa.der", "sig-rsa-ber.der", "sig-pss.der", "sig-ec.der"] {
        let (signed, content) = verify(&data(name), Some(BODY)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(content, BODY);
        let who = cert::info(&signed.signer).unwrap();
        assert!(who.name == "ana" || who.name == "zoe", "{name}: {}", who.name);
        assert!(signed.signed_at.is_some_and(|t| (t - NOW).abs() < 86_400 * 30), "{name}");
        assert!(!signed.weak);
        assert!(signed.reads.contains(&oids::AES256_CBC), "{name}: OpenSSL says what it reads");
    }
    // The content carried in the signature comes out.
    let (signed, content) = verify(&data("opaque-ber.der"), None).unwrap();
    assert_eq!(content, BODY);
    assert_eq!(carried(&data("opaque-ber.der")).unwrap(), BODY);
    assert_eq!(cert::info(&signed.signer).unwrap().name, "ana");
}

#[test]
fn a_changed_message_or_signature_is_caught() {
    let changed = String::from_utf8_lossy(BODY).replace("approved", "rejected").into_bytes();
    assert_eq!(verify(&data("sig-rsa.der"), Some(&changed)).err(), Some(Error::BadSignature));
    // One byte of the signature itself changed.
    let mut sig = data("sig-rsa.der");
    let at = sig.len() - 10;
    sig[at] ^= 1;
    assert_eq!(verify(&sig, Some(BODY)).err(), Some(Error::BadSignature));
    // Not a signature at all, or cut short: an error, never a panic.
    let whole = data("sig-rsa.der");
    for cut in [0, 1, 10, whole.len() / 2, whole.len() - 1] {
        assert!(verify(&whole[..cut], Some(BODY)).is_err());
    }
    assert!(verify(&data("enc-cbc.der"), Some(BODY)).is_err());
}

#[test]
fn trust_needs_a_root_the_address_and_dates() {
    let ca = pem("ca.pem");
    let (signed, _) = verify(&data("sig-rsa.der"), Some(BODY)).unwrap();
    assert_eq!(cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "ana@posteo.example").unwrap(), vec![]);
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "ANA@posteo.example ").unwrap(),
        vec![],
        "addresses compare without case"
    );
    // Unknown root, another sender, out of date.
    assert_eq!(cert::check(&signed.signer, &signed.certs, &[], NOW, "ana@posteo.example").unwrap(), vec![Problem::UnknownIssuer]);
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "boss@posteo.example").unwrap(),
        vec![Problem::OtherAddress(vec!["ana@posteo.example".into()])]
    );
    let later = NOW + 86_400 * 900;
    assert!(matches!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), later, "ana@posteo.example").unwrap().as_slice(),
        [Problem::Expired(_)]
    ));
    // A chain that doesn't reach the root: a certificate signed by a CA with the same name, but
    // another key, isn't trusted (the signature on it is checked, not only the name).
    let zoe_ca_pretender = pem("zoe.pem");
    assert_eq!(
        cert::check(&signed.signer, &[], std::slice::from_ref(&zoe_ca_pretender), NOW, "ana@posteo.example").unwrap(),
        vec![Problem::UnknownIssuer]
    );
}

#[test]
fn openssl_encryption_decrypts() {
    let id = riley();
    for (name, authenticated) in
        [("enc-cbc.der", false), ("enc-cbc-ber.der", false), ("enc-3des.der", false), ("enc-gcm.der", true), ("enc-oaep.der", false)]
    {
        let (plain, auth) = decrypt(&data(name), &id, &mut rng()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(plain, BODY, "{name}");
        assert_eq!(auth, authenticated, "{name}");
    }
    // The same with the legacy and 3DES files' key.
    for p12 in ["riley-legacy.p12", "riley-3des.p12"] {
        let id = read_p12(&data(p12), "secret").unwrap();
        assert_eq!(decrypt(&data("enc-cbc.der"), &id, &mut rng()).unwrap().0, BODY);
    }
    // Mail for someone else.
    assert_eq!(decrypt(&data("enc-not-mine.der"), &id, &mut rng()), Err(Error::Decrypt));
}

#[test]
fn changed_encrypted_mail_fails_closed() {
    let id = riley();
    // GCM: any byte changed fails the tag.
    let gcm = data("enc-gcm.der");
    for at in [gcm.len() - 1, gcm.len() - 20, gcm.len() - 40] {
        let mut m = gcm.clone();
        m[at] ^= 0x55;
        assert!(decrypt(&m, &id, &mut rng()).is_err(), "byte {at}");
    }
    // Every cut and every flipped byte of a CBC message: an error or (CBC can't tell) other
    // bytes, never a panic.
    let cbc = data("enc-cbc.der");
    for at in 0..cbc.len() {
        let mut m = cbc.clone();
        m[at] ^= 0xFF;
        let _ = decrypt(&m, &id, &mut rng());
        let _ = decrypt(&cbc[..at], &id, &mut rng());
    }
}

#[test]
fn what_mailcraft_signs_verifies() {
    for p12 in ["riley-aes.p12", "zoe-aes.p12"] {
        let id = read_p12(&data(p12), "secret").unwrap();
        let sig = sign(BODY, &id, NOW, &mut rng()).unwrap();
        let (signed, content) = verify(&sig, Some(BODY)).unwrap_or_else(|e| panic!("{p12}: {e}"));
        assert_eq!(content, BODY);
        assert_eq!(signed.signer, id.chain[0]);
        assert_eq!(signed.signed_at, Some(NOW));
        assert!(signed.reads_gcm(), "it says it reads GCM");
        assert_eq!(signed.certs, vec![id.chain[0].clone()], "the root isn't sent");
        let ca = pem("ca.pem");
        let email = cert::info(&id.chain[0]).unwrap().emails[0].clone();
        assert_eq!(cert::check(&signed.signer, &signed.certs, &[ca], NOW, &email).unwrap(), vec![]);
        assert_eq!(verify(&sig, Some(b"other")).err(), Some(Error::BadSignature));
    }
}

#[test]
fn what_mailcraft_encrypts_decrypts_for_each_recipient() {
    let riley = riley();
    let ana = pem("ana.pem");
    for cipher in [Cipher::Aes256Cbc, Cipher::Aes256Gcm] {
        let sealed = encrypt(BODY, &[riley.chain[0].clone(), ana.clone()], cipher, &mut rng()).unwrap();
        let (plain, auth) = decrypt(&sealed, &riley, &mut rng()).unwrap();
        assert_eq!(plain, BODY);
        assert_eq!(auth, cipher == Cipher::Aes256Gcm);
        // Encrypted again, it isn't the same bytes (a fresh key and IV each time).
        assert_ne!(sealed, encrypt(BODY, &[riley.chain[0].clone()], cipher, &mut rng()).unwrap());
    }
    // An EC certificate can't be encrypted to (yet): said, not a wrong message.
    assert!(matches!(encrypt(BODY, &[pem("zoe.pem")], Cipher::Aes256Cbc, &mut rng()), Err(Error::Unsupported(_))));
    assert!(encrypt(BODY, &[], Cipher::Aes256Cbc, &mut rng()).is_err());
}

#[test]
fn junk_never_panics() {
    let id = riley();
    let mut junk = vec![0x30, 0x80];
    for i in 0..300u32 {
        junk.push((i * 37 % 251) as u8);
        let _ = verify(&junk, Some(BODY));
        let _ = verify(&junk, None);
        let _ = decrypt(&junk, &id, &mut rng());
        let _ = cert::info(&junk);
        let _ = read_p12(&junk, "secret");
    }
}

/// Writes MailCraft's signatures and encryption to `$MAILCRAFT_SMIME_OUT`, for OpenSSL to check
/// (`openssl cms -verify` / `-decrypt`): run by hand.
#[test]
#[ignore]
fn write_for_openssl() {
    let Some(dir) = std::env::var_os("MAILCRAFT_SMIME_OUT") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let riley = riley();
    let zoe = read_p12(&data("zoe-aes.p12"), "secret").unwrap();
    std::fs::write(dir.join("mc-body.txt"), BODY).unwrap();
    std::fs::write(dir.join("mc-sig-rsa.der"), sign(BODY, &riley, NOW, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-sig-ec.der"), sign(BODY, &zoe, NOW, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-enc-cbc.der"), encrypt(BODY, &[riley.chain[0].clone()], Cipher::Aes256Cbc, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-enc-gcm.der"), encrypt(BODY, &[riley.chain[0].clone()], Cipher::Aes256Gcm, &mut rng()).unwrap()).unwrap();
}
