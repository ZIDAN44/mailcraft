use pgp::composed::{EncryptionCaps, KeyType, SecretKeyParamsBuilder, SubkeyParamsBuilder};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::ser::Serialize as _;

use super::*;

mod gnupg;
mod hostile;

const NOW: i64 = 1_800_000_000;

/// A key made here: Ed25519 to sign, X25519 (or none) to encrypt, locked with `passphrase`.
fn make(uid: &str, passphrase: Option<&str>, encrypts: bool) -> Vec<u8> {
    let rng = rand_core::OsRng;
    let mut sub = Vec::new();
    if encrypts {
        sub.push(
            SubkeyParamsBuilder::default()
                .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
                .can_encrypt(EncryptionCaps::All)
                .passphrase(passphrase.map(str::to_string))
                .build()
                .unwrap(),
        );
    }
    let params = SecretKeyParamsBuilder::default()
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id(uid.into())
        .passphrase(passphrase.map(str::to_string))
        .subkeys(sub)
        .build()
        .unwrap();
    params.generate(rng).unwrap().to_armored_bytes(Default::default()).unwrap()
}

fn unlocked(uid: &str) -> Unlocked {
    read_secret_key(&make(uid, Some("pw"), true), "pw").unwrap()
}

#[test]
fn a_secret_key_opens_with_its_passphrase_only() {
    let file = make("Ana Lima <Ana@Posteo.example>", Some("correct horse"), true);
    assert_eq!(read_secret_key(&file, "wrong").unwrap_err(), Error::Passphrase);
    let k = read_secret_key(&file, "correct horse").unwrap();
    assert_eq!(k.info.emails, ["ana@posteo.example"]);
    assert_eq!(k.info.name, "Ana Lima");
    assert!(k.info.encrypts && k.info.signs && k.info.revoked.is_none() && k.info.expires.is_none());
    assert_eq!(k.info.fingerprint.len(), 40);
    assert!(!format!("{k:?}").contains("secret: ["), "never in a log");
    // What's kept opens without it, and its public part is the same key.
    let again = read_secret_key(&k.secret, "").unwrap();
    assert_eq!(again.info.fingerprint, k.info.fingerprint);
    assert!(same_key(&k.public, &again.public));
    // A key without a passphrase needs none; a public key isn't a secret one.
    let open = make("Bo <bo@studio.example>", None, true);
    assert!(read_secret_key(&open, "anything").is_ok());
    let public = public_keys(&open).unwrap();
    assert_eq!(read_secret_key(&public[0], "").unwrap_err(), Error::NoSecretKey);
}

#[test]
fn mail_encrypted_and_signed_reads_back() {
    let (ana, bo) = (unlocked("Ana <ana@posteo.example>"), unlocked("Bo <bo@studio.example>"));
    let text = b"Content-Type: text/plain\r\n\r\nThe plan.\r\n";
    let sealed = encrypt(text, &[&ana.public, &bo.public], Some(&bo.secret), NOW, &mut rand_core::OsRng).unwrap();
    assert!(sealed.starts_with("-----BEGIN PGP MESSAGE-----"));
    assert!(is_encrypted(sealed.as_bytes()));
    assert_eq!(recipients(sealed.as_bytes()).len(), 2);
    // Ana reads it, the signature checked with the keys that could have made it.
    let d = decrypt(sealed.as_bytes(), &[&ana.secret], &[&ana.public, &bo.public]).unwrap();
    assert_eq!(d.data, text);
    assert_eq!(d.check, Some(Check::Good { key: 1, at: Some(NOW), weak: false }));
    // Without Bo's key: signed by someone unknown, named by key ID.
    let d = decrypt(sealed.as_bytes(), &[&ana.secret], &[]).unwrap();
    assert_eq!(d.check, Some(Check::Unknown(bo.info.key_id.clone())));
    // Someone else's key opens nothing.
    let zoe = unlocked("Zoe <zoe@posteo.example>");
    assert_eq!(decrypt(sealed.as_bytes(), &[&zoe.secret], &[]).unwrap_err(), Error::Decrypt);
    // Not signed: no check.
    let plain = encrypt(text, &[&ana.public], None, NOW, &mut rand_core::OsRng).unwrap();
    assert_eq!(decrypt(plain.as_bytes(), &[&ana.secret], &[&bo.public]).unwrap().check, None);
}

#[test]
fn a_changed_message_doesnt_decrypt() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let sealed = encrypt(&[b'x'; 2000], &[&ana.public], None, NOW, &mut rand_core::OsRng).unwrap();
    let binary = {
        let (m, _) = pgp::composed::Message::from_string(&sealed).unwrap();
        let _ = m;
        // Flip a byte in the middle of the armored body: the base64 or the integrity check fails.
        let mut b = sealed.clone().into_bytes();
        let mid = b.len() / 2;
        b[mid] = if b[mid] == b'A' { b'B' } else { b'A' };
        b
    };
    assert!(decrypt(&binary, &[&ana.secret], &[]).is_err());
}

#[test]
fn detached_signatures_hold_or_dont() {
    let (bo, zoe) = (unlocked("Bo <bo@studio.example>"), unlocked("Zoe <zoe@posteo.example>"));
    let part = b"Content-Type: text/plain\r\n\r\nSigned.\r\n";
    let sig = sign(part, &bo.secret, NOW, &mut rand_core::OsRng).unwrap();
    assert!(sig.starts_with("-----BEGIN PGP SIGNATURE-----"));
    assert_eq!(verify(part, sig.as_bytes(), &[&zoe.public, &bo.public]).unwrap(), Check::Good { key: 1, at: Some(NOW), weak: false });
    assert_eq!(verify(b"Content-Type: text/plain\r\n\r\nChanged.\r\n", sig.as_bytes(), &[&bo.public]).unwrap(), Check::Bad);
    assert_eq!(verify(part, sig.as_bytes(), &[&zoe.public]).unwrap(), Check::Unknown(bo.info.key_id.clone()));
    assert!(verify(part, b"-----BEGIN PGP SIGNATURE-----\n\nAAAA\n-----END PGP SIGNATURE-----\n", &[&bo.public]).is_err());
    // A signed (not encrypted) message: opened and checked.
    let signed = {
        let k: pgp::composed::SignedSecretKey = pgp::composed::Deserializable::from_bytes(&bo.secret[..]).unwrap();
        let mut b = pgp::composed::MessageBuilder::from_bytes("", &b"hello"[..]);
        b.sign(&k.primary_key, pgp::types::Password::empty(), pgp::crypto::hash::HashAlgorithm::Sha256);
        b.to_armored_string(rand_core::OsRng, Default::default()).unwrap()
    };
    assert!(!is_encrypted(signed.as_bytes()));
    let d = open(signed.as_bytes(), &[&bo.public]).unwrap();
    assert_eq!(d.data, b"hello");
    assert!(matches!(d.check, Some(Check::Good { key: 0, .. })));
}

#[test]
fn cleartext_signed_text_is_checked() {
    let bo = unlocked("Bo <bo@studio.example>");
    let k: pgp::composed::SignedSecretKey = pgp::composed::Deserializable::from_bytes(&bo.secret[..]).unwrap();
    let csf =
        pgp::composed::CleartextSignedMessage::sign(rand_core::OsRng, "Hello\n- dashes\nthere\n", &k.primary_key, &pgp::types::Password::empty())
            .unwrap()
            .to_armored_string(Default::default())
            .unwrap();
    let Some(Inline::Signed(block)) = inline(csf.as_bytes()) else { panic!("{csf}") };
    let (text, check) = verify_cleartext(block, &[&bo.public]).unwrap();
    assert!(text.starts_with("Hello\r\n- dashes"), "{text:?}");
    assert!(matches!(check, Check::Good { key: 0, .. }));
    let changed = csf.replace("there", "where");
    assert_eq!(verify_cleartext(changed.as_bytes(), &[&bo.public]).unwrap().1, Check::Bad);
}

#[test]
fn a_key_without_an_encryption_subkey_says_so() {
    let sign_only = read_secret_key(&make("Sig <sig@studio.example>", None, false), "").unwrap();
    assert!(!sign_only.info.encrypts && sign_only.info.signs);
    assert_eq!(sign_only.info.encryptable(NOW), Err(Problem::NoEncryption));
    assert_eq!(encrypt(b"x", &[&sign_only.public], None, NOW, &mut rand_core::OsRng).unwrap_err(), Error::CantEncrypt("sig@studio.example".into()));
    // It still signs.
    assert!(sign(b"x", &sign_only.secret, NOW, &mut rand_core::OsRng).is_ok());
}

#[test]
fn expiry_and_withdrawal_are_problems() {
    let mut i = KeyInfo { encrypts: true, created: 100, ..Default::default() };
    assert!(i.problems(NOW).is_empty() && i.encryptable(NOW).is_ok());
    i.expires = Some(NOW - 10);
    assert_eq!(i.problems(NOW), [Problem::Expired(NOW - 10)]);
    assert_eq!(i.encryptable(NOW), Err(Problem::Expired(NOW - 10)));
    i.expires = None;
    i.encrypt_until = Some(NOW - 5);
    assert_eq!(i.encryptable(NOW), Err(Problem::Expired(NOW - 5)));
    i.revoked = Some(200);
    assert_eq!(i.encryptable(NOW), Err(Problem::Revoked(200)));
    assert_eq!(KeyInfo { fingerprint: "ABCD1234EF".into(), ..Default::default() }.spaced(), "ABCD 1234 EF");
}

#[test]
fn autocrypt_sends_the_least_of_a_key() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let small = minimal(&ana.public, "ana@posteo.example").unwrap();
    let i = info(&small).unwrap();
    assert_eq!(i.fingerprint, ana.info.fingerprint);
    assert!(i.encrypts && i.is_for("ANA@posteo.example"));
    let h = autocrypt::header("ana@posteo.example", &small, false);
    let back = autocrypt::parse(h.trim_start_matches("Autocrypt:")).unwrap();
    assert!(same_key(&back.keydata, &ana.public));
}

#[test]
fn keys_come_from_any_file_of_them() {
    let (ana, bo) = (unlocked("Ana <ana@posteo.example>"), unlocked("Bo <bo@studio.example>"));
    let armored = |k: &[u8]| {
        let p: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(k).unwrap();
        p.to_armored_string(Default::default()).unwrap()
    };
    // Two armored blocks with text around them, and binary ones one after the other.
    let file = format!("Here are our keys:\n{}\nand\n{}\n-- \nsig", armored(&ana.public), armored(&bo.public));
    assert_eq!(public_keys(file.as_bytes()).unwrap().len(), 2);
    assert_eq!(public_keys(&[ana.public.clone(), bo.public.clone()].concat()).unwrap().len(), 2);
    assert_eq!(public_keys(b"not a key").unwrap_err(), Error::NoKey);
    let p: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    assert_eq!(p.to_bytes().unwrap(), ana.public);
}

/// Someone else's encryption subkey bound to Ana's key (a signature that isn't hers) doesn't
/// count: mail to Ana is never encrypted to it.
#[test]
fn a_subkey_someone_else_bound_doesnt_count() {
    let (ana, eve) = (unlocked("Ana <ana@posteo.example>"), unlocked("Eve <eve@posteo.example>"));
    let mut a: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    let e: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&eve.public[..]).unwrap();
    a.public_subkeys = e.public_subkeys.clone();
    let forged = a.to_bytes().unwrap();
    let i = info(&forged).unwrap();
    assert!(!i.encrypts, "Eve's subkey isn't Ana's");
    assert!(encrypt(b"x", &[&forged], None, NOW, &mut rand_core::OsRng).is_err());
}

#[test]
fn junk_never_panics() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let mut inputs: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"-----BEGIN PGP MESSAGE-----\n\n-----END PGP MESSAGE-----\n".to_vec(),
        b"-----BEGIN PGP MESSAGE-----\n\n!!!!\n-----END PGP MESSAGE-----\n".to_vec(),
        b"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nx\n-----BEGIN PGP SIGNATURE-----\n\n-----END PGP SIGNATURE-----\n".to_vec(),
        vec![0xFF; 64],
        vec![0x84; 10],
        (0..=255u8).collect(),
    ];
    // Every prefix of a real message and key.
    let sealed = encrypt(b"hello", &[&ana.public], Some(&ana.secret), NOW, &mut rand_core::OsRng).unwrap().into_bytes();
    for n in (0..sealed.len()).step_by(7) {
        inputs.push(sealed[..n].to_vec());
    }
    for n in (0..ana.public.len()).step_by(5) {
        inputs.push(ana.public[..n].to_vec());
    }
    for x in &inputs {
        let _ = decrypt(x, &[&ana.secret], &[&ana.public]);
        let _ = open(x, &[&ana.public]);
        let _ = verify(b"data", x, &[&ana.public]);
        let _ = verify_cleartext(x, &[&ana.public]);
        let _ = public_keys(x);
        let _ = info(x);
        let _ = read_secret_key(x, "pw");
        let _ = recipients(x);
        let _ = is_encrypted(x);
        let _ = inline(x);
        let _ = minimal(x, "a@b");
    }
}
