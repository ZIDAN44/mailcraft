//! Engine tests: S/MIME on the sample mailbox, with OpenSSL's messages (`crates/smime/testdata`:
//! a test CA, Ana and Riley; the sample account is Riley's address). The network layer's part
//! (the key, decrypting, signing) is played here with the smime crate itself.

use super::*;
use crate::session::{Lock, Seal};
use mailcraft_smime as smime;

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
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(raw, key.clone(), mailcraft_model::Flags::default(), 0, s.clock.now));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
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
    s.apply(Event::Identity { account: "sample".into(), chain: riley().chain, error: None });
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
    done(s);
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
    done(s);
}

#[test]
fn without_a_certificate_encrypted_mail_says_why() {
    let mut s = sample_session("smime-nocert");
    open_raw(&mut s, 9321, &testdata("encrypted-cbc.eml"));
    assert!(matches!(security(&s).lock, Some(Lock::Failed(e)) if e.contains("no certificate")));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Decrypt { .. })));
    done(s);
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
    done(s);
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
    s.apply(Event::Identity { account: "sample".into(), chain: Vec::new(), error: Some("the password is wrong".into()) });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("the password is wrong")));
    done(s);
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
    done(s);
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
    done(s);
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
    done(s);
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
    // Nothing used before anything's found.
    assert!(s.run("security.useKey", &json!({"index": 0})).is_err());
    let card = mailcraft_model::HeldKey { label: "riley (PIV)".into(), cert: riley().chain[0].clone(), at: "{\"kind\":\"card\"}".into() };
    s.apply(Event::HeldKeys { keys: vec![card.clone()], error: None });
    assert_eq!((s.card_search().searching, s.card_search().keys.len()), (false, 1));
    assert_eq!(s.card_search().modules, ["/usr/lib/opensc-pkcs11.so"], "kept across searches");
    s.run("security.useKey", &json!({"index": 0})).unwrap();
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::UseHeldKey { .. })).unwrap();
    assert!(matches!(&job, Job::UseHeldKey { at, cert, pin: Some(p), .. } if *at == card.at && *cert == card.cert && p.0 == "123456"));
    // The network layer's answer: the certificate is the account's, as an imported one is.
    s.apply(Event::Identity { account: "sample".into(), chain: vec![card.cert.clone()], error: None });
    assert_eq!(s.identity("sample").unwrap().emails, ["riley@studio.example"]);
    // Nothing found says so.
    s.run("security.findKeys", &json!({"keychain": true})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FindHeldKeys { module: None, pin: None })));
    s.apply(Event::HeldKeys { keys: Vec::new(), error: None });
    assert!(s.card_search().error.is_some());
    done(s);
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
    done(s);
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
    done(s);
}
