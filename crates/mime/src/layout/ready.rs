//! A message's HTML made ready to read (issue #47): its style sheets' rules written into each
//! element's `style`; the message as it's shown, without what it hides from Outlook
//! (`mso-hide: all`); and the HTML its text is read from, without it too and with the tables
//! Outlook's copy is laid out with read as blocks, one cell after another (found in review: a
//! newsletter's columns came out side by side in its text, lines of spaces between them).
//!
//! Outlook's copy is read from the tree only when the tree reshapes it, and the tree is written
//! out only when all of the message is in it: one that reaches a cap, or a copy of Outlook's
//! nesting more than two dozen tables ([`MAX_COPY_TABLES`]), is read as it came, as it was
//! before Outlook's copy was read (found in review: written from a tree that left elements out,
//! paragraphs ran together, and tables past those nested deep were read a cell to a line).
//! What's hidden from Outlook is marked where it is in the HTML as it came
//! ([`crate::html::marked_hidden`]), so that a message isn't written out again just to leave it
//! out (found in review: a megabyte table with it anywhere took twice as long to read).

use std::borrow::Cow;

use super::*;
use crate::html::OutlookCopy;

/// [`ready`]'s HTML.
pub(crate) struct Ready {
    /// Style sheet rules as inline styles ([`inline_css`]): the text it hides is read from it.
    pub(crate) styled: String,
    /// As it's shown: without what's hidden from Outlook, or with it marked (`hidden`).
    pub(crate) shown: String,
    /// What its text is read from: without what's hidden from Outlook, or with it marked.
    pub(crate) text: String,
    /// What's hidden from Outlook is marked in what's shown and in the text, for the converters
    /// to leave out ([`crate::html::marked_hidden`]).
    pub(crate) hidden: bool,
    /// It's Outlook's copy; else the message as it came, Outlook's copy dropped.
    pub(crate) copy: bool,
}

/// The message made ready to read: Outlook's `copy` of it when there is one and all of it is
/// read, else the message as it came (`raw`), as it's read without Outlook's copy. The tree is
/// read only when something needs it: style sheets, or Outlook's copy laid out with tables of
/// Outlook's own (its layout tables read as blocks in its text) or leaving elements open
/// ([`crate::html::OutlookCopy::nests`]: one after another, the comments may nest what follows
/// ever deeper, past what's read, and it's read as it came then).
///
/// A message Outlook reads otherwise than it came, whose copy for Outlook isn't read, is read as
/// it was before Outlook's copy was, what it hides from Outlook shown: that's the copy standing
/// for Outlook's, dropped (found in review: a newsletter's button, in Outlook's copy and in a
/// copy hidden from Outlook, was in neither what was shown nor its text, past a cap on its tree).
pub(crate) fn ready(raw: &str, copy: &OutlookCopy) -> Ready {
    let otherwise = matches!(copy.html, Cow::Owned(_));
    if otherwise && let Some(r) = read(&copy.html, Some(copy), true) {
        return r;
    }
    let hide = !otherwise;
    read(raw, None, hide).unwrap_or_else(|| marked(raw.to_string(), raw, None, false, hide))
}

/// [`ready`] for `html`, Outlook's copy or not (`outlook`), what it hides from Outlook hidden or
/// shown (`hide`); `None` when its tree reaches a cap.
fn read(html: &str, outlook: Option<&OutlookCopy>, hide: bool) -> Option<Ready> {
    let copy = outlook.is_some();
    let (outlooks, nests) = outlook.map_or((false, false), |c| (c.tables, c.nests));
    let lower = html.to_ascii_lowercase();
    let styles = lower.contains("<style") || lower.contains("font-size") || lower.contains("<font");
    if !(styles || outlooks || nests) {
        return Some(marked(html.to_string(), html, None, copy, hide));
    }
    // What's written from it is read up to a multiple of the cap on what's read; past it, it's
    // read as it came.
    if html.len() > crate::WRITTEN_MAX && html.chars().count() > crate::WRITTEN_MAX {
        return None;
    }
    let done = std::panic::catch_unwind(|| {
        let tree = parse_as(html, copy)?;
        // Outlook's copy nesting more than two dozen tables is read as the message came, without
        // it: each table nested in another costs the text converter more, and none of them
        // is made blocks to spare it (found in review: a copy of tables left open, two dozen deep,
        // took a second and a half to read; then, its outermost tables made blocks, a table of
        // columns or of values around a deep one lost its columns).
        if copy && tables_nested(&tree.root) > MAX_COPY_TABLES {
            return None;
        }
        let mut root = tree.root;
        let mut sheet_hides = false;
        if styles {
            let mut text = String::new();
            style_text(&root, &mut text, 0);
            sheet_hides = hide && text.to_ascii_lowercase().contains("mso-hide");
            let sheet = crate::css::Sheet::parse(&text);
            apply_css(&mut root, &sheet, &mut Vec::new(), 0);
        }
        let mut root = Kid::El(root);
        // The text written from the tree when Outlook's copy lays the message out with tables
        // (read as blocks), or the style sheet hides something from Outlook (only the tree knows
        // what): before titles are made headings (the text converter would mark them), and
        // without the styles, which the text converter doesn't read (found in review: written
        // into every element, they made a long newsletter's text longer than is read).
        let text = (outlooks || sheet_hides).then(|| {
            let mut out = String::with_capacity(html.len());
            write_as(&root, &mut out, How { leave_hidden: true, blocks: outlooks, styles: false, in_values: false }, false);
            out
        });
        // Written from the tree when its styles are, or it reads otherwise than as it came; as it
        // came else, not written out a second time (found in review: every copy was, and a
        // newsletter of 4 MB took longer to read).
        let styled = if styles || tree.reshaped {
            if let Kid::El(n) = &mut root
                && styles
            {
                make_headings(n, 0);
            }
            let mut out = String::with_capacity(html.len() + html.len() / 4);
            write_as(&root, &mut out, How { leave_hidden: false, blocks: false, styles: true, in_values: false }, false);
            out
        } else {
            html.to_string()
        };
        let long = |s: &str| s.len() > crate::WRITTEN_MAX && s.chars().count() > crate::WRITTEN_MAX;
        if long(&styled) || text.as_deref().is_some_and(long) {
            return None;
        }
        Some(marked(styled, html, text, copy, hide))
    });
    done.ok().flatten()
}

/// The message ready to read from its `styled` HTML and the HTML it `came` as, its `text`
/// written from the tree, or read from `came`: what's hidden from Outlook marked in what's
/// shown, and in its text read as it came, when it's hidden (`hide`). Neither, if the sender
/// wrote the mark.
fn marked(styled: String, came: &str, text: Option<String>, copy: bool, hide: bool) -> Ready {
    // Each marked: `None` with the sender's mark, `Some(None)` with nothing to mark.
    let owned = |c: Cow<'_, str>| match c {
        Cow::Owned(s) => Some(s),
        Cow::Borrowed(_) => None,
    };
    let both = hide.then(|| {
        let shown = crate::html::marked_hidden(&styled).map(owned);
        let text_marked = match text {
            Some(_) => Some(None),
            None => crate::html::marked_hidden(came).map(owned),
        };
        (shown, text_marked)
    });
    match both {
        Some((Some(shown), Some(text_marked))) => Ready {
            hidden: shown.is_some() || text_marked.is_some(),
            shown: shown.unwrap_or_else(|| styled.clone()),
            text: text.or(text_marked).unwrap_or_else(|| came.to_string()),
            styled,
            copy,
        },
        _ => Ready { shown: styled.clone(), styled, text: text.unwrap_or_else(|| came.to_string()), hidden: false, copy },
    }
}

/// How the tree is written.
#[derive(Clone, Copy)]
pub(super) struct How {
    /// What's hidden from Outlook left out.
    pub(super) leave_hidden: bool,
    /// Layout tables written as blocks.
    pub(super) blocks: bool,
    /// Elements' styles written (the text converter reads none for the text).
    pub(super) styles: bool,
    /// In a table written as a table: no table in its cells is written as blocks, so its rows
    /// stay a row to a line (found in review: an item's picture and name in a table of
    /// `role="presentation"`, the name went to a line of its own under each row).
    pub(super) in_values: bool,
}

/// Parts of a table, written as blocks in a layout table read as blocks.
fn table_part(tag: &str) -> bool {
    matches!(tag, "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" | "caption")
}

/// Markup for a part of the tree, `how` says (`layout`: the table parts in it are a layout
/// table's, written as blocks). It goes no deeper than the tree, whose depth is capped
/// ([`MAX_OPEN`]).
pub(super) fn write_as(k: &Kid, out: &mut String, how: How, layout: bool) {
    match k {
        Kid::Text(t) => escaped(t, out, false),
        Kid::El(n) => {
            if n.tag.starts_with('#') {
                for k in &n.kids {
                    write_as(k, out, how, layout);
                }
                return;
            }
            if how.leave_hidden && n.attr("style").is_some_and(crate::html::mso_hidden) {
                return;
            }
            let (tag, layout, how) = match n.tag.as_str() {
                // Its text as it's written, in a box that ends (`<plaintext>` doesn't, and the
                // tags written after it showed as text; found in review).
                "plaintext" => ("pre", layout, how),
                // A layout table, read as blocks in the text.
                "table" if how.blocks && !how.in_values && layout_table(n) => ("div", true, how),
                "table" => ("table", false, How { in_values: true, ..how }),
                t if layout && table_part(t) => ("div", true, how),
                t => (t, layout, how),
            };
            out.push('<');
            out.push_str(tag);
            for (k, v) in &n.attrs {
                // Outlook's tables' mark is read here, not written out again.
                if k == crate::html::OUTLOOK_TABLE
                    || (k == "style" && !how.styles)
                    || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':')
                {
                    continue;
                }
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                escaped(v, out, true);
                out.push('"');
            }
            out.push('>');
            if !void(&n.tag) {
                for k in &n.kids {
                    match k {
                        // A style sheet, script or text box goes back as it was (escaping would
                        // break it), only never closed early.
                        Kid::Text(t) if raw_text(&n.tag) => raw(t, &n.tag, out),
                        k => write_as(k, out, how, layout),
                    }
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        }
    }
}

/// Text, or an attribute's value (`value`), written as markup: escaped only where it must be, so
/// that what's written is about as long as what it came from (found in review: every `&`
/// written `&amp;` made a long newsletter's HTML twice as long, and slower to read). A `&` that
/// could start a character reference (before a letter, a digit or `#`, or at the end, where
/// what's written next may follow it), a `<`, and a value's `"`.
fn escaped(t: &str, out: &mut String, value: bool) {
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' if chars.peek().is_none_or(|n| n.is_ascii_alphanumeric() || *n == '#') => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' if value => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Raw text (a style sheet's, a text box's) as it was, its own closing tag kept from ending it
/// early (found in review: every `</` was dropped, and a text box showed `TAb>` for `TA</b>`):
/// only where it would, before a space, `/` or `>`, or at its end (found in review: every one
/// written `<\/`, a style sheet of them came out longer, and slower to read). A style sheet's or
/// a script's, which nothing shows, without its `</` at all, as before Outlook's copy was read:
/// what's left is quicker for the text converter to read as a style sheet (found in review: a
/// sheet of them took twice as long).
fn raw(t: &str, tag: &str, out: &mut String) {
    if matches!(tag, "style" | "script") {
        // Each `</` left out as it's written, so that none is left, not even one that leaving
        // another out makes (`<<//`).
        let start = out.len();
        for c in t.chars() {
            if c == '/' && out.len() > start && out.ends_with('<') {
                out.pop();
            } else {
                out.push(c);
            }
        }
        return;
    }
    let lower = t.to_ascii_lowercase();
    let closing = format!("</{tag}");
    let mut from = 0;
    for (at, _) in lower.match_indices(&closing) {
        let ends = lower.as_bytes().get(at + closing.len()).is_none_or(|c| c.is_ascii_whitespace() || *c == b'/' || *c == b'>');
        if !ends {
            continue;
        }
        out.push_str(t.get(from..at + 1).unwrap_or(""));
        out.push('\\');
        from = at + 1;
    }
    out.push_str(t.get(from..).unwrap_or(""));
}

/// Whether a table plainly lays a message out rather than holding values in rows and columns:
/// Outlook's own copy's ([`crate::html::OUTLOOK_TABLE`]), or one that says so
/// (`role="presentation"`). Every other table is read as a table, as it was before Outlook's copy
/// was read (found in review: told apart by what their cells held, an invoice's rows with a long
/// description, a receipt's with a picture of each item, and an order whose items each had their
/// picture and name in a table of their own read a cell to a line).
fn layout_table(n: &Node) -> bool {
    n.attr(crate::html::OUTLOOK_TABLE).is_some() || n.attr("role").is_some_and(|r| r.trim().eq_ignore_ascii_case("presentation"))
}

/// How many tables are nested one in another in `n`, itself counted. It goes no deeper than the
/// tree ([`MAX_OPEN`]).
fn tables_nested(n: &Node) -> usize {
    let most = n.kids.iter().map(|k| if let Kid::El(e) = k { tables_nested(e) } else { 0 }).max().unwrap_or(0);
    most.saturating_add(usize::from(n.tag == "table"))
}
