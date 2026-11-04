//! Which parts the attachment bar lists and what their bytes are, as mail-parser 1.0 reads them:
//! what MailCraft kept from 0.11 and what it takes from 1.0.

use super::*;

const PNG: &str = "iVBORw0KGgo=";

fn related(picture_headers: &str) -> Vec<u8> {
    format!(
        "From: a@studio.example\r\nSubject: Logo\r\nContent-Type: multipart/related; boundary=\"r\"\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n<p>Hi</p><img src=\"cid:logo@x\">\r\n--r\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-ID: <logo@x>\r\n{picture_headers}Content-Transfer-Encoding: base64\r\n\r\n{PNG}\r\n--r--\r\n"
    )
    .into_bytes()
}

/// A picture the HTML shows by `cid:` is in both of mail-parser's bodies and so, since 1.0
/// (RFC 8621), not among its attachments: MailCraft still lists it, inline, so the reading pane
/// finds it and a reply quotes it; and still gives no paperclip for it.
#[test]
fn a_picture_the_html_shows_stays_listed_and_inline() {
    for headers in ["", "Content-Disposition: inline\r\n"] {
        let raw = related(headers);
        let p = parse(&raw).unwrap();
        let got: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline, a.content_id.as_deref())).collect();
        assert_eq!(got, [("logo.png", true, Some("logo@x"))], "{headers:?}");
        assert!(p.images.iter().any(|i| i == "cid:logo@x"));
        assert_eq!(attachment_data(&raw, 0).as_deref(), Some(&b"\x89PNG\r\n\x1a\n"[..]));
        assert!(!summary_from_full(&raw, key(), Flags::default(), 0, 0).has_attachments);
        assert_eq!(index_doc(&raw).files, 0);
    }
}

fn mixed(file: &str) -> Vec<u8> {
    format!(
        "From: a@studio.example\r\nSubject: Files\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSee the file.\r\n--m\r\n{file}\r\n--m--\r\n"
    )
    .into_bytes()
}

/// A file whose base64 has a byte out of place opens as the file (mail-parser 1.0 passes over
/// it; 0.11 gave the encoded text in its place).
#[test]
fn a_file_with_a_stray_byte_in_its_base64_is_the_file() {
    let raw = mixed(
        "Content-Type: image/png; name=\"a.png\"\r\nContent-Disposition: attachment; filename=\"a.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBO!Rw0KGgo=",
    );
    let p = parse(&raw).unwrap();
    assert_eq!(p.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["a.png"]);
    assert_eq!(attachment_data(&raw, 0).as_deref(), Some(&b"\x89PNG\r\n\x1a\n"[..]));
}

/// A message whose own text has damaged base64 shows that text (mail-parser 1.0 keeps it in the
/// body; 0.11 turned it into a file of its encoded text).
#[test]
fn a_body_with_damaged_base64_is_still_the_body() {
    let text = encodify::base64::STANDARD.encode("Meet at noon.\r\n");
    let raw = format!("From: a@studio.example\r\nSubject: Noon\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n{text}!\r\n");
    let p = parse(raw.as_bytes()).unwrap();
    assert_eq!((p.text.trim(), p.attachments.len()), ("Meet at noon.", 0));
}

/// Input with no header field and no blank line isn't a message: there is nothing to show
/// (mail-parser 0.11 gave one with an empty body, its text dropped).
#[test]
fn text_with_no_header_is_no_message() {
    assert_eq!(parse(b"just some words"), None);
    let s = summary_from_header(b"just some words", key(), Flags::default(), 0, 7);
    assert_eq!((s.date, s.subject.as_str()), (7, ""));
}
