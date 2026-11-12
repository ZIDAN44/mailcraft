//! Plain text as it arrives: a body in several parts, lines its sender wrapped softly
//! (`format=flowed`, RFC 3676), and a header whose bytes aren't UTF-8.

use std::borrow::Cow;

use mail_parser::{Charset, Message, MessagePart, PartKind};

/// The plain text a message shows: each of its text parts in order, a blank line between them
/// (a note under an attachment, a mailing list's footer), not the first alone.
pub(crate) fn plain_body(m: &Message<'_>) -> Option<String> {
    let mut out: Option<String> = None;
    for p in m.text_body().filter(|p| p.is_text() && !matches!(p.kind(), PartKind::Html)) {
        let Some(t) = p.text() else { continue };
        let t = if is_flowed(&p) { unflow(&t, has(&p, "delsp", "yes")) } else { t.into_owned() };
        match &mut out {
            None => out = Some(t),
            Some(all) => {
                if !t.trim().is_empty() {
                    all.push_str("\n\n");
                    all.push_str(&t);
                }
            }
        }
    }
    out
}

/// The HTML a message shows: its HTML body, and with it the text parts that come before or
/// after (a mailing list's footer under an HTML message, a note over one), each as a paragraph
/// of its own. They were left out: the first part alone was shown.
pub(crate) fn html_whole<'m>(m: &'m Message<'m>) -> Option<Cow<'m, str>> {
    if m.html_body().len() < 2 {
        return m.body_html(0);
    }
    let mut out = String::new();
    for p in m.html_body() {
        let Some(t) = p.text() else { continue };
        if matches!(p.kind(), PartKind::Html) {
            out.push_str(&t);
        } else if !t.trim().is_empty() {
            let t = if is_flowed(&p) { unflow(&t, has(&p, "delsp", "yes")) } else { t.into_owned() };
            out.push_str("<p>");
            for c in t.trim_end().chars() {
                match c {
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '&' => out.push_str("&amp;"),
                    '\n' => out.push_str("<br>"),
                    '\r' => {}
                    c => out.push(c),
                }
            }
            out.push_str("</p>");
        }
    }
    Some(Cow::Owned(out))
}

fn has(p: &MessagePart<'_>, attribute: &str, value: &str) -> bool {
    p.content_type().and_then(|c| c.attribute(attribute)).is_some_and(|v| v.trim().eq_ignore_ascii_case(value))
}

fn is_flowed(p: &MessagePart<'_>) -> bool {
    has(p, "format", "flowed")
}

/// `format=flowed` text with its paragraphs whole again (RFC 3676, 4.2): a line ending in a
/// space carries on in the next one of the same quote depth, a space before a line's text is
/// stuffing, and `-- ` (a signature's start) never carries on. With `delsp`, the space that
/// ends a line was added to wrap it and goes.
fn unflow(text: &str, delsp: bool) -> String {
    let mut out = String::with_capacity(text.len());
    // The paragraph being put together: its quote depth and its text so far.
    let mut open: Option<(usize, String)> = None;
    fn close(out: &mut String, open: &mut Option<(usize, String)>) {
        if let Some((depth, text)) = open.take() {
            for _ in 0..depth {
                out.push('>');
            }
            if depth > 0 && !text.is_empty() {
                out.push(' ');
            }
            out.push_str(&text);
            out.push('\n');
        }
    }
    for line in text.lines() {
        let depth = line.bytes().take_while(|b| *b == b'>').count();
        let rest = line.get(depth..).unwrap_or_default();
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        if open.as_ref().is_some_and(|(d, _)| *d != depth) {
            close(&mut out, &mut open);
        }
        let soft = rest.ends_with(' ') && rest != "-- ";
        if rest == "-- " {
            close(&mut out, &mut open);
        }
        let piece = if soft && delsp { rest.get(..rest.len().saturating_sub(1)).unwrap_or(rest) } else { rest };
        match &mut open {
            Some((_, text)) => text.push_str(piece),
            None => open = Some((depth, piece.to_string())),
        }
        if !soft {
            close(&mut out, &mut open);
        }
    }
    close(&mut out, &mut open);
    out
}

/// A message whose header has bytes that aren't UTF-8 (an old mailer's `Subject: café` in
/// Latin-1, unencoded), with those lines read in the character set the message names for its
/// text, or else Windows-1252: they showed as `caf�`. Any other message is returned as it is.
pub(crate) fn readable_headers(raw: &[u8]) -> Cow<'_, [u8]> {
    let end = header_end(raw);
    let head = raw.get(..end).unwrap_or(raw);
    if head.is_ascii() || std::str::from_utf8(head).is_ok() {
        return Cow::Borrowed(raw);
    }
    let named = named_charset(head);
    let latin = Charset::from_label(b"windows-1252");
    let mut out = Vec::with_capacity(raw.len() + 64);
    for line in head.split_inclusive(|b| *b == b'\n') {
        if std::str::from_utf8(line).is_ok() {
            out.extend_from_slice(line);
            continue;
        }
        // The named set, where every byte of the line is a character in it.
        let read = named.as_ref().map(|c| c.decode(line)).filter(|t| !t.contains('\u{FFFD}')).or_else(|| latin.as_ref().map(|c| c.decode(line)));
        match read {
            Some(t) => out.extend_from_slice(t.as_bytes()),
            None => out.extend_from_slice(line),
        }
    }
    out.extend_from_slice(raw.get(end..).unwrap_or_default());
    Cow::Owned(out)
}

/// Where the header ends: after its last line, before the empty one.
fn header_end(raw: &[u8]) -> usize {
    let mut at = 0;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        if line == b"\n" || line == b"\r\n" {
            break;
        }
        at += line.len();
    }
    at
}

/// The character set a header names (`charset=iso-8859-2`), when it's one whose letters and
/// digits are ASCII's: a header read as UTF-16 or ISO-2022-JP would be no header at all.
fn named_charset(head: &[u8]) -> Option<Charset> {
    const KEY: &[u8] = b"charset=";
    let at = head.windows(KEY.len()).position(|w| w.eq_ignore_ascii_case(KEY))?;
    let rest = head.get(at + KEY.len()..)?;
    let label: Vec<u8> =
        rest.iter().copied().skip_while(|b| *b == b'"').take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')).take(40).collect();
    let lower = label.to_ascii_lowercase();
    let ascii_based = ["iso-8859", "windows-125", "koi8", "gb", "big5", "euc-", "shift_jis", "ibm", "cp", "latin", "tis-620", "macintosh"];
    ascii_based.iter().any(|p| lower.starts_with(p.as_bytes())).then(|| Charset::from_label(&label)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowed_lines_are_joined() {
        let t = "One long line wrapped by the \nsender, joined again.\n\n> quoted soft \n> line\n>> deeper \n> not joined\n From stuffed\n-- \nSig \nline\n";
        assert_eq!(
            unflow(t, false),
            "One long line wrapped by the sender, joined again.\n\n> quoted soft line\n>> deeper \n> not joined\nFrom stuffed\n-- \nSig line\n"
        );
        // delsp: the wrapping space goes, so a word cut in two is one again.
        assert_eq!(unflow("para \ngraph and \nmore\n", true), "paragraph andmore\n");
        assert_eq!(unflow("", false), "");
    }

    #[test]
    fn a_body_in_parts_is_whole() {
        let raw = b"Subject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=Z\r\n\r\n--Z\r\nContent-Type: text/plain\r\n\r\nfirst\r\n--Z\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--Z\r\nContent-Type: text/plain; format=flowed\r\n\r\nsecond \r\nhalf\r\n--Z--\r\n";
        let p = crate::parse(raw).unwrap_or_default();
        assert_eq!(p.text, "first\n\nsecond half");
        assert_eq!(p.attachments.len(), 1);
    }

    #[test]
    fn text_after_an_html_body_is_shown_with_it() {
        let raw = b"Subject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=Z\r\n\r\n--Z\r\nContent-Type: text/html\r\n\r\n<p>The <b>message</b></p>\r\n--Z\r\nContent-Type: text/plain\r\n\r\n-- \r\nlist <footer> & more\r\n--Z--\r\n";
        let html = crate::html_body(raw).unwrap_or_default();
        assert!(html.contains("<b>message</b>") && html.contains("list &lt;footer&gt; &amp; more"), "{html}");
        let p = crate::parse(raw).unwrap_or_default();
        let shown: String = p.rich.iter().map(|r| format!("{r:?}")).collect();
        assert!(shown.contains("message") && shown.contains("list <footer> & more"), "{shown}");
        assert!(p.text.contains("message") && p.text.contains("list <footer> & more"), "{}", p.text);
        // One HTML body is as it came.
        let one = b"Subject: s\r\nContent-Type: text/html\r\n\r\n<p>only</p>\r\n";
        assert_eq!(crate::html_body(one).unwrap_or_default().trim(), "<p>only</p>");
    }

    #[test]
    fn a_header_that_is_not_utf8_is_read() {
        let raw = b"From: J\xf6rg M\xfcller <jorg@studio.example>\nSubject: caf\xe9\nContent-Type: text/plain; charset=iso-8859-1\n\nBody caf\xe9\n";
        let p = crate::parse(raw).unwrap_or_default();
        assert_eq!((p.from.name.as_str(), p.subject.as_str(), p.text.as_str()), ("Jörg Müller", "café", "Body café"));
        // In the set the message names for its text.
        let raw = b"Subject: \xf0\xd2\xc9\xd7\xc5\xd4\nContent-Type: text/plain; charset=koi8-r\n\nx\n";
        assert_eq!(crate::parse(raw).unwrap_or_default().subject, "Привет");
        // No set named, or one a header can't be in: Windows-1252.
        assert_eq!(
            crate::parse(b"Subject: \x93x\x94\nContent-Type: text/plain; charset=utf-16\n\n").unwrap_or_default().subject,
            "\u{201c}x\u{201d}"
        );
        // UTF-8 and ASCII headers aren't touched, nor is the body.
        let utf8 = "Subject: café\n\nbody \u{e9}".as_bytes();
        assert!(matches!(readable_headers(utf8), Cow::Borrowed(_)));
        let body_only = b"Subject: x\n\ncaf\xe9";
        assert!(matches!(readable_headers(body_only), Cow::Borrowed(_)));
    }
}
