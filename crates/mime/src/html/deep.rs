//! HTML nested deeper than any mail is (tables left open one in another by the thousand, say):
//! read as its words and links, flat, rather than through the text converter and the layout,
//! whose time grows with each table nested (a megabyte took over ten minutes). Mail as written,
//! newsletters with a dozen tables one in another included, is nowhere near the limits.

/// Tables open one in another past which HTML is read flat.
const MAX_TABLES: usize = 48;
/// Boxes (`div`, `blockquote`) open one in another past which HTML is read flat.
const MAX_BOXES: usize = 2048;

/// The tags of `html` in order, outside comments and the text of `script`, `style`, `textarea`
/// and `title`, each as (its name, lowercase; whether it ends an element; where it is): the one
/// pass both [`too_deep`] and [`flattened`] make, linear in `html`.
fn tags(html: &str, mut each: impl FnMut(&str, bool, std::ops::Range<usize>) -> bool) {
    let bytes = html.as_bytes();
    let mut at = 0;
    let mut raw: Option<&'static str> = None;
    while let Some(lt) = bytes.get(at..).and_then(|b| b.iter().position(|&c| c == b'<')).map(|p| p + at) {
        if bytes.get(lt..lt + 4) == Some(b"<!--") {
            at = html.get(lt + 4..).and_then(|s| s.find("-->")).map_or(bytes.len(), |e| lt + 4 + e + 3);
            continue;
        }
        let end = bytes.get(lt + 1).is_some_and(|&c| c == b'/');
        let start = lt + 1 + usize::from(end);
        let name_len = bytes.get(start..).map_or(0, |b| b.iter().take_while(|c| c.is_ascii_alphanumeric()).count());
        // Not a tag (`<` and no name), or, inside a raw text element, not its own end tag: on
        // from the next byte, before its end is looked for. Looked for first, a megabyte of
        // `<<<<` was read from each `<` to the end of the message, and took minutes.
        let name = html.get(start..start + name_len).unwrap_or_default().to_ascii_lowercase();
        if name_len == 0 || raw.is_some_and(|r| !(end && name == r)) {
            at = lt + 1;
            continue;
        }
        // The tag's end, past quoted values (a `>` in one doesn't end it).
        let mut close = start + name_len;
        let mut quote = None;
        while let Some(&c) = bytes.get(close) {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (None, b'"' | b'\'') => quote = Some(c),
                (None, b'>') => break,
                _ => {}
            }
            close += 1;
        }
        let next = (close + 1).min(bytes.len());
        if raw.is_some() {
            // Inside a raw text element only its own end tag counts: this is it.
            raw = None;
        } else if !end {
            raw = ["script", "style", "textarea", "title"].into_iter().find(|r| *r == name);
        }
        if !each(&name, end, lt..next) {
            return;
        }
        at = next;
    }
}

/// Whether `html` nests tables, or boxes, past what any mail does.
pub(crate) fn too_deep(html: &str) -> bool {
    // Too few to nest that deep: looked at no further.
    let lower = html.as_bytes();
    let count = |needle: &[u8]| lower.windows(needle.len()).filter(|w| w.eq_ignore_ascii_case(needle)).count();
    if count(b"<table") <= MAX_TABLES && count(b"<div") + count(b"<blockquote") <= MAX_BOXES {
        return false;
    }
    let (mut tables, mut boxes, mut deep) = (0usize, 0usize, false);
    tags(html, |name, end, _| {
        let depth = match name {
            "table" => &mut tables,
            "div" | "blockquote" => &mut boxes,
            _ => return true,
        };
        *depth = if end { depth.saturating_sub(1) } else { depth.saturating_add(1) };
        deep = tables > MAX_TABLES || boxes > MAX_BOXES;
        !deep
    });
    deep
}

/// `html` with nothing nested: each block's tags a line break, links, pictures and what marks
/// words (bold, italic…) as they were, everything else left out but its text.
pub(crate) fn flattened(html: &str) -> String {
    const KEPT: &[&str] = &["a", "b", "strong", "i", "em", "u", "s", "strike", "code", "img", "br", "head", "style", "script", "title"];
    const BLOCKS: &[&str] =
        &["p", "div", "table", "tr", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "hr", "dl", "dt", "dd", "center"];
    let mut out = String::with_capacity(html.len() / 2);
    let mut at = 0;
    tags(html, |name, _, range| {
        out.push_str(html.get(at..range.start).unwrap_or_default());
        if KEPT.contains(&name) {
            out.push_str(html.get(range.clone()).unwrap_or_default());
        } else if BLOCKS.contains(&name) {
            out.push_str("<br>");
        } else if matches!(name, "td" | "th") {
            out.push(' ');
        }
        at = range.end;
        true
    });
    out.push_str(html.get(at..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Found in review: tables left open one in another (a cell, a box, a link, again and
    /// again) took seconds to read at 64 KB, and over ten minutes at a megabyte, in the text
    /// converter and the layout. Read flat, they take no time, their words and links kept; mail
    /// that nests as mail does is read as before.
    #[test]
    fn tables_left_open_by_the_thousand_are_read_flat_and_quick() {
        let unit = r#"<table><tr><td><div><a href="https://x.example/a">link</a>"#;
        let html = unit.repeat(1_000_000 / unit.len());
        assert!(too_deep(&html));
        let started = std::time::Instant::now();
        let p = crate::parse(format!("Content-Type: text/html\r\n\r\n{html}").as_bytes());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        let p = p.unwrap();
        assert!(p.text.contains("link") && p.text.contains("https://x.example/a"), "{:?}", &p.text[..p.text.len().min(200)]);
        assert!(p.rich.iter().flatten().any(|s| s.link.as_deref() == Some("https://x.example/a")));
        // As mail nests: as before.
        let nested = format!("{}<p>Deep</p>{}", "<table><tr><td>".repeat(MAX_TABLES), "</td></tr></table>".repeat(MAX_TABLES));
        assert!(!too_deep(&nested));
        assert!(!too_deep(&"<div>".repeat(MAX_BOXES)));
        assert!(too_deep(&"<div>".repeat(MAX_BOXES + 1)));
        // Tags in a comment, a style sheet or a script, or a `>` in a value, nest nothing.
        let quiet = format!("<!--{}--><style>{}</style><a title=\"{}\">x</a>", "<table>".repeat(100), "<table>".repeat(100), "<table>".repeat(100));
        assert!(!too_deep(&quiet));
    }

    /// Found in review: `<` with no name after it, and `<a` inside a `style` left open, were
    /// each read to the end of the message, a megabyte of them for minutes.
    #[test]
    fn a_megabyte_of_stray_angle_brackets_is_read_once() {
        let tables = "<table></table>".repeat(MAX_TABLES + 1);
        let started = std::time::Instant::now();
        for junk in ["<".repeat(1 << 20), "< ".repeat(1 << 19), format!("<style>{}", "<a ".repeat(1 << 18)), "<\"".repeat(1 << 19)] {
            let html = format!("{tables}{junk}");
            assert!(!too_deep(&html));
            let _ = flattened(&html);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
        // What the tags are is as before.
        let mut seen = Vec::new();
        tags("<p>a < b <i>c</i><style>x<y</style><script>if (a</b>) {}</script><b>", |name, end, _| {
            seen.push(format!("{}{name}", if end { "/" } else { "" }));
            true
        });
        assert_eq!(seen, ["p", "i", "/i", "style", "/style", "script", "/script", "b"]);
    }

    #[test]
    fn flat_html_keeps_words_links_and_lines() {
        let flat =
            flattened(r#"<table><tr><td>One<td><b>Two</b></tr><tr><td><a href="h">Three</a></table><!-- <p>no --><script>if (a<b) x()</script>"#);
        assert_eq!(flat, r#"<br><br> One <b>Two</b><br><br> <a href="h">Three</a><br><!-- <p>no --><script>if (a<b) x()</script>"#);
    }
}
