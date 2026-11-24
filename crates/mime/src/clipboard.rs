//! HTML copied on Windows: its clipboard's "HTML Format" (CF_HTML) is a header of byte offsets,
//! `StartHTML`, `EndHTML`, `StartFragment`, `EndFragment`, then the page. What was copied is the
//! fragment; the page around it is its context. A program that copies part of a page puts what
//! it needs inside the fragment, but Excel's fragment is the rows alone: the `<table>` they're
//! in, and the style sheet with their lines and colours, are outside it (issue #268).

/// Most lines of the header read: it has seven at most, and the page starts after them.
const HEADER_LINES: usize = 16;

/// What was copied, from the clipboard's "HTML Format" bytes: the fragment, or the whole page when
/// the fragment is part of a table that opens before it (cells copied in Excel), so the table
/// and its style sheet come with it. `None` when it holds no HTML. Its offsets are those of the
/// program that copied: one past the end, or a fragment outside the page, is the page alone.
pub fn windows_clipboard_html(raw: &[u8]) -> Option<String> {
    let mut end = raw.len();
    while end > 0 && raw.get(end - 1) == Some(&0) {
        end -= 1;
    }
    let raw = raw.get(..end)?;
    let (mut page, mut fragment) = ((None, None), (None, None));
    let mut any = false;
    for line in raw.split(|&b| b == b'\n').take(HEADER_LINES) {
        let line = String::from_utf8_lossy(line);
        let line = line.trim();
        if line.starts_with('<') {
            break;
        }
        let Some((key, value)) = line.split_once(':') else { break };
        let at = value.trim().parse::<usize>().ok();
        match key.trim() {
            "StartHTML" => page.0 = at,
            "EndHTML" => page.1 = at,
            "StartFragment" => fragment.0 = at,
            "EndFragment" => fragment.1 = at,
            _ => {}
        }
        any = true;
    }
    if !any {
        // No header: HTML as it is.
        return text(raw).filter(|t| t.trim_start().starts_with('<'));
    }
    let piece = |(from, to): (Option<usize>, Option<usize>)| {
        let from = from?;
        raw.get(from..to.unwrap_or(raw.len())).map(|bytes| (from, bytes))
    };
    let page = piece(page);
    let fragment = piece(fragment);
    match (page, fragment) {
        (Some((page_at, whole)), Some((fragment_at, part))) => {
            let before = fragment_at.checked_sub(page_at).and_then(|n| whole.get(..n));
            if before.is_some_and(opens_table) && fragment_at + part.len() <= page_at + whole.len() { text(whole) } else { text(part) }
        }
        (None, Some((_, part))) => text(part),
        (Some((_, whole)), None) => text(whole),
        (None, None) => None,
    }
}

fn text(bytes: &[u8]) -> Option<String> {
    let t = String::from_utf8_lossy(bytes).into_owned();
    (!t.trim().is_empty()).then_some(t)
}

/// Whether more tables open in `html` than close: what follows it is inside a table.
fn opens_table(html: &[u8]) -> bool {
    let lower = html.to_ascii_lowercase();
    let count = |tag: &[u8]| {
        lower
            .windows(tag.len() + 1)
            .filter(|w| w.starts_with(tag) && w.last().is_some_and(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/'))
            .count()
    };
    count(b"<table") > count(b"</table")
}

#[cfg(test)]
mod tests;
