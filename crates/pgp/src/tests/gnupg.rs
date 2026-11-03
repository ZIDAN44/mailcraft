//! Keys and messages GnuPG 2.4 made, run as a command-line tool in a keyring of its own
//! (`--quick-gen-key`, `--quick-add-key`; the expired key with `--faked-system-time 20200101T000000`
//! and a day's validity, the withdrawn one with its revocation certificate imported; the messages
//! with `--sign --encrypt`, `--encrypt`, `--clearsign` and `--detach-sign`), read here: what other
//! programs send reads in MailCraft. Ana's passphrase is `secret`; Bo's key has none.

use super::*;

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name)).unwrap()
}

fn public(name: &str) -> Vec<u8> {
    public_keys(&testdata(name)).unwrap().remove(0)
}

#[test]
fn gnupg_keys_read_as_they_are() {
    let ana = read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    assert_eq!(ana.info.emails, ["ana@posteo.example"]);
    assert!(ana.info.encrypts && ana.info.signs && ana.info.expires.is_none() && ana.info.revoked.is_none());
    assert_eq!(read_secret_key(&testdata("ana-secret.asc"), "Secret").unwrap_err(), Error::Passphrase);
    // Binary, without a passphrase.
    let bo = read_secret_key(&testdata("bo-secret.gpg"), "").unwrap();
    assert_eq!((bo.info.name.as_str(), bo.info.algorithm.as_str()), ("Bo Berg", "RSA"));
    assert!(same_key(&bo.public, &public("bo.asc")));
    // One that signs only, one that ran out, one withdrawn.
    let sig = info(&public("sign-only.asc")).unwrap();
    assert!(sig.signs && !sig.encrypts);
    let old = info(&public("expired.asc")).unwrap();
    assert_eq!(old.expires, Some(1_577_836_800 + 86_400));
    assert_eq!(old.encryptable(NOW), Err(Problem::Expired(1_577_836_800 + 86_400)));
    let rev = info(&public("revoked.asc")).unwrap();
    assert!(rev.revoked.is_some());
    assert!(matches!(rev.encryptable(NOW), Err(Problem::Revoked(_))));
    for k in ["sign-only.asc", "expired.asc", "revoked.asc"] {
        assert!(encrypt(b"x", &[&public(k)], None, NOW, &mut rand_core::OsRng).is_err(), "{k}");
    }
}

#[test]
fn gnupg_mail_decrypts_and_verifies() {
    let ana = read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    let bo = public("bo.asc");
    let d = decrypt(&testdata("to-ana-signed.asc"), &[&ana.secret], &[&bo]).unwrap();
    assert_eq!(d.data, testdata("part.txt"));
    assert!(matches!(d.check, Some(Check::Good { key: 0, .. })), "{:?}", d.check);
    // Encrypted to Bo alone: Ana can't, and the message says to whom it went.
    assert_eq!(decrypt(&testdata("to-bo.asc"), &[&ana.secret], &[]).unwrap_err(), Error::Decrypt);
    let bo_secret = read_secret_key(&testdata("bo-secret.gpg"), "").unwrap();
    assert_eq!(recipients(&testdata("to-bo.asc")).len(), 1);
    assert_eq!(decrypt(&testdata("to-bo.asc"), &[&ana.secret, &bo_secret.secret], &[]).unwrap().data, testdata("part.txt"));
    // A detached signature, and a cleartext-signed text with a dash-escaped line.
    assert!(matches!(verify(&testdata("part.txt"), &testdata("part.sig"), &[&bo]).unwrap(), Check::Good { key: 0, .. }));
    assert_eq!(verify(b"Content-Type: text/plain\r\n\r\nchanged\r\n", &testdata("part.sig"), &[&bo]).unwrap(), Check::Bad);
    let (text, check) = verify_cleartext(&testdata("clearsigned.txt"), &[&bo]).unwrap();
    assert!(matches!(check, Check::Good { .. }));
    assert!(text.contains("\r\n- this line starts with a dash\r\n"), "{text:?}");
}
