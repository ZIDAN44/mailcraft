//! A signature given as HTML code (issue #11): a business card in two columns, the photo beside
//! the name, which the signature editor's text can't hold. The code goes into messages as written;
//! the editor shows its text, its tables' cells one after another.

use super::*;

/// Longest signature code kept (its pictures written in count).
pub const SIGNATURE_CODE_MAX: usize = 200_000 + SIGNATURE_PICTURES * (SIGNATURE_PICTURE_MAX / 3 * 4 + 64);

/// Elements left out of a signature's code with all they hold: nothing in a signature runs,
/// and nothing of the page it was saved in comes along.
const DROPPED: [&str; 9] = ["script", "iframe", "object", "embed", "form", "head", "title", "meta", "link"];

/// A signature's HTML code as it goes into messages: what's in its `<body>` when it has one (an
/// Outlook `.htm` signature, a page saved by a signature maker), without scripts, frames, forms
/// and the page's head, and without `on…` handlers and `javascript:` links. Its pictures stay as
/// written (`data:`, or on the web).
pub fn signature_code(html: &str) -> String {
    let html: String = html.chars().take(SIGNATURE_CODE_MAX).collect();
    let lower = html.to_ascii_lowercase();
    // Inside <body>, when there is one.
    let inner = lower
        .find("<body")
        .and_then(|b| lower.get(b..)?.find('>').map(|e| b + e + 1))
        .map(|start| {
            let end = lower.get(start..).and_then(|r| r.rfind("</body")).map_or(html.len(), |e| start + e);
            html.get(start..end).unwrap_or("")
        })
        .unwrap_or(&html);
    let mut out = String::with_capacity(inner.len());
    let lower = inner.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower.get(at..).and_then(|r| r.find('<')).map(|i| at + i) {
        out.push_str(inner.get(at..i).unwrap_or(""));
        // A comment (Outlook's conditional ones among them) goes whole.
        if lower.get(i..).is_some_and(|r| r.starts_with("<!--")) {
            at = lower.get(i..).and_then(|r| r.find("-->")).map_or(inner.len(), |e| i + e + 3);
            continue;
        }
        let Some(end) = lower.get(i..).and_then(|r| r.find('>')).map(|e| i + e + 1) else {
            at = inner.len();
            break;
        };
        let tag = inner.get(i..end).unwrap_or("");
        // Outlook's `<![if !vml]>` and `<![endif]>` around its pictures: the pictures stay.
        if tag.starts_with("<!") {
            at = end;
            continue;
        }
        let name: String =
            tag.trim_start_matches(['<', '/']).chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if DROPPED.contains(&name.as_str()) {
            at = end;
            // The element's content too, up to its end.
            if !tag.starts_with("</") && !tag.ends_with("/>") && !matches!(name.as_str(), "meta" | "link" | "embed") {
                let close = format!("</{name}");
                at = lower
                    .get(end..)
                    .and_then(|r| r.find(&close))
                    .and_then(|c| lower.get(end + c..)?.find('>').map(|e| end + c + e + 1))
                    .unwrap_or(inner.len());
            }
            continue;
        }
        out.push_str(&harmless_tag(tag));
        at = end;
    }
    out.push_str(inner.get(at..).unwrap_or(""));
    out.trim().to_string()
}

/// A tag without its `on…` attributes and with `javascript:` addresses emptied.
fn harmless_tag(tag: &str) -> String {
    let lower = tag.to_ascii_lowercase();
    if !lower.contains(" on") && !lower.contains("javascript:") && !lower.contains("\non") && !lower.contains("\ton") {
        return tag.to_string();
    }
    let mut out = String::with_capacity(tag.len());
    let chars: Vec<char> = tag.chars().collect();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let starts_attr = c.is_whitespace()
            && chars.get(i + 1).is_some_and(|c| c.eq_ignore_ascii_case(&'o'))
            && chars.get(i + 2).is_some_and(|c| c.eq_ignore_ascii_case(&'n'))
            && chars.get(i + 3).is_some_and(char::is_ascii_alphabetic);
        if !starts_attr {
            out.push(c);
            i += 1;
            continue;
        }
        // Skip the name, `=` and its value (quoted, or up to a space).
        i += 1;
        while chars.get(i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-') {
            i += 1;
        }
        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        if chars.get(i) == Some(&'=') {
            i += 1;
            while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                i += 1;
            }
            match chars.get(i) {
                Some(&q @ ('"' | '\'')) => {
                    i += 1;
                    while chars.get(i).is_some_and(|c| *c != q) {
                        i += 1;
                    }
                    i += 1;
                }
                _ => {
                    while chars.get(i).is_some_and(|c| !c.is_whitespace() && *c != '>') {
                        i += 1;
                    }
                }
            }
        }
    }
    while let Some(j) = out.to_ascii_lowercase().find("javascript:") {
        out.replace_range(j..j + "javascript:".len(), "#");
    }
    out
}

/// A signature saved as a web page (Outlook's `.htm` signatures, a signature maker's file) as
/// text: UTF-8, else the character set its `<meta>` names, else Windows' Western one (Outlook
/// saves in the computer's own).
pub fn signature_file(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim_start_matches('\u{feff}').to_string();
    }
    let head = String::from_utf8_lossy(bytes.get(..4096.min(bytes.len())).unwrap_or(&[])).to_ascii_lowercase();
    let named = head.find("charset=").map(|i| {
        let rest = head.get(i + 8..).unwrap_or("").trim_start_matches(['"', '\'']);
        rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect::<String>()
    });
    let decode = named
        .and_then(|n| mail_parser::decoders::charsets::map::charset_decoder(n.as_bytes()))
        .or_else(|| mail_parser::decoders::charsets::map::charset_decoder(b"windows-1252"));
    match decode {
        Some(d) => d(bytes),
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `html` with its pictures beside it (`src="Riley_files/image001.png"`, as Outlook saves a
/// signature's) given as `file:` addresses, for [`local_pictures`] to write in; others stay.
pub fn pictures_beside(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let (mut last, mut from) = (0, 0);
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        from = end + 1;
        let tag = html.get(start..end).unwrap_or("");
        let Some(src) = tag_attr(tag, "src") else { continue };
        let s = src.trim();
        // An address with a scheme (`data:`, `https:`, `cid:`, `file:`, `C:`) is left alone.
        let scheme = s.split_once(':').is_some_and(|(a, _)| !a.is_empty() && !a.contains(['/', '\\']));
        if s.is_empty() || scheme || s.starts_with("//") {
            continue;
        }
        let Some(at) = tag.find(src).map(|a| start + a) else { continue };
        out.push_str(html.get(last..at).unwrap_or(""));
        out.push_str("file:");
        out.push_str(s);
        last = at + src.len();
    }
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// A signature's code to read and edit: each picture written into it (`data:`, kilobytes of
/// letters) as `picture:N` instead; the pictures, in order, for [`code_with_pictures`].
pub fn code_without_pictures(code: &str) -> (String, Vec<String>) {
    let lower = code.to_ascii_lowercase();
    let mut out = String::with_capacity(code.len().min(1 << 16));
    let mut pictures: Vec<String> = Vec::new();
    let (mut last, mut from) = (0, 0);
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        from = end + 1;
        let tag = code.get(start..end).unwrap_or("");
        let Some(src) = tag_attr(tag, "src").filter(|s| s.trim().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:"))) else { continue };
        let Some(at) = tag.find(src).map(|a| start + a) else { continue };
        pictures.push(src.trim().to_string());
        out.push_str(code.get(last..at).unwrap_or(""));
        out.push_str(&format!("picture:{}", pictures.len()));
        last = at + src.len();
    }
    out.push_str(code.get(last..).unwrap_or(""));
    (out, pictures)
}

/// [`code_without_pictures`]' code with its pictures back where `picture:N` stands.
pub fn code_with_pictures(code: &str, pictures: &[String]) -> String {
    let mut out = code.to_string();
    // From the last, so `picture:1` doesn't take `picture:10`'s start.
    for (n, p) in pictures.iter().enumerate().rev() {
        for q in ['"', '\''] {
            out = out.replace(&format!("{q}picture:{}{q}", n + 1), &format!("{q}{p}{q}"));
        }
    }
    out
}

/// A signature's code as it goes into a message: its `data:` pictures as parts of the message,
/// each under `cid:signatureN.{tag}@mailcraft` (a `cid:` of this message's own, RFC 2392).
pub fn signature_code_parts(code: &str, tag: &str) -> (String, Vec<InlinePicture>) {
    let (mut html, mut pictures) = data_pictures(code);
    for p in &mut pictures {
        let own = format!("{}.{tag}@mailcraft", p.cid.trim_end_matches("@mailcraft"));
        html = html.replace(&format!("cid:{}\"", p.cid), &format!("cid:{own}\"")).replace(&format!("cid:{}'", p.cid), &format!("cid:{own}'"));
        p.cid = own;
    }
    (html, pictures)
}

/// Tables laid out as their cells one after another (a cell a block), for a signature's text: a
/// business card's photo, then its name and numbers, rather than a table drawn in lines.
pub(super) fn cells_in_turn(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(i) = lower.get(at..).and_then(|r| r.find('<')).map(|i| at + i) {
        out.push_str(html.get(at..i).unwrap_or(""));
        let Some(end) = lower.get(i..).and_then(|r| r.find('>')).map(|e| i + e + 1) else {
            at = i;
            break;
        };
        let tag = html.get(i..end).unwrap_or("");
        let closing = tag.starts_with("</");
        let name: String =
            tag.trim_start_matches(['<', '/']).chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        match name.as_str() {
            "table" | "tbody" | "thead" | "tfoot" | "tr" | "colgroup" | "col" => {}
            "td" | "th" if closing => out.push_str("</div>"),
            "td" | "th" => {
                out.push_str("<div");
                out.push_str(tag.get(1 + name.len()..).unwrap_or(">"));
            }
            _ => out.push_str(tag),
        }
        at = end;
    }
    out.push_str(html.get(at..).unwrap_or(""));
    out
}
