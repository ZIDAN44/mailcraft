//! HTML mail read as desktop Outlook reads it (issue #47). Newsletters give Outlook a copy of
//! their own: `<!--[if mso]> … <![endif]-->` holds markup only Outlook reads (tables standing in
//! for columns laid side by side with `float` or `inline-block`, which Outlook's Word engine
//! doesn't lay out, and VML for rounded buttons and pictures behind text), and
//! `<!--[if !mso]><!--> … <!--<![endif]-->` what every program but Outlook reads. MailCraft lays
//! out tables well, so it takes Outlook's copy: the conditions are worked out as Outlook 2016
//! and later answer them (`mso` is 16, `IE` false), and the VML it draws is made plain HTML (a
//! rounded button a link with its colour, a shape's picture its background). Office's own
//! `<xml>` settings go.
//!
//! The conditional comments are a convention of the mail's own markup, written by the programs
//! that make newsletters. Like everything from mail, it is hostile input: the conditions are
//! short and shallow or they're false, an unfinished comment is left as it was (a comment, which
//! hides nothing that wasn't hidden), and nothing here panics. What comes out is read as any
//! mail is: its pictures held back unless allowed, its links opened as links are, what it hides
//! from Outlook (`mso-hide: all`) left out.

use std::borrow::Cow;

use reading::{Comments, RAW_TEXT, TABLE_END, closed, has_text, raw_text_end, shows_something};

/// The version of Office desktop Outlook reports (2016, 2019, 2021 and Microsoft 365).
const MSO: u32 = 16;
/// The longest condition worked out; a longer one is false.
const MAX_CONDITION: usize = 200;
/// How deep a condition's brackets and negations go at most.
const MAX_NESTING: usize = 16;
/// VML shapes open at once, at most (their pictures go to the innermost).
const MAX_SHAPES: usize = 32;
/// How far a tag is looked through for its end, at most.
const MAX_TAG: usize = 8_192;
/// Office `<xml>` blocks dropped at most (each is looked through for its end).
const MAX_XML: usize = 16;
/// Put just before each table a comment only Outlook reads leaves open: a table a newsletter
/// gives Outlook to lay its columns out with, whose cells hold what follows the comment, so that
/// its text reads it as blocks, one cell after another ([`crate::layout::ready`]). A table opened
/// and closed in the same comment is read as any other is, a table of values a row to a line
/// (found in review: every table in Outlook's copy was read as blocks, and an order's rows came
/// out a cell to a line). A comment, put only where it takes no more room than the conditional
/// comment's own (`<!--[if mso]>`, `<![endif]-->`): marking the copy never pushes what follows
/// past the cap on what's read ([`crate::HTML_MAX`]; found in review: a mark written into each
/// table did, and a long newsletter lost its end). A sender that writes one in a message of its
/// own only changes how the table after it reads in its text; one that ends up in text (a text
/// box's, an attribute's) is left out of it as the tree is read.
pub(crate) const OUTLOOK_MARK: &str = "<!--mso-->";
/// The attribute the tables just after [`OUTLOOK_MARK`] are given in the tree the layout reads
/// ([`crate::layout::ready`]); one a sender writes is left out.
pub(crate) const OUTLOOK_TABLE: &str = "data-mailcraft-outlook";

/// `html` as desktop Outlook reads it: its conditional comments worked out and its VML made HTML.
/// The same HTML when it has none.
pub fn as_outlook_reads(html: &str) -> Cow<'_, str> {
    let copy = read_as_outlook(html, false).html;
    match hidden_from_outlook(&copy) {
        Cow::Borrowed(_) => copy,
        Cow::Owned(hidden) => Cow::Owned(hidden),
    }
}

/// A message's HTML as Outlook reads it ([`outlook_copy`]).
pub(crate) struct OutlookCopy<'a> {
    /// Its conditional comments worked out and its VML made HTML, what it hides from Outlook not
    /// hidden yet ([`hidden_from_outlook`]).
    pub(crate) html: Cow<'a, str>,
    /// It gives Outlook a copy of its own, so that what it hides from Outlook is the copy for
    /// everyone else rather than words kept from the reader.
    pub(crate) own: bool,
    /// It has a copy for everyone else that Outlook's leaves out, with text in it, whose hidden
    /// text is read on the HTML as it came: any part a browser shows that Outlook's copy drops
    /// (`<!--[if !mso]><!-->`, `<!--[if !mso]><!-- -->`, `<!--[if IE]>-->`, `<![if !mso]>`, an
    /// `<xml>` outside a comment).
    pub(crate) theirs: bool,
    /// Outlook's own copy has tables of its own: the message is laid out with tables, and its
    /// text reads those that lay it out as blocks ([`OUTLOOK_MARK`]). Only a comment only Outlook
    /// reads sets it (found in review: Office's settings, VML, or the words "[if needed]"
    /// anywhere in a message did, and an invoice's rows read a cell to a line).
    pub(crate) tables: bool,
    /// A comment only Outlook reads, kept, leaves elements open or nests them deep: one after
    /// another, they may nest what follows ever deeper, each in the last, as a browser never
    /// would, the comments being what it leaves out ([`crate::layout::ready`]).
    pub(crate) nests: bool,
}

/// `html` as Outlook reads it, the tables Outlook's own copy leaves open marked ([`OUTLOOK_MARK`]).
pub(crate) fn outlook_copy(html: &str) -> OutlookCopy<'_> {
    read_as_outlook(html, true)
}

/// [`outlook_copy`], where Outlook's own tables are marked or not (`mark`).
fn read_as_outlook(html: &str, mark: bool) -> OutlookCopy<'_> {
    let lower = html.to_ascii_lowercase();
    if !(lower.contains("[if") || lower.contains("<v:") || lower.contains("<xml")) {
        return OutlookCopy { html: Cow::Borrowed(html), own: false, theirs: false, tables: false, nests: false };
    }
    let read = conditions(html, &lower, mark);
    let (copy, xml_shown) = vml(&read.html, &read.kept);
    OutlookCopy { html: Cow::Owned(copy), own: read.own, theirs: read.theirs || xml_shown, tables: read.tables, nests: read.nests }
}

/// [`conditions`]' answer.
struct Read {
    /// The HTML with its conditional comments worked out.
    html: String,
    /// A comment only Outlook reads was kept, with something of its own to show.
    own: bool,
    /// Something a browser shows was dropped, with text in it.
    theirs: bool,
    /// A comment only Outlook reads, kept, opened a table.
    tables: bool,
    /// A comment only Outlook reads, kept, leaves elements open or nests them deep.
    nests: bool,
    /// Where in `html` what's kept of comments only Outlook reads is, in order.
    kept: Vec<std::ops::Range<usize>>,
}

/// The conditional comments worked out: Outlook's own copy kept as markup, the others' dropped.
/// Whether one was Outlook's own: a comment only Outlook reads, kept, with something of its own
/// to show ([`shows_something`]); and whether what a browser shows of one dropped had text
/// ([`has_text`]), in whatever form it's written (found in review: only `<!--[if !mso]><!-->`
/// counted, and text hidden in the others' copy written another way went unseen). With `mark`,
/// the tables Outlook's own copy leaves open are marked ([`OUTLOOK_MARK`]).
fn conditions(html: &str, lower: &str, mark: bool) -> Read {
    let mut read = Read { html: String::with_capacity(html.len()), own: false, theirs: false, tables: false, nests: false, kept: Vec::new() };
    let out = &mut read.html;
    // Where each comment's end is, found once: looking for it from each comment again would take
    // as long as the mail for each of thousands.
    let endifs: Vec<usize> = lower.match_indices(ENDIF).map(|(at, _)| at).collect();
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|rest| rest.find("<!")) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        match conditional(lower, at, &endifs) {
            Some(c) => {
                if c.keep {
                    let content = html.get(at + c.content.start..at + c.content.end).unwrap_or("");
                    read.own = read.own || (c.hidden && shows_something(content));
                    let kept = closed(content);
                    // In the room the comment took, less what's kept of it, which may be longer
                    // than its content (found in review: the closing tag of a style sheet left
                    // open, with the marks, made it longer than the comment). The tables closed
                    // past the few kept open aren't counted: the outermost are marked still, and
                    // what's written is read past the cap ([`crate::WRITTEN_MAX`]).
                    let room = c.end.saturating_sub(kept.html.len().saturating_sub(kept.closes.saturating_mul(TABLE_END.len())));
                    let marks = if mark && c.hidden { kept.open.len().min(room / OUTLOOK_MARK.len()) } else { 0 };
                    let from = out.len();
                    let mut last = 0;
                    for &table in kept.open.iter().take(marks) {
                        out.push_str(kept.html.get(last..table).unwrap_or(""));
                        out.push_str(OUTLOOK_MARK);
                        last = table;
                    }
                    out.push_str(kept.html.get(last..).unwrap_or(""));
                    if c.hidden {
                        read.kept.push(from..out.len());
                        read.tables |= kept.tables;
                        read.nests |= kept.nests;
                    }
                } else if !read.theirs
                    && let Some(seen) = c.seen
                {
                    // What a browser shows of a comment Outlook's copy drops (the others' copy),
                    // when it has text.
                    read.theirs = lower.get(at + seen..at + c.content.end).is_some_and(has_text);
                }
                i = at + c.end;
            }
            None => {
                out.push_str("<!");
                i = at + 2;
            }
        }
    }
    out.push_str(html.get(i..).unwrap_or(""));
    read
}

/// Where a conditional comment's content ends: `<![endif]-->` (`<!--<![endif]-->` when its
/// content was markup for everyone else).
const ENDIF: &str = "<![endif]";

/// A conditional comment at `at` in `lower` (the HTML lowercased): where its content is, where it
/// ends, and whether Outlook reads its content. Byte offsets into `lower`, as `at`.
struct Conditional {
    content: std::ops::Range<usize>,
    end: usize,
    keep: bool,
    /// Hidden from every program but those it names (`<!--[if mso]>`).
    hidden: bool,
    /// Where in it a browser starts showing what it holds, when one does before its end.
    seen: Option<usize>,
}

fn conditional(lower: &str, at: usize, endifs: &[usize]) -> Option<Conditional> {
    let rest = lower.get(at..)?;
    // `<!--[if c]>` (hidden from all but those it names) or `<![if c]>` (shown to all but those
    // it leaves out).
    let (open, hidden) = if rest.starts_with("<!--[if") {
        ("<!--[if", true)
    } else if rest.starts_with("<![if") {
        ("<![if", false)
    } else {
        return None;
    };
    // The condition ends at its first `]`, which a `>` must follow: Word's own comments
    // (`<!--[if !supportLists]-->`, a list's number after it) are comments, as in a browser.
    let head = rest.get(open.len()..)?;
    let limit = (0..=head.len().min(MAX_CONDITION + 8)).rev().find(|n| head.is_char_boundary(*n)).unwrap_or(0);
    let close = head.get(..limit)?.find(']')? + open.len();
    if rest.as_bytes().get(close + 1) != Some(&b'>') {
        return None;
    }
    let condition = rest.get(open.len()..close)?;
    let mut start = close + 2;
    // `<!--[if !mso]><!-->`: the comment closed at once, its content markup for everyone else.
    let revealed = !hidden || rest.get(start..).is_some_and(|r| r.starts_with("<!-->"));
    if hidden && revealed {
        start += "<!-->".len();
    }
    // Its end: the first `<![endif]` after its start (none: it's left as it was).
    let start = at + start;
    let end = *endifs.get(endifs.partition_point(|e| *e < start))?;
    let tail = lower.get(end + ENDIF.len()..).unwrap_or("");
    let past = end + ENDIF.len() + if tail.starts_with("-->") { 3 } else { usize::from(tail.starts_with('>')) };
    // Where a browser starts showing its content: `<![if …]>` is a declaration it skips;
    // `<!--[if …]>` a comment it ends at the first `-->` or `--!>`, which may come before the
    // content ends (`<!--[if !mso]><!-- -->`, `<!--[if IE]>-->`). Looked for up to its end only,
    // so that each comment is read through once.
    let seen = match hidden {
        false => Some(start - at),
        true => lower.get(..end).and_then(|l| Comments::new(l).end(at + 4)).map(|e| e - at),
    };
    // `<!--<![endif]-->`: the comment opened again just before its end is left out with the rest
    // of what's left open ([`closed`]).
    Some(Conditional { content: start - at..end - at, end: past - at, keep: outlook_reads(condition), hidden: hidden && !revealed, seen })
}

/// Whether Outlook reads what `condition` (lowercased, without `if`) guards: `mso`, `gte mso 9`,
/// `!mso`, `(mso)|(ie)`, `mso & !ie`… Anything it can't make out is false.
fn outlook_reads(condition: &str) -> bool {
    let condition = condition.trim();
    if condition.len() > MAX_CONDITION {
        return false;
    }
    let tokens = tokens(condition);
    let mut at = 0;
    let value = expr(&tokens, &mut at, 0);
    value.filter(|_| at == tokens.len()).unwrap_or(false)
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Num(u32),
    Not,
    Or,
    And,
    Open,
    Close,
}

fn tokens(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            '!' => out.push(Tok::Not),
            '|' => out.push(Tok::Or),
            '&' => out.push(Tok::And),
            '(' => out.push(Tok::Open),
            ')' => out.push(Tok::Close),
            c if c.is_ascii_digit() => {
                let mut n: u32 = 0;
                while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                    n = n.saturating_mul(10).saturating_add(d);
                    chars.next();
                }
                // A version's minor part (`mso 12.0`) doesn't change the answer.
                while chars.peek().is_some_and(|c| *c == '.' || c.is_ascii_digit()) {
                    chars.next();
                }
                out.push(Tok::Num(n));
                continue;
            }
            c if c.is_ascii_alphabetic() => {
                let mut w = String::new();
                while let Some(&c) = chars.peek().filter(|c| c.is_ascii_alphanumeric()) {
                    w.push(c);
                    chars.next();
                }
                out.push(Tok::Word(w));
                continue;
            }
            _ => {}
        }
        chars.next();
    }
    out
}

fn expr(t: &[Tok], at: &mut usize, depth: usize) -> Option<bool> {
    let mut v = term(t, at, depth)?;
    while t.get(*at) == Some(&Tok::Or) {
        *at += 1;
        v |= term(t, at, depth)?;
    }
    Some(v)
}

fn term(t: &[Tok], at: &mut usize, depth: usize) -> Option<bool> {
    let mut v = factor(t, at, depth)?;
    while t.get(*at) == Some(&Tok::And) {
        *at += 1;
        v &= factor(t, at, depth)?;
    }
    Some(v)
}

fn factor(t: &[Tok], at: &mut usize, depth: usize) -> Option<bool> {
    if depth > MAX_NESTING {
        return None;
    }
    match t.get(*at)? {
        Tok::Not => {
            *at += 1;
            factor(t, at, depth + 1).map(|v| !v)
        }
        Tok::Open => {
            *at += 1;
            let v = expr(t, at, depth + 1)?;
            (t.get(*at) == Some(&Tok::Close)).then(|| *at += 1)?;
            Some(v)
        }
        Tok::Word(w) => {
            // `gte mso 9`: a comparison, then what, then the version.
            let cmp = matches!(w.as_str(), "gt" | "gte" | "lt" | "lte").then(|| w.clone());
            if cmp.is_some() {
                *at += 1;
            }
            let Some(Tok::Word(what)) = t.get(*at) else { return None };
            *at += 1;
            let version = match t.get(*at) {
                Some(Tok::Num(n)) => {
                    *at += 1;
                    Some(*n)
                }
                _ => None,
            };
            // Outlook is Office (`mso`); it isn't Internet Explorer. VML is false: Word's own
            // copy of a pasted picture (`[if gte vml 1]`) has the picture the others are given
            // beside it (`<![if !vml]>`), and newsletters put their VML under `[if mso]`.
            let have = match what.as_str() {
                "mso" | "outlook" => Some(MSO),
                _ => None,
            };
            Some(match (have, cmp.as_deref(), version) {
                (None, ..) => false,
                (Some(_), None, None) => true,
                (Some(h), None, Some(v)) => h == v,
                (Some(h), Some("gt"), Some(v)) => h > v,
                (Some(h), Some("gte"), Some(v)) => h >= v,
                (Some(h), Some("lt"), Some(v)) => h < v,
                (Some(h), Some("lte"), Some(v)) => h <= v,
                _ => false,
            })
        }
        _ => None,
    }
}

/// A tag at the start of `rest`: its lowercased name, where it ends, whether it closes.
fn tag(rest: &str) -> Option<(String, usize, bool)> {
    let inner = rest.strip_prefix('<')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(i) => (true, i),
        None => (false, inner),
    };
    let name: String = inner.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-').collect::<String>().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    // The end of the tag, past quoted values (not looked for past MAX_TAG).
    let mut quote = None;
    for (n, c) in rest.char_indices().skip(1).take_while(|(n, _)| *n < MAX_TAG) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some((name, n + 1, closing)),
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

/// A tag's attributes, read as a browser reads them: names lowercased, values quoted or not, a
/// name inside another's quoted value not taken for an attribute, entities in values decoded.
fn attrs(tag: &str) -> Vec<(String, String)> {
    attrs_at(tag).into_iter().map(|(name, value, _)| (name, value)).collect()
}

/// [`attrs`], with where each value starts in `tag` (inside its quotes).
fn attrs_at(tag: &str) -> Vec<(String, String, usize)> {
    let mut out = Vec::new();
    let mut chars = tag.char_indices().peekable();
    // Past `<` and the tag's name.
    chars.next();
    while chars.peek().is_some_and(|(_, c)| !c.is_ascii_whitespace() && *c != '>' && *c != '/') {
        chars.next();
    }
    loop {
        while chars.peek().is_some_and(|(_, c)| c.is_ascii_whitespace() || *c == '/') {
            chars.next();
        }
        let Some(&(start, c)) = chars.peek() else { break };
        if c == '>' {
            break;
        }
        while chars.peek().is_some_and(|(_, c)| !c.is_ascii_whitespace() && !matches!(c, '=' | '>' | '/')) {
            chars.next();
        }
        let end = chars.peek().map_or(tag.len(), |(n, _)| *n);
        let name = tag.get(start..end).unwrap_or("").to_ascii_lowercase();
        while chars.peek().is_some_and(|(_, c)| c.is_ascii_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        let mut value_at = end;
        if chars.peek().is_some_and(|(_, c)| *c == '=') {
            chars.next();
            while chars.peek().is_some_and(|(_, c)| c.is_ascii_whitespace()) {
                chars.next();
            }
            value_at = chars.peek().map_or(tag.len(), |(n, _)| *n);
            match chars.peek().map(|(_, c)| *c) {
                Some(q @ ('"' | '\'')) => {
                    chars.next();
                    value_at += 1;
                    for (_, c) in chars.by_ref() {
                        if c == q {
                            break;
                        }
                        value.push(c);
                    }
                }
                _ => {
                    while let Some(&(_, c)) = chars.peek().filter(|(_, c)| !c.is_ascii_whitespace() && *c != '>') {
                        value.push(c);
                        chars.next();
                    }
                }
            }
        }
        if name.is_empty() {
            // A stray character: past it.
            chars.next();
        } else if out.len() < 64 {
            out.push((name, decoded(&value).into_owned(), value_at));
        }
    }
    out
}

/// An attribute's value with its character references made characters, as a browser reads it
/// (`&amp;` in every address that has more than one parameter): `&amp;`, `&lt;`, `&gt;`,
/// `&quot;`, `&apos;` and numbered ones (`&#38;`, `&#x26;`). Others stay as written.
fn decoded(v: &str) -> Cow<'_, str> {
    if !v.contains('&') {
        return Cow::Borrowed(v);
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        // The reference's name, up to its `;` (a short one; a longer run is text).
        let named = after.find(';').filter(|n| *n <= 10).and_then(|n| Some((after.get(..n)?, n + 1)));
        let c = named.and_then(|(name, len)| {
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let n = name.strip_prefix('#')?;
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => n.parse::<u32>().ok()?,
                    };
                    Some(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{fffd}'))
                }
            }?;
            Some((c, len))
        });
        match c {
            Some((c, len)) => {
                out.push(c);
                rest = after.get(len..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// An attribute's value in a tag's text.
fn attr(tag: &str, name: &str) -> Option<String> {
    attrs(tag).into_iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// A colour or address (as read, its entities decoded) safe to write back into a quoted
/// attribute, a style or `url()`: nothing that ends a value or a declaration, and an address's
/// `&` written `&amp;` again.
fn safe(v: &str, url: bool) -> Option<String> {
    let v = v.trim();
    let ok = if url {
        v.len() < 2048 && v.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | '\\' | ';'))
    } else {
        v.len() < 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || "#(),. %".contains(c))
    };
    (ok && !v.is_empty()).then(|| v.replace('&', "&amp;"))
}

/// A picture's size in pixels from its `width` and `height`, or its style's (`width:1px`).
fn picture_size(tag: &str) -> (Option<u32>, Option<u32>) {
    let style = attr(tag, "style").unwrap_or_default().to_ascii_lowercase();
    let from_style = |prop: &str| {
        style.split(';').find_map(|d| {
            let (p, v) = d.split_once(':')?;
            (p.trim() == prop).then(|| v.trim().trim_end_matches("px").trim().parse::<f32>().ok()).flatten()
        })
    };
    let px = |prop: &str| {
        let v = attr(tag, prop).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok()).or_else(|| from_style(prop))?;
        (v.is_finite() && v >= 0.0).then(|| v.round().min(10_000.0) as u32)
    };
    (px("width"), px("height"))
}

/// Outlook's VML as HTML: a rounded button (`v:roundrect`) a link with its fill colour; a shape
/// (`v:rect`, `v:oval`…) a box, its `v:fill` picture behind it; `v:image` a picture at its size;
/// `v:textbox` what the box holds. Office's `<xml>` settings are dropped. The rest is left as it
/// came, comments and the content of scripts, style sheets and text boxes whole: what's in them
/// is text, not elements (found in review: an `<xml>` in a comment dropped what a browser shows
/// up to the `</xml>` in another). Whether an `<xml>` dropped was one a browser shows, with text
/// in it: outside what's kept of the comments only Outlook reads (`kept`, in order).
fn vml(html: &str, kept: &[std::ops::Range<usize>]) -> (String, bool) {
    let lower = html.to_ascii_lowercase();
    let mut comments = Comments::new(&lower);
    let mut out = String::with_capacity(html.len());
    // Open shapes: their tag written, and where in `out` their style ends (for a picture).
    let mut shapes: Vec<(String, usize)> = Vec::new();
    // What goes into a shape's style once its fill is read: where, and what. Made at the end, in
    // one pass (inserting each as it came took as long as the message for each of thousands).
    let mut fills: Vec<(usize, String)> = Vec::new();
    let mut xml = 0;
    let mut shown = false;
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|rest| rest.find('<')) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        if lower.get(at..).is_some_and(|r| r.starts_with("<!--")) {
            let e = comments.end(at + 4).unwrap_or(lower.len());
            out.push_str(html.get(at..e).unwrap_or(""));
            i = e;
            continue;
        }
        let Some((name, len, closing)) = lower.get(at..).and_then(tag) else {
            out.push('<');
            i = at + 1;
            continue;
        };
        let text = html.get(at..at + len).unwrap_or("");
        i = at + len;
        if !closing && RAW_TEXT.contains(&name.as_str()) {
            let e = raw_text_end(&lower, i, &name).unwrap_or(lower.len());
            out.push_str(html.get(at..e).unwrap_or(""));
            i = e;
            continue;
        }
        // Office's document settings: none of it is the message. One outside a comment is shown
        // by a browser, though (found in review: text hidden in one went unseen).
        if name == "xml" && !closing && xml < MAX_XML {
            xml += 1;
            if let Some(end) = lower.get(i..).and_then(|r| r.find("</xml")).map(|e| i + e) {
                let outlooks = kept.get(kept.partition_point(|r| r.end <= at)).is_some_and(|r| r.start <= at);
                shown = shown || (!outlooks && lower.get(i..end).is_some_and(has_text));
                i = lower.get(end..).and_then(|r| r.find('>')).map_or(end, |g| end + g + 1);
            }
            continue;
        }
        let kind = match name.strip_prefix("v:") {
            Some(kind) => kind,
            None if name == "center" && shapes.last().is_some_and(|(t, _)| t == "a") => "center",
            None => {
                out.push_str(text);
                continue;
            }
        };
        match (kind, closing) {
            ("roundrect" | "rect" | "oval" | "shape", false) => {
                if shapes.len() >= MAX_SHAPES {
                    continue;
                }
                let fill = attr(text, "fillcolor").and_then(|c| safe(&c, false));
                let href = attr(text, "href").and_then(|h| safe(&h, true));
                let (open, tag_name) = match &href {
                    Some(h) => (format!("<a href=\"{h}\" style=\"display:inline-block"), "a"),
                    None => ("<div style=\"display:block".to_string(), "div"),
                };
                out.push_str(&open);
                if let Some(c) = fill {
                    out.push_str(&format!(";background-color:{c}"));
                }
                if kind == "roundrect" {
                    out.push_str(&rounded(text));
                }
                let style_end = out.len();
                out.push_str("\">");
                shapes.push((tag_name.to_string(), style_end));
            }
            ("roundrect" | "rect" | "oval" | "shape", true) => {
                if let Some((tag_name, _)) = shapes.pop() {
                    out.push_str(&format!("</{tag_name}>"));
                }
            }
            // A shape's picture, behind what it holds, and its colour under the picture.
            ("fill", false) => {
                let src = attr(text, "src").and_then(|s| safe(&s, true)).map(|s| format!(";background-image:url({s})"));
                let color = attr(text, "color").and_then(|c| safe(&c, false)).map(|c| format!(";background-color:{c}"));
                if let Some((_, end)) = shapes.last() {
                    fills.push((*end, format!("{}{}", color.unwrap_or_default(), src.unwrap_or_default())));
                }
            }
            // A button's words, centred in it: inline, so the button stays where its line puts it.
            ("center", true) => out.push_str("</span>"),
            ("center", false) => {
                out.push_str("<span");
                out.push_str(text.get("<center".len()..).unwrap_or(">"));
            }
            // A picture, at its size (a pixel there to see who opened the mail is one too).
            ("image" | "imagedata", false) => {
                if let Some(src) = attr(text, "src").and_then(|s| safe(&s, true)) {
                    let (w, h) = picture_size(text);
                    let size = |n: &str, v: Option<u32>| v.map(|v| format!(" {n}=\"{v}\"")).unwrap_or_default();
                    out.push_str(&format!("<img src=\"{src}\"{}{}>", size("width", w), size("height", h)));
                }
            }
            // What a box holds is its content; the rest of VML (strokes, shadows) draws nothing here.
            _ => {}
        }
    }
    out.push_str(html.get(i..).unwrap_or(""));
    for (tag_name, _) in shapes.into_iter().rev() {
        out.push_str(&format!("</{tag_name}>"));
    }
    (with_fills(out, fills), shown)
}

/// What a VML button (`v:roundrect`, its tag `tag`) says of its box, as CSS: its size (`width`
/// and `height` in its style), its words in the middle of it, and its corners, `arcsize` a share
/// of its shorter side (`10%`, `0.1`, or in 65536ths, `6554f`; VML's 0.2 when it says none), at
/// most half of it. Only numbers MailCraft writes go into the style.
fn rounded(tag: &str) -> String {
    let style = attr(tag, "style").unwrap_or_default().to_ascii_lowercase();
    let side = |prop: &str| {
        let v = style.split(';').find_map(|d| d.split_once(':').filter(|(p, _)| p.trim() == prop).map(|(_, v)| v.trim().to_string()))?;
        crate::layout::length_px(&v).filter(|px| *px >= 1.0).map(|px| px.min(4000.0).round())
    };
    let (w, h) = (side("width"), side("height"));
    let mut out = String::from(";text-align:center");
    if let Some(w) = w {
        out.push_str(&format!(";width:{w}px"));
    }
    if let Some(h) = h {
        out.push_str(&format!(";height:{h}px"));
    }
    let arc = match attr(tag, "arcsize").map(|a| a.trim().to_ascii_lowercase()) {
        None => Some(0.2),
        Some(a) => match (a.strip_suffix('%'), a.strip_suffix('f')) {
            (Some(pc), _) => pc.trim().parse::<f32>().ok().map(|p| p / 100.0),
            (_, Some(f)) => f.trim().parse::<f32>().ok().map(|f| f / 65536.0),
            _ => a.parse::<f32>().ok(),
        },
    };
    let shorter = match (w, h) {
        (Some(w), Some(h)) => Some(w.min(h)),
        (one, other) => one.or(other),
    };
    if let (Some(arc), Some(shorter)) = (arc.filter(|a| a.is_finite() && *a > 0.0), shorter) {
        let r = (arc.min(0.5) * shorter).round();
        if r >= 1.0 {
            out.push_str(&format!(";border-radius:{r}px"));
        }
    }
    out
}

/// `out` with each shape's fill put into its style, in one pass (`at` in order: shapes are
/// written in turn, and a shape's fills follow one another).
fn with_fills(out: String, mut fills: Vec<(usize, String)>) -> String {
    if fills.is_empty() {
        return out;
    }
    fills.sort_by_key(|(at, _)| *at);
    let mut done = String::with_capacity(out.len() + fills.iter().map(|(_, f)| f.len()).sum::<usize>());
    let mut last = 0;
    for (at, fill) in fills {
        if at < last || !out.is_char_boundary(at) {
            continue;
        }
        done.push_str(out.get(last..at).unwrap_or(""));
        done.push_str(&fill);
        last = at;
    }
    done.push_str(out.get(last..).unwrap_or(""));
    done
}

/// What `html` hides from Outlook (`mso-hide: all` in an element's style, a newsletter's own copy
/// of what its VML draws for Outlook) hidden here as well, `display:none;` written into its
/// style. The same HTML when nothing is.
pub(crate) fn hidden_from_outlook(html: &str) -> Cow<'_, str> {
    in_place::hidden_in_place(html, in_place::Hide::Style)
}

/// Whether an element's style (its `style` attribute, as read) hides it from Outlook: its
/// `mso-hide` says `all`, the last declaration of it winning, or the last `!important` one (in
/// any case and spacing). The one test of it for the message as it's shown, its text, and the
/// text it hides.
pub(crate) fn mso_hidden(style: &str) -> bool {
    let mut said: Option<bool> = None;
    let mut important = false;
    for (p, v) in style.split(';').filter_map(|d| d.split_once(':')) {
        if !p.trim().eq_ignore_ascii_case("mso-hide") {
            continue;
        }
        let v: String = v.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        let (v, this_important) = match v.strip_suffix("!important") {
            Some(v) => (v.to_string(), true),
            None => (v, false),
        };
        if important && !this_important {
            continue;
        }
        important |= this_important;
        said = Some(v == "all");
    }
    said.unwrap_or(false)
}

mod in_place;
mod reading;
pub(crate) use in_place::{HIDDEN_RULE, marked_hidden};
#[cfg(test)]
mod tests;
