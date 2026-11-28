//! Engine tests: the sender named inside a protected message (RFC 9788 header protection), when
//! it isn't the one its mail server took it from (GHSA-54mx-6wcm-6mjg). Anyone with the reader's
//! certificate or key could encrypt a message that names anyone inside, send it from their own
//! domain (which the reader's server checks), and have it shown as from whoever they named. The
//! outer sender is shown unless a signature that holds is by the one named (RFC 9788 §4.4.3), and
//! the one named is said beside it (§4.4.2).

use super::*;

/// A message from `inner` (its protected header fields, inside) and `outer` (outside, as its
/// server took it): `(outer header fields, entity to protect)`, its subject protected.
fn naming(inner: &str, outer: &str, encrypted: bool) -> (Vec<u8>, Vec<u8>) {
    let raw = format!(
        "From: {inner}\r\nTo: riley@studio.example\r\nSubject: Pay invoice 4711 today\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
         Message-ID: <named@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlease wire the money.\r\n"
    );
    let (head, entity) = mailcraft_mime::smime::entity_of(raw.as_bytes());
    let (_, protected) = mailcraft_mime::smime::protect_headers(&head, &entity, encrypted, false);
    let outside = String::from_utf8_lossy(&head).replace(&format!("From: {inner}"), &format!("From: {outer}")).into_bytes();
    (outside, protected)
}

fn shown_from(s: &Session) -> String {
    s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.email.clone()).unwrap_or_default()
}

/// The reporter's case: encrypted to Riley (AES-GCM, so nothing says it was changed), not
/// signed, naming the director inside and Mallory outside. Mallory is shown, the director said
/// beside it, and an answer goes to Mallory.
#[test]
fn an_unsigned_encrypted_message_cant_name_another_sender() {
    let mut s = with_certificate("hp-unsigned-from");
    let (outside, protected) = naming("The Director <director@studio.example>", "Mallory <mallory@evil.example>", true);
    let cms = smime::encrypt(&protected, &[riley().chain[0].clone()], smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
    open_raw(&mut s, 9901, &mailcraft_mime::smime::encrypted_message(&outside, &cms, true));
    answer_decrypt(&mut s);
    let sec = security(&s);
    assert!(sec.signed.is_none() && sec.headers);
    assert_eq!(sec.lock, Some(Lock::Open { authenticated: true }));
    assert_eq!(shown_from(&s), "mallory@evil.example", "the sender its server took it from");
    assert_eq!(sec.claimed_from.as_deref(), Some("director@studio.example"), "said beside it");
    let parsed = s.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert_eq!(parsed.subject, "Pay invoice 4711 today", "the rest of what's protected is still shown");
    s.run("mail.reply", &json!({})).unwrap();
    let to = s.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default();
    assert!(to.contains("mallory@evil.example") && !to.contains("director@"), "{to}");
}

/// Signed but not by the one named inside: Riley signs a message that names the director, and
/// sends it as Riley. Shown as from Riley, the director said beside it (it showed the director,
/// with only the signature's own warning).
#[test]
fn a_signature_by_someone_else_doesnt_confirm_the_sender_named() {
    let mut s = with_certificate("hp-signed-other");
    let (outside, protected) = naming("The Director <director@posteo.example>", "Riley <riley@studio.example>", false);
    let signature = smime::sign(&protected, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    open_raw(&mut s, 9902, &mailcraft_mime::smime::signed_message(&outside, &protected, &signature, "=_s2"));
    let sec = security(&s);
    assert!(matches!(&sec.signed.as_ref().unwrap().seal, Seal::Untrusted(p) if p.iter().any(|p| matches!(p, smime::Problem::OtherAddress(_)))));
    assert_eq!(shown_from(&s), "riley@studio.example");
    assert_eq!(sec.claimed_from.as_deref(), Some("director@posteo.example"));
}

/// A signature that holds and is by the one named inside confirms them: shown so, with nothing
/// said, though the outer sender differs (a list that sent it on); read again unchecked (an
/// answer), still them.
#[test]
fn a_signature_by_the_one_named_confirms_them() {
    let mut s = with_certificate("hp-signed-confirmed");
    let (outside, protected) = naming("Riley <riley@studio.example>", "Studio list <list@studio.example>", false);
    let signature = smime::sign(&protected, &riley(), LATER, &mut rand_core::OsRng).unwrap();
    open_raw(&mut s, 9903, &mailcraft_mime::smime::signed_message(&outside, &protected, &signature, "=_s3"));
    let sec = security(&s);
    assert_eq!(sec.signed.as_ref().unwrap().seal, Seal::Trusted);
    assert_eq!(shown_from(&s), "riley@studio.example");
    assert_eq!(sec.claimed_from, None);
    let key = s.reading.as_ref().map(|r| r.key.clone()).unwrap();
    let raw = s.readable_raw(&key).unwrap();
    assert_eq!(mailcraft_mime::parse(&raw).unwrap().from.email, "riley@studio.example", "read again unchecked");
}
