use super::*;

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

fn account(id: &str) -> AccountConfig {
    AccountConfig { id: id.into(), email: format!("{id}@posteo.example"), ..Default::default() }
}

/// GnuPG's Ana kept in the keychain (unlocked once, the passphrase never kept), a message signed
/// and encrypted to her as PGP/MIME, then decrypted with what the keychain has and its signature
/// checked over exactly what was written.
#[test]
fn a_kept_key_signs_encrypts_and_decrypts() {
    let creds = MemoryCredentials::default();
    let a = account("ana");
    let ana = pgp::read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    take(&creds, &a, &ana).unwrap();
    assert_eq!(list(&creds, &a), std::slice::from_ref(&ana.info.fingerprint));
    let kept = creds.get_secret(&a, &entry(&ana.info.fingerprint)).unwrap();
    assert!(!kept.contains("secret") && unhex(&kept).is_some());
    assert_eq!(publics(&creds, &a).len(), 1);
    let bo = pgp::public_keys(&testdata("bo.asc")).unwrap().remove(0);
    let raw = b"From: Ana <ana@posteo.example>\r\nTo: bo@studio.example\r\nSubject: Plan\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nThe plan =E2=9C=93\r\n";
    let sent = protect(raw, &secret(&creds, &a, &ana.info.fingerprint).unwrap(), true, &[bo.clone(), ana.public.clone()], true).unwrap();
    let text = String::from_utf8_lossy(&sent).to_string();
    assert!(text.contains("Subject: [...]") && !text.contains("The plan"), "{text}");
    let mailcraft_mime::pgp::Protection::Encrypted(data) = mailcraft_mime::pgp::protection(&sent) else { panic!("{text}") };
    let (entity, signature) = decrypt(&creds, &a, &data, false, "", &[]).unwrap();
    assert_eq!(signature, None, "signed inside the encryption, as a part of its own (RFC 3156 §6.1)");
    let inner = mailcraft_mime::smime::with_entity(&sent, &entity);
    let mailcraft_mime::pgp::Protection::Signed { content, signature } = mailcraft_mime::pgp::protection(&inner) else { panic!() };
    assert!(matches!(pgp::verify(&content, &signature, &[&ana.public]).unwrap(), pgp::Check::Good { key: 0, .. }));
    let read = mailcraft_mime::parse(&mailcraft_mime::smime::with_entity_protected(&inner, &content).0).unwrap();
    assert_eq!(read.subject, "Plan");
    assert!(read.text.contains("The plan \u{2713}"));
    // A second key takes over; the first still decrypts. Forgotten, nothing is left.
    let mut second = ana.clone();
    second.info.fingerprint = "00".repeat(20);
    take(&creds, &a, &second).unwrap();
    assert_eq!(list(&creds, &a).len(), 2);
    assert!(decrypt(&creds, &a, &data, false, "", &[]).is_ok(), "the older key is tried too");
    use_key(&creds, &a, &ana.info.fingerprint).unwrap();
    assert_eq!(list(&creds, &a)[0], ana.info.fingerprint);
    forget_all(&creds, &a);
    assert!(list(&creds, &a).is_empty() && creds.get_secret(&a, &entry(&ana.info.fingerprint)).is_none());
    assert!(decrypt(&creds, &a, &data, false, "", &[]).unwrap_err().contains("no OpenPGP key"));
}

/// Mail GnuPG encrypted to Bo alone: Ana's keychain says to which key it went.
#[test]
fn mail_for_someone_else_says_so() {
    let creds = MemoryCredentials::default();
    let a = account("ana");
    take(&creds, &a, &pgp::read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap()).unwrap();
    let e = decrypt(&creds, &a, &testdata("to-bo.asc"), false, "", &[]).unwrap_err();
    assert!(e.contains("encrypted to a key MailCraft doesn't have ("), "{e}");
    // GnuPG's own signed and encrypted mail to Ana, read as inline PGP: a text, Bo's signature good.
    let bo = pgp::public_keys(&testdata("bo.asc")).unwrap();
    let (entity, signature) = decrypt(&creds, &a, &testdata("to-ana-signed.asc"), true, "iso-8859-1", &bo).unwrap();
    assert!(entity.starts_with(b"Content-Type: text/plain; charset=utf-8\r\n"));
    assert!(matches!(signature, Some(PgpSignature::Good { key: 0, .. })));
}

#[test]
fn at_most_so_many_keys() {
    let creds = MemoryCredentials::default();
    let a = account("ana");
    let ana = pgp::read_secret_key(&testdata("ana-secret.asc"), "secret").unwrap();
    for n in 0..KEYS_MAX {
        let mut k = ana.clone();
        k.info.fingerprint = format!("{n:040X}");
        take(&creds, &a, &k).unwrap();
    }
    let mut one_more = ana.clone();
    one_more.info.fingerprint = "F".repeat(40);
    assert!(take(&creds, &a, &one_more).unwrap_err().contains("at most"));
    // The same one again isn't one more.
    let mut again = ana;
    again.info.fingerprint = format!("{:040X}", 3);
    take(&creds, &a, &again).unwrap();
}
