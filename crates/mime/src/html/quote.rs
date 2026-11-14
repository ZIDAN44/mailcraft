//! The original under a reply or forward as it goes out: its own HTML after the reply's text, as
//! Outlook sends it, so its columns, colours and pictures come through as they were.

use super::*;

/// The class of the `<div>` the original goes out in, after the reply's own text: how a message
/// saved or taken back to change finds it again ([`split_marked`]).
pub const QUOTE_CLASS: &str = "mailcraft-quote";

/// The class of the `<div>` a signature written as HTML code goes out in, in its place in the
/// text ([`split_marked`]).
pub const SIGNATURE_CLASS: &str = "mailcraft-signature";

/// HTML MailCraft wrote with a part marked `class` in it (the original under a reply, a
/// signature's code): the HTML before it, the part's own and what follows it. `None` when there's
/// none. The part ends at the `</div>` that closes its `<div>`, the ones in it counted.
pub fn split_marked(html: &str, class: &str) -> Option<(String, String, String)> {
    let mark = format!("<div class=\"{class}\">");
    let start = html.find(&mark)?;
    let inner = start + mark.len();
    let lower = html.to_ascii_lowercase();
    let (mut depth, mut at) = (1usize, inner);
    let end = loop {
        let rest = lower.get(at..)?;
        let open = rest.find("<div").map(|i| at + i);
        let close = rest.find("</div").map(|i| at + i);
        match (open, close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                at = o + 4;
            }
            (_, Some(c)) => {
                depth -= 1;
                if depth == 0 {
                    break c;
                }
                at = c + 5;
            }
            // Never closed: it runs to the end.
            (_, None) => break html.len(),
        }
    };
    let after = html.get(end..).map(|r| r.find('>').map_or("", |e| r.get(e + 1..).unwrap_or(""))).unwrap_or("");
    Some((html.get(..start)?.to_string(), html.get(inner..end)?.to_string(), after.to_string()))
}

/// `p` (a message as parsed) with `html`, a part of its HTML (the original under a reply it
/// sent), as its formatted text: that part's layout, lines, pictures and text.
pub fn with_html(p: &Parsed, html: &str) -> Parsed {
    let h = layout::inline_css(html);
    let layout = layout::html_layout(&h);
    let mut images = html_images(&h);
    for src in layout::background_pictures(&layout) {
        if !images.contains(&src) && images.len() < 200 {
            images.push(src);
        }
    }
    Parsed { rich: html_to_rich(&h), layout, images, text: html_to_text(&h), had_html: true, ..p.clone() }
}

/// Elements left out of a quoted message with all they hold: what runs or sends, and its head
/// and style sheets (their rules are written into its tags first, so they don't restyle the reply).
const QUOTE_DROPPED: [&str; 11] = ["script", "iframe", "object", "embed", "form", "head", "title", "meta", "link", "base", "style"];

/// `html` (a message's body, as written) as it goes under a reply or forward: its style sheet's
/// rules written into its tags, then what's in its `<body>` without scripts, frames, forms, its
/// head and `on…` handlers. Each picture it shows from the message goes by the reply's own `cid:`
/// (`given`: the picture's address as written, or as the converter gave it, and its `cid:`); a
/// tracking picture (`trackers`) goes; one on the web stays at its address, as Outlook keeps it.
pub fn quote_html(html: &str, given: &[(String, String)], trackers: &[String]) -> String {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let code = harmless_code(&layout::inline_css(html), &QUOTE_DROPPED);
    let lower = code.to_ascii_lowercase();
    let mut out = String::with_capacity(code.len());
    let (mut last, mut from) = (0, 0);
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = crate::html::tag_end(lower.as_bytes(), start + 1).map(|e| e + 1) else { break };
        from = end;
        let tag = code.get(start..end).unwrap_or("");
        let Some(src) = tag_attr(tag, "src") else { continue };
        let plain = src.trim().replace("&amp;", "&");
        let is = |s: &String| *s == plain || s.trim() == src.trim();
        if trackers.iter().any(is) {
            out.push_str(code.get(last..start).unwrap_or(""));
            last = end;
        } else if let Some((_, cid)) = given.iter().find(|(s, _)| is(s))
            && let Some(at) = tag.find(src).map(|a| start + a)
        {
            out.push_str(code.get(last..at).unwrap_or(""));
            out.push_str(&format!("cid:{}", cid.replace(['"', '\'', '<', '>'], "")));
            last = at + src.len();
        }
    }
    out.push_str(code.get(last..).unwrap_or(""));
    // Pictures behind boxes, by the same `cid:`s (one kept under its own `cid:` needs none; of
    // two given for one address, the first).
    let mut behind: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for (src, cid) in given {
        let own = format!("cid:{cid}");
        if src.trim().is_empty() || *src == own || src.contains(['"', '\'', '(', ')']) {
            continue;
        }
        behind.entry(src.as_str()).or_insert_with(|| own.replace(['"', '\'', '<', '>', '(', ')'], ""));
    }
    if behind.is_empty() { out } else { pictures_behind(&out, &behind) }
}

/// `html` with each picture behind a box (`background="…"` or `'…'`, `url(…)`, `url('…')` or
/// `url("…")`) that `behind` has an address for given that one instead, in one pass: each
/// address looked up as it comes (a pass over the whole message for each picture and each way
/// of writing one took 0.17 s for a 2 MB newsletter with 100 of them).
fn pictures_behind(html: &str, behind: &std::collections::HashMap<&str, String>) -> String {
    const OPENERS: [&str; 2] = ["background=", "url("];
    let next = |from: usize, opener: &str| html.get(from..).and_then(|r| r.find(opener)).map(|i| from + i);
    let mut found = [next(0, OPENERS[0]), next(0, OPENERS[1])];
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    // The nearest opener still to come, each in turn.
    while let Some((which, at)) = found.iter().enumerate().filter_map(|(w, f)| f.map(|at| (w, at))).min_by_key(|(_, at)| *at) {
        let url = which == 1;
        let open = at + OPENERS.get(which).map_or(0, |o| o.len());
        let quote = match html.as_bytes().get(open) {
            Some(b'"') => Some('"'),
            Some(b'\'') => Some('\''),
            _ => None,
        };
        let start = open + usize::from(quote.is_some());
        // Where the address ends: its quote (and `)` for `url(`), or `)` for an unquoted `url(`;
        // `background=` without a quote isn't one of them.
        let close = match (quote, url) {
            (Some(q), true) => Some(format!("{q})")),
            (Some(q), false) => Some(q.to_string()),
            (None, true) => Some(")".to_string()),
            (None, false) => None,
        };
        let end = close.and_then(|c| html.get(start..)?.find(&c)).map(|e| start + e);
        if let Some(end) = end
            && let Some(own) = html.get(start..end).and_then(|a| behind.get(a))
        {
            out.push_str(html.get(last..start).unwrap_or(""));
            out.push_str(own);
            last = end;
        }
        // On past this one, and to the next of its kind.
        if let Some(f) = found.get_mut(which) {
            *f = next(end.unwrap_or(open).max(open), OPENERS.get(which).copied().unwrap_or(""));
        }
    }
    out.push_str(html.get(last..).unwrap_or(""));
    out
}
