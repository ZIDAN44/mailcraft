//! Text files saved as they came ([`attachment_data`]): the parser hands a `text/*` part out as
//! text, so a file in Latin-1 or Windows-1252 was saved, forwarded and sized as UTF-8 with its
//! other bytes replaced. Shown, it's still read in the charset it names ([`attachment_text`]).

use super::*;
use proptest::prelude::*;

/// A short file in Windows-1252: an accent, curly quotes, a pound sign, CR LF line ends.
const LATIN: &[u8] = b"Caf\xe9 au lait\r\n\x93quoted\x94 costs \xa35\r\nlast line\r\n";

/// Base64 in lines of 76.
fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for (i, chunk) in data.chunks(3).enumerate() {
        if i > 0 && i % 19 == 0 {
            out.push_str("\r\n");
        }
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * k) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Quoted-printable with every byte that isn't plain ASCII escaped (line ends too), in lines
/// kept short by soft breaks.
fn quoted_printable(data: &[u8]) -> String {
    let mut out = String::new();
    let mut line = 0;
    for &b in data {
        let piece = if (33..=126).contains(&b) && b != b'=' { char::from(b).to_string() } else { format!("={b:02X}") };
        if line + piece.len() > 72 {
            out.push_str("=\r\n");
            line = 0;
        }
        line += piece.len();
        out.push_str(&piece);
    }
    out
}

/// A message with a short text body and these parts after it, each `(headers, body)`.
fn message(parts: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut raw = b"From: Sam <sam@studio.example>\r\nTo: Lee <lee@studio.example>\r\nSubject: Files\r\nMIME-Version: 1.0\r\n\
        Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHere.\r\n"
        .to_vec();
    for (headers, body) in parts {
        raw.extend_from_slice(b"--b1\r\n");
        raw.extend_from_slice(headers.as_bytes());
        raw.extend_from_slice(b"\r\n\r\n");
        raw.extend_from_slice(body);
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(b"--b1--\r\n");
    raw
}

/// A text file's headers: its type (with any charset), name and transfer encoding.
fn file(ctype: &str, name: &str, encoding: &str) -> String {
    format!("Content-Type: {ctype}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: {encoding}")
}

/// The four transfer encodings, without a charset and with one, and an `.htm` file (the
/// Rich Text items of a data file carry those), each come out byte for byte.
#[test]
fn text_files_in_every_encoding_are_saved_byte_for_byte() {
    let raw = message(&[
        (file("text/plain", "seven.txt", "7bit"), LATIN.to_vec()),
        (file("text/plain", "eight.txt", "8bit"), LATIN.to_vec()),
        (file("text/plain; charset=windows-1252", "base64.txt", "base64"), base64(LATIN).into_bytes()),
        (file("text/plain; charset=iso-8859-1", "qp.txt", "quoted-printable"), quoted_printable(LATIN).into_bytes()),
        (file("text/html", "page.htm", "base64"), base64(b"<p>R\xe9sum\xe9</p>\r\n").into_bytes()),
    ]);
    let p = parse(&raw).unwrap();
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["seven.txt", "eight.txt", "base64.txt", "qp.txt", "page.htm"]);
    for a in &p.attachments[..4] {
        assert_eq!(attachment_data(&raw, a.index).as_deref(), Some(LATIN), "{}", a.name);
    }
    assert_eq!(attachment_data(&raw, 4).as_deref(), Some(&b"<p>R\xe9sum\xe9</p>\r\n"[..]));
    // From one parse too, as the reading pane takes them.
    let all = attachments_data(&raw, &[0, 1, 2, 3], usize::MAX, usize::MAX);
    assert!(all.len() == 4 && all.iter().all(|(_, d)| d == LATIN));
    // Shown, a file that names its charset is read in it; one that doesn't, as UTF-8 can.
    assert_eq!(attachment_text(&raw, 2).as_deref(), Some("Caf\u{e9} au lait\r\n\u{201c}quoted\u{201d} costs \u{a3}5\r\nlast line\r\n"));
    // (Latin-1 is read as Windows-1252, as browsers read it.)
    assert_eq!(attachment_text(&raw, 3), attachment_text(&raw, 2));
    assert!(attachment_text(&raw, 0).is_some_and(|t| t.starts_with("Caf\u{fffd} au lait")));
    assert_eq!(attachment_text(&raw, 9), None);
}

/// A text file inside an attached message comes out of it as it came too.
#[test]
fn a_text_file_inside_an_attached_message_is_saved_byte_for_byte() {
    let inner = message(&[(file("text/plain; charset=windows-1252", "inner.txt", "quoted-printable"), quoted_printable(LATIN).into_bytes())]);
    // Its own boundary.
    let inner = String::from_utf8_lossy(&inner).replace("b1", "b2").into_bytes();
    let raw = message(&[("Content-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"fwd.eml\"".into(), inner.clone())]);
    let eml = attachment_data(&raw, 0).unwrap();
    assert_eq!(eml, inner);
    assert_eq!(attachment_data(&eml, 0).as_deref(), Some(LATIN));
}

/// UTF-8 text, a file with an empty body and a part whose base64 the parser can't read are
/// what they were.
#[test]
fn other_text_files_are_what_they_were() {
    let utf8 = "na\u{ef}ve \u{2603}\r\n".as_bytes();
    let raw = message(&[
        (file("text/plain; charset=utf-8", "u.txt", "base64"), base64(utf8).into_bytes()),
        (file("text/plain", "empty.txt", "7bit"), Vec::new()),
        (file("text/plain", "broken.txt", "base64"), b"@@not base64@@".to_vec()),
    ]);
    assert_eq!(attachment_data(&raw, 0).as_deref(), Some(utf8));
    assert_eq!(attachment_data(&raw, 1).as_deref(), Some(&b""[..]));
    assert_eq!(attachment_data(&raw, 2).as_deref(), Some(&b"@@not base64@@"[..]));
}

proptest! {
    /// Any bytes sent as a text file, in base64 or quoted-printable, come back as they were.
    #[test]
    fn any_text_file_round_trips(data in proptest::collection::vec(any::<u8>(), 0..600), qp in any::<bool>()) {
        let (encoding, body) = if qp { ("quoted-printable", quoted_printable(&data)) } else { ("base64", base64(&data)) };
        let raw = message(&[(file("text/plain; charset=iso-8859-1", "f.txt", encoding), body.into_bytes())]);
        prop_assert_eq!(attachment_data(&raw, 0), Some(data));
    }
}
