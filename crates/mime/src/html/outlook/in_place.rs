//! What a message hides from Outlook (`mso-hide: all`) hidden where it is, in its HTML as it
//! came: its tags read as a browser reads them (a quote only after `=` starts a value, a `<` in a
//! value is text), past comments, the text of scripts, style sheets and text boxes, and nothing
//! after `<plaintext>`. Only the tags that hide something are written into, so that the message
//! is read as it came (found in review: written out again from the tree, a megabyte table took
//! twice as long to read, and past the elements the tree reads its paragraphs ran together).

use std::borrow::Cow;

use super::reading::{Comments, RAW_TEXT, raw_text_end, tag_end_at};
use super::{decoded, mso_hidden};

/// The attribute what's hidden from Outlook is marked with ([`marked_hidden`]).
pub(crate) const HIDDEN_MARK: &str = "data-mailcraft-hidden";
/// The rule the text converters hide what's marked with: the reader's own, over any the message
/// sets (`display: block !important` in its style sheet), as the elements' own styles can't.
pub(crate) const HIDDEN_RULE: &str = "[data-mailcraft-hidden] { display: none !important }";

/// How what's hidden from Outlook is hidden.
pub(super) enum Hide {
    /// `display:none;` written into its style, for any program to read.
    Style,
    /// Marked ([`HIDDEN_MARK`]), for the text converters, which read no style for the text.
    Mark,
}

/// `html` with each element it hides from Outlook marked ([`HIDDEN_MARK`]), as it came when it
/// hides none, and `None` when it has the mark already: the sender's, which would hide what it
/// marks unseen (nothing is hidden in place then; the hidden text is read from the message).
pub(crate) fn marked_hidden(html: &str) -> Option<Cow<'_, str>> {
    if html.to_ascii_lowercase().contains(HIDDEN_MARK) {
        return None;
    }
    Some(hidden_in_place(html, Hide::Mark))
}

/// `html` with what it hides from Outlook hidden `how`; the same HTML when it hides nothing.
pub(super) fn hidden_in_place(html: &str, how: Hide) -> Cow<'_, str> {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("mso-hide") {
        return Cow::Borrowed(html);
    }
    let b = lower.as_bytes();
    let mut comments = Comments::new(&lower);
    let mut out = String::new();
    // Up to where `html` is in `out`.
    let mut copied = 0;
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|rest| rest.find('<')) {
        let at = i + off;
        let past = |from: usize| lower.get(from..).and_then(|r| r.find('>')).map_or(lower.len(), |g| from + g + 1);
        match b.get(at + 1) {
            // A comment, or a declaration or processing instruction (read as one to its `>`).
            Some(b'!') if lower.get(at..).is_some_and(|r| r.starts_with("<!--")) => {
                i = comments.end(at + 4).unwrap_or(lower.len());
                continue;
            }
            Some(b'!' | b'?') => {
                i = past(at + 2);
                continue;
            }
            Some(b'/') => {
                i = match b.get(at + 2) {
                    Some(c) if c.is_ascii_alphabetic() => tag_end_at(b, at + 2).map_or(lower.len(), |g| g + 1),
                    _ => past(at + 2),
                };
                continue;
            }
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => {
                i = at + 1;
                continue;
            }
        }
        // A start tag: its name up to a space, `/` or `>`, as a browser reads it.
        let name_end =
            (at + 1..lower.len()).find(|&n| b.get(n).is_some_and(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>')).unwrap_or(lower.len());
        // One not ended is no tag: a browser drops it, and all after it with it.
        let Some(end) = tag_end_at(b, name_end).map(|g| g + 1) else { break };
        let name = lower.get(at + 1..name_end).unwrap_or("");
        if lower.get(at..end).is_some_and(|t| t.contains("mso-hide"))
            && let Some(value) = style_at(html, name_end, end)
            && mso_hidden(&decoded(html.get(value.clone()).unwrap_or("")))
        {
            let (to, what) = match how {
                Hide::Style => (value.start, "display:none;".to_string()),
                Hide::Mark => (name_end, format!(" {HIDDEN_MARK}")),
            };
            out.push_str(html.get(copied..to).unwrap_or(""));
            out.push_str(&what);
            copied = to;
        }
        i = end;
        // Their text is no markup: nothing in it is hidden (found in review: a text box showed
        // the tag written into). Nothing ends `<plaintext>`.
        if name == "plaintext" {
            break;
        }
        if RAW_TEXT.contains(&name) {
            i = raw_text_end(&lower, i, name).unwrap_or(lower.len());
        }
    }
    if copied == 0 {
        return Cow::Borrowed(html);
    }
    out.push_str(html.get(copied..).unwrap_or(""));
    Cow::Owned(out)
}

/// Where the value of the first `style` among the attributes of a tag from `from` to `end` (past
/// its `>`) is, unquoted, read as a browser reads them (the first of a name counts).
fn style_at(html: &str, from: usize, end: usize) -> Option<std::ops::Range<usize>> {
    let b = html.as_bytes();
    let stop = |c: u8| c.is_ascii_whitespace() || c == b'/' || c == b'>';
    let mut j = from;
    while j < end {
        // Past spaces and `/` to a name, which runs to a space, `/`, `>` or `=` (one at its start
        // is its own).
        while j < end && b.get(j).is_some_and(|c| stop(*c) && *c != b'>') {
            j += 1;
        }
        if j >= end || b.get(j) == Some(&b'>') {
            return None;
        }
        let name_at = j;
        j += 1;
        while j < end && b.get(j).is_some_and(|c| !stop(*c) && *c != b'=') {
            j += 1;
        }
        let is_style = html.get(name_at..j).is_some_and(|n| n.eq_ignore_ascii_case("style"));
        let mut k = j;
        while k < end && b.get(k).is_some_and(u8::is_ascii_whitespace) {
            k += 1;
        }
        if b.get(k) != Some(&b'=') {
            // No value: the next name, or the end.
            if is_style {
                return None;
            }
            continue;
        }
        k += 1;
        while k < end && b.get(k).is_some_and(u8::is_ascii_whitespace) {
            k += 1;
        }
        let value = match b.get(k) {
            Some(&q @ (b'"' | b'\'')) => {
                let close = (k + 1..end).find(|&n| b.get(n) == Some(&q))?;
                j = close + 1;
                k + 1..close
            }
            _ => {
                let close = (k..end).find(|&n| b.get(n).is_some_and(|c| c.is_ascii_whitespace() || *c == b'>')).unwrap_or(end);
                j = close;
                k..close
            }
        };
        if is_style {
            return Some(value);
        }
    }
    None
}
