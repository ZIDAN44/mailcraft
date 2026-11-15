//! A summary's preview line from a message whose header pass found none ([`summary_from_full`]):
//! the text the reading pane shows, read without the rest of what it reads (the rich text, the
//! layout, the pictures, the trackers, the files).

use super::*;

/// A message of these parts (`(content type, body)`), `multipart/mixed`.
fn message_of(parts: &[(&str, &str)]) -> String {
    let mut raw =
        "From: a@shop.example\r\nTo: b@x.example\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"BB\"\r\n\r\n"
            .to_string();
    for (kind, body) in parts {
        raw.push_str(&format!("--BB\r\n{kind}\r\n\r\n{body}\r\n"));
    }
    raw.push_str("--BB--\r\n");
    raw
}

const TEXT: &str = "Content-Type: text/plain; charset=utf-8";
const HTML: &str = "Content-Type: text/html; charset=utf-8";
const FILE: &str = "Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=a.pdf\r\nContent-Transfer-Encoding: base64";

/// The preview is the text the reading pane shows, as it was when the whole message was read
/// for it: from the HTML when the text part is a stub, from what's shown to Outlook, nothing
/// for a message of files alone.
#[test]
fn a_preview_from_what_the_reading_pane_shows() {
    let outlook = "<!--[if mso]><p>Seen in Outlook</p><![endif]--><!--[if !mso]><!--><p>Seen elsewhere</p><!--<![endif]--><div style=\"mso-hide:all\">hidden</div>";
    let styled = "<style>.t{font-size:28px}</style><table><tr><td class=t>Big title</td><td>Then words</td></tr></table>";
    for parts in [
        vec![(TEXT, ""), (HTML, "<p>From the <b>HTML</b></p>")],
        vec![(TEXT, " \r\n"), (HTML, outlook)],
        vec![(HTML, styled), (TEXT, "a note after")],
        vec![(FILE, "JVBERi0xLjQ="), (TEXT, "")],
        vec![(FILE, "JVBERi0xLjQ=")],
        vec![(TEXT, "> quoted only\r\n-- \r\nsig"), (HTML, "<p>the HTML</p>")],
    ] {
        let raw = message_of(&parts);
        let summary = summary_from_full(raw.as_bytes(), key(), Flags::default(), 0, 0);
        let parsed = parse(raw.as_bytes()).expect("parses");
        // (Mail in HTML alone has its preview from the header pass.)
        if summary_from_header(raw.as_bytes(), key(), Flags::default(), 0, 0).preview.is_empty() {
            assert_eq!(summary.preview, preview(&parsed.text), "{parts:?}");
        }
    }
    let raw = message_of(&[(TEXT, ""), (HTML, outlook)]);
    assert_eq!(summary_from_full(raw.as_bytes(), key(), Flags::default(), 0, 0).preview, "Seen in Outlook");
}

/// What a preview costs when the header pass finds none: a message of a large file alone, and a
/// newsletter of tables under an empty text part. Run under the heavy lock:
/// `cargo test --release -p mailcraft-mime --lib measure_previews -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_previews() {
    let file = mailcraft_model::encoding::base64::encode(&vec![7u8; 4 << 20]);
    let files = message_of(&[(FILE, &file)]);
    let cell = "<td style=\"font-size:14px;color:#333\"><a href=\"https://shop.example/?utm_source=n\">An item</a> and some words of a story</td>";
    let rows: String = (0..1500).map(|_| format!("<tr>{cell}{cell}</tr>")).collect();
    let html = format!("<style>.t{{font-weight:bold}}</style><table width=\"600\" role=\"presentation\">{rows}</table>");
    let newsletter = message_of(&[(TEXT, ""), (HTML, &html)]);
    for (what, raw) in [("a 4 MB file alone", &files), ("a newsletter of 3,000 cells", &newsletter)] {
        let t = std::time::Instant::now();
        let mut n = 0;
        for _ in 0..5 {
            n += summary_from_full(raw.as_bytes(), key(), Flags::default(), 0, 0).preview.len();
        }
        println!("MEASURE a preview of {what}: {} ms each ({n})", t.elapsed().as_millis() / 5);
    }
}
