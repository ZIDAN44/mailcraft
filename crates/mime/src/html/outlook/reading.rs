//! Outlook's copy read through as a browser reads it (tags and their quoted values, raw text,
//! comments): what's left open at the end of a conditional comment, closed or cut so it can't
//! hide what follows, and whether the comment shows something of its own.

use super::*;

/// Elements whose content is text up to their own closing tag, as a browser reads them: one left
/// open hides all after it.
pub(super) const RAW_TEXT: &[&str] = &["script", "style", "title", "textarea", "xmp", "iframe", "noembed", "noframes"];

/// Tables left open in a conditional comment, at most: the outermost, whose place is kept (the
/// comment's own markup has room for the marks of two). Those in them are closed at its end.
const MAX_OPEN_TABLES: usize = 4;

/// [`closed`]'s answer.
pub(super) struct Closed<'a> {
    /// The content with nothing left open at its end.
    pub(super) html: Cow<'a, str>,
    /// It opens a table.
    pub(super) tables: bool,
    /// Where in `html` the tables it leaves open start, outermost first (up to
    /// [`MAX_OPEN_TABLES`]).
    pub(super) open: Vec<usize>,
    /// It leaves elements open, which hold what follows it, or nests them deeper than
    /// [`DEEP`]: in comments one after another, they may be nested ever deeper.
    pub(super) nests: bool,
    /// How many tables left open past [`MAX_OPEN_TABLES`] are closed at its end.
    pub(super) closes: usize,
}

/// How deep a conditional comment's content nests elements and still reads as it came.
const DEEP: usize = 32;

/// What closes a table left open past [`MAX_OPEN_TABLES`] (and the rows and cells open in it).
pub(super) const TABLE_END: &str = "</table>";

/// A conditional comment's content with nothing left open at its end, so that it can't hide what
/// follows the comment. It's read through as a browser reads it (tags and their quoted values,
/// raw text, comments ended by `-->` or `--!>`): a comment, tag or quoted value not ended where
/// the copy ends is cut off there; a raw-text element (a script, style sheet, title, text box…)
/// left open is closed; a template left open is left out from its start, as nothing in it is
/// shown; and `<plaintext>`, which nothing closes, is left out. And whether it opens a table,
/// and the tables it leaves open (outside templates), which hold what follows the comment: a
/// newsletter's comment leaves one or two open, a column's ghost table; past a few, those in
/// them are closed (found in review: a comment that left thousands open, each in the last one's
/// cell, nested all that followed it two dozen tables deep, and it took seconds to read).
pub(super) fn closed(content: &str) -> Closed<'_> {
    let lower = content.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut end = content.len();
    let mut close: Option<&str> = None;
    // What's left out, in order (`<plaintext>`).
    let mut cut: Vec<std::ops::Range<usize>> = Vec::new();
    let mut tables = false;
    // Tables open: how many, and where the outermost start.
    let mut open_tables = 0usize;
    let mut open: Vec<usize> = Vec::new();
    let mut templates = 0usize;
    // Where the outermost template open starts.
    let mut template_at = None;
    // Elements open, and the most at once ([`Closed::nests`]).
    let mut depth = 0usize;
    let mut deepest = 0usize;
    let mut comments = Comments::new(&lower);
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|r| r.find('<')) {
        let at = i + off;
        let rest = lower.get(at..).unwrap_or("");
        if rest.starts_with("<!--") {
            match comments.end(at + 4) {
                Some(e) => i = e,
                None => {
                    end = at;
                    break;
                }
            }
            continue;
        }
        let closing = rest.starts_with("</");
        let name_at = at + 1 + usize::from(closing);
        let name = tag_name(&lower, name_at);
        if name.is_empty() {
            // A declaration, `<?…>` or `</` without a name: a comment up to its `>`. Else `<`
            // in text.
            if rest.starts_with("<!") || rest.starts_with("<?") || closing {
                match rest.find('>') {
                    Some(g) => i = at + g + 1,
                    None => {
                        end = at;
                        break;
                    }
                }
            } else {
                i = at + 1;
            }
            continue;
        }
        let Some(gt) = tag_end_at(b, name_at + name.len()) else {
            end = at;
            break;
        };
        i = gt + 1;
        if closing {
            depth = depth.saturating_sub(1);
            if name == "template" {
                templates = templates.saturating_sub(1);
                if templates == 0 {
                    template_at = None;
                }
            } else if name == "table" && templates == 0 && open_tables > 0 {
                open_tables -= 1;
                open.truncate(open_tables);
            }
            continue;
        }
        // An element that has content, ended by a tag of its own (a raw-text one is read to its
        // end below): counted as it's opened.
        if !crate::layout::void(name) && b.get(gt.saturating_sub(1)) != Some(&b'/') && !RAW_TEXT.contains(&name) && name != "plaintext" {
            depth = depth.saturating_add(1);
            deepest = deepest.max(depth);
        }
        match name {
            "plaintext" => cut.push(at..i),
            "table" => {
                tables = true;
                if templates == 0 {
                    open_tables = open_tables.saturating_add(1);
                    if open.len() < MAX_OPEN_TABLES {
                        open.push(at);
                    }
                }
            }
            "template" => {
                template_at = template_at.or(Some(at));
                templates = templates.saturating_add(1);
            }
            n if RAW_TEXT.contains(&n) => match raw_text_end(&lower, i, n) {
                Some(e) => i = e,
                None => {
                    close = Some(n);
                    break;
                }
            },
            _ => {}
        }
    }
    // A template left open: what's in it, to the end, is never shown (found in review: closing
    // each one took as long again as the copy, for a copy of nothing but templates).
    if let Some(t) = template_at {
        end = end.min(t);
        close = None;
    }
    // Where the tables left open start in what's kept: those cut off left out, and each moved
    // back by what's left out before it.
    open.retain(|t| *t < end);
    for t in &mut open {
        let before: usize = cut.iter().filter(|r| r.end <= *t).map(|r| r.len()).sum();
        *t = t.saturating_sub(before);
    }
    let nests = depth > 0 || deepest > DEEP;
    let closes = open_tables.saturating_sub(MAX_OPEN_TABLES);
    if end == content.len() && close.is_none() && cut.is_empty() && closes == 0 {
        return Closed { html: Cow::Borrowed(content), tables, open, nests, closes };
    }
    let mut out = String::with_capacity(end + 32);
    let mut from = 0;
    for r in cut.into_iter().filter(|r| r.end <= end) {
        out.push_str(content.get(from..r.start).unwrap_or(""));
        from = r.end;
    }
    out.push_str(content.get(from..end).unwrap_or(""));
    if let Some(name) = close {
        out.push_str(&format!("</{name}>"));
    }
    out.push_str(&TABLE_END.repeat(closes));
    Closed { html: Cow::Owned(out), tables, open, nests, closes }
}

/// A tag's name at `at` in `lower` (past `<` or `</`), as a browser reads one: a letter, then
/// anything up to a space, `/` or `>`. Empty when there's none.
fn tag_name(lower: &str, at: usize) -> &str {
    let rest = lower.get(at..).unwrap_or("");
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return "";
    }
    let len = rest.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(rest.len());
    rest.get(..len).unwrap_or("")
}

/// Where a tag whose attributes start at `from` ends (its `>`), as a browser reads it: past
/// values quoted after their `=`, a quote anywhere else being part of a name or value. `None`
/// when it doesn't end.
pub(super) fn tag_end_at(b: &[u8], from: usize) -> Option<usize> {
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

/// Where the comments in a text end, for a text read through from its start: each end is
/// looked for from where the last search found one or gave up, so that the text is looked
/// through once (found in review: `--!>` was looked for to the end of the text from each
/// comment, and a megabyte of comments took half a minute).
pub(super) struct Comments<'a> {
    lower: &'a [u8],
    /// The next `-->` and `--!>`.
    dashes: Next,
    bang: Next,
}

impl<'a> Comments<'a> {
    pub(super) fn new(lower: &'a str) -> Self {
        Comments { lower: lower.as_bytes(), dashes: Next::new(b"-->"), bang: Next::new(b"--!>") }
    }

    /// Where a comment whose text starts at `from` (past `<!--`) ends, past its `-->` (or `--!>`,
    /// as browsers end one too; `<!-->` and `<!--->` are empty ones). `None` when it doesn't end.
    pub(super) fn end(&mut self, from: usize) -> Option<usize> {
        let rest = self.lower.get(from..)?;
        if rest.starts_with(b">") {
            return Some(from + 1);
        }
        if rest.starts_with(b"->") {
            return Some(from + 2);
        }
        match (self.dashes.at(self.lower, from).map(|e| e + 3), self.bang.at(self.lower, from).map(|e| e + 4)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Where a few bytes are next found in a text from a place on, remembered: asked again from a
/// place up to where they were found, the answer is the same, and once they aren't found they
/// aren't from any place further on either.
struct Next {
    needle: &'static [u8],
    /// Where they were last looked for from, and what was found (`None`: not looked for yet).
    last: Option<(usize, Option<usize>)>,
}

impl Next {
    fn new(needle: &'static [u8]) -> Self {
        Next { needle, last: None }
    }

    fn at(&mut self, hay: &[u8], from: usize) -> Option<usize> {
        if let Some((asked, found)) = self.last
            && from >= asked
            && found.is_none_or(|f| f >= from)
        {
            return found;
        }
        let found = hay.get(from..).and_then(|r| r.windows(self.needle.len()).position(|w| w == self.needle)).map(|n| from + n);
        self.last = Some((from, found));
        found
    }
}

/// Where a raw-text element `name` whose content starts at `from` ends, past its closing tag
/// (`</style>`, `</STYLE >`). `None` when it isn't closed.
pub(super) fn raw_text_end(lower: &str, from: usize, name: &str) -> Option<usize> {
    raw_text_close(lower, from, name).map(|(_, end)| end)
}

/// [`raw_text_end`], with where the closing tag starts (where the element's text ends).
fn raw_text_close(lower: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let closing = format!("</{name}");
    let mut i = from;
    loop {
        let at = i + lower.get(i..)?.find(&closing)?;
        let after = at + closing.len();
        match lower.as_bytes().get(after) {
            Some(c) if c.is_ascii_whitespace() || *c == b'/' || *c == b'>' => return tag_end_at(lower.as_bytes(), after).map(|g| (at, g + 1)),
            Some(_) => i = after,
            None => return None,
        }
    }
}

/// Whether Outlook's copy in a conditional comment shows something of its own, so that what the
/// message hides from Outlook is the others' copy of it: text (outside comments, Office's `<xml>`
/// settings, style sheets, scripts and templates; a text box's text counts), a picture with a
/// source, or VML. A comment with nothing of the kind (a table's opening tags, an empty one)
/// isn't Outlook's own copy of anything (found in review: an empty `<!--[if mso]><![endif]-->`
/// turned off the warning of hidden text).
pub(super) fn shows_something(content: &str) -> bool {
    shows(content, true)
}

/// Whether what a browser shows of a part of a message Outlook's copy leaves out has text in it,
/// read as [`shows_something`] reads Outlook's copy, but with what's in an `<xml>` taken as text
/// (a browser shows it), and neither pictures nor VML counted (they aren't words).
pub(super) fn has_text(content: &str) -> bool {
    shows(content, false)
}

/// [`shows_something`] (`outlook`), or [`has_text`].
fn shows(content: &str, outlook: bool) -> bool {
    let lower = content.to_ascii_lowercase();
    if outlook && lower.contains("<v:") {
        return true;
    }
    let b = lower.as_bytes();
    let mut comments = Comments::new(&lower);
    // Templates open: what's in one isn't shown.
    let mut templates = 0usize;
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'<' {
            let rest = lower.get(i..).unwrap_or("");
            if rest.starts_with("<!--") {
                let Some(e) = comments.end(i + 4) else { return false };
                i = e;
                continue;
            }
            let closing = rest.starts_with("</");
            let name_at = i + 1 + usize::from(closing);
            let name = tag_name(&lower, name_at);
            if name.is_empty() {
                // A declaration, `<?…>` or `</` without a name: a comment up to its `>`, and all
                // the rest when it has none, as [`closed`] reads it. Else `<` in text.
                if rest.starts_with("<!") || rest.starts_with("<?") || closing {
                    let Some(g) = rest.find('>') else { return false };
                    i += g + 1;
                } else {
                    i += 1;
                }
                continue;
            }
            let Some(gt) = tag_end_at(b, name_at + name.len()) else { return false };
            // A picture that comes from somewhere (found in review: a copy that was only a
            // banner wasn't Outlook's own, and the others' copy of it counted as hidden text).
            if outlook
                && templates == 0
                && !closing
                && name == "img"
                && lower.get(i..=gt).filter(|t| t.contains("src")).and_then(|t| attr(t, "src")).is_some_and(|s| !s.trim().is_empty())
            {
                return true;
            }
            i = gt + 1;
            match (closing, name) {
                (true, "template") => templates = templates.saturating_sub(1),
                (true, _) => {}
                (false, "template") => templates = templates.saturating_add(1),
                // A text box's text, and `<xmp>`'s, are shown as written, to the end when they
                // aren't closed (found in review: they were taken for a script's, and the text
                // the others' copy hid in one went unseen).
                (false, "textarea" | "xmp") => {
                    let (shut, e) = raw_text_close(&lower, i, name).unwrap_or((lower.len(), lower.len()));
                    if templates == 0 && words_in(lower.get(i..shut).unwrap_or(""), name == "textarea") {
                        return true;
                    }
                    i = e;
                }
                (false, n) if (outlook && n == "xml") || RAW_TEXT.contains(&n) => {
                    let Some(e) = raw_text_end(&lower, i, name) else { return false };
                    i = e;
                }
                (false, _) => {}
            }
            continue;
        }
        if c == b'&' {
            i += reference_len(b, i);
            continue;
        }
        let ch = lower.get(i..).and_then(|r| r.chars().next());
        if templates == 0 && ch.is_some_and(char::is_alphanumeric) {
            return true;
        }
        i += ch.map_or(1, char::len_utf8);
    }
    false
}

/// How long a character reference at `at` is (a spacer's `&nbsp;` or `&#847;`, which isn't text
/// of its own), or 1 for `&` alone: its `;` looked for in the few bytes a name takes, not to the
/// end of the copy.
fn reference_len(b: &[u8], at: usize) -> usize {
    b.get(at + 1..).and_then(|r| r.iter().take(11).position(|c| *c == b';')).map_or(1, |n| n + 2)
}

/// Whether raw text has a word in it: a letter or digit, character references not counted when
/// they're read (`refs`: a text box's; `<xmp>` shows them as written).
fn words_in(text: &str, refs: bool) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if refs && c == b'&' {
            i += reference_len(b, i);
            continue;
        }
        let ch = text.get(i..).and_then(|r| r.chars().next());
        if ch.is_some_and(char::is_alphanumeric) {
            return true;
        }
        i += ch.map_or(1, char::len_utf8);
    }
    false
}
