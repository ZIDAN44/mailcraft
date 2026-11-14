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
