//! Raised and lowered text (`<sup>`, `<sub>`): marked for the rich text, which sets it above and
//! below its line, and written plainly in plain text.

use super::*;

/// Where raised and lowered text start, and where they end ([`mark_scripts`]).
const SUP_MARK: char = '\u{E002}';
const SUB_MARK: char = '\u{E003}';
const SCRIPT_END: char = '\u{E004}';

/// `html` with `<sup>` and `<sub>` made spans, a mark at each end of what's in them, and any mark
/// already in it removed: the text converter writes `^{$}`, and digits alone in characters of
/// their own (issue #47: a price's `$` and cents), which were shown as they came.
pub(crate) fn mark_scripts(html: &str) -> String {
    let html = html.replace([SUP_MARK, SUB_MARK, SCRIPT_END], "");
    let lower = html.to_ascii_lowercase();
    if !lower.contains("<sup") && !lower.contains("<sub") {
        return html;
    }
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find('<')) {
        let start = from + i;
        let Some((closing, mark)) = script_tag(lower.get(start..).unwrap_or("")) else {
            from = start + 1;
            continue;
        };
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        out.push_str(html.get(last..start).unwrap_or(""));
        if closing {
            out.push(SCRIPT_END);
            out.push_str("</span>");
        } else {
            out.push_str("<span");
            out.push_str(html.get(start + 4..=end).unwrap_or(">"));
            out.push(mark);
        }
        last = end + 1;
        from = end + 1;
    }
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// Whether `lower` starts with a `<sup>` or `<sub>` tag: a closing one, and the mark for what's in it.
fn script_tag(lower: &str) -> Option<(bool, char)> {
    let (closing, rest) = match lower.strip_prefix("</") {
        Some(r) => (true, r),
        None => (false, lower.strip_prefix('<')?),
    };
    let mark = match rest.get(..3)? {
        "sup" => SUP_MARK,
        "sub" => SUB_MARK,
        _ => return None,
    };
    rest.as_bytes().get(3).is_some_and(|b| *b == b'>' || *b == b'/' || b.is_ascii_whitespace()).then_some((closing, mark))
}

/// The spans of a line split where what's raised or lowered starts and ends ([`mark_scripts`]),
/// without the marks. What's left open ends with its line.
pub(super) fn scripts_split(spans: Vec<Span>) -> Vec<Span> {
    if !spans.iter().any(|s| s.text.contains([SUP_MARK, SUB_MARK, SCRIPT_END])) {
        return spans;
    }
    let mut out: Vec<Span> = Vec::with_capacity(spans.len() + 4);
    let mut now = Rise::Normal;
    for sp in spans {
        let mut part = String::new();
        for c in sp.text.chars() {
            let next = match c {
                SUP_MARK => Rise::Sup,
                SUB_MARK => Rise::Sub,
                SCRIPT_END => Rise::Normal,
                c => {
                    part.push(c);
                    continue;
                }
            };
            if !part.is_empty() {
                out.push(Span { text: std::mem::take(&mut part), rise: now, ..sp.clone() });
            }
            now = next;
        }
        if !part.is_empty() || sp.image.is_some() {
            out.push(Span { text: part, rise: now, ..sp });
        }
    }
    out
}

/// `html` for plain text: a `<sup>` or `<sub>` written as what's in it, unless that's digits,
/// which the text converter writes as raised digits (issue #47: `^{$}15⁹⁸` in a price).
pub(crate) fn plain_scripts(html: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let lower = html.to_ascii_lowercase();
    if !lower.contains("<sup") {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<sup")) {
        let start = from + i;
        let Some((false, _)) = script_tag(lower.get(start..).unwrap_or("")) else {
            from = start + 4;
            continue;
        };
        let Some(open_end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        let Some(close) = lower.get(open_end..).and_then(|r| r.find("</sup")).map(|e| open_end + e) else { break };
        let Some(close_end) = lower.get(close..).and_then(|r| r.find('>')).map(|e| close + e) else { break };
        let inner = html.get(open_end + 1..close).unwrap_or("").trim();
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(html.get(last..=close_end).unwrap_or(""));
        } else {
            out.push_str(html.get(last..start).unwrap_or(""));
            out.push_str(html.get(open_end + 1..close).unwrap_or(""));
        }
        last = close_end + 1;
        from = close_end + 1;
    }
    out.push_str(html.get(last..).unwrap_or(""));
    Cow::Owned(out)
}

#[cfg(test)]
mod tests;
