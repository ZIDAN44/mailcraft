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
