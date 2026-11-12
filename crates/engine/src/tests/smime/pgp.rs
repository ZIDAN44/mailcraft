//! Engine tests: OpenPGP on the sample mailbox (Riley's address), with GnuPG's keys and mail
//! (`crates/pgp/testdata`: Riley's key, and Ana's, who writes to Riley). The network layer's part
//! (the keychain, decrypting, signing) is played here with the pgp crate itself, as the pool does.

use super::*;
use mailcraft_model::{PgpSignature, PgpWork, Private};
use mailcraft_pgp as pgp;

/// After GnuPG made the test keys (and before the S/MIME test certificates run out).
const PGP_NOW: i64 = 1_800_000_000;

/// The sample session, at [`PGP_NOW`].
fn session(name: &str) -> Session {
    let mut s = sample_session(name);
    s.clock.now = PGP_NOW;
    s
}

fn pgpdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name)).unwrap()
}

fn pgppath(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata").join(name).display().to_string()
}

fn riley_key() -> pgp::Unlocked {
    pgp::read_secret_key(&pgpdata("riley-secret.asc"), "").unwrap()
}

fn ana_public() -> Vec<u8> {
    pgp::public_keys(&pgpdata("ana.asc")).unwrap().remove(0)
}

/// The sample session with Riley's OpenPGP key imported (the network layer's answer played).
fn with_key(name: &str) -> Session {
    let mut s = session(name);
    s.run("pgp.import", &json!({"path": pgppath("riley-secret.asc")})).unwrap();
    let jobs = s.take_jobs();
    let Some(Job::Pgp { work: PgpWork::Import { file, .. }, .. }) = jobs.into_iter().find(|j| matches!(j, Job::Pgp { .. })) else { panic!() };
    let k = pgp::read_secret_key(&file.0, "").unwrap();
    s.apply(Event::PgpKeys { account: "sample".into(), keys: vec![k.public], imported: Some(k.info.fingerprint), error: None });
    assert_eq!(s.pgp_key("sample").unwrap().emails, ["riley@studio.example"]);
    assert!(s.smime_settings("sample").sign, "imported to be used: new messages are signed");
    s
}

/// Ana's mail: `body` (a MIME entity's header and body), from her, with her key in an
/// `Autocrypt:` header when `autocrypt`.
fn from_ana(autocrypt: bool, content_type: &str, body: &str) -> Vec<u8> {
    let mut m = String::from(
        "From: Ana Lima <ana@posteo.example>\r\nTo: riley@studio.example\r\nSubject: Proof\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n",
    );
    if autocrypt {
        let small = pgp::minimal(&ana_public(), "ana@posteo.example").unwrap();
        m.push_str(&pgp::autocrypt::header("ana@posteo.example", &small, true));
    }
    m.push_str(&format!("MIME-Version: 1.0\r\nContent-Type: {content_type}\r\n\r\n{body}"));
    m.into_bytes()
}

/// Ana's PGP/MIME signed message (GnuPG's detached signature over the part, which is all that's
/// before the line break ahead of the next boundary).
fn signed_by_ana(autocrypt: bool, part: &[u8]) -> Vec<u8> {
    let body = format!(
        "--sig\r\n{}\r\n--sig\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\r\n{}\r\n--sig--\r\n",
        String::from_utf8_lossy(part),
        String::from_utf8_lossy(&pgpdata("ana-part.sig"))
    );
    from_ana(autocrypt, "multipart/signed; micalg=pgp-sha512; protocol=\"application/pgp-signature\"; boundary=\"sig\"", &body)
}

/// Plays the network layer: decrypts what was asked for with Riley's key.
fn answer_decrypt(s: &mut Session) {
    let riley = riley_key();
    for j in s.take_jobs() {
        if let Job::Pgp { work: PgpWork::Decrypt { key, data, inline, charset, verify_with }, .. } = j {
            let keys: Vec<&[u8]> = verify_with.iter().map(Vec::as_slice).collect();
            let event = match pgp::decrypt(&data, &[&riley.secret], &keys) {
                Ok(d) => Event::PgpDecrypted {
                    key,
                    content: Some(Private(if inline { mailcraft_mime::pgp::text_entity(&d.data, &charset) } else { d.data })),
                    signature: d.check.map(|c| match c {
                        pgp::Check::Good { key, at, weak } => PgpSignature::Good { key, at, weak },
                        pgp::Check::Bad => PgpSignature::Bad,
                        pgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
                    }),
                    error: None,
                },
                Err(e) => Event::PgpDecrypted { key, content: None, signature: None, error: Some(e.to_string()) },
            };
            s.apply(event);
        }
    }
}

fn text(s: &Session) -> String {
    s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap_or_default()
}

#[test]
fn pgp_mime_signatures_say_who_signed() {
    let mut s = session("pgp-signed");
    let part = pgpdata("ana-part.txt");
    // Without Ana's key: signed, but it can't be checked.
    open_raw(&mut s, 9501, &signed_by_ana(false, &part));
    let sec = security(&s);
    assert!(sec.pgp);
    assert!(matches!(sec.signed.as_ref().unwrap().seal, Seal::UnknownKey(_)), "{sec:?}");
    assert!(text(&s).contains("the proof is approved"), "the signed text shows");
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().attachments.is_empty(), "signature.asc isn't an attachment");
    // Her mail brings her key (Autocrypt): kept, and the signature holds.
    open_raw(&mut s, 9502, &signed_by_ana(true, &part));
    let signed = security(&s).signed.unwrap();
    assert_eq!(signed.seal, Seal::Trusted);
    assert_eq!(signed.key.unwrap().emails, ["ana@posteo.example"]);
    let known = s.pgp_settings("sample").known;
    assert!(known.get("ana@posteo.example").is_some_and(|k| k.mutual && k.from == crate::KeySource::Autocrypt));
    // Changed after it was signed.
    let changed = String::from_utf8(signed_by_ana(false, &part)).unwrap().replace("is approved", "is rejected");
    open_raw(&mut s, 9503, changed.as_bytes());
    assert!(matches!(security(&s).signed.unwrap().seal, Seal::Broken(_)));
    // Someone else with Ana's signature on their mail: not for that sender.
    let posing =
        String::from_utf8(signed_by_ana(false, &part)).unwrap().replace("From: Ana Lima <ana@posteo.example>", "From: Boss <boss@posteo.example>");
    open_raw(&mut s, 9504, posing.as_bytes());
    assert!(matches!(security(&s).signed.unwrap().seal, Seal::UnknownKey(_)), "Boss has no key: Ana's isn't his");
    // An Autocrypt header for another address than the sender's is ignored.
    let other = String::from_utf8(from_ana(true, "text/plain", "hi\r\n"))
        .unwrap()
        .replace("From: Ana Lima <ana@posteo.example>", "From: Eve <eve@posteo.example>");
    open_raw(&mut s, 9505, other.as_bytes());
    assert!(!s.pgp_settings("sample").known.contains_key("eve@posteo.example"));
    done(s);
}

#[test]
fn pgp_mail_is_decrypted_and_its_signature_checked() {
    let mut s = with_key("pgp-encrypted");
    let armored = String::from_utf8(pgpdata("ana-to-riley.asc")).unwrap();
    let body = format!(
        "--enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--enc\r\nContent-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\r\n{armored}\r\n--enc--\r\n"
    );
    let raw = from_ana(true, "multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"enc\"", &body);
    let key = open_raw(&mut s, 9511, &raw);
    assert_eq!(security(&s).lock, Some(Lock::Opening));
    assert!(!text(&s).contains("approved"), "nothing shows before it's decrypted");
    answer_decrypt(&mut s);
    let sec = security(&s);
    assert_eq!(sec.lock, Some(Lock::Open { authenticated: true }));
    assert_eq!(sec.signed.as_ref().unwrap().seal, Seal::Trusted, "signed inside by Ana, whose key came with it");
    assert!(text(&s).contains("the proof is approved"));
    // Not asked for again, nothing decrypted on disk, nothing from the web.
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { .. })));
    assert_eq!(s.store.as_ref().unwrap().read_body(&key).unwrap(), raw);
    assert!(s.remote_pictures().is_empty());
    // A reply is encrypted and signed, with OpenPGP, to Ana.
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.encrypt && c.sign && c.scheme == Some(crate::Scheme::Pgp));
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Everyone(crate::Scheme::Pgp)));
    s.run("compose.discard", &json!({})).unwrap();
    // Inline: Ana's encrypted text, and her cleartext-signed one.
    open_raw(&mut s, 9512, &from_ana(false, "text/plain; charset=us-ascii", &String::from_utf8(pgpdata("ana-inline.asc")).unwrap()));
    answer_decrypt(&mut s);
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Trusted);
    assert!(text(&s).contains("signed in the text itself"), "{}", text(&s));
    open_raw(&mut s, 9513, &from_ana(false, "text/plain; charset=us-ascii", &String::from_utf8(pgpdata("ana-clearsigned.txt")).unwrap()));
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Trusted);
    assert!(text(&s).contains("signed in the text itself") && !text(&s).contains("BEGIN PGP"));
    done(s);
}

#[test]
fn pgp_mail_for_someone_else_says_why() {
    let mut s = session("pgp-nokey");
    let body = String::from_utf8(pgpdata("to-bo.asc")).unwrap();
    open_raw(&mut s, 9521, &from_ana(false, "text/plain", &body));
    assert!(matches!(security(&s).lock, Some(Lock::Failed(e)) if e.contains("no OpenPGP key")));
    // With Riley's key, the network layer's answer says it went to someone else.
    let mut s = with_key("pgp-other");
    open_raw(&mut s, 9522, &from_ana(false, "text/plain", &body));
    answer_decrypt(&mut s);
    assert!(matches!(security(&s).lock, Some(Lock::Failed(_))));
    assert!(!text(&s).contains("The plan"));
    done(s);
}

#[test]
fn sending_with_pgp_encrypts_to_known_keys_and_self() {
    let mut s = with_key("pgp-send");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Plan"})).unwrap();
    // No key for Ana yet: the compose window says so, and encrypting is refused naming her.
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Missing { scheme: crate::Scheme::Pgp, who: vec!["ana@posteo.example".into()] }));
    s.run("compose.security", &json!({"encrypt": true})).unwrap();
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("ana@posteo.example") && err.contains("OpenPGP"), "{err}");
    // Her key imported from a file: reachable, and it goes signed and encrypted to her and Riley.
    s.run("pgp.importPublic", &json!({"path": pgppath("ana.asc")})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Everyone(crate::Scheme::Pgp)));
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert!(sent.get("protecting").is_some(), "{sent}");
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::Pgp { work: PgpWork::Protect { .. }, .. })).unwrap();
    let Job::Pgp { work: PgpWork::Protect { id, raw, sign, encrypt_to, hide_subject, .. }, .. } = job else { panic!() };
    assert!(sign && hide_subject);
    assert_eq!(encrypt_to.len(), 2, "Ana, and Riley for the Sent copy");
    let head = String::from_utf8_lossy(&raw.0).to_string();
    assert!(head.starts_with("Autocrypt: addr=riley@studio.example; keydata="), "Riley's key goes along: {head}");
    // Played as the pool does it: Ana reads it, and the signature inside holds by Riley's key.
    let riley = riley_key();
    let keys: Vec<&[u8]> = encrypt_to.iter().map(Vec::as_slice).collect();
    let mut sig = |e: &[u8]| pgp::sign(e, &riley.secret, PGP_NOW, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let mut enc = |e: &[u8]| pgp::encrypt(e, &keys, None, PGP_NOW, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let sealed = mailcraft_mime::pgp::protect(&raw.0, Some(&mut sig), Some(&mut enc), hide_subject, "=_t").unwrap();
    assert!(String::from_utf8_lossy(&sealed).contains("Subject: [...]"));
    s.apply(Event::Protected { account: "sample".into(), id, raw: Some(Private(sealed)), error: None });
    assert!(s.compose.as_ref().is_none_or(|c| c.id != id), "it went");
    // Another recipient without a key: the window names them, and encryption that followed the
    // recipients (the account's option) turns off by itself.
    s.run("security.defaults", &json!({"autoEncrypt": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example"})).unwrap();
    s.encryption_reach();
    assert!(s.compose.as_ref().unwrap().encrypt, "everyone can receive it: encrypted by itself");
    s.run("compose.update", &json!({"to": "ana@posteo.example; zoe@posteo.example"})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Missing { scheme: crate::Scheme::Pgp, who: vec!["zoe@posteo.example".into()] }));
    assert!(!s.compose.as_ref().unwrap().encrypt);
    // Chosen by hand, it stays as chosen.
    s.run("compose.security", &json!({"encrypt": true})).unwrap();
    s.encryption_reach();
    assert!(s.compose.as_ref().unwrap().encrypt && !s.compose.as_ref().unwrap().encrypt_auto);
    // A draft of it (on a server) is encrypted to Riley's key before it's saved.
    s.run("compose.update", &json!({"to": "ana@posteo.example"})).unwrap();
    assert!(s.seals_with_pgp(s.compose.as_ref().unwrap()));
    done(s);
}

#[test]
fn with_both_the_recipients_choose() {
    let mut s = with_key("pgp-both");
    s.set_trust_roots(vec![ca()]);
    s.run("security.import", &json!({"path": path("riley-aes.p12"), "password": "secret"})).unwrap();
    s.take_jobs();
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, older: Vec::new(), for_reading: false, error: None });
    // Ana has a certificate (her signed mail) and an OpenPGP key; Bo only a key.
    open_raw(&mut s, 9531, &testdata("signed-clear.eml"));
    s.run("pgp.importPublic", &json!({"path": pgppath("ana.asc")})).unwrap();
    s.run("pgp.importPublic", &json!({"path": pgppath("bo.asc")})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example"})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Everyone(crate::Scheme::Smime)), "both reach her: S/MIME");
    s.run("compose.update", &json!({"to": "ana@posteo.example, bo@studio.example"})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Everyone(crate::Scheme::Pgp)), "only OpenPGP reaches Bo");
    // Chosen by hand: S/MIME, which Bo can't receive.
    s.run("compose.security", &json!({"scheme": "smime"})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Missing { scheme: crate::Scheme::Smime, who: vec!["bo@studio.example".into()] }));
    assert!(s.run("compose.security", &json!({"scheme": "quantum"})).is_err());
    let r = s.run("compose.encryption", &json!({})).unwrap();
    assert_eq!(r["missing"], json!(["bo@studio.example"]));
    done(s);
}

#[test]
fn keys_are_listed_kept_and_forgotten() {
    let mut s = with_key("pgp-keys");
    let keys = s.run("pgp.keys", &json!({})).unwrap();
    assert_eq!(keys["inUse"]["emails"], json!(["riley@studio.example"]));
    assert!(s.run("pgp.importPublic", &json!({"path": pgppath("part.txt")})).is_err(), "no key in it");
    // Expired, withdrawn and sign-only keys are kept, but can't be encrypted to.
    for f in ["expired.asc", "revoked.asc", "sign-only.asc"] {
        s.run("pgp.importPublic", &json!({"path": pgppath(f)})).unwrap();
    }
    let known = s.run("pgp.known", &json!({})).unwrap();
    let list = known["known"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|k| k["canEncrypt"] == json!(false)), "{known}");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "old@studio.example"})).unwrap();
    assert!(matches!(s.encryption_reach(), Some(crate::Reach::Missing { .. })));
    s.run("compose.discard", &json!({})).unwrap();
    s.run("pgp.forgetPublic", &json!({"email": "OLD@studio.example"})).unwrap();
    assert!(s.run("pgp.forgetPublic", &json!({"email": "old@studio.example"})).is_err());
    // A key imported by hand isn't replaced by one from mail.
    s.run("pgp.importPublic", &json!({"path": pgppath("ana.asc")})).unwrap();
    let mine = s.pgp_settings("sample").known["ana@posteo.example"].clone();
    assert_eq!(mine.from, crate::KeySource::Imported);
    // The account's own key: forgotten (and signing goes off, with nothing left to sign with).
    let fp = s.pgp_key("sample").unwrap().fingerprint;
    s.run("pgp.forget", &json!({"fingerprint": fp})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Pgp { work: PgpWork::Forget { .. }, .. })));
    assert!(s.pgp_key("sample").is_none());
    assert!(!s.smime_settings("sample").sign);
    // Nothing secret in what MailCraft keeps.
    let doc = std::fs::read_to_string(s.store.as_ref().unwrap().root().join("accounts").join("sample").join(crate::PGP_DOC)).unwrap_or_default();
    assert!(!doc.contains("PRIVATE"));
    done(s);
}

/// Issue #81: mail from another of the account's addresses is signed with the key that names
/// it, kept beside the one in use, which goes along in its Autocrypt header; with none that
/// does it isn't signed, and carries no key.
#[test]
fn mail_from_another_address_is_signed_with_its_own_key() {
    let mut s = with_key("pgp-addresses");
    s.run("account.setAddress", &json!({"account": "sample", "address": "ana@posteo.example"})).unwrap();
    assert!(s.pgp_key_from("sample", "ana@posteo.example").is_none());
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "ana@posteo.example", "to": "bo@posteo.example", "subject": "Hello"})).unwrap();
    assert!(!s.compose.as_ref().unwrap().sign, "nothing signs for that address");
    s.compose.as_mut().unwrap().sign = true;
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("no OpenPGP key for ana@posteo.example"), "{err}");
    s.run("compose.discard", &json!({})).unwrap();
    // Unsigned from it: no key of another address goes along.
    let plain = b"From: Ana <ana@posteo.example>\r\nTo: bo@posteo.example\r\nSubject: x\r\n\r\nHi\r\n".to_vec();
    assert_eq!(s.with_autocrypt("sample", plain.clone()), plain);

    // Its key comes: kept beside Riley's, it signs mail from that address.
    let riley = riley_key();
    let ana = ana_public();
    let ana_info = pgp::info(&ana).unwrap();
    s.apply(Event::PgpKeys { account: "sample".into(), keys: vec![riley.public.clone(), ana.clone()], imported: None, error: None });
    assert_eq!(s.pgp_key_from("sample", "ana@posteo.example").unwrap().fingerprint, ana_info.fingerprint);
    assert_eq!(s.pgp_key_from("sample", "").unwrap().fingerprint, riley.info.fingerprint);
    assert_eq!(s.signed_by_key("sample", &ana_info.fingerprint), ["ana@posteo.example"]);
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "ana@posteo.example", "to": "bo@posteo.example", "subject": "Hello"})).unwrap();
    assert!(s.compose.as_ref().unwrap().sign);
    s.run("compose.send", &json!({})).unwrap();
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::Pgp { work: PgpWork::Protect { .. }, .. })).unwrap();
    let Job::Pgp { work: PgpWork::Protect { raw, sign, signer, .. }, .. } = job else { panic!() };
    assert!(sign);
    assert_eq!(signer.as_deref(), Some(ana_info.fingerprint.as_str()), "the key that names the address it's from");
    let head = String::from_utf8_lossy(&raw.0).to_string();
    assert!(head.starts_with("Autocrypt: addr=ana@posteo.example; keydata="), "that address's key goes along: {head}");
    done(s);
}
