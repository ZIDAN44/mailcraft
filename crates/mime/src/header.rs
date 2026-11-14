//! A message's header block read from its source, as hostile mail writes it as well as the RFCs
//! do: where the header ends (the first blank line, its lines ending in CRLF, a bare LF or a mix),
//! its fields by name (any case, folded lines unfolded) and a report's `report-type`. One reading
//! for every place that looks at a header without parsing the whole message.

use std::ops::Range;

/// The longest `report-type` read; the ones in use are a few words.
const MAX_REPORT_TYPE: usize = 64;

/// The blank line that ends a message's header (RFC 5322 2.1), as the range from where it starts
/// (the header's length, its last line's end included) to where the body starts. A line is blank
/// when it is empty before its `\n` or its `\r\n`, whatever the other lines end with. `None` when
/// there is none: all of `raw` is header. A source that starts with a blank line has an empty
/// header (`0..`).
pub fn header_end(raw: &[u8]) -> Option<Range<usize>> {
    let mut at = 0usize;
    loop {
        let rest = raw.get(at..)?;
        if rest.starts_with(b"\n") {
            return Some(at..at.saturating_add(1));
        }
        if rest.starts_with(b"\r\n") {
            return Some(at..at.saturating_add(2));
        }
        at = at.saturating_add(memchr::memchr(b'\n', rest)?).saturating_add(1);
    }
}

/// A header block without its last line's end (`\r\n` or `\n`).
pub fn without_line_end(head: &[u8]) -> &[u8] {
    let head = head.strip_suffix(b"\n").unwrap_or(head);
    head.strip_suffix(b"\r").unwrap_or(head)
}

/// Each field of a header block: its name as written (trailing white space taken off) and where
/// it is, from its name to the end of its last folded line, that line's end left out. A line
/// that starts with a space or a tab continues the field above it; a line with no colon, or with
/// nothing before it, is no field and ends the one above; a blank line ends it too (`head` is
/// the header alone, cut at [`header_end`]: a blank line in it is skipped, not taken as the
/// header's end).
fn fields(head: &[u8]) -> impl Iterator<Item = (&[u8], Range<usize>)> {
    let mut lines = head.split_inclusive(|b| *b == b'\n').scan(0usize, |at, line| {
        let start = *at;
        *at = at.saturating_add(line.len());
        Some((start, without_line_end(line)))
    });
    let mut next = lines.next();
    std::iter::from_fn(move || {
        loop {
            let (start, line) = next.take()?;
            next = lines.next();
            let folded = line.first().is_some_and(|b| *b == b' ' || *b == b'\t');
            let name = line.iter().position(|b| *b == b':').and_then(|colon| line.get(..colon)).map(<[u8]>::trim_ascii_end);
            let Some(name) = name.filter(|n| !folded && !n.is_empty()) else { continue };
            let mut end = start.saturating_add(line.len());
            while let Some((more_start, more)) = next.filter(|(_, l)| l.first().is_some_and(|b| *b == b' ' || *b == b'\t')) {
                end = more_start.saturating_add(more.len());
                next = lines.next();
            }
            return Some((name, start..end));
        }
    })
}

/// Where the first field named `name` (any case) is in a header block: from its name to the end
/// of its last folded line, that line's end left out ([`fields`]).
pub fn field_span(head: &[u8], name: &str) -> Option<Range<usize>> {
    fields(head).find(|(n, _)| n.eq_ignore_ascii_case(name.as_bytes())).map(|(_, at)| at)
}

/// The values of every field named `name` (any case) in a header block, in order: each unfolded,
/// its lines' text trimmed and joined by a space, and read as UTF-8 (a byte that isn't, as
/// U+FFFD). `head` is the header alone ([`header_end`]); either line end, folded lines and
/// `Name :` are read.
pub fn header_fields<'h>(head: &'h [u8], name: &'h str) -> impl Iterator<Item = String> + 'h {
    fields(head).filter(move |(n, _)| n.eq_ignore_ascii_case(name.as_bytes())).map(|(_, at)| {
        let field = head.get(at).unwrap_or_default();
        let value = field.iter().position(|b| *b == b':').and_then(|colon| field.get(colon.saturating_add(1)..)).unwrap_or_default();
        let pieces: Vec<&[u8]> = value.split(|b| *b == b'\n').map(<[u8]>::trim_ascii).filter(|p| !p.is_empty()).collect();
        String::from_utf8_lossy(&pieces.join(&b' ')).into_owned()
    })
}

/// The value of the first field named `name` in a header block ([`header_fields`]).
pub fn header_field(head: &[u8], name: &str) -> Option<String> {
    header_fields(head, name).next()
}

/// How much of a source is read for its `Message-ID` ([`written_message_id`]).
const MAX_ID_HEAD: usize = 64 * 1024;

/// The `Message-ID` a message was written with, from its header (never its body), without its
/// brackets: what says which draft a save was of. Empty when it has none. Its lines may end in
/// CRLF or a bare LF ([`header_end`], [`header_field`]): only CRLF was read, and with bare LFs
/// the body counted as header.
pub fn written_message_id(raw: &[u8]) -> String {
    let end = header_end(raw).map_or(raw.len(), |blank| blank.start).min(MAX_ID_HEAD);
    header_field(raw.get(..end).unwrap_or(raw), "message-id").map(|v| mailcraft_model::thread::norm_id(&v)).unwrap_or_default()
}

/// A `multipart/report` Content-Type's `report-type` (RFC 6522): lower case, its quotes off, when
/// it is a token of at most 64 characters; `None` for any other type, or a report without one.
pub fn report_type(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    if !parts.next()?.trim().eq_ignore_ascii_case("multipart/report") {
        return None;
    }
    let kind = parts.filter_map(|p| p.split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("report-type"))?.1;
    let kind = kind.trim().trim_matches('"').trim().to_ascii_lowercase();
    let token = !kind.is_empty() && kind.len() <= MAX_REPORT_TYPE && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._+".contains(&b));
    token.then_some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_header_ends_at_its_first_blank_line_whatever_its_line_ends() {
        // CRLF, bare LF, and the two mixed either way.
        assert_eq!(header_end(b"A: 1\r\nB: 2\r\n\r\nbody\r\n\r\nmore"), Some(12..14));
        assert_eq!(header_end(b"A: 1\nB: 2\n\nbody"), Some(10..11));
        assert_eq!(header_end(b"A: 1\r\nB: 2\n\r\nbody"), Some(11..13));
        assert_eq!(header_end(b"A: 1\nB: 2\r\n\nbody"), Some(11..12));
        // A bare-LF blank line in the body of a CRLF message isn't reached.
        assert_eq!(header_end(b"A: 1\r\n\r\nx\n\ny"), Some(6..8));
        // No blank line: all header. One to start with: no header.
        assert_eq!(header_end(b"A: 1\r\nB: 2\r\n"), None);
        assert_eq!(header_end(b"no header end"), None);
        assert_eq!(header_end(b""), None);
        assert_eq!(header_end(b"\r\nbody\r\n\r\n"), Some(0..2));
        assert_eq!(header_end(b"\nbody"), Some(0..1));
        // A line of a lone CR, or of spaces, isn't blank.
        assert_eq!(header_end(b"A: 1\r\n\r\r\n \r\nB\r\n\r\n"), Some(15..17));
        assert_eq!(without_line_end(b"A: 1\r\n"), b"A: 1");
        assert_eq!(without_line_end(b"A: 1\n"), b"A: 1");
        assert_eq!(without_line_end(b"A: 1"), b"A: 1");
    }

    #[test]
    fn fields_by_name_folded_and_in_any_case() {
        let head = b"Subject: x\r\nX-Long: a\r\n  b\r\n\tc \r\nimportance : High\nTo: z\r\nX-Long: second\r\n";
        assert_eq!(header_field(head, "Importance").as_deref(), Some("High"));
        assert_eq!(header_field(head, "x-long").as_deref(), Some("a b c"));
        assert_eq!(header_fields(head, "X-LONG").collect::<Vec<_>>(), ["a b c", "second"]);
        assert_eq!(header_field(head, "Missing"), None);
        // The span: from the name to the end of its last folded line, that line's end left out.
        let at = field_span(head, "X-Long").unwrap();
        assert_eq!(&head[at], b"X-Long: a\r\n  b\r\n\tc ");
        // A field named only in a folded line, in a value, or by a longer name isn't it.
        let head = b"Subject: Re\r\n Message-ID: <no@x>\r\nX-Message-ID: <no@y>\r\nMessage-IDs: <no@z>\r\n";
        assert_eq!(header_field(head, "Message-ID"), None);
        // An empty value, one folded onto the next line, and a line that is no field ending one.
        let head = b"Empty:\r\nNext:\r\n <a@b>\r\nCut: a\r\ngarbage\r\n more\r\n";
        assert_eq!(header_field(head, "empty").as_deref(), Some(""));
        assert_eq!(header_field(head, "next").as_deref(), Some("<a@b>"));
        assert_eq!(header_field(head, "cut").as_deref(), Some("a"));
        // A blank line is skipped, not the end: the caller cuts the header first.
        assert_eq!(header_field(b"A: 1\r\n\r\nB: 2\r\n", "b").as_deref(), Some("2"));
        // Bytes that aren't UTF-8 are read as U+FFFD.
        assert_eq!(header_field(b"Subject: caf\xE9\r\n", "subject").as_deref(), Some("caf\u{FFFD}"));
    }

    #[test]
    fn a_report_s_type() {
        assert_eq!(report_type("multipart/report; report-type=delivery-status; boundary=b").as_deref(), Some("delivery-status"));
        assert_eq!(
            report_type("Multipart/Report; boundary=b; REPORT-TYPE=\" Disposition-Notification \"").as_deref(),
            Some("disposition-notification")
        );
        assert_eq!(report_type("multipart/mixed; report-type=delivery-status"), None);
        assert_eq!(report_type("multipart/report; boundary=b"), None);
        assert_eq!(report_type("multipart/report; report-type="), None);
        assert_eq!(report_type("multipart/report; report-type=\"delivery status\""), None);
        assert_eq!(report_type(&format!("multipart/report; report-type={}", "x".repeat(65))), None);
        assert_eq!(report_type(&format!("multipart/report; report-type={}", "x".repeat(64))).map(|t| t.len()), Some(64));
        assert_eq!(report_type(""), None);
    }

    proptest! {
        /// Any bytes: the blank line found is one, nothing before it is, and the fields read never
        /// panic and come from before it.
        #[test]
        fn any_source(raw in proptest::collection::vec(prop_oneof![Just(b'\r'), Just(b'\n'), Just(b':'), Just(b' '), Just(b'a'), any::<u8>()], 0..200)) {
            match header_end(&raw) {
                Some(r) => {
                    let blank = &raw[r.clone()];
                    prop_assert!(blank == b"\n" || blank == b"\r\n");
                    prop_assert!(r.start == 0 || raw[r.start - 1] == b'\n');
                    let head = &raw[..r.start];
                    prop_assert!(!head.split_inclusive(|b| *b == b'\n').any(|l| l == b"\n" || l == b"\r\n"));
                }
                None => prop_assert!(!raw.split_inclusive(|b| *b == b'\n').any(|l| l == b"\n" || l == b"\r\n")),
            }
            let head = &raw[..header_end(&raw).map_or(raw.len(), |r| r.start)];
            if let Some(at) = field_span(head, "a") {
                prop_assert!(at.end <= head.len());
            }
            let _ = header_fields(head, "a").count();
        }
    }

    /// A draft's Message-ID from its header: any case, folded, without brackets; never one its
    /// body quotes, nor a header whose name only starts the same; its lines ending in CRLF or a
    /// bare LF (a bare-LF message's was never found: its whole source was read as one line).
    #[test]
    fn a_messages_written_id_is_read_from_its_header() {
        assert_eq!(written_message_id(b"Subject: x\r\nMessage-ID: <a1@studio.example>\r\n\r\nbody"), "a1@studio.example");
        assert_eq!(written_message_id(b"message-id:\r\n <b2@studio.example>\r\nTo: z\r\n\r\n"), "b2@studio.example");
        assert_eq!(written_message_id(b"Message-IDs: <no@x>\r\nSubject: x\r\n\r\nMessage-ID: <quoted@x>\r\n"), "");
        assert_eq!(written_message_id(b""), "");
        assert_eq!(written_message_id(b"Subject: x\nMessage-ID: <c3@studio.example>\n\nbody"), "c3@studio.example");
        assert_eq!(written_message_id(b"Subject: x\n\nMessage-ID: <quoted@x>\n"), "");
    }
}
