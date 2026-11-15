//! Engine tests: S/MIME on the sample mailbox, with OpenSSL's messages (`crates/smime/testdata`:
//! a test CA, Ana and Riley; the sample account is Riley's address). The network layer's part
//! (the key, decrypting, signing) is played here with the smime crate itself.

use super::*;
use crate::session::{Lock, Seal};
use mailcraft_smime as smime;

mod addresses;
mod pgp;

/// After the test certificates were made, before they run out.
const LATER: i64 = 1_792_900_000;

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name)).unwrap()
}

fn path(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name).display().to_string()
}

fn ca() -> Vec<u8> {
    let text = String::from_utf8(testdata("ca.pem")).unwrap();
    der::pem::decode_vec(text.as_bytes()).unwrap().1
}

fn riley() -> smime::Identity {
    smime::read_p12(&testdata("riley-aes.p12"), "secret").unwrap()
}

/// Put `raw` in the sample Inbox and open it.
fn open_raw(s: &mut Session, uid: u32, raw: &[u8]) -> mailcraft_model::MessageKey {
    let key = mailcraft_model::MessageKey { uid, ..next_key(s, "sample", "INBOX") };
    let summary = mailcraft_mime::summary_from_full(raw, key, mailcraft_model::Flags::default(), 0, s.clock.now);
    let key = file_in(s, summary, Some(raw));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

/// The sample session with Riley's certificate imported (the network layer's answer played).
fn with_certificate(name: &str) -> Session {
    let mut s = sample_session(name);
    s.clock.now = LATER;
    s.run("security.import", &json!({"path": path("riley-aes.p12"), "password": "secret"})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::ImportIdentity { data, password, .. } if !data.is_empty() && password.0 == "secret")));
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, older: Vec::new(), for_reading: false, error: None });
    assert_eq!(s.identity("sample").unwrap().emails, ["riley@studio.example"]);
    assert!(s.smime_settings("sample").sign, "imported to be used: new messages are signed");
    s
}

fn security(s: &Session) -> crate::Security {
    s.reading.as_ref().and_then(|r| r.security.clone()).unwrap_or_default()
}

/// Plays the network layer: decrypts what was asked for with Riley's key.
fn answer_decrypt(s: &mut Session) {
    for j in s.take_jobs() {
        if let Job::Decrypt { key, cms, .. } = j {
            let event = match smime::decrypt(&cms, &riley(), &mut rand_core::OsRng) {
                Ok((c, authenticated)) => Event::Decrypted { key, content: Some(mailcraft_model::Private(c)), authenticated, error: None },
                Err(e) => Event::Decrypted { key, content: None, authenticated: false, error: Some(e.to_string()) },
            };
            s.apply(event);
        }
    }
}

#[test]
fn signed_mail_says_who_signed_and_whether_to_trust_it() {
    let mut s = sample_session("smime-signed");
    s.clock.now = LATER;
    // No trusted roots (and no certificate of one's own): the signature holds, its issuer is
    // unknown.
    open_raw(&mut s, 9301, &testdata("signed-clear.eml"));
    let sec = security(&s);
    let signed = sec.signed.unwrap();
    assert_eq!(signed.seal, Seal::Untrusted(vec![smime::Problem::UnknownIssuer]));
    assert_eq!(signed.cert.unwrap().name, "ana");
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("the proof is approved"), "the signed text shows");
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().attachments.is_empty(), "smime.p7s isn't an attachment");
    // With the CA trusted: trusted. Ana's certificate is remembered, to encrypt to her.
    s.set_trust_roots(vec![ca()]);
    open_raw(&mut s, 9302, &testdata("signed-opaque.eml"));
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Trusted);
    assert!(s.smime_settings("sample").known.contains_key("ana@posteo.example"));
    // Changed after it was signed: said.
    open_raw(&mut s, 9303, &testdata("signed-tampered.eml"));
    assert!(matches!(security(&s).signed.unwrap().seal, Seal::Broken(_)));
    // Mail from someone else with Ana's signature on it: not for that sender.
    let posing =
        String::from_utf8(testdata("signed-clear.eml")).unwrap().replace("From: Ana <ana@posteo.example>", "From: Boss <boss@posteo.example>");
    open_raw(&mut s, 9304, posing.as_bytes());
    assert!(matches!(security(&s).signed.unwrap().seal, Seal::Untrusted(p) if matches!(p.as_slice(), [smime::Problem::OtherAddress(_)])));
}

#[test]
fn encrypted_mail_is_decrypted_and_kept_off_the_web() {
    let mut s = with_certificate("smime-encrypted");
    s.set_trust_roots(vec![ca()]);
    let key = open_raw(&mut s, 9311, &testdata("encrypted-cbc.eml"));
    assert_eq!(security(&s).lock, Some(Lock::Opening));
    answer_decrypt(&mut s);
    let sec = security(&s);
    assert_eq!(sec.lock, Some(Lock::Open { authenticated: false }));
    assert!(sec.unauthenticated(), "CBC and unsigned: said");
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("the proof is approved"));
    // Opened again, it's not asked for again (it's held in memory) and nothing went to disk.
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { .. })));
    assert_eq!(s.store.as_ref().unwrap().read_body(&key).unwrap(), testdata("encrypted-cbc.eml"));
    assert!(s.remote_pictures().is_empty(), "nothing from the web in encrypted mail");
    // GCM, and signed inside: authenticated and the signature checked.
    open_raw(&mut s, 9312, &testdata("encrypted-gcm.eml"));
    answer_decrypt(&mut s);
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    open_raw(&mut s, 9313, &testdata("signed-encrypted.eml"));
    answer_decrypt(&mut s);
    let sec = security(&s);
    assert_eq!(sec.lock, Some(Lock::Open { authenticated: false }));
    assert_eq!(sec.signed.as_ref().unwrap().seal, Seal::Trusted);
    assert!(!sec.unauthenticated(), "signed inside: it can't have been changed");
    // A reply to encrypted mail is encrypted and signed.
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.encrypt && c.sign);
    s.run("compose.discard", &json!({})).unwrap();
}

/// Audit #265: with more messages decrypted than are held in memory (64), the oldest is let go
/// first, but never one shown, which would ask for its key (a keychain or card prompt) again.
#[test]
fn the_oldest_decrypted_message_not_shown_is_let_go_first() {
    let mut s = with_certificate("smime-opened-kept");
    let raw = testdata("encrypted-gcm.eml");
    let shown = open_raw(&mut s, 9315, &raw);
    answer_decrypt(&mut s);
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    // 64 more decrypted while it stays shown: the oldest of them goes, the shown one stays
    // though it's older still.
    let entity = b"Content-Type: text/plain\r\n\r\nElsewhere.\r\n".to_vec();
    let mut others = Vec::new();
    for n in 0..64 {
        let key = mailcraft_model::MessageKey { uid: 20_000 + n, ..next_key(&s, "sample", "INBOX") };
        let summary = mailcraft_mime::summary_from_full(&raw, key, mailcraft_model::Flags::default(), 0, s.clock.now);
        let key = file_in(&mut s, summary, Some(&raw));
        s.apply(Event::Decrypted { key: key.clone(), content: Some(mailcraft_model::Private(entity.clone())), authenticated: true, error: None });
        others.push(key);
    }
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), Some(shown.clone()), "still shown");
    let asked = |s: &mut Session, key: &mailcraft_model::MessageKey| {
        s.take_jobs();
        s.run("list.open", &json!({"key": key.to_id()})).unwrap();
        s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { .. }))
    };
    assert!(!asked(&mut s, &others[1]), "the second oldest is held");
    assert!(!asked(&mut s, &others[63]), "and the newest");
    assert!(!asked(&mut s, &shown), "the one shown is held");
    assert_eq!(security(&s).lock, Some(Lock::Open { authenticated: true }));
    assert!(asked(&mut s, &others[0]), "the oldest not shown went");
}

#[test]
fn without_a_certificate_encrypted_mail_says_why() {
    let mut s = sample_session("smime-nocert");
    open_raw(&mut s, 9321, &testdata("encrypted-cbc.eml"));
    assert!(matches!(security(&s).lock, Some(Lock::Failed(e)) if e.contains("no certificate")));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { .. })));
}

#[test]
fn sending_signs_and_encrypts_to_known_people_only() {
    let mut s = with_certificate("smime-send");
    s.set_trust_roots(vec![ca()]);
    // Nobody's certificate yet: encrypting to Ana is refused, saying whose is missing.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Plan"})).unwrap();
    s.run("compose.security", &json!({"encrypt": true})).unwrap();
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("ana@posteo.example"), "{err}");
    // Once she's sent signed mail, it goes: signed and encrypted to her and to Riley.
    open_raw(&mut s, 9331, &testdata("signed-clear.eml"));
    s.activate_compose(s.composes.first().map(|c| c.id).unwrap_or_default());
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert!(sent.get("protecting").is_some(), "{sent}");
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::Protect { .. })).unwrap();
    let Job::Protect { id, raw, sign, encrypt_to, gcm, .. } = job else { panic!("{job:?}") };
    assert!(sign);
    assert_eq!(encrypt_to.len(), 2, "Ana, and Riley for the Sent copy");
    assert!(!gcm, "OpenSSL's signature didn't say it reads GCM");
    assert!(String::from_utf8_lossy(&raw.0).contains("Subject: Plan"));
    // The network layer's answer: sent (the sample account files it in Sent itself).
    s.apply(Event::Protected {
        account: "sample".into(),
        id,
        raw: Some(mailcraft_model::Private(b"From: riley@studio.example\r\nSubject: Plan\r\n\r\nsealed\r\n".to_vec())),
        error: None,
    });
    assert!(s.compose.as_ref().is_none_or(|c| c.id != id) && s.composes.iter().all(|c| c.id != id), "it went");
    // With the certificate out of date, nothing is signed with it.
    let now = s.clock.now;
    s.clock.now = now + 86_400 * 1000;
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Late"})).unwrap();
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("your certificate is valid from"), "{err}");
    s.run("compose.discard", &json!({})).unwrap();
    s.clock.now = now;
    // Signing fails on the way: said over the message, which stays.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Again"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let id = s.compose.as_ref().unwrap().id;
    s.apply(Event::Protected { account: "sample".into(), id, raw: None, error: Some("the key is gone".into()) });
    let c = s.compose.clone().unwrap();
    assert!(!c.sending && c.error.as_deref().is_some_and(|e| e.contains("the key is gone")));
}

#[test]
fn the_certificate_can_be_forgotten() {
    let mut s = with_certificate("smime-forget");
    s.run("security.defaults", &json!({"encrypt": true})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().encrypt && s.compose.as_ref().unwrap().sign, "new messages take the account's settings");
    s.run("compose.discard", &json!({})).unwrap();
    s.run("security.forget", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ForgetIdentity { .. })));
    assert!(s.identity("sample").is_none());
    assert!(s.run("security.defaults", &json!({"sign": true})).is_err());
    s.run("mail.new", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().sign);
    // A wrong password is said.
    s.apply(Event::Identity {
        account: "sample".into(),
        chain: Vec::new(),
        older: Vec::new(),
        for_reading: false,
        error: Some("the password is wrong".into()),
    });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("the password is wrong")));
}

/// Issue #6: a renewed certificate. The older one comes back from the network layer as kept for
/// reading: it's said so, listed, and mail that couldn't be read is tried again; it can be used
/// for sending again or forgotten, and with only it left, encrypted mail is still sent to be read.
#[test]
fn an_older_certificate_is_kept_for_reading() {
    let mut s = with_certificate("smime-older");
    let old = smime::read_p12(&testdata("riley-2025.p12"), "secret").unwrap();
    let old_info = smime::cert::info(&old.chain[0]).unwrap();
    s.run("security.defaults", &json!({"sign": false})).unwrap();
    // Mail sent to the old certificate, before it was imported: it fails for want of a key.
    let sealed = smime::encrypt(
        b"Content-Type: text/plain\r\n\r\nFrom last year.\r\n",
        std::slice::from_ref(&old.chain[0]),
        smime::Cipher::Aes256Gcm,
        &mut rand_core::OsRng,
    )
    .unwrap();
    let outer = b"From: Ana <ana@posteo.example>\r\nTo: riley@studio.example\r\nSubject: Old\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\nMessage-ID: <old@posteo.example>\r\n";
    let raw = mailcraft_mime::smime::encrypted_message(outer, &sealed, true);
    let key = open_raw(&mut s, 9100, &raw);
    let asked = s.take_jobs();
    assert!(asked.iter().any(|j| matches!(j, Job::Decrypt { .. })));
    s.apply(Event::Decrypted {
        key: key.clone(),
        content: None,
        authenticated: false,
        error: Some("it was encrypted to a certificate MailCraft doesn't have".into()),
    });
    assert!(matches!(security(&s).lock, Some(Lock::Failed(_))));

    // The older one imported: kept for reading, said so; sending settings untouched.
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, older: vec![old.chain[0].clone()], for_reading: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("older than the one in use")), "{:?}", s.notices.last());
    assert!(!s.smime_settings("sample").sign, "an older one doesn't switch signing on");
    assert_eq!(s.older_identities("sample"), vec![old_info.clone()]);
    assert_eq!(s.identity("sample").unwrap().fingerprint, smime::cert::info(&riley().chain[0]).unwrap().fingerprint);
    // The message shown is asked for again.
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { key: k, .. } if *k == key)));
    let listed = s.run("security.certificates", &json!({})).unwrap();
    assert_eq!(listed["older"][0]["fingerprint"], old_info.fingerprint);

    // Used for sending again; then forgotten, by its SHA-256 in any case.
    s.run("security.useOlder", &json!({"older": old_info.fingerprint.to_lowercase()})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::UseOlderIdentity { fingerprint, .. } if *fingerprint == old_info.fingerprint)));
    assert!(s.run("security.useOlder", &json!({"older": "00:11"})).is_err());
    s.run("security.forget", &json!({"older": old_info.fingerprint.to_lowercase()})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ForgetOlderIdentity { .. })));
    assert!(s.older_identities("sample").is_empty() && s.smime_settings("sample").older.is_empty());
    assert!(s.run("security.forget", &json!({"older": old_info.fingerprint})).is_err(), "already gone");

    // Only an older one left (the one in use removed): encrypted mail is still asked to be read.
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, older: vec![old.chain[0].clone()], for_reading: true, error: None });
    s.run("security.forget", &json!({})).unwrap();
    s.take_jobs();
    let key = open_raw(&mut s, 9101, &raw);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { key: k, .. } if *k == key)), "the older key may read it");
}

#[test]
fn the_root_that_came_with_ones_own_certificate_is_trusted() {
    // Riley's .p12 holds the test CA: once imported, mail signed under it is trusted, with no
    // system root at all.
    let mut s = with_certificate("smime-own-root");
    open_raw(&mut s, 9341, &testdata("signed-clear.eml"));
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Trusted);
    // And no paperclip for a signature or an encrypted message.
    let store = s.store.clone().unwrap();
    let inbox = store.load_folder("sample", "INBOX").unwrap();
    assert!(!inbox.messages.iter().find(|m| m.key.uid == 9341).unwrap().has_attachments);
    let enc = mailcraft_mime::summary_from_full(&testdata("encrypted-cbc.eml"), Default::default(), Default::default(), 0, 0);
    assert!(!enc.has_attachments);
}

/// A message whose header fields are protected inside its signature (RFC 9788): the subject and
/// sender shown are the ones inside, and it says so.
#[test]
fn protected_headers_are_the_ones_shown() {
    let mut s = with_certificate("smime-hp");
    let raw =
        b"From: Riley <riley@studio.example>\r\nTo: riley@studio.example\r\nSubject: The real subject\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <hp1@studio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nInside.\r\n";
    let (outer, entity) = mailcraft_mime::smime::entity_of(raw);
    let (_, protected) = mailcraft_mime::smime::protect_headers(&outer, &entity, false, false);
    let signature = smime::sign(&protected, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    // On the way, someone changed the subject outside the signature.
    let changed = String::from_utf8_lossy(&outer).replace("The real subject", "Changed on the way").into_bytes();
    let sent = mailcraft_mime::smime::signed_message(&changed, &protected, &signature, "=_t1");
    open_raw(&mut s, 9351, &sent);
    let sec = security(&s);
    assert!(sec.headers);
    assert_eq!(sec.signed.unwrap().seal, Seal::Trusted);
    assert_eq!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().subject, "The real subject");

    // Found in review: the signature was checked for the sender on the outside, and the one
    // inside was shown. Riley signs a message that names someone else inside: the name shown
    // is the one inside, and the signature isn't theirs.
    let raw = b"From: The Director <director@posteo.example>\r\nTo: riley@studio.example\r\nSubject: Pay this today\r\n\
Date: Sat, 24 Oct 2026 09:00:00 +0000\r\nMessage-ID: <hp2@studio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nInside.\r\n";
    let (outer, entity) = mailcraft_mime::smime::entity_of(raw);
    let (_, protected) = mailcraft_mime::smime::protect_headers(&outer, &entity, false, false);
    let signature = smime::sign(&protected, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let outside = String::from_utf8_lossy(&outer).replace("The Director <director@posteo.example>", "Riley <riley@studio.example>").into_bytes();
    let sent = mailcraft_mime::smime::signed_message(&outside, &protected, &signature, "=_t2");
    open_raw(&mut s, 9352, &sent);
    assert_eq!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().from.email, "director@posteo.example");
    let seal = security(&s).signed.unwrap().seal;
    assert!(matches!(&seal, Seal::Untrusted(p) if p.iter().any(|p| matches!(p, smime::Problem::OtherAddress(_)))), "{seal:?}");
}

#[test]
fn a_withdrawn_certificate_is_said_only_when_asked_for() {
    let mut s = sample_session("smime-revocation");
    s.clock.now = LATER;
    s.set_trust_roots(vec![ca()]);
    // Off at first: no authority is asked.
    open_raw(&mut s, 9351, &testdata("signed-clear.eml"));
    assert_eq!(security(&s).signed.unwrap().revocation, None);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    s.run("view.checkRevocation", &json!({"on": true})).unwrap();
    open_raw(&mut s, 9352, &testdata("signed-clear.eml"));
    let signed = security(&s).signed.unwrap();
    assert_eq!((signed.seal, signed.revocation), (Seal::Trusted, Some(crate::RevocationCheck::Asking)));
    let chain = s.take_jobs().into_iter().find_map(|j| if let Job::CheckRevocation { chain } = j { Some(chain) } else { None }).unwrap();
    assert_eq!(chain.len(), 2, "Ana's certificate and the CA's");
    assert_eq!(chain.last(), Some(&ca()));
    // Shown again while asking: not asked twice.
    open_raw(&mut s, 9353, &testdata("signed-clear.eml"));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    // The authority's answer shows at once, and is kept until it runs out.
    let cert = chain[0].clone();
    let good = mailcraft_model::Revocation::Good { until: Some(s.clock.now + 600) };
    s.apply(Event::Revocation { cert: cert.clone(), status: good });
    assert_eq!(security(&s).signed.unwrap().revocation, Some(crate::RevocationCheck::Good));
    open_raw(&mut s, 9354, &testdata("signed-clear.eml"));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    // An answer that couldn't be had is said, and asked again later; never "withdrawn".
    s.clock.now += 700;
    open_raw(&mut s, 9355, &testdata("signed-clear.eml"));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })), "the answer ran out");
    s.apply(Event::Revocation { cert: cert.clone(), status: mailcraft_model::Revocation::Unknown { why: "no answer".into() } });
    let signed = security(&s).signed.unwrap();
    assert_eq!((signed.seal, signed.revocation), (Seal::Trusted, Some(crate::RevocationCheck::Unknown("no answer".into()))));
    s.clock.now += 7_200;
    open_raw(&mut s, 9356, &testdata("signed-clear.eml"));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    // Withdrawn: said in place of trusted, and it stays so.
    s.apply(Event::Revocation { cert: cert.clone(), status: mailcraft_model::Revocation::Revoked { at: LATER - 60 } });
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Untrusted(vec![smime::Problem::Revoked(LATER - 60)]));
    s.apply(Event::Revocation { cert, status: mailcraft_model::Revocation::Good { until: None } });
    s.clock.now += 30 * 86_400;
    open_raw(&mut s, 9357, &testdata("signed-clear.eml"));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    assert!(matches!(security(&s).signed.unwrap().seal, Seal::Untrusted(p) if p.contains(&smime::Problem::Revoked(LATER - 60))));
}

#[test]
fn a_certificate_on_a_smart_card_becomes_the_accounts() {
    let mut s = sample_session("smime-card");
    s.set_card_modules(vec!["/usr/lib/opensc-pkcs11.so".into()]);
    assert!(s.run("security.findKeys", &json!({"module": "  "})).is_err(), "which module?");
    s.run("security.findKeys", &json!({"module": "/usr/lib/opensc-pkcs11.so", "pin": "123456"})).unwrap();
    assert!(s.card_search().searching);
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::FindHeldKeys { .. })).unwrap();
    assert!(matches!(&job, Job::FindHeldKeys { module: Some(m), pin: Some(p) } if m == "/usr/lib/opensc-pkcs11.so" && p.0 == "123456"));
    // Kept for the key chosen, never written out (in a log, or what's shown of the search).
    assert!(!format!("{:?}", s.card_search()).contains("123456"), "{:?}", s.card_search());
    // Nothing used before anything's found.
    assert!(s.run("security.useKey", &json!({"index": 0})).is_err());
    let card = mailcraft_model::HeldKey { label: "riley (PIV)".into(), cert: riley().chain[0].clone(), at: "{\"kind\":\"card\"}".into() };
    s.apply(Event::HeldKeys { keys: vec![card.clone()], error: None });
    assert_eq!((s.card_search().searching, s.card_search().keys.len()), (false, 1));
    assert_eq!(s.card_search().modules, ["/usr/lib/opensc-pkcs11.so"], "kept across searches");
    // Its certificate read once, as it came, for the dialog to show whose it is.
    assert_eq!(s.card_search().info(0).map(|i| i.emails.clone()), Some(vec!["riley@studio.example".to_string()]));
    assert!(s.card_search().info(1).is_none());
    s.run("security.useKey", &json!({"index": 0})).unwrap();
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::UseHeldKey { .. })).unwrap();
    assert!(matches!(&job, Job::UseHeldKey { at, cert, pin: Some(p), .. } if *at == card.at && *cert == card.cert && p.0 == "123456"));
    // The network layer's answer: the certificate is the account's, as an imported one is.
    s.apply(Event::Identity { account: "sample".into(), chain: vec![card.cert.clone()], older: Vec::new(), for_reading: false, error: None });
    assert_eq!(s.identity("sample").unwrap().emails, ["riley@studio.example"]);
    // Nothing found says so.
    s.run("security.findKeys", &json!({"keychain": true})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FindHeldKeys { module: None, pin: None })));
    s.apply(Event::HeldKeys { keys: Vec::new(), error: None });
    assert!(s.card_search().error.is_some());
    assert!(s.card_search().info(0).is_none(), "the last search's certificates are gone");
}

#[test]
fn a_certificate_comes_from_its_bytes_or_the_one_kept() {
    let mut s = sample_session("smime-bytes");
    // The browser's picker gives the file itself.
    let p12 = testdata("riley-aes.p12");
    s.run("security.import", &json!({"data": p12, "password": "secret"})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::ImportIdentity { data, .. } if *data == p12)));
    // Unlocking the one kept: no file at all.
    s.run("security.import", &json!({"kept": true, "password": "secret"})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ImportIdentity { data, password, .. } if data.is_empty() && password.0 == "secret")));
    assert!(s.run("security.import", &json!({"data": vec![0u8; (1 << 20) + 1]})).is_err(), "too big");
    assert!(s.run("security.import", &json!({"data": "not bytes", "path": ""})).is_err());
}

#[test]
fn a_withdrawn_answer_outlives_a_full_cache() {
    let mut s = sample_session("smime-revocation-full");
    s.clock.now = LATER;
    s.set_trust_roots(vec![ca()]);
    s.run("view.checkRevocation", &json!({"on": true})).unwrap();
    open_raw(&mut s, 9361, &testdata("signed-clear.eml"));
    let chain = s.take_jobs().into_iter().find_map(|j| if let Job::CheckRevocation { chain } = j { Some(chain) } else { None }).unwrap();
    let ana = chain[0].clone();
    s.apply(Event::Revocation { cert: ana.clone(), status: mailcraft_model::Revocation::Revoked { at: LATER - 60 } });
    // Three hundred other signers' answers: more than are kept.
    for i in 0..300u32 {
        s.apply(Event::Revocation { cert: i.to_be_bytes().to_vec(), status: mailcraft_model::Revocation::Good { until: None } });
    }
    // Ana's is still withdrawn: not asked again, and said.
    open_raw(&mut s, 9362, &testdata("signed-clear.eml"));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::CheckRevocation { .. })));
    assert_eq!(security(&s).signed.unwrap().seal, Seal::Untrusted(vec![smime::Problem::Revoked(LATER - 60)]));
}

/// Gap 7: the browser keeps an account's certificates unlocked between visits when asked (the
/// page seals their password); the desktop, which keeps keys in the system's keychain, has
/// nothing to keep and says so.
#[test]
fn certificates_are_kept_unlocked_in_the_browser_when_asked() {
    let mut s = sample_session("keep-unlocked");
    let id = s
        .run("account.add", &json!({"email": "me@fastmail.example", "password": "pw", "jmap": "https://api.fastmail.example/jmap/session"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    assert!(s.run("security.keepUnlocked", &json!({"account": id, "on": true})).is_err(), "the desktop has nothing to keep");
    s.jmap_only = true;
    s.run("security.keepUnlocked", &json!({"account": id, "on": true})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::KeepUnlocked { account, on: true }] if account.id == id));
    let saved = std::fs::read_to_string(s.store.as_ref().unwrap().root().join(&id).join("smime.json")).unwrap_or_default();
    assert!(saved.contains("keep_unlocked") || saved.contains("keepUnlocked"), "{saved}");
    s.run("security.keepUnlocked", &json!({"account": id, "on": false})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::KeepUnlocked { on: false, .. }]));
}

/// A recipient's certificate withdrawn (its authority says so): the message bar says at once
/// that they can't receive it encrypted. What reached them was kept for up to an hour.
#[test]
fn a_withdrawn_certificate_reaches_no_one_at_once() {
    let mut s = with_certificate("smime-reach-withdrawn");
    s.set_trust_roots(vec![ca()]);
    open_raw(&mut s, 9361, &testdata("signed-clear.eml"));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Plan"})).unwrap();
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Everyone(crate::Scheme::Smime)));
    let cert = s.smime_settings("sample").known.get("ana@posteo.example").map(|k| k.cert.clone()).unwrap();
    s.apply(Event::Revocation { cert, status: mailcraft_model::Revocation::Revoked { at: LATER - 60 } });
    assert_eq!(s.encryption_reach(), Some(crate::Reach::Missing { scheme: crate::Scheme::Smime, who: vec!["ana@posteo.example".into()] }));
}

/// The settings of an account that has none kept (or of no account) read as the defaults.
#[test]
fn settings_never_kept_are_the_defaults() {
    let s = sample_session("smime-none-kept");
    for account in ["sample", "nobody"] {
        assert_eq!(*s.smime_settings(account), crate::SmimeSettings::default(), "{account}");
        assert_eq!(*s.pgp_settings(account), crate::PgpSettings::default(), "{account}");
    }
}

/// Audit #468: the account's own certificates, read once each time they change, are what its
/// settings hold after every change (imported, renewed, an older one forgotten, the one in use
/// forgotten) and when MailCraft opens again; a damaged one is left out, the rest still found.
#[test]
fn own_certificates_follow_every_change() {
    let fingerprint = |der: &[u8]| smime::cert::info(der).unwrap().fingerprint;
    let mut s = with_certificate("smime-own-follow");
    let store = s.store.clone().unwrap();
    let old = smime::read_p12(&testdata("riley-2025.p12"), "secret").unwrap();
    let now = riley().chain[0].clone();
    let shown = |s: &Session| {
        (s.identity("sample").map(|i| i.fingerprint.clone()), s.older_identities("sample").iter().map(|i| i.fingerprint.clone()).collect::<Vec<_>>())
    };
    assert_eq!(shown(&s), (Some(fingerprint(&now)), vec![]));
    // Renewed: the older one kept for reading, a damaged one beside it left out.
    s.apply(Event::Identity {
        account: "sample".into(),
        chain: riley().chain,
        older: vec![b"junk".to_vec(), old.chain[0].clone()],
        for_reading: true,
        error: None,
    });
    assert_eq!(shown(&s), (Some(fingerprint(&now)), vec![fingerprint(&old.chain[0])]));
    // The one in use signs the account's mail, the certificate with what's read of it.
    let (der, info) = s.own_cert_for("sample", "").unwrap();
    assert_eq!((der, info.fingerprint.clone()), (now.as_slice(), fingerprint(&now)));
    // As MailCraft opens again.
    let again = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(LATER, 0));
    assert_eq!(shown(&again), shown(&s));
    // The one in use forgotten: the older one signs, itself, not the damaged one before it.
    s.run("security.forget", &json!({})).unwrap();
    assert_eq!(shown(&s), (None, vec![fingerprint(&old.chain[0])]));
    let (der, info) = s.own_cert_for("sample", "").unwrap();
    assert_eq!((der, info.fingerprint.clone()), (old.chain[0].as_slice(), fingerprint(&old.chain[0])));
    // Then the older one.
    s.run("security.forget", &json!({"older": fingerprint(&old.chain[0])})).unwrap();
    assert_eq!(shown(&s), (None, vec![]));
    let again = Session::open(Some(store), mailcraft_model::dates::Clock::new(LATER, 0));
    assert_eq!(shown(&again), (None, vec![]));
}

/// Audit #264: what a message being written and sent reads of the account's settings (whether
/// it signs, sends its key) costs the same with 5000 people's certificates and keys known as
/// with none. Run in release: `cargo test --release -p mailcraft-engine --lib
/// settings_read_with_many_known -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn settings_read_with_many_known() {
    let s = sample_session("smime-many-known");
    let store = s.store.clone().unwrap();
    let blob: Vec<u8> = (0..1500u32).map(|i| (i % 251) as u8).collect();
    let known =
        |kind: &str| -> serde_json::Map<String, Value> { (0..5000).map(|n| (format!("person{n}@far.example"), json!({ kind: blob }))).collect() };
    store.save_account_doc("sample", crate::session::SMIME_DOC, &json!({"sign": true, "known": known("cert")})).unwrap();
    store.save_account_doc("sample", crate::session::PGP_DOC, &json!({"known": known("key")})).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!((s.smime_settings("sample").known.len(), s.pgp_settings("sample").known.len()), (5000, 5000));
    s.run("mail.new", &json!({})).unwrap();
    let raw = b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Hi\r\n\r\nHello\r\n".to_vec();
    const ROUNDS: u32 = 200;
    let t = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let _ = s.with_autocrypt("sample", raw.clone());
        s.signing_follows_from("sample", "riley@studio.example");
    }
    println!("MEASURE a send's and a From change's settings read, 5000 known: {:.3} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(ROUNDS));
}

/// Audit #468: what File › Info's Security card asks of the account's own certificates on every
/// frame (the one in use, the older ones, which addresses each signs, whether it can protect),
/// and a message being written (which certificate signs it), parse none of them. Run in
/// release: `cargo test --release -p mailcraft-engine --lib own_certificates_read_measure --
/// --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn own_certificates_read_measure() {
    let mut s = with_certificate("smime-own-measure");
    let old = smime::read_p12(&testdata("riley-2025.p12"), "secret").unwrap();
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, older: vec![old.chain[0].clone(); 4], for_reading: true, error: None });
    assert_eq!(s.older_identities("sample").len(), 4);
    const ROUNDS: u32 = 2000;
    let t = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let in_use = s.identity("sample").map(|c| c.not_after);
        let mut signs = 0;
        for c in s.older_identities("sample") {
            signs += s.signed_by_certificate("sample", &c.fingerprint).len();
        }
        let signer = s.identity_for("sample", "riley@studio.example").is_some();
        std::hint::black_box((in_use, signs, signer, s.can_protect("sample")));
    }
    println!(
        "MEASURE a frame's questions of an account's certificates, 1 in use and 4 older: {:.1} µs",
        t.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS)
    );
}
