//! Table footers (`<tfoot>`) for the text converter, which reads only a table's `<thead>` and
//! `<tbody>` and left a footer's rows out of the message and its text (an invoice's total, found
//! in review), and for the layout, which drew it where it was written. A table's first footer
//! becomes a `<tbody>` at the end of the table, where a browser draws it wherever it's written;
//! any other is a row group where it is, as a browser takes it (CSS 2.1 §17.2). One pass over the
//! HTML, as it came otherwise.

use std::borrow::Cow;

/// Tables open one in another that are followed, at most; their footers past it are rows where
/// they're written.
const MAX_TABLES: usize = 256;

/// A table open, as [`footers_last`] reads.
#[derive(Default)]
struct Table {
    /// Its first footer, start to end.
    foot: Option<(usize, usize)>,
    /// Where its first footer starts, while it's open.
    open: Option<usize>,
    /// It has had a footer.
    footed: bool,
}

impl Table {
    /// The footer open ends at `at`.
    fn end_foot(&mut self, at: usize) {
        if let Some(start) = self.open.take() {
            self.foot = Some((start, at));
        }
    }
}

/// `html` with each table's first footer as a row group at its end; as it came when there's none.
pub(crate) fn footers_last(html: &str) -> Cow<'_, str> {
    let b = html.as_bytes();
    if !b.windows(6).any(|w| w.eq_ignore_ascii_case(b"<tfoot")) {
        return Cow::Borrowed(html);
    }
    let mut bytes = b.to_vec();
    let mut tables: Vec<Table> = Vec::new();
    // Tables open past the ones followed.
    let mut deeper = 0usize;
    // Each table's first footer, and where it goes (its end tag).
    let mut moves: Vec<((usize, usize), usize)> = Vec::new();
    let mut i = 0;
    while let Some(at) = b.get(i..).and_then(|r| r.iter().position(|&c| c == b'<')).map(|o| i + o) {
        let rest = b.get(at..).unwrap_or_default();
        if rest.starts_with(b"<!--") {
            i = find(b, at + 4, b"-->").map_or(b.len(), |e| e + 3);
            continue;
        }
        // A declaration, `<![CDATA[`, or `<?`: to the next `>`, as a browser reads it.
        if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
            i = find(b, at + 2, b">").map_or(b.len(), |e| e + 1);
            continue;
        }
        let close = rest.get(1) == Some(&b'/');
        let name_at = at + 1 + usize::from(close);
        let name_len = b.get(name_at..).unwrap_or_default().iter().take_while(|c| c.is_ascii_alphanumeric()).count();
        // A `<` before no name is text (`1 < 2`), and what follows it read on.
        if name_len == 0 {
            i = at + 1;
            continue;
        }
        let name = b.get(name_at..name_at + name_len).unwrap_or_default();
        let end = tag_end(b, name_at + name_len);
        i = end.max(at + 1);
        let is = |tag: &[u8]| name.eq_ignore_ascii_case(tag);
        // What these hold is text: a footer named in it is no footer.
        if !close && ["script", "style", "textarea", "title", "xmp"].iter().any(|t| is(t.as_bytes())) {
            let mut closing = b"</".to_vec();
            closing.extend_from_slice(name);
            i = find_ignoring_case(b, i, &closing).unwrap_or(b.len());
            continue;
        }
        match (close, name) {
            (false, _) if is(b"table") => {
                if tables.len() < MAX_TABLES {
                    tables.push(Table::default());
                } else {
                    deeper += 1;
                }
            }
            (true, _) if is(b"table") => {
                if deeper > 0 {
                    deeper -= 1;
                } else if let Some(mut t) = tables.pop() {
                    t.end_foot(at);
                    if let Some(foot) = t.foot {
                        moves.push((foot, at));
                    }
                }
            }
            _ if is(b"tfoot") => {
                // Read as a row group wherever it ends up.
                if let Some(n) = bytes.get_mut(name_at..name_at + 5) {
                    n.copy_from_slice(b"tbody");
                }
                if let Some(t) = tables.last_mut().filter(|_| deeper == 0) {
                    if close {
                        t.end_foot(end);
                    } else {
                        // A footer begun ends the one open; only the first is the table's footer.
                        t.end_foot(at);
                        if !t.footed {
                            t.footed = true;
                            t.open = Some(at);
                        }
                    }
                }
            }
            // A row group begun ends the footer open, as a browser ends it.
            (false, _) if is(b"tbody") || is(b"thead") => {
                if let Some(t) = tables.last_mut().filter(|_| deeper == 0) {
                    t.end_foot(at);
                }
            }
            _ => {}
        }
    }
    // A table in another's footer goes with that footer, as it is.
    let mut spans: Vec<(usize, usize)> = moves.iter().map(|(foot, _)| *foot).collect();
    spans.sort_unstable();
    let mut outer: Vec<(usize, usize)> = Vec::new();
    for (s, e) in spans {
        match outer.last_mut() {
            Some(last) if s < last.1 => last.1 = last.1.max(e),
            _ => outer.push((s, e)),
        }
    }
    let inside = |at: usize| {
        let n = outer.partition_point(|(s, _)| *s <= at);
        n > 0 && outer.get(n - 1).is_some_and(|(_, e)| at < *e)
    };
    moves.retain(|(_, to)| !inside(*to));
    let mut cuts: Vec<(usize, usize)> = moves.iter().map(|(foot, _)| *foot).collect();
    cuts.sort_unstable();
    moves.sort_unstable_by_key(|(_, to)| *to);
    let mut out = Vec::with_capacity(bytes.len());
    let mut from = 0;
    let mut cut = cuts.iter().peekable();
    for ((s, e), to) in &moves {
        while let Some(&&(cs, ce)) = cut.peek().filter(|(cs, _)| cs < to) {
            out.extend_from_slice(bytes.get(from..cs).unwrap_or_default());
            from = ce;
            cut.next();
        }
        out.extend_from_slice(bytes.get(from..*to).unwrap_or_default());
        out.extend_from_slice(bytes.get(*s..*e).unwrap_or_default());
        from = *to;
    }
    for &(cs, ce) in cut {
        out.extend_from_slice(bytes.get(from..cs).unwrap_or_default());
        from = ce;
    }
    out.extend_from_slice(bytes.get(from..).unwrap_or_default());
    String::from_utf8(out).map_or(Cow::Borrowed(html), Cow::Owned)
}

/// Where the tag whose name ends at `from` ends, past its `>`: a `>` in a quoted value is the
/// value's (found in review: `title="a>b"` ended the tag there, and the rest of it was taken for
/// markup). One left open runs to the end, as a browser reads it.
fn tag_end(b: &[u8], from: usize) -> usize {
    let mut i = from;
    let mut after_equals = false;
    while let Some(&c) = b.get(i) {
        match c {
            b'>' => return i + 1,
            b'"' | b'\'' if after_equals => {
                i = b.get(i + 1..).and_then(|r| r.iter().position(|&q| q == c)).map_or(b.len(), |o| i + 1 + o);
                after_equals = false;
            }
            b'=' => after_equals = true,
            c if c.is_ascii_whitespace() => {}
            _ => after_equals = false,
        }
        i += 1;
    }
    b.len()
}

/// Where `what` is in `b` from `from`.
fn find(b: &[u8], from: usize, what: &[u8]) -> Option<usize> {
    b.get(from..)?.windows(what.len()).position(|w| w == what).map(|o| from + o)
}

/// [`find`], whatever the letters' case.
fn find_ignoring_case(b: &[u8], from: usize, what: &[u8]) -> Option<usize> {
    b.get(from..)?.windows(what.len()).position(|w| w.eq_ignore_ascii_case(what)).map(|o| from + o)
}

#[cfg(test)]
mod tests;
