use super::*;

const STATUS: &[u8] =
    b"Reporting-MTA: dns; mx.studio.example\r\n\r\nFinal-Recipient: rfc822; gone@example.com\r\nAction: failed\r\nStatus: 5.1.1\r\n";

/// The status part's Content-Transfer-Encoding.
fn status_encoding(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let part = &text[text.find("Content-Type: message/delivery-status").unwrap()..];
    let start = part.find("Content-Transfer-Encoding: ").unwrap() + "Content-Transfer-Encoding: ".len();
    part[start..].split("\r\n").next().unwrap().to_string()
}

fn built(data: &[u8]) -> Vec<u8> {
    let from = EmailAddress::new("Mail Delivery", "postmaster@studio.example");
    let file = OutAttachment { name: "details.txt".into(), mime: "message/delivery-status".into(), data: data.to_vec(), ..Default::default() };
    let d = Draft { subject: "Undeliverable".into(), body: "It didn't arrive.".into(), attachments: vec![file], ..Default::default() };
    build(&d, &from, 1_700_000_000, "m@x").unwrap()
}

/// A report's status goes as it is (RFC 3464 §2.1), readable without decoding, and comes back
/// byte for byte.
#[test]
fn a_report_status_goes_as_seven_bit_text() {
    let raw = built(STATUS);
    let text = String::from_utf8_lossy(&raw);
    assert_eq!(status_encoding(&raw), "7bit", "{text}");
    assert!(text.contains("Final-Recipient: rfc822; gone@example.com\r\nAction: failed"), "{text}");
    let p = parse(&raw).unwrap();
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(attachment_data(&raw, 0).unwrap(), STATUS);
}

/// One that isn't 7bit (bare line ends, other bytes, a NUL, an over-long line) is encoded, and
/// still comes back whole.
#[test]
fn a_report_status_that_isnt_seven_bit_is_encoded() {
    let long = format!("X-Note: {}\r\n", "a".repeat(1000));
    for data in [&b"Action: failed\nStatus: 5.1.1\n"[..], b"Action: failed\r", b"Note: caf\xe9\r\n", b"A\x00B\r\n", long.as_bytes()] {
        let raw = built(data);
        assert_eq!(status_encoding(&raw), "base64", "{data:?}");
        assert_eq!(attachment_data(&raw, 0).unwrap(), data, "{data:?}");
    }
    assert!(seven_bit(b"") && seven_bit(b"a\r\n\r\nb") && seven_bit("a".repeat(998).as_bytes()));
    assert!(!seven_bit(b"\nA") && !seven_bit(b"A\r\r\n"));
}

/// A status with a line that starts with `--` is encoded: sent as it is, a line that matches the
/// message's boundary would close it early and lose the parts after it (here, the returned
/// message).
#[test]
fn a_report_status_with_a_boundary_line_is_encoded() {
    let from = EmailAddress::new("Mail Delivery", "postmaster@studio.example");
    let seed = format!("{:016x}", fnv("m@x\u{0}1700000000"));
    let data = format!("Action: failed\r\n--=_mc{seed}_1--\r\n").into_bytes();
    let status = OutAttachment { name: "details.txt".into(), mime: "message/delivery-status".into(), data: data.clone(), ..Default::default() };
    let returned = OutAttachment {
        name: "lunch.eml".into(),
        mime: "message/rfc822".into(),
        data: b"Subject: Lunch\r\n\r\nNoon.\r\n".to_vec(),
        ..Default::default()
    };
    let d = Draft { subject: "Undeliverable".into(), body: "It didn't arrive.".into(), attachments: vec![status, returned], ..Default::default() };
    let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
    assert_eq!(status_encoding(&raw), "base64");
    assert_eq!(parse(&raw).unwrap().attachments.len(), 2);
    assert_eq!(attachment_data(&raw, 0).unwrap(), data);
    assert!(!seven_bit(b"--") && !seven_bit(b"a\r\n--b") && seven_bit(b"a-\r\n-b\r\n a--"));
}

/// Found in review: an attached message saved with bare line feeds went with them, which mail
/// servers refuse (and a line of one dot in it wasn't doubled as the message was sent). Its
/// lines end CR LF as it goes; one that has them already goes untouched.
#[test]
fn an_attached_message_goes_with_whole_line_ends() {
    let from = EmailAddress::new("Riley", "riley@studio.example");
    let eml = |data: &[u8]| OutAttachment { name: "note.eml".into(), mime: "message/rfc822".into(), data: data.to_vec(), ..Default::default() };
    let bare = b"Subject: Lunch\n\nNoon.\n.\nAfter the dot.\rAnd a bare return.\r\nDone.\n";
    let whole = b"Subject: Lunch\r\n\r\nNoon.\r\n.\r\nAfter the dot.\r\nAnd a bare return.\r\nDone.\r\n";
    for data in [&bare[..], &whole[..]] {
        let d = Draft { subject: "Fwd".into(), body: "See attached.".into(), attachments: vec![eml(data)], ..Default::default() };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        assert!(!crate::smime::bare_line_ends(&raw), "{}", String::from_utf8_lossy(&raw));
        assert_eq!(attachment_data(&raw, 0).unwrap(), whole);
    }
    assert!(crate::smime::bare_line_ends(b"a\nb") && crate::smime::bare_line_ends(b"a\r") && !crate::smime::bare_line_ends(b"a\r\nb\r\n"));
}

/// A status sent as it is keeps its Content-ID, as an encoded one does.
#[test]
fn a_seven_bit_report_status_keeps_its_content_id() {
    let from = EmailAddress::new("Mail Delivery", "postmaster@studio.example");
    for data in [STATUS, b"Action: failed\n"] {
        let file = OutAttachment {
            name: "details.txt".into(),
            mime: "message/delivery-status".into(),
            data: data.to_vec(),
            content_id: "status@studio.example".into(),
        };
        let d = Draft { subject: "Undeliverable".into(), body: "It didn't arrive.".into(), attachments: vec![file], ..Default::default() };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        let p = parse(&raw).unwrap();
        assert_eq!(p.attachments[0].content_id.as_deref(), Some("status@studio.example"), "{data:?}");
    }
}
