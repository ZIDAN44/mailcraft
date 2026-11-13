//! Quoted strings with escapes. imap-proto 0.17 hands a quoted string back as it was sent, its
//! backslashes still in: a folder the server lists as `"Back\\slash"` or `"Say \"hi\""` was
//! shown, and then asked for, under a name the server doesn't have, and an attachment's name
//! read from `BODYSTRUCTURE` kept its backslashes. Such a string is given to the parser as the
//! literal it stands for (`{n}` and the bytes, RFC 3501 section 4.3), which it takes anywhere
//! it takes a string.

use super::conn::literal_len;

/// `raw` (one response: lines, and the literals they announce) with each quoted string that
/// has an escape in it written as a literal. Left as they are: a status response's text (free
/// text, not strings), a folder listing's hierarchy delimiter (`"\\"`: the parser wants that
/// one quoted, and takes its first character), and anything that isn't a well-formed quoted
/// string.
pub(super) fn plain_quotes(raw: Vec<u8>) -> Vec<u8> {
    if !raw.contains(&b'\\') || is_status(&raw) {
        return raw;
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    let mut first_line = true;
    while at < raw.len() {
        let rest = raw.get(at..).unwrap_or_default();
        let end = rest.iter().position(|&b| b == b'\n').map_or(rest.len(), |nl| nl + 1);
        let line = rest.get(..end).unwrap_or_default();
        // A LIST's first quoted string is its delimiter, whatever it is (when it has one:
        // `NIL` there, the first is the name).
        let list = first_line && is_list(line) && has_delimiter(line);
        line_plain(line, list, &mut out);
        first_line = false;
        at += end;
        // The literal this line announces goes as it is.
        let text = line.strip_suffix(b"\n").unwrap_or(line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        if let Some(n) = literal_len(text) {
            let to = at.saturating_add(n).min(raw.len());
            out.extend_from_slice(raw.get(at..to).unwrap_or_default());
            at = to;
        }
    }
    out
}

/// One line's quoted strings, those with escapes as literals. In a folder listing with a
/// delimiter (`list`) the first is that, left as it is.
fn line_plain(line: &[u8], list: bool, out: &mut Vec<u8>) {
    let mut skip = list;
    let mut i = 0usize;
    while let Some(&b) = line.get(i) {
        if b != b'"' {
            out.push(b);
            i += 1;
            continue;
        }
        let Some((inside, len)) = quoted_at(line.get(i..).unwrap_or_default()) else {
            // No closing quote: nothing here is a string.
            out.extend_from_slice(line.get(i..).unwrap_or_default());
            return;
        };
        let whole = line.get(i..i + len).unwrap_or_default();
        let escaped = whole.contains(&b'\\');
        if !escaped || skip {
            out.extend_from_slice(whole);
        } else {
            out.extend_from_slice(format!("{{{}}}\r\n", inside.len()).as_bytes());
            out.extend_from_slice(&inside);
        }
        skip = false;
        i += len;
    }
}

/// The quoted string `text` starts with: what it stands for, and how many bytes it takes with
/// its quotes. None when it never closes.
fn quoted_at(text: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut inside = Vec::new();
    let mut i = 1usize;
    loop {
        match *text.get(i)? {
            b'"' => return Some((inside, i + 1)),
            b'\\' => {
                inside.push(*text.get(i + 1)?);
                i += 2;
            }
            b'\r' | b'\n' => return None,
            b => {
                inside.push(b);
                i += 1;
            }
        }
    }
}

/// Whether the response is a status one (`* OK …`, `a1 NO …`) or a continuation: free text.
fn is_status(raw: &[u8]) -> bool {
    if raw.first() == Some(&b'+') {
        return true;
    }
    let mut words = raw.split(|&b| b == b' ' || b == b'\r' || b == b'\n');
    let _tag = words.next();
    let word = words.next().unwrap_or_default();
    [&b"OK"[..], b"NO", b"BAD", b"BYE", b"PREAUTH"].iter().any(|s| word.eq_ignore_ascii_case(s))
}

/// Whether a folder listing's delimiter is a quoted string (not `NIL`): what follows its flags.
fn has_delimiter(line: &[u8]) -> bool {
    let after = line.iter().position(|&b| b == b')').and_then(|at| line.get(at + 1..)).unwrap_or_default();
    after.iter().find(|b| **b != b' ') == Some(&b'"')
}

/// Whether the line is a folder listing (`* LIST`, `* LSUB`, Gmail's old `* XLIST`).
fn is_list(line: &[u8]) -> bool {
    let mut words = line.split(|&b| b == b' ');
    let (star, word) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    star == b"*" && [&b"LIST"[..], b"LSUB", b"XLIST"].iter().any(|s| word.eq_ignore_ascii_case(s))
}
