//! A page's own background left to the page: not a highlight behind each run of its text.

use mailcraft_model::work::scanned;

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
    while let Some(off) = lower.get(from..).and_then(|r| scanned(r, r.find('<'))) {
        let start = from + off;
        let rest = lower.get(start..).unwrap_or("");
        let page =
            ["<body", "<html"].iter().any(|t| rest.starts_with(t) && rest.as_bytes().get(5).is_none_or(|c| c.is_ascii_whitespace() || *c == b'>'));
        if rest.starts_with("<style") {
            let Some(open) = crate::html::tag_end(rest.as_bytes(), 1) else { break };
            let body = start + open + 1;
            let end = lower.get(body..).and_then(|r| scanned(r, r.find("</style"))).map_or(html.len(), |e| body + e);
            out.push_str(html.get(at..body).unwrap_or(""));
            out.push_str(&page_rules_without_background(html.get(body..end).unwrap_or("")));
            at = end;
            from = end;
        } else if page {
            let Some(close) = crate::html::tag_end(rest.as_bytes(), 1) else { break };
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
    mailcraft_model::work::steps(css.len());
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

/// A `<body>` or `<html>` tag without its `bgcolor` and its style's backgrounds: its attributes
/// read as every other pass reads them ([`crate::html::attributes`]), quoted or not. Each was
/// looked for by its name anywhere in the tag, and `data-bgcolor` or a `style=` in another's
/// value was cut instead.
fn without_background_attrs(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    let mut last = 0;
    for a in crate::html::attributes(tag).1 {
        let (cut, kept) = match a.name.as_str() {
            "bgcolor" => (a.span, String::new()),
            "style" => {
                let decls = tag.get(a.value.clone()).unwrap_or("");
                let kept: Vec<&str> = decls.split(';').filter(|d| !d.trim().to_ascii_lowercase().starts_with("background")).collect();
                (a.value, kept.join(";"))
            }
            _ => continue,
        };
        let Some(before) = tag.get(last..cut.start) else { continue };
        out.push_str(before);
        out.push_str(&kept);
        last = cut.end;
    }
    out.push_str(tag.get(last..).unwrap_or(""));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page's `bgcolor` and its style's backgrounds go, read as attributes: not a name inside
    /// another (`data-bgcolor`) or in another's value, and an unquoted style too.
    #[test]
    fn the_pages_own_background_goes_and_nothing_else() {
        let html = r##"<body data-bgcolor="#123" title="style=background:red" bgcolor="#eee" style="color:#111; background:#fff"><p>Hi</p>"##;
        assert_eq!(no_page_background(html), r##"<body data-bgcolor="#123" title="style=background:red"  style="color:#111"><p>Hi</p>"##);
        assert_eq!(no_page_background("<BODY BGCOLOR=#eee STYLE=background:#fff;color:red>x"), "<BODY  STYLE=color:red>x");
        assert_eq!(no_page_background("<html bgcolor='x'><body>x"), "<html ><body>x");
        // A box keeps its own.
        assert_eq!(no_page_background(r##"<td bgcolor="#eee">x"##), r##"<td bgcolor="#eee">x"##);
        for junk in ["<body bgcolor", "<body style=\"background", "<body bgcolor= >", "<body \"style=x\" é=é bgcolor=é>"] {
            let _ = no_page_background(junk);
        }
    }
}
