//! Reading a tag as a browser does, for every pass over a message's HTML: where it ends, and
//! which elements have no content. Each pass found a tag's end its own way (at the first `>`,
//! or past any quote), so the same tag ended at different places in different passes.

/// Where the tag whose attributes start at `from` in `b` ends (the index of its `>`), as a
/// browser reads it: past values quoted after their `=` (a `>` in one doesn't end the tag), a
/// quote anywhere else being part of a name or value (`<p title=it's>`). `None` when it doesn't
/// end.
pub(crate) fn tag_end(b: &[u8], from: usize) -> Option<usize> {
    let mut j = from;
    // Just past an attribute's `=`, its value still to come.
    let mut value = false;
    while let Some(&c) = b.get(j) {
        match c {
            b'>' => return Some(j),
            b'=' if !value => value = true,
            b'"' | b'\'' if value => {
                j = j + 1 + b.get(j + 1..)?.iter().position(|x| *x == c)?;
                value = false;
            }
            c if c.is_ascii_whitespace() => {}
            _ if value => {
                while b.get(j + 1).is_some_and(|d| !d.is_ascii_whitespace() && *d != b'>') {
                    j += 1;
                }
                value = false;
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Elements that never have content (no end tag), by their lower-cased name.
pub(crate) fn void(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr")
}

/// An attribute of a tag as a browser reads it: its name (lower-cased), where its value is in
/// the tag (without its quotes) and whether it was quoted, where the whole of it is (its name to
/// its value's end) and where its name ends.
pub(crate) struct Attribute {
    pub(crate) name: String,
    pub(crate) value: std::ops::Range<usize>,
    pub(crate) quoted: bool,
    pub(crate) span: std::ops::Range<usize>,
    pub(crate) name_end: usize,
}

/// Where the tag's name ends, and its attributes, read as a browser reads them (the HTML
/// standard's tokenizer): separated by spaces or `/`, or by nothing after a quoted value; a value
/// quoted, or up to a space or `>` (a `/` in it is part of it). A name in another's quoted value
/// (`alt=" src=x"`) isn't one.
pub(crate) fn attributes(tag: &str) -> (usize, Vec<Attribute>) {
    let b = tag.as_bytes();
    let space = |i: usize| b.get(i).is_some_and(u8::is_ascii_whitespace);
    let at = |i: usize| b.get(i).copied();
    let mut i = 1;
    while at(i).is_some_and(|c| !c.is_ascii_whitespace() && c != b'/' && c != b'>') {
        i += 1;
    }
    let name_end = i;
    let mut out = Vec::new();
    loop {
        while space(i) || at(i) == Some(b'/') {
            i += 1;
        }
        if at(i).is_none_or(|c| c == b'>') {
            break;
        }
        let start = i;
        // A name: an `=` first is part of it.
        i += 1;
        while at(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'/' | b'>' | b'=')) {
            i += 1;
        }
        let name_end_here = i;
        let mut j = i;
        while space(j) {
            j += 1;
        }
        let (mut value, mut quoted) = (i..i, false);
        if at(j) == Some(b'=') {
            j += 1;
            while space(j) {
                j += 1;
            }
            match at(j) {
                Some(q @ (b'"' | b'\'')) => {
                    let mut k = j + 1;
                    while at(k).is_some_and(|c| c != q) {
                        k += 1;
                    }
                    (value, quoted) = (j + 1..k, true);
                    i = (k + 1).min(b.len());
                }
                _ => {
                    let mut k = j;
                    while at(k).is_some_and(|c| !c.is_ascii_whitespace() && c != b'>') {
                        k += 1;
                    }
                    value = j..k;
                    i = k;
                }
            }
        }
        let name = tag.get(start..name_end_here).unwrap_or("").to_ascii_lowercase();
        out.push(Attribute { name, value, quoted, span: start..i, name_end: name_end_here });
    }
    (name_end, out)
}

/// The value of a tag's attribute `name` (lower-case), as written: the first of that name, as a
/// browser takes it; `""` for one with no value.
pub(crate) fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    attributes(tag).1.into_iter().find(|a| a.name == name).and_then(|a| tag.get(a.value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_ends_where_a_browser_ends_it() {
        for (tag, end) in [
            ("<p>", Some(2)),
            ("<img alt=\"a>b\" src=x>", Some(20)),
            ("<img alt='a>b'>", Some(14)),
            ("<p title=it's>x", Some(13)),
            ("<p a=b\"c>", Some(8)),
            ("<p \"x>", Some(5)),
            ("<img alt=\"never closed>", None),
            ("<p", None),
        ] {
            assert_eq!(tag_end(tag.as_bytes(), 1), end, "{tag}");
        }
    }

    /// The passes over a message's HTML end a tag where a browser does: a picture whose `alt`
    /// holds a `>` was cut there, and its address never seen (nor asked before it's loaded).
    #[test]
    fn a_picture_with_a_quoted_gt_is_found() {
        let html = "<p>Hi</p><img alt=\"5 > 4\" src=\"https://pictures.example/a.png\"><img src=\"https://pictures.example/b.png\">";
        assert_eq!(crate::html::html_images(html), ["https://pictures.example/a.png", "https://pictures.example/b.png"]);
        // Nor missed as a tracking picture where it is one.
        let tracker = "<img alt=\"a>b\" src=\"https://t.mailtrack.example/open?id=1\" width=\"1\" height=\"1\">";
        assert_eq!(crate::trackers::trackers(tracker).pictures.len(), 1);
    }
}
