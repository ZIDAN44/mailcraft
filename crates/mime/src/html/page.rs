//! A page's own background left to the page: not a highlight behind each run of its text.

/// `html` without its page's own background (`<body bgcolor>`, a `body { background }` rule): a
/// page's colour is the page's, behind everything, not a highlight behind each run of its text,
/// which is how it came out (a newsletter's light grey behind every word of its white columns,
/// issue #47). The boxes on the page keep theirs.
pub(super) fn no_page_background(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("background") && !lower.contains("bgcolor") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    let mut from = 0;
    while let Some(off) = lower.get(from..).and_then(|r| r.find('<')) {
        let start = from + off;
        let rest = lower.get(start..).unwrap_or("");
        let page =
            ["<body", "<html"].iter().any(|t| rest.starts_with(t) && rest.as_bytes().get(5).is_none_or(|c| c.is_ascii_whitespace() || *c == b'>'));
        if rest.starts_with("<style") {
            let Some(open) = rest.find('>') else { break };
            let body = start + open + 1;
            let end = lower.get(body..).and_then(|r| r.find("</style")).map_or(html.len(), |e| body + e);
            out.push_str(html.get(at..body).unwrap_or(""));
            out.push_str(&page_rules_without_background(html.get(body..end).unwrap_or("")));
            at = end;
            from = end;
        } else if page {
            let Some(close) = rest.find('>') else { break };
            out.push_str(html.get(at..start).unwrap_or(""));
            out.push_str(&without_background_attrs(html.get(start..=start + close).unwrap_or("")));
            at = start + close + 1;
            from = at;
        } else {
            from = start + 1;
        }
    }
    out.push_str(html.get(at..).unwrap_or(""));
    out
}

/// A `<style>` block's `body` and `html` rules without their backgrounds; the rest as it was. One
/// pass, its blocks (`@media` around rules) followed by their nesting: looking for each block's
/// end from each `{` took as long as the sheet for each of thousands.
fn page_rules_without_background(css: &str) -> String {
    // Each open block: where it opens, where its selector starts, and whether it holds blocks.
    let mut open: Vec<(usize, usize, bool)> = Vec::new();
    // The blocks of page rules, their declarations without backgrounds.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut boundary = 0;
    for (i, c) in css.char_indices() {
        match c {
            '{' => {
                if let Some(parent) = open.last_mut() {
                    parent.2 = true;
                }
                open.push((i, boundary, false));
                boundary = i + 1;
            }
            '}' => {
                if let Some((at, from, holds)) = open.pop()
                    && !holds
                {
                    let selector = css.get(from..at).unwrap_or("");
                    let page =
                        !selector.trim().is_empty() && selector.split(',').all(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "body" | "html"));
                    if page {
                        let block = css.get(at + 1..i).unwrap_or("");
                        let kept: Vec<&str> = block.split(';').filter(|d| !d.trim().to_ascii_lowercase().starts_with("background")).collect();
                        edits.push((at + 1, i, kept.join(";")));
                    }
                }
                boundary = i + 1;
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(css.len());
    let mut last = 0;
    for (from, to, kept) in edits {
        out.push_str(css.get(last..from).unwrap_or(""));
        out.push_str(&kept);
        last = to;
    }
    out.push_str(css.get(last..).unwrap_or(""));
    out
}

/// A `<body>` or `<html>` tag without its `bgcolor` and its style's backgrounds.
fn without_background_attrs(tag: &str) -> String {
    let lower = tag.to_ascii_lowercase();
    let mut out = tag.to_string();
    // `bgcolor="#eef1f4"`, quoted or not.
    if let Some(at) = lower.find("bgcolor") {
        let value = lower.get(at + 7..).unwrap_or("").trim_start();
        let skip = lower.len() - value.len();
        let end = match value.strip_prefix('=').map(str::trim_start) {
            Some(v) => {
                let start = lower.len() - v.len();
                match v.chars().next() {
                    Some(q @ ('"' | '\'')) => v.get(1..).and_then(|r| r.find(q)).map_or(lower.len(), |e| start + e + 2),
                    _ => v.find(|c: char| c.is_ascii_whitespace() || c == '>').map_or(lower.len(), |e| start + e),
                }
            }
            None => skip,
        };
        if out.is_char_boundary(at) && out.is_char_boundary(end) && at <= end {
            out.replace_range(at..end, "");
        }
    }
    // `style="…; background: …"`.
    let lower = out.to_ascii_lowercase();
    if let Some(s) = lower.find("style=") {
        let quote = lower.as_bytes().get(s + 6).copied();
        if let Some(q @ (b'"' | b'\'')) = quote
            && let Some(len) = lower.get(s + 7..).and_then(|r| r.find(char::from(q)))
        {
            let decls = out.get(s + 7..s + 7 + len).unwrap_or("");
            let kept: Vec<&str> = decls.split(';').filter(|d| !d.trim().to_ascii_lowercase().starts_with("background")).collect();
            let kept = kept.join(";");
            out.replace_range(s + 7..s + 7 + len, &kept);
        }
    }
    out
}
