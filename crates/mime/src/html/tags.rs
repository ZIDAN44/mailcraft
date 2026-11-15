//! Reading a tag as a browser does, for every pass over a message's HTML: where it ends, and
//! which elements have no content. Each pass found a tag's end its own way (at the first `>`,
//! or past any quote), so the same tag ended at different places in different passes.

/// Where the tag whose name is at `from` in `b` (or has just ended there) ends (the index of
/// its `>`), as a browser reads it: past values quoted after their name's `=` (a `>` in one
/// doesn't end the tag), a quote anywhere else being part of a name or value (`<p title=it's>`),
/// and an `=` that follows no name being part of one (`<p=">` ends at its `>`: read as a value's
/// start, it hid what followed, a script among it, inside the tag). `None` when it doesn't end.
pub(crate) fn tag_end(b: &[u8], from: usize) -> Option<usize> {
    let end = end_of_tag(b, from);
    mailcraft_model::work::steps(end.map_or(b.len(), |e| e.saturating_add(1)).saturating_sub(from));
    end
}

/// [`tag_end`], uncounted.
fn end_of_tag(b: &[u8], from: usize) -> Option<usize> {
    read(b, from, |_| {}).1
}

/// An attribute as [`read`] finds it: where it starts, where its name ends, where its value is
/// (without its quotes), whether that was quoted, and where the whole of it ends.
struct Found {
    start: usize,
    name_end: usize,
    value: std::ops::Range<usize>,
    quoted: bool,
    end: usize,
}

/// The one reading of a tag, for where it ends and for its attributes, so the two can't differ:
/// the HTML standard's tokenizer from the tag's name on. `from` is in the name or just past it;
/// each attribute goes to `each`. Gives where the name ends and where the tag does (its `>`).
///
/// The name runs to a space, `/` or `>`. Attributes are separated by spaces or `/`, or by nothing
/// after a quoted value; a name runs to a space, `/`, `>` or `=`, an `=` first being part of it;
/// only after a name's `=` is there a value: quoted, or up to a space or `>`. What starts `<!` or
/// `<?`, and a `</` no letter follows, is no tag to a browser and ends at its first `>`.
fn read(b: &[u8], from: usize, mut each: impl FnMut(Found)) -> (usize, Option<usize>) {
    let at = |i: usize| b.get(i).copied();
    let space = |i: usize| b.get(i).is_some_and(u8::is_ascii_whitespace);
    let first_gt = |i: usize| b.get(i..).and_then(|r| r.iter().position(|c| *c == b'>')).map(|e| i + e);
    let mut i = from;
    if from.checked_sub(1).and_then(at) == Some(b'<') {
        if matches!(at(i), Some(b'!' | b'?')) {
            return (i, first_gt(i));
        }
        if at(i) == Some(b'/') {
            i += 1;
            if !at(i).is_some_and(|c| c.is_ascii_alphabetic()) {
                return (i, first_gt(i));
            }
        }
    }
    while at(i).is_some_and(|c| !c.is_ascii_whitespace() && c != b'/' && c != b'>') {
        i += 1;
    }
    let name_end = i;
    loop {
        while space(i) || at(i) == Some(b'/') {
            i += 1;
        }
        match at(i) {
            None => return (name_end, None),
            Some(b'>') => return (name_end, Some(i)),
            Some(_) => {}
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
                    let k = b.get(j + 1..).and_then(|r| r.iter().position(|c| *c == q)).map_or(b.len(), |e| j + 1 + e);
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
        each(Found { start, name_end: name_end_here, value, quoted, end: i });
    }
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
    let mut out = Vec::new();
    let (name_end, _) = read(tag.as_bytes(), 1, |f| {
        let name = tag.get(f.start..f.name_end).unwrap_or("").to_ascii_lowercase();
        out.push(Attribute { name, value: f.value, quoted: f.quoted, span: f.start..f.end, name_end: f.name_end });
    });
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
            // An `=` no name comes before is part of a name, and no value's start.
            ("<p=\"> <script>", Some(4)),
            ("<p =\"x> <script> y=\">", Some(6)),
            ("<p a=\"b\"=\"c> <script> d=\">", Some(11)),
            ("<p a = \"b>c\">", Some(12)),
            ("<p a=>b", Some(5)),
            ("<p/=\">\">", Some(5)),
            ("</p a=\">\">", Some(9)),
            // No tags to a browser, which ends them at their first `>`.
            ("<!x a=\">\">", Some(7)),
            ("<?x a=\">\">", Some(7)),
            ("</ a=\">\">", Some(6)),
        ] {
            assert_eq!(tag_end(tag.as_bytes(), 1), end, "{tag}");
        }
    }

    /// Where a tag (`<p` and `rest`) ends, by the HTML standard's tokenizer written out state
    /// by state.
    fn browser_end(rest: &[u8]) -> Option<usize> {
        enum State {
            TagName,
            BeforeName,
            Name,
            AfterName,
            BeforeValue,
            Quoted(u8),
            Unquoted,
            AfterQuoted,
        }
        use State::*;
        let mut state = TagName;
        let mut i = 0;
        while let Some(&c) = rest.get(i) {
            let space = c.is_ascii_whitespace();
            // Whether the character is taken (not read again in the next state).
            let mut taken = true;
            state = match state {
                TagName if space || c == b'/' => BeforeName,
                TagName if c == b'>' => return Some(i + 2),
                TagName => TagName,
                BeforeName if space || c == b'/' => BeforeName,
                BeforeName if c == b'>' => return Some(i + 2),
                BeforeName => Name,
                Name if space || c == b'/' || c == b'>' => {
                    taken = false;
                    AfterName
                }
                Name if c == b'=' => BeforeValue,
                Name => Name,
                AfterName if space => AfterName,
                AfterName if c == b'/' => BeforeName,
                AfterName if c == b'=' => BeforeValue,
                AfterName if c == b'>' => return Some(i + 2),
                AfterName => Name,
                BeforeValue if space => BeforeValue,
                BeforeValue if c == b'"' || c == b'\'' => Quoted(c),
                BeforeValue if c == b'>' => return Some(i + 2),
                BeforeValue => Unquoted,
                Quoted(q) if c == q => AfterQuoted,
                Quoted(q) => Quoted(q),
                Unquoted if space => BeforeName,
                Unquoted if c == b'>' => return Some(i + 2),
                Unquoted => Unquoted,
                AfterQuoted => {
                    taken = false;
                    BeforeName
                }
            };
            i += usize::from(taken);
        }
        None
    }

    /// Every tag of up to seven characters that names, values, quotes and `>` are made of ends
    /// where a browser ends it, and its attributes are read to the same place: no `>` before
    /// its end is outside a quoted value, and none of its attributes is past it.
    #[test]
    fn every_small_tag_ends_where_a_browser_ends_it() {
        const LETTERS: &[u8] = b"a=\"' />";
        let mut rest = Vec::new();
        for len in 0..=7u32 {
            for n in 0..LETTERS.len().pow(len) {
                rest.clear();
                let mut m = n;
                for _ in 0..len {
                    rest.push(LETTERS[m % LETTERS.len()]);
                    m /= LETTERS.len();
                }
                let tag = format!("<p{}", String::from_utf8_lossy(&rest));
                let end = end_of_tag(tag.as_bytes(), 1);
                assert_eq!(end, browser_end(&rest), "{tag}");
                // From just past its name too, as some passes ask.
                assert_eq!(end_of_tag(tag.as_bytes(), 2), end, "{tag}");
                let attrs = attributes(tag.get(..end.unwrap_or(tag.len())).unwrap_or("")).1;
                assert_eq!(attrs.len(), attributes(&tag).1.len(), "{tag}");
                for (at, _) in tag.bytes().enumerate().take(end.unwrap_or(0)).filter(|(_, c)| *c == b'>') {
                    assert!(attrs.iter().any(|a| a.quoted && a.value.contains(&at)), "{tag}: a `>` at {at} before its end");
                }
                assert!(attrs.iter().all(|a| a.span.end <= end.unwrap_or(tag.len())), "{tag}");
            }
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

    /// A picture's address is read whole, from its `src` wherever the tag writes it: without
    /// quotes (cut at its first slash, or keeping a `/>`'s slash, it was another address), and
    /// as the other passes read it (the rich text, its sizes): after a tab, with spaces around
    /// its `=`, not inside another attribute's quoted value. Missed here, a picture the text
    /// shows wasn't asked about.
    #[test]
    fn a_pictures_address_is_read_whole_wherever_its_src_is() {
        assert_eq!(
            crate::html_images("<img src=https://x.example/a/b.png width=1><img src=cid:logo@x /><img src=https://x.example/c.png/>"),
            ["https://x.example/a/b.png", "cid:logo@x", "https://x.example/c.png"]
        );
        assert_eq!(
            crate::html_images(
                "<img\tsrc=\"https://x.example/t.png\"><img src = \"https://x.example/s.png\"><img alt=\" src=x\" src=\"https://x.example/r.png\">"
            ),
            ["https://x.example/t.png", "https://x.example/s.png", "https://x.example/r.png"]
        );
    }
}
