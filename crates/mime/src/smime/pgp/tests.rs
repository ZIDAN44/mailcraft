use super::*;

const SIGNED: &str = "From: Bo <bo@studio.example>\r\nSubject: Hi\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\"; boundary=\"b1\"\r\n\r\n\
This is an OpenPGP/MIME signed message.\r\n--b1\r\nContent-Type: text/plain\r\n\r\nHello\r\n\r\n--b1\r\n\
Content-Type: application/pgp-signature; name=signature.asc\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\n\r\niQ\r\n-----END PGP SIGNATURE-----\r\n--b1--\r\n";

const ENCRYPTED: &str = "From: Bo <bo@studio.example>\r\nSubject: [...]\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"e1\"\r\n\r\n\
--e1\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--e1\r\n\
Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\r\n-----BEGIN PGP MESSAGE-----\r\n\r\nhQ\r\n-----END PGP MESSAGE-----\r\n--e1--\r\n";

#[test]
fn pgp_mime_is_told_from_the_rest() {
    let Protection::Signed { content, signature } = protection(SIGNED.as_bytes()) else { panic!() };
    assert_eq!(content, b"Content-Type: text/plain\r\n\r\nHello\r\n");
    assert!(signature.starts_with(b"-----BEGIN PGP SIGNATURE-----"));
    // Stored with bare LFs, it's still what was signed; the protocol in another case too.
    let lf = SIGNED.replace("\r\n", "\n").replace("application/pgp-signature\"", "Application/PGP-Signature\"");
    assert!(matches!(protection(lf.as_bytes()), Protection::Signed { content, .. } if content == b"Content-Type: text/plain\r\n\r\nHello\r\n"));
    let Protection::Encrypted(body) = protection(ENCRYPTED.as_bytes()) else { panic!() };
    assert!(body.starts_with(b"-----BEGIN PGP MESSAGE-----"));
    // Header names and values in any case.
    let shouting = ENCRYPTED
        .replace(
            "Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"",
            "CONTENT-TYPE: MULTIPART/ENCRYPTED; PROTOCOL=\"APPLICATION/PGP-ENCRYPTED\"",
        )
        .replace("Content-Type: application/octet-stream", "content-type: Application/Octet-Stream");
    assert!(matches!(protection(shouting.as_bytes()), Protection::Encrypted(_)), "{shouting}");
    // S/MIME isn't PGP, and a PGP/MIME message inside another multipart is an attachment.
    assert_eq!(protection(SIGNED.replace("pgp-signature\";", "pkcs7-signature\";").as_bytes()), Protection::None);
    assert_eq!(super::super::protection(SIGNED.as_bytes()), super::super::Protection::None);
    let wrapped = format!(
        "From: a@b.example\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n{}\r\n--m--\r\n",
        &ENCRYPTED[ENCRYPTED.find("Content-Type").unwrap_or(0)..]
    );
    assert_eq!(protection(wrapped.as_bytes()), Protection::None);
    // No signature part, no encrypted part: nothing.
    assert_eq!(protection(SIGNED.replace("application/pgp-signature; name", "text/plain; name").as_bytes()), Protection::None);
}

#[test]
fn inline_pgp_counts_in_a_text_alone() {
    let inline = "From: a@b.example\r\nContent-Type: text/plain; charset=ISO-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n\
-----BEGIN PGP MESSAGE-----\r\n\r\nhQ=3D\r\n-----END PGP MESSAGE-----\r\n";
    let Protection::Inline { body, charset } = protection(inline.as_bytes()) else { panic!() };
    assert!(body.starts_with(b"-----BEGIN PGP MESSAGE-----\r\n\r\nhQ=\r\n"), "transfer encoding undone");
    assert_eq!(charset, "iso-8859-1");
    // No Content-Type at all is text too; a block quoted inside a line isn't one.
    assert!(matches!(
        protection(b"From: a@b.example\r\n\r\n-----BEGIN PGP SIGNED MESSAGE-----\r\nHash: SHA256\r\n\r\nhi\r\n"),
        Protection::Inline { .. }
    ));
    assert_eq!(protection(b"From: a@b.example\r\n\r\nsee > -----BEGIN PGP MESSAGE-----\r\n"), Protection::None);
    // An HTML message with a block in it isn't inline PGP.
    assert_eq!(protection(b"From: a@b.example\r\nContent-Type: text/html\r\n\r\n-----BEGIN PGP MESSAGE-----\r\n"), Protection::None);
    let e = text_entity("caf\u{e9}\n".as_bytes(), "iso-8859-1");
    assert_eq!(e, "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\ncaf\u{e9}\r\n".as_bytes());
    assert!(String::from_utf8_lossy(&text_entity(&[0x63, 0xE9], "x\r\nBcc: evil")).starts_with("Content-Type: text/plain; charset=xBcc:evil\r\n"));
}

#[test]
fn keys_a_message_carries_are_found() {
    let raw = "From: Ana <ana@posteo.example>\r\nAutocrypt: addr=ana@posteo.example; keydata=\r\n AQID\r\n BAUG\r\nautocrypt: addr=x@y.example; keydata=Bw==\r\n\
Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nMy key\r\n--m\r\n\
Content-Type: application/pgp-keys; name=\"ana.asc\"\r\n\r\n-----BEGIN PGP PUBLIC KEY BLOCK-----\r\n-----END PGP PUBLIC KEY BLOCK-----\r\n--m--\r\n";
    assert_eq!(autocrypt(raw.as_bytes()), ["addr=ana@posteo.example; keydata= AQID BAUG", "addr=x@y.example; keydata=Bw=="]);
    let keys = key_parts(raw.as_bytes());
    assert_eq!(keys.len(), 1);
    assert!(keys[0].starts_with(b"-----BEGIN PGP PUBLIC KEY BLOCK-----"));
    assert!(autocrypt(b"From: a@b\r\n\r\nAutocrypt: addr=a@b; keydata=AQID\r\n").is_empty(), "the body isn't the header");
    // What's worth opening, from the header (and a text's own bytes).
    assert!(may_carry(raw.as_bytes()) && may_carry(SIGNED.as_bytes()) && may_carry(ENCRYPTED.as_bytes()));
    assert!(may_carry(b"From: a@b\r\n\r\n-----BEGIN PGP MESSAGE-----\r\n"));
    let attached = raw.replace("Autocrypt", "X-Other");
    assert!(may_carry(attached.as_bytes()), "an attached key");
    assert!(!may_carry(b"From: a@b\r\nContent-Type: text/plain\r\n\r\nHello\r\n"));
    assert!(!may_carry(b"From: a@b\r\nContent-Type: text/html\r\n\r\n-----BEGIN PGP MESSAGE-----\r\n"));
    let pictures = format!(
        "From: a@b\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n{}\r\napplication/pgp-keys\r\n--m--\r\n",
        "A".repeat(KEYS_LOOKED_MAX)
    );
    assert!(!may_carry(pictures.as_bytes()), "a big message isn't read through");
}

#[test]
fn protected_mail_is_written_as_pgp_mime() {
    let raw = b"Autocrypt: addr=riley@studio.example; keydata=\r\n AQID\r\nFrom: Riley <riley@studio.example>\r\nTo: ana@posteo.example\r\nSubject: Plan\r\nMIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\nThe plan.\r\n";
    let mut signed_over = Vec::new();
    let mut sign = |e: &[u8]| {
        signed_over = e.to_vec();
        Ok(b"-----BEGIN PGP SIGNATURE-----\n\nsig\n-----END PGP SIGNATURE-----\n".to_vec())
    };
    let mut encrypted = Vec::new();
    let mut encrypt = |e: &[u8]| {
        encrypted = e.to_vec();
        Ok(b"-----BEGIN PGP MESSAGE-----\n\nmsg\n-----END PGP MESSAGE-----\n".to_vec())
    };
    let sent = protect(raw, Some(&mut sign), Some(&mut encrypt), true, "=_b1").unwrap();
    let text = String::from_utf8_lossy(&sent).to_string();
    assert!(text.contains("Subject: [...]") && !text.contains("The plan"), "{text}");
    let Protection::Encrypted(body) = protection(&sent) else { panic!("{text}") };
    assert!(body.starts_with(b"-----BEGIN PGP MESSAGE-----\r\n"));
    // What was encrypted is the signed message, which reads back as what was signed.
    let inner = with_entity(&sent, &encrypted);
    let Protection::Signed { content, signature } = protection(&inner) else { panic!("{}", String::from_utf8_lossy(&inner)) };
    assert_eq!(content, signed_over, "the signature covers exactly what's there");
    assert!(text.contains("Autocrypt: addr=riley") && !String::from_utf8_lossy(&encrypted).contains("Autocrypt"), "the key goes outside only");
    assert!(signature.starts_with(b"-----BEGIN PGP SIGNATURE-----\r\n"));
    let read = crate::parse(&super::super::with_entity_protected(&inner, &content).0).unwrap_or_default();
    assert_eq!(read.subject, "Plan");
    assert_eq!(read.text.trim(), "The plan.");
    assert!(read.attachments.is_empty());
    // Signed only.
    let s = protect(raw, Some(&mut |_: &[u8]| Ok(b"sig".to_vec())), None, true, "=_b2").unwrap();
    assert!(String::from_utf8_lossy(&s).contains("Subject: Plan"));
    assert!(matches!(protection(&s), Protection::Signed { .. }));
    assert!(is_pgp_part("application/pgp-signature", "signature.asc") && is_pgp_part("application/octet-stream", "Encrypted.asc"));
    assert!(super::super::is_smime_part("application/pgp-encrypted", ""));
}

#[test]
fn junk_never_panics() {
    for raw in [
        "",
        "\r\n\r\n",
        "Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=x\r\n\r\n--x",
        "Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"\r\n\r\n",
        "Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=x\r\n\r\n--x\r\n--x--",
        "\r\n-----BEGIN PGP MESSAGE-----",
        "Autocrypt:\r\nAutocrypt",
    ] {
        let _ = protection(raw.as_bytes());
        let _ = autocrypt(raw.as_bytes());
        let _ = key_parts(raw.as_bytes());
        let _ = text_entity(raw.as_bytes(), raw);
    }
}
