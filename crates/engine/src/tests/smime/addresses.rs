//! Engine tests: a certificate for each of an account's addresses (issue #81). Mail from one
//! of its other addresses is signed with the certificate that names it, kept beside the one in
//! use; with none that does, it isn't signed as someone else.

use super::*;

fn zoe() -> smime::Identity {
    smime::read_p12(&testdata("zoe-aes.p12"), "secret").unwrap()
}

fn fingerprint(cert: &[u8]) -> String {
    smime::cert::info(cert).unwrap().fingerprint
}

/// The message handed to the network layer to sign: which certificate signs it, whether it's
/// signed, and how many it's encrypted to.
fn protect(s: &mut Session) -> (Option<Vec<u8>>, bool, Vec<Vec<u8>>) {
    let jobs = s.take_jobs();
    let Some(Job::Protect { signer, sign, encrypt_to, .. }) = jobs.into_iter().find(|j| matches!(j, Job::Protect { .. })) else {
        panic!("nothing to sign")
    };
    (signer, sign, encrypt_to)
}

#[test]
fn mail_from_another_address_is_signed_with_its_own_certificate() {
    let mut s = with_certificate("smime-addresses");
    s.run("account.setAddress", &json!({"account": "sample", "address": "zoe@posteo.example"})).unwrap();
    s.run("account.setAddress", &json!({"account": "sample", "address": "bare@studio.example"})).unwrap();
    let (riley, zoe) = (riley().chain[0].clone(), zoe().chain[0].clone());
    // No certificate names the other address yet: a message from it isn't signed by itself,
    // signing it is refused naming the address, and sent signed all the same it doesn't go.
    assert!(s.identity_for("sample", "zoe@posteo.example").is_none() && !s.can_sign_from("sample", "zoe@posteo.example"));
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().sign, "from the account's own address: signed, as its settings say");
    s.run("compose.update", &json!({"from": "zoe@posteo.example", "to": "ana@posteo.example", "subject": "Hello"})).unwrap();
    assert!(!s.compose.as_ref().unwrap().sign, "nothing signs for that address");
    let err = s.run("compose.security", &json!({"sign": true})).unwrap_err().to_string();
    assert!(err.contains("zoe@posteo.example"), "{err}");
    s.compose.as_mut().unwrap().sign = true;
    let err = s.run("compose.send", &json!({})).unwrap_err().to_string();
    assert!(err.contains("no certificate for zoe@posteo.example"), "{err}");
    // Back to the account's own: signed again, with the one in use.
    s.run("compose.update", &json!({"from": ""})).unwrap();
    s.run("compose.security", &json!({"sign": false})).unwrap();
    s.run("compose.update", &json!({"from": "bare@studio.example"})).unwrap();
    assert!(!s.compose.as_ref().unwrap().sign, "turned off for this message: it stays off");
    s.run("compose.discard", &json!({})).unwrap();

    // Zoe's certificate comes: kept beside the one in use, it signs mail from her address.
    s.apply(Event::Identity { account: "sample".into(), chain: vec![riley.clone()], older: vec![zoe.clone()], for_reading: false, error: None });
    assert_eq!(s.identity_for("sample", "ZOE@posteo.example").unwrap().emails, ["zoe@posteo.example"]);
    assert_eq!(s.identity_for("sample", "").unwrap().emails, ["riley@studio.example"]);
    assert_eq!(s.signed_by_certificate("sample", &fingerprint(&zoe)), ["zoe@posteo.example"]);
    assert_eq!(s.signed_by_certificate("sample", &fingerprint(&riley)), ["riley@studio.example"]);
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "zoe@posteo.example", "to": "ana@posteo.example", "subject": "Hello"})).unwrap();
    assert!(s.compose.as_ref().unwrap().sign, "signed by itself now");
    s.run("compose.send", &json!({})).unwrap();
    let (signer, sign, _) = protect(&mut s);
    assert!(sign);
    assert_eq!(signer.as_deref(), Some(zoe.as_slice()), "the certificate that names the address it's from");
    let id = s.compose.as_ref().map_or(0, |c| c.id);
    s.apply(Event::Protected {
        account: "sample".into(),
        id,
        raw: Some(mailcraft_model::Private(b"From: zoe@posteo.example\r\n\r\nsigned\r\n".to_vec())),
        error: None,
    });
    // From the account's own address: the one in use, as ever.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Own"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert_eq!(protect(&mut s).0.as_deref(), Some(riley.as_slice()));
    let id = s.compose.as_ref().map_or(0, |c| c.id);
    s.apply(Event::Protected {
        account: "sample".into(),
        id,
        raw: Some(mailcraft_model::Private(b"From: riley@studio.example\r\n\r\nsigned\r\n".to_vec())),
        error: None,
    });

    // Encrypted to one of the account's addresses: with that address's certificate. From an
    // address with none, only encrypted: the account's own does for the copy it keeps.
    s.set_trust_roots(vec![ca()]);
    open_raw(&mut s, 9341, &testdata("signed-clear.eml"));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"from": "bare@studio.example", "to": "ana@posteo.example; zoe@posteo.example", "subject": "Sealed"})).unwrap();
    assert!(!s.compose.as_ref().unwrap().sign);
    s.run("compose.security", &json!({"encrypt": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let (signer, sign, encrypt_to) = protect(&mut s);
    assert!(!sign);
    assert_eq!(signer.as_deref(), Some(riley.as_slice()));
    assert!(encrypt_to.contains(&zoe) && encrypt_to.contains(&riley) && encrypt_to.len() == 3, "Ana, Zoe's own, and the copy kept");
    // Zoe's certificate out of date: nothing signs for her address any more.
    s.clock.now += 86_400 * 2000;
    assert!(s.identity_for("sample", "zoe@posteo.example").is_none());
    done(s);
}

/// Found in review: a draft kept encrypted (a message to be sent encrypted is, on the server)
/// opened again forgot that, and went in the clear unless the account encrypts everything.
/// It's to be sent encrypted still; and before it's been decrypted it doesn't open at all,
/// rather than as the file that encrypts it.
#[test]
fn a_draft_kept_encrypted_is_still_sent_encrypted() {
    let mut s = with_certificate("smime-draft");
    s.run("security.defaults", &json!({"sign": false, "encrypt": false})).unwrap();
    let sealed = smime::encrypt(
        b"Content-Type: text/plain; charset=utf-8\r\n\r\nThe figures, as agreed.\r\n",
        std::slice::from_ref(&riley().chain[0]),
        smime::Cipher::Aes256Gcm,
        &mut rand_core::OsRng,
    )
    .unwrap();
    let outer = b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Figures\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\nMessage-ID: <d1@studio.example>\r\n";
    let raw = mailcraft_mime::smime::encrypted_message(outer, &sealed, true);
    let key = open_raw(&mut s, 9371, &raw);
    let err = s.run("draft.edit", &json!({"key": key.to_id()})).unwrap_err().to_string();
    assert!(err.contains("encrypted") && s.compose.is_none(), "{err}");
    answer_decrypt(&mut s);
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed."), "{}", c.body);
    assert!(c.encrypt && !c.encrypt_auto, "to be sent encrypted, as it was kept");
    s.run("compose.discard", &json!({})).unwrap();
    // One kept in the clear opens as the account's settings say.
    let plain =
        b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Lunch\r\nMessage-ID: <d2@studio.example>\r\n\r\nAt one?\r\n";
    let key = open_raw(&mut s, 9372, plain);
    s.run("draft.edit", &json!({"key": key.to_id()})).unwrap();
    assert!(!s.compose.as_ref().unwrap().encrypt);
    done(s);
}

/// Found in review: a signed message waiting in the Outbox, taken back to be changed, opened as
/// the signature's own file with the message gone; an encrypted one as the encryption's. It
/// opens as it was written, to be signed again as it goes; an encrypted one once it's been
/// decrypted, and never while it can't be read. What was decrypted for one row isn't another's
/// when the rows move.
#[test]
fn a_protected_message_in_the_outbox_opens_as_it_was_written() {
    let mut s = with_certificate("smime-outbox");
    s.run("security.defaults", &json!({"sign": false, "encrypt": false})).unwrap();
    let store = s.store.clone().unwrap();
    let written = b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Figures\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <o1@studio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe figures, as agreed.\r\n";
    let (outer, entity) = mailcraft_mime::smime::entity_of(written);
    let signature = smime::sign(&entity, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    let signed = mailcraft_mime::smime::signed_message(&outer, &entity, &signature, "=_o1");
    let sealed = smime::encrypt(&entity, std::slice::from_ref(&riley().chain[0]), smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
    let encrypted = mailcraft_mime::smime::encrypted_message(&outer, &sealed, true);
    let later = s.clock.now + 3600;
    let item = |id: u64, not_before: i64| mailcraft_store::OutboxItem {
        id,
        from: "riley@studio.example".into(),
        recipients: vec!["ana@posteo.example".into()],
        sent_folder: None,
        queued: 0,
        attempts: 0,
        error: None,
        not_before,
    };
    store.outbox_add("sample", &item(71, 0), &encrypted).unwrap();
    store.outbox_add("sample", &item(72, later), &signed).unwrap();
    s.refresh_outbox();
    s.run("folder.outbox", &json!({})).unwrap();
    let key = |s: &Session, id: u64| {
        let row = s.outbox.iter().position(|(_, i)| i.id == id).unwrap();
        mailcraft_model::MessageKey {
            account: "sample".into(),
            folder: crate::session::OUTBOX.into(),
            validity: 0,
            uid: u32::try_from(row + 1).unwrap(),
        }
    };
    // The encrypted one: not while it can't be read, and it stays where it was.
    let k = key(&s, 71);
    let err = s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap_err().to_string();
    assert!(err.contains("encrypted") && s.compose.is_none() && s.outbox.len() == 2, "{err}");
    // Shown, it's decrypted; then it opens as written, to be encrypted again.
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    answer_decrypt(&mut s);
    s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed.") && c.encrypt && !c.sign, "{}", c.body);
    assert_eq!(c.subject, "Figures");
    s.run("compose.discard", &json!({})).unwrap();
    // The rows moved: nothing decrypted is left under the first row's key.
    let k = key(&s, 72);
    assert_eq!(k.uid, 1);
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    assert!(security(&s).lock.is_none() && security(&s).signed.is_some(), "the signed one, not what the other held");
    // The signed one, held back: as written, signed again as it goes, at the time it was to.
    let k = key(&s, 72);
    s.run("outbox.edit", &json!({"key": k.to_id()})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.body.contains("The figures, as agreed.") && c.sign && !c.encrypt, "{}", c.body);
    assert!(c.attachments.is_empty(), "no signature's file in it");
    assert_eq!(c.send_after, Some(later));
    assert!(s.outbox.is_empty());
    done(s);
}

/// Found in review: any signed mail that named a sender put its certificate in the place of
/// the one kept for them, to encrypt to: one from no authority this computer trusts over a
/// trusted one, and the one they had before over the one they have now (an old message read
/// again). The lesser one doesn't take the better one's place.
#[test]
fn a_lesser_certificate_doesnt_replace_the_one_kept() {
    let old = smime::read_p12(&testdata("riley-2025.p12"), "secret").unwrap();
    let signed_by = |who: &smime::Identity, n: u32| {
        let raw = format!(
            "From: Riley <riley@studio.example>\r\nTo: riley@studio.example\r\nSubject: Note {n}\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
Message-ID: <k{n}@studio.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nNote.\r\n"
        );
        let (outer, entity) = mailcraft_mime::smime::entity_of(raw.as_bytes());
        let signature = smime::sign(&entity, who, LATER, &mut rand_core::OsRng).unwrap();
        mailcraft_mime::smime::signed_message(&outer, &entity, &signature, "=_k")
    };
    let kept = |s: &Session| s.smime_settings("sample").known.get("riley@studio.example").cloned().unwrap();
    let (new_cert, old_cert) = (riley().chain[0].clone(), old.chain[0].clone());
    let in_date = |cert: &[u8]| smime::cert::info(cert).is_ok_and(|i| i.not_after >= LATER);

    // Trusted and current: kept. The one before it, read again: not in its place.
    let mut s = sample_session("smime-kept");
    s.clock.now = LATER;
    s.set_trust_roots(vec![ca()]);
    open_raw(&mut s, 9401, &signed_by(&riley(), 1));
    assert!(kept(&s).trusted && kept(&s).cert == new_cert);
    open_raw(&mut s, 9402, &signed_by(&old, 2));
    assert!(kept(&s).cert == new_cert, "the older one doesn't take its place");
    // Its authority not known here any more: neither does one that isn't trusted, and the
    // same one seen again is still the trusted one it was.
    s.set_trust_roots(Vec::new());
    open_raw(&mut s, 9403, &signed_by(&old, 3));
    open_raw(&mut s, 9404, &signed_by(&riley(), 4));
    assert!(kept(&s).trusted && kept(&s).cert == new_cert);
    done(s);

    // With none kept, one from an authority not trusted here is (it's all there is to encrypt
    // to them with), when it's in date; among such, the one that came last is kept, whatever
    // their dates (found in a second review: kept by date, one made today in someone's name
    // stayed for good over the one their real mail is signed with).
    let mut s = sample_session("smime-kept-first");
    s.clock.now = LATER;
    open_raw(&mut s, 9411, &signed_by(&old, 1));
    assert_eq!(s.smime_settings("sample").known.contains_key("riley@studio.example"), in_date(&old_cert));
    open_raw(&mut s, 9412, &signed_by(&riley(), 2));
    assert!(!kept(&s).trusted && kept(&s).cert == new_cert);
    open_raw(&mut s, 9413, &signed_by(&old, 3));
    assert_eq!(kept(&s).cert == old_cert, in_date(&old_cert), "the last that came, when it can be encrypted to");
    done(s);
}

/// Found in review: a message closed while it was being signed, whose signing then failed, was
/// only said to have failed: its text was nowhere. It opens again, as one the server refused.
#[test]
fn a_message_closed_while_it_was_signed_comes_back_when_that_fails() {
    let mut s = with_certificate("smime-closed");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@posteo.example", "subject": "Hello", "body": "Not to be lost"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let id = s.compose.as_ref().map(|c| c.id).unwrap();
    assert!(s.compose.as_ref().unwrap().sending);
    s.let_go_compose();
    assert!(s.compose.is_none());
    s.apply(Event::Protected { account: "sample".into(), id, raw: None, error: Some("the card was taken out".into()) });
    let c = s.compose.as_ref().expect("open again");
    assert!(c.body.contains("Not to be lost") && !c.sending);
    assert!(c.error.as_deref().is_some_and(|e| e.contains("the card was taken out")), "{:?}", c.error);
    done(s);
}
