//! HTML mail as text and as rich text: pasted and received HTML, its pictures, headings, links.

use super::*;

/// Most lines kept of a rich rendering (the rest is shown as text).
pub(super) const RICH_MAX_LINES: usize = 20_000;

/// Word's and Outlook's paragraphs (`class=MsoNormal`) have no space between them (their style
/// sheet, which a copied fragment leaves behind, says so): as lines, not paragraphs.
pub(super) fn word_paragraphs(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("mso") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|rest| rest.find('<')) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        let tag = lower.get(at..).unwrap_or("");
        let after = |n: usize| tag.as_bytes().get(n).is_none_or(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/'));
        if tag.starts_with("<p") && after(2) {
            out.push_str("<div");
            i = at + 2;
        } else if tag.starts_with("</p") && after(3) {
            out.push_str("</div");
            i = at + 3;
        } else {
            out.push('<');
            i = at + 1;
        }
    }
    out.push_str(html.get(i..).unwrap_or(""));
    out
}

/// HTML pasted from another program (a signature copied from Outlook or Word, part of a web
/// page) as text and styles to put into a message: its formatting and links, without pictures
/// (they'd point at the other program's files or the web) or the empty lines around it.
pub fn pasted_html(html: &str) -> (String, rich::Styles) {
    let (marked, fonts) = fonts::mark(&word_paragraphs(html));
    let mut lines = html_to_rich(&marked);
    for line in &mut lines {
        line.retain(|sp| sp.image.is_none());
    }
    let blank = |l: &Vec<Span>| l.iter().all(|sp| sp.text.trim().is_empty());
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !blank(l)).unwrap_or(lines.len());
    lines.drain(..first);
    let (text, styles) = rich::from_rich(&lines);
    fonts::apply(&text, &styles, &fonts)
}

/// HTML → lines of formatted spans: paragraphs, emphasis, links, list bullets, quotes, tables
/// laid out as text, and pictures by reference. No scripts, styles or remote loads: what comes
/// out is plain data. Never panics; empty when conversion fails.
pub fn html_to_rich(html: &str) -> RichText {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    // Wide enough that paragraphs stay one line each (the UI wraps them to the pane).
    let marked = close_css_rules(&mark_pictures(&mark_headings(html)));
    // The message's own CSS is read for colours and hidden parts (`display: none`), nothing else.
    let result = std::panic::catch_unwind(|| html2text::config::rich().use_doc_css().lines_from_read(marked.as_bytes(), 2000));
    let Ok(Ok(lines)) = result else { return Vec::new() };
    let mut out: RichText = Vec::new();
    let mut blank = 0;
    for line in lines.into_iter().take(RICH_MAX_LINES) {
        let mut spans: Vec<Span> = Vec::new();
        for ts in line.tagged_strings() {
            let mut sp = Span { text: ts.s.clone(), ..Default::default() };
            for a in &ts.tag {
                match a {
                    html2text::render::RichAnnotation::Strong => sp.bold = true,
                    html2text::render::RichAnnotation::Emphasis => sp.italic = true,
                    html2text::render::RichAnnotation::Strikeout => sp.strike = true,
                    html2text::render::RichAnnotation::Code | html2text::render::RichAnnotation::Preformat(_) => sp.code = true,
                    html2text::render::RichAnnotation::Link(l) => sp.link = Some(l.clone()),
                    html2text::render::RichAnnotation::Image(src) => sp.image = Some(src.clone()),
                    html2text::render::RichAnnotation::Colour(c) => sp.color = Some([c.r, c.g, c.b]),
                    html2text::render::RichAnnotation::BgColour(c) => sp.background = Some([c.r, c.g, c.b]),
                    _ => {}
                }
            }
            // Merge with the previous span when the formatting is the same.
            match spans.last_mut() {
                Some(last) if Span { text: String::new(), ..last.clone() } == Span { text: String::new(), ..sp.clone() } => {
                    last.text.push_str(&sp.text)
                }
                _ => spans.push(sp),
            }
        }
        if let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
        }
        // Headings come out as `## <mark>Title` (see `mark_headings`): drop both, keep the level.
        let mut level = 0u8;
        for sp in &mut spans {
            if let Some(i) = sp.text.find(HEADING_MARK) {
                let before = sp.text.get(..i).unwrap_or("");
                level = u8::try_from(before.chars().filter(|c| *c == '#').count().clamp(1, 6)).unwrap_or(1);
                sp.text = sp.text.get(i + HEADING_MARK.len_utf8()..).unwrap_or("").to_string();
            }
        }
        if level > 0 {
            if let Some(first) = spans.first_mut()
                && first.text.trim_start_matches('#').trim().is_empty()
                && first.image.is_none()
            {
                first.text.clear();
            }
            for sp in &mut spans {
                sp.heading = level;
            }
        }
        for sp in &mut spans {
            sp.text = sp.text.replace([HEADING_MARK, PICTURE_MARK], "");
        }
        // List bullets (`* item`, indented when nested) as real bullets.
        if let Some(first) = spans.first_mut() {
            let indent = first.text.len() - first.text.trim_start_matches(' ').len();
            if first.text.get(indent..).is_some_and(|r| r.starts_with("* ")) {
                first.text = format!("{}• {}", " ".repeat(indent), first.text.get(indent + 2..).unwrap_or(""));
            }
        }
        // Tables come laid out with box-drawing borders. A layout table (one column) only adds
        // rules around its text: drop those. A data table's rows line up only in a monospace font.
        let whole: String = spans.iter().map(|s| s.text.as_str()).collect();
        if !whole.trim().is_empty() && whole.chars().all(|c| matches!(c, '─' | '━' | '═' | ' ')) {
            spans.clear();
        } else if whole.contains(['│', '┼', '┬', '┴']) {
            for sp in &mut spans {
                sp.code = true;
            }
        }
        spans.retain(|s| !s.text.is_empty() || s.image.is_some());
        if spans.is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push(spans);
    }
    while out.last().is_some_and(Vec::is_empty) {
        out.pop();
    }
    out
}

/// Marks where a heading starts, so a paragraph that just begins with `#` isn't taken for one.
pub(super) const HEADING_MARK: char = '\u{E000}';

/// The description given to pictures that have none (a report's charts, a logo), which the
/// converter would otherwise leave out: removed again once they're pictures.
pub(super) const PICTURE_MARK: char = '\u{E001}';

/// An attribute's value in a tag's text (`<img alt="x" …>`), as written; `Some("")` when it's
/// there and empty.
pub(super) fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find(name)).map(|i| i + from) {
        from = i + name.len();
        let before = lower.as_bytes().get(i.wrapping_sub(1)).copied().unwrap_or(b' ');
        if !before.is_ascii_whitespace() || i == 0 {
            continue;
        }
        let after = lower.get(from..).unwrap_or("").trim_start();
        if !after.starts_with('=') {
            // A bare attribute (`alt`): there, and empty.
            if after.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/') || after.is_empty() {
                return Some("");
            }
            continue;
        }
        let at = tag.len() - after.len() + 1;
        let v = tag.get(at..).unwrap_or("").trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v.get(1..).and_then(|r| r.split(q).next()).unwrap_or(""),
            _ => v.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or(""),
        });
    }
    None
}

/// `html` with a description on every picture that has none, except tracking pixels (1 by 1).
pub(super) fn mark_pictures(html: &str) -> String {
    let html = html.replace(PICTURE_MARK, "");
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        let tag = html.get(start..end).unwrap_or("");
        let tiny = |a: &str| tag_attr(tag, a).is_some_and(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().is_ok_and(|n| n <= 2.0));
        let described = tag_attr(tag, "alt").is_some_and(|a| !a.trim().is_empty());
        if !described && !tiny("width") && !tiny("height") && tag_attr(tag, "src").is_some_and(|s| !s.trim().is_empty()) {
            out.push_str(html.get(last..start + 4).unwrap_or(""));
            out.push_str(&format!(" alt=\"{PICTURE_MARK}\""));
            last = start + 4;
        }
        from = end + 1;
    }
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// The attachment a picture of the HTML is, by its address: `cid:` (RFC 2392, %-escapes
/// decoded) matched to a `Content-ID`, or to a file name when there's no such id; any other
/// address that isn't on the web (`logo.png`, `images/chart.png`) matched to a part's
/// `Content-Location` (RFC 2557) or its file name, as Outlook finds them.
pub fn picture_attachment(p: &Parsed, src: &str) -> Option<usize> {
    let src = src.trim();
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") || src.is_empty() || src.len() > 4096 {
        return None;
    }
    let decoded = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while let Some(&c) = b.get(i) {
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            match (c, hex) {
                (b'%', Some(v)) => {
                    out.push(v);
                    i += 3;
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let last_part = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_string();
    let name_is = |a: &AttachmentInfo, n: &str| !n.is_empty() && a.name.eq_ignore_ascii_case(n);
    if let Some(cid) = src.get(..4).filter(|s| s.eq_ignore_ascii_case("cid:")).and_then(|_| src.get(4..)) {
        let cid = cid.trim().trim_matches(['<', '>']);
        let plain = decoded(cid);
        let by_id = |id: &str| p.attachments.iter().find(|a| a.content_id.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(id)));
        return by_id(cid)
            .or_else(|| by_id(&plain))
            .or_else(|| p.attachments.iter().find(|a| name_is(a, &plain)))
            .or_else(|| p.attachments.iter().find(|a| a.content_location.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(&plain))))
            .map(|a| a.index);
    }
    let path = decoded(src.split(['?', '#']).next().unwrap_or(src));
    let file = last_part(&path);
    p.attachments
        .iter()
        .find(|a| a.content_location.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(&path) || last_part(l).eq_ignore_ascii_case(&file)))
        .or_else(|| p.attachments.iter().find(|a| name_is(a, &file) || a.content_id.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&file))))
        .map(|a| a.index)
}

/// End every CSS declaration block in `<style>` elements with a `;` (the last declaration may
/// lack one, which html2text's parser would otherwise skip with the whole rule), and every
/// `<style>` with an empty rule: html2text 0.17 panics on a style sheet that stops inside an
/// attribute selector (`[s=`), which a cut-off or hostile message can end with.
pub(super) fn close_css_rules(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut at = 0;
    while let Some(start) = lower.get(at..).and_then(|r| r.find("<style")).map(|i| i + at) {
        // An unclosed `<style>` runs to the end of the document.
        let end = lower.get(start..).and_then(|r| r.find("</style")).map_or(html.len(), |i| i + start);
        out.push_str(html.get(at..start).unwrap_or(""));
        out.push_str(&html.get(start..end).unwrap_or("").replace('}', ";}"));
        out.push_str("\n*{}");
        at = end;
    }
    out.push_str(html.get(at..).unwrap_or(""));
    out
}

/// `html` with a mark after every `<h1>`…`<h6>` start tag (and any mark already in it removed).
pub(super) fn mark_headings(html: &str) -> String {
    let html = html.replace(HEADING_MARK, "");
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<h")) {
        let start = from + i;
        let is_heading = lower.as_bytes().get(start + 2).is_some_and(|b| (b'1'..=b'6').contains(b))
            && lower.as_bytes().get(start + 3).is_some_and(|b| *b == b'>' || b.is_ascii_whitespace());
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        if is_heading {
            out.push_str(html.get(last..=end).unwrap_or(""));
            out.push(HEADING_MARK);
            last = end + 1;
        }
        from = end + 1;
    }
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// Web pictures a rich message refers to (`http:`/`https:` only), in order, without repeats.
pub fn remote_images(rich: &RichText) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sp in rich.iter().flatten() {
        if let Some(src) = &sp.image
            && (src.starts_with("https://") || src.starts_with("http://"))
            && !out.contains(src)
        {
            out.push(src.clone());
        }
    }
    out
}

/// Every `<img src>` of an HTML body, in order, without repeats (at most 200). Pictures without
/// a description (tracking pixels, mostly) leave no trace in the formatted text, so they are
/// found here, to be blocked or shown like the rest.
pub fn html_images(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let end = lower.get(start..).and_then(|r| r.find('>')).map_or(lower.len(), |e| start + e);
        let tag_lower = lower.get(start..end).unwrap_or("");
        let tag = html.get(start..end).unwrap_or("");
        if let Some(p) = tag_lower.find(" src=").or_else(|| tag_lower.find("\nsrc=")) {
            let v = tag.get(p + 5..).unwrap_or("");
            let src = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v.get(1..).and_then(|r| r.split(q).next()).unwrap_or(""),
                _ => v.split([' ', '\t', '\n', '/']).next().unwrap_or(""),
            };
            let src = src.trim().replace("&amp;", "&");
            if !src.is_empty() && src.len() < 4096 && !out.contains(&src) {
                out.push(src);
            }
        }
        from = end.max(start + 4);
        if out.len() >= 200 {
            break;
        }
    }
    out
}

/// HTML → readable text. Never panics; returns an empty string when conversion fails.
pub fn html_to_text(html: &str) -> String {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let result = std::panic::catch_unwind(|| html2text::from_read(html.as_bytes(), 400));
    match result {
        Ok(Ok(s)) => tidy_text(&s),
        _ => String::new(),
    }
}

/// The web addresses an HTML body links to (`href`), once each and in order: whole, however
/// long, where [`html_to_text`] keeps them as footnotes it wraps at its line width (a long Teams
/// join link came out cut). Only `http` and `https`; bounded in what it reads and returns.
pub fn html_links(html: &str) -> Vec<String> {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while out.len() < 50 {
        let Some(rel) = lower.get(at..).and_then(|r| r.find("href")) else { break };
        let mut i = at + rel + 4;
        at = i;
        let rest = html.get(i..).unwrap_or("");
        let trimmed = rest.trim_start();
        let Some(after_eq) = trimmed.strip_prefix('=') else { continue };
        i += rest.len() - trimmed.len() + 1;
        let value = after_eq.trim_start();
        i += after_eq.len() - value.len();
        let (quote, body) = match value.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), value.get(1..).unwrap_or("")),
            _ => (None, value),
        };
        let end = match quote {
            Some(q) => body.find(q),
            None => body.find(|c: char| c.is_whitespace() || c == '>'),
        }
        .unwrap_or(body.len());
        at = i + end.max(1);
        let raw = body.get(..end).unwrap_or("").trim();
        if raw.len() > 8192 {
            continue;
        }
        let url = raw.replace("&amp;", "&").replace("&#38;", "&").replace("&quot;", "\"").replace("&#39;", "'");
        let lower_url = url.to_ascii_lowercase();
        if (lower_url.starts_with("https://") || lower_url.starts_with("http://")) && !url.contains(char::is_whitespace) && !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// Trim trailing spaces and collapse runs of blank lines.
pub(super) fn tidy_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let l = line.trim_end();
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim_matches('\n').to_string()
}
