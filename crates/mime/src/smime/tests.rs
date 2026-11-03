//! S/MIME's MIME: what protects a message, taking it apart and putting it back together.

use super::*;

const SIGNED: &str = "From: Ana <ana@posteo.example>\r\nSubject: Hi\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256; boundary=\"b1\"\r\n\r\n\
This is a signed message.\r\n--b1\r\nContent-Type: text/plain\r\n\r\nHello\r\n\r\n--b1\r\n\
Content-Type: application/pkcs7-signature; name=smime.p7s\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n--b1--\r\n";

#[test]
fn a_signed_message_gives_what_was_signed() {
    assert_eq!(
        protection(SIGNED.as_bytes()),
        Protection::Signed { content: b"Content-Type: text/plain\r\n\r\nHello\r\n".to_vec(), signature: vec![1, 2, 3] }
    );
    // Stored with bare LFs, the signed part is still the CRLF one that was signed.
    let lf = SIGNED.replace("\r\n", "\n");
    assert!(matches!(protection(lf.as_bytes()), Protection::Signed { content, .. } if content == b"Content-Type: text/plain\r\n\r\nHello\r\n"));
    // Another protocol, or no signature part: not S/MIME.
    assert_eq!(protection(SIGNED.replace("pkcs7-signature\";", "pgp-signature\";").as_bytes()), Protection::None);
    assert_eq!(protection(b"From: a@b\r\nSubject: x\r\n\r\nplain\r\n"), Protection::None);
}

#[test]
fn encrypted_and_opaque_messages_are_told_apart() {
    let p7m = |kind: &str| {
        format!(
            "From: a@b.example\r\nContent-Type: application/pkcs7-mime; smime-type={kind}; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n"
        )
    };
    assert_eq!(protection(p7m("enveloped-data").as_bytes()), Protection::Encrypted(vec![1, 2, 3]));
    assert_eq!(protection(p7m("authEnveloped-data").as_bytes()), Protection::Encrypted(vec![1, 2, 3]));
    assert_eq!(protection(p7m("signed-data").as_bytes()), Protection::Opaque(vec![1, 2, 3]));
    let x = "From: a@b.example\r\nContent-Type: application/x-pkcs7-mime; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n";
    assert_eq!(protection(x.as_bytes()), Protection::Encrypted(vec![1, 2, 3]));
    // A .p7m among other parts is an attachment, not the message's protection.
    let mixed = "From: a@b.example\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSee attached\r\n--m\r\n\
Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n--m--\r\n";
    assert_eq!(protection(mixed.as_bytes()), Protection::None);
}

/// A `.p7m` sent as `application/octet-stream` is encrypted mail, named in any of the header's
/// ways: the Content-Type `name` (quoted or not, any case, padded), the Content-Disposition
/// `filename` alone (folded or not), or both; another name, or none, is a file like any other.
#[test]
fn an_octet_stream_p7m_is_protected_in_each_header_form() {
    let message = |headers: &str| format!("From: a@b.example\r\n{headers}Content-Transfer-Encoding: base64\r\n\r\nAQID\r\n");
    for headers in [
        "Content-Type: application/octet-stream; name=smime.p7m\r\n",
        "Content-Type: application/octet-stream; name=\"SMIME.P7M \"\r\n",
        "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"smime.p7m\"\r\n",
        "Content-Type: Application/Octet-Stream\r\nContent-Disposition: attachment;\r\n filename=message.p7m\r\n",
        "Content-Type: application/octet-stream; name=smime.p7m\r\nContent-Disposition: attachment; filename=smime.p7m\r\n",
    ] {
        let raw = message(headers);
        assert!(is_protected(raw.as_bytes()), "{headers}");
        assert_eq!(protection(raw.as_bytes()), Protection::Encrypted(vec![1, 2, 3]), "{headers}");
    }
    for headers in [
        "Content-Type: application/octet-stream; name=report.pdf\r\n",
        "Content-Type: application/octet-stream\r\n",
        "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=smime.p7m.txt\r\n",
        "Content-Type: text/plain; name=smime.p7m\r\n",
    ] {
        let raw = message(headers);
        assert!(!is_protected(raw.as_bytes()), "{headers}");
        assert_eq!(protection(raw.as_bytes()), Protection::None, "{headers}");
    }
}

/// Only the header decides: a message that isn't protected is never read past it (a body that
/// would look signed or encrypted if it were the header changes nothing), and one bigger than
/// [`RAW_MAX`] isn't looked at.
#[test]
fn the_header_alone_says_whether_a_message_is_protected() {
    assert!(is_protected(SIGNED.as_bytes()));
    assert!(is_protected(b"Content-Type: application/x-pkcs7-mime\r\n\r\nAQID\r\n"));
    assert!(!is_protected(SIGNED.replace("pkcs7-signature\";", "unknown-signature\";").as_bytes()));
    // PGP/MIME is too (taken apart by `pgp::protection`).
    assert!(is_protected(SIGNED.replace("pkcs7-signature\";", "pgp-signature\";").as_bytes()));
    assert!(!is_protected(b"Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"\r\n\r\nno boundary\r\n"));
    let disguised = "From: a@b.example\r\nContent-Type: text/plain\r\n\r\nContent-Type: application/pkcs7-mime; name=smime.p7m\r\n\r\nAQID\r\n";
    assert!(!is_protected(disguised.as_bytes()));
    assert_eq!(protection(disguised.as_bytes()), Protection::None);
    let mut big = b"Content-Type: application/pkcs7-mime\r\n\r\n".to_vec();
    big.resize(RAW_MAX + 1, b'A');
    assert!(!is_protected(&big));
    assert_eq!(protection(&big), Protection::None);
    for junk in ["", "\r\n\r\n", ":", "Content-Type:\r\n\r\n", "Content-Type: application/octet-stream; name=\r\n"] {
        assert!(!is_protected(junk.as_bytes()), "{junk:?}");
    }
}

#[test]
fn messages_come_apart_and_back_together() {
    let raw = b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Proof\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nApproved =E2=9C=93\r\n";
    let (outer, entity) = entity_of(raw);
    assert_eq!(outer, b"From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Proof\r\n");
    assert_eq!(entity, b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nApproved =E2=9C=93\r\n");
    assert!(safe_to_sign(&entity));
    assert!(!safe_to_sign("Content-Type: text/plain\r\n\r\ncaf\u{e9}\r\n".as_bytes()));
    // Signed and taken apart again: the same entity.
    let message = signed_message(&outer, &entity, &[0xAB; 100], "=_s1");
    assert!(matches!(protection(&message), Protection::Signed { content, signature } if content == entity && signature == [0xAB; 100]));
    // Encrypted, and its content put back: a message that reads like the original.
    let sealed = encrypted_message(&outer, &[7; 300], false);
    assert_eq!(protection(&sealed), Protection::Encrypted(vec![7; 300]));
    let back = with_entity(&sealed, &entity);
    let parsed = crate::parse(&back).unwrap_or_else(|| crate::parse(b"x").unwrap_or_default());
    assert_eq!(parsed.subject, "Proof");
    assert_eq!(parsed.text.trim(), "Approved \u{2713}");
    assert!(is_smime_part("application/pkcs7-signature", "x") && is_smime_part("application/octet-stream", "smime.p7m"));
}

/// A CMS blob's S/MIME type is read from the content type that starts it, with its length in any
/// form; anything else is none, and junk never a crash. Its message reads back as that type.
#[test]
fn a_cms_says_what_it_is() {
    let pkcs7 = |last: u8| [0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, last];
    let signed = [&[0x30, 0x82, 0x01, 0x00][..], &pkcs7(2), &[0xA0, 0x00]].concat();
    assert_eq!(smime_type_of(&signed), Some("signed-data"));
    assert_eq!(smime_type_of(&[&[0x30, 0x80][..], &pkcs7(3)].concat()), Some("enveloped-data"));
    assert_eq!(smime_type_of(&[&[0x30, 0x20][..], &pkcs7(3)].concat()), Some("enveloped-data"));
    let auth = [0x30, 0x81, 0x40, 0x06, 0x0B, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01, 0x17];
    assert_eq!(smime_type_of(&auth), Some("authEnveloped-data"));
    // Data (7.1), compressed data (9.16.1.9), another tag, cut short.
    for junk in [&[0x30, 0x10][..], &[0x31, 0x10], &[0x30, 0x85, 1, 2, 3, 4, 5], &[], &[0x30]] {
        assert_eq!(smime_type_of(junk), None);
    }
    assert_eq!(smime_type_of(&[&[0x30, 0x20][..], &pkcs7(1)].concat()), None);
    assert_eq!(smime_type_of(&[&auth[..15], &[9]].concat()), None);
    for n in 0..signed.len() {
        let _ = smime_type_of(&signed[..n]);
    }
    let message = pkcs7_message(b"Subject: Proof\r\n", &signed, "signed-data");
    assert_eq!(protection(&message), Protection::Opaque(signed.clone()));
}

/// What MailCraft writes for a signed message is 7-bit, short-lined and without trailing
/// spaces, whatever it holds: accented text, trailing spaces, a long line, an attached message
/// with 8-bit text.
#[test]
fn a_message_to_sign_is_safe_to_sign() {
    let d = crate::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Ana", "ana@posteo.example")],
        subject: "Caf\u{e9}".into(),
        body: format!("Ol\u{e1}, a line with trailing spaces   \nand a long line {}", "x".repeat(1200)),
        attachments: vec![crate::OutAttachment {
            name: "earlier.eml".into(),
            mime: "message/rfc822".into(),
            data: "Subject: x\r\n\r\nd\u{e9}j\u{e0}\r\n".as_bytes().to_vec(),
            ..Default::default()
        }],
        seven_bit: true,
        ..Default::default()
    };
    let raw = crate::build(&d, &mailcraft_model::EmailAddress::new("Riley", "riley@studio.example"), 1_800_000_000, "<a@b>").unwrap_or_default();
    let (_, entity) = entity_of(&raw);
    let bad: Vec<String> = entity
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_ascii() || l.len() > 998 || l.strip_suffix(b"\r").unwrap_or(l).ends_with(b" "))
        .map(|l| String::from_utf8_lossy(l).chars().take(80).collect())
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    assert!(safe_to_sign(&entity));
    // And it reads back: the attached message whole.
    let parsed = crate::parse(&with_entity(&raw, &entity)).unwrap_or_default();
    assert!(parsed.text.starts_with("Ol\u{e1}"));
    assert_eq!(parsed.attachments.len(), 1);
}

/// The list can tell signed and encrypted mail from its header alone.
#[test]
fn the_header_says_what_protects_it() {
    use mailcraft_model::Smime;
    let summary = |raw: &str| crate::summary_from_header(raw.as_bytes(), Default::default(), Default::default(), 0, 0).smime;
    assert_eq!(summary(SIGNED), Smime::Signed);
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=smime.p7m

"
        ),
        Smime::Encrypted
    );
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: application/pkcs7-mime; smime-type=signed-data

"
        ),
        Smime::Signed
    );
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: application/octet-stream; name=\"smime.p7m\"

"
        ),
        Smime::Encrypted
    );
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=x

"
        ),
        Smime::Signed
    );
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=x

"
        ),
        Smime::Encrypted
    );
    assert_eq!(
        summary(
            "From: a@b.example
Content-Type: text/plain

hi"
        ),
        Smime::None
    );
}

/// Header protection (RFC 9788) round trip: what's written protected reads back as the
/// message's own, the Legacy Display part unseen, and a stray header inside without `hp=`
/// changes nothing.
#[test]
fn protected_headers_read_back_as_the_messages() {
    let raw = "From: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: =?utf-8?q?Caf=C3=A9_plans?=\r\nKeywords: secret\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe plan.\r\n";
    let (outer, entity) = entity_of(raw.as_bytes());
    // Encrypted, the subject hidden outside.
    let (out_outer, protected) = protect_headers(&outer, &entity, true, true);
    let out_outer = String::from_utf8_lossy(&out_outer).to_string();
    assert!(out_outer.contains("Subject: [...]") && !out_outer.contains("Keywords") && out_outer.contains("From: Riley"), "{out_outer}");
    let inside = String::from_utf8_lossy(&protected).to_string();
    assert!(
        inside.contains("HP-Outer: Subject: [...]") && inside.contains("hp=\"cipher\"") && inside.contains("hp-legacy-display=\"1\""),
        "{inside}"
    );
    assert!(safe_to_sign(&protected), "it can be signed as it is");
    // What a reader sees once it's decrypted: the real subject, the text alone.
    let sent = encrypted_message(out_outer.as_bytes(), &[1, 2, 3], false);
    let (back, was) = with_entity_protected(&sent, &protected);
    assert!(was);
    let p = crate::parse(&back).unwrap_or_default();
    assert_eq!(p.subject, "Caf\u{e9} plans");
    assert_eq!(p.text.trim(), "The plan.", "the Legacy Display part isn't shown");
    // Signed only: everything stays outside as it was, and inside too.
    let (o2, e2) = protect_headers(&outer, &entity, false, true);
    assert!(String::from_utf8_lossy(&o2).contains("=?utf-8?q?Caf=C3=A9_plans?="));
    assert!(String::from_utf8_lossy(&e2).contains("hp=\"clear\""));
    // Someone's header fields inside an entity without hp= are no one's.
    let forged = b"From: Boss <boss@posteo.example>\r\nContent-Type: text/plain\r\n\r\nPay me.\r\n";
    let (back, was) = with_entity_protected(raw.as_bytes(), forged);
    assert!(!was);
    assert_eq!(crate::parse(&back).unwrap_or_default().from.email, "riley@studio.example");
    assert!(String::from_utf8_lossy(&quoted_printable("a\u{e9}= ")).contains("a=C3=A9=3D=20"));
}

#[test]
fn base64_is_standard() {
    assert_eq!(base64_lines(b""), b"");
    assert_eq!(base64_lines(b"f"), b"Zg==\r\n");
    assert_eq!(base64_lines(b"fo"), b"Zm8=\r\n");
    assert_eq!(base64_lines(b"foobar"), b"Zm9vYmFy\r\n");
    assert!(base64_lines(&[0u8; 100]).split(|b| *b == b'\n').all(|l| l.len() <= 77));
}

#[test]
fn junk_never_panics() {
    for raw in [
        "",
        "\r\n\r\n",
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=x\r\n\r\n--x",
        "Content-Type: application/pkcs7-mime\r\n\r\n",
    ] {
        assert!(is_protected(raw.as_bytes()) || protection(raw.as_bytes()) == Protection::None);
        let _ = protection(raw.as_bytes());
        let _ = entity_of(raw.as_bytes());
        let _ = with_entity(raw.as_bytes(), raw.as_bytes());
    }
}

/// A 50 MB message of pictures that isn't signed or encrypted is told so from its header: well
/// under the time a full parse of it takes. Timed, so in release only:
/// `cargo test -p mailcraft-mime --release -- --ignored a_big_plain_message`.
#[test]
#[ignore]
fn a_big_plain_message_is_not_parsed_to_say_it_is_unprotected() {
    let mut raw = b"From: a@b.example\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n".to_vec();
    for i in 0..50 {
        raw.extend_from_slice(format!("--m\r\nContent-Type: image/png; name=p{i}.png\r\nContent-Transfer-Encoding: base64\r\n\r\n").as_bytes());
        for _ in 0..(1 << 20) / 78 {
            raw.extend_from_slice(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAA\r\n");
        }
    }
    raw.extend_from_slice(b"--m--\r\n");
    let at = std::time::Instant::now();
    assert_eq!(protection(&raw), Protection::None);
    assert!(!is_protected(&raw));
    assert!(at.elapsed() < std::time::Duration::from_millis(50), "{:?}", at.elapsed());
}
