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
    harmless_code(&html, &DROPPED)
}

/// What's in `html`'s `<body>` (all of it when it has none), without the elements `dropped` names
/// and all they hold, comments, `on…` handlers and `javascript:` links.
pub(super) fn harmless_code(html: &str, dropped: &[&str]) -> String {
    let lower = html.to_ascii_lowercase();
    // Inside <body>, when there is one.
    let inner = lower
        .find("<body")
        .and_then(|b| crate::html::tag_end(lower.as_bytes(), b + 1).map(|e| e + 1))
        .map(|start| {
            let end = lower.get(start..).and_then(|r| r.rfind("</body")).map_or(html.len(), |e| start + e);
            html.get(start..end).unwrap_or("")
        })
        .unwrap_or(html);
    let mut out = String::with_capacity(inner.len());
    let lower = inner.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower.get(at..).and_then(|r| work::scanned(r, r.find('<'))).map(|i| at + i) {
        out.push_str(inner.get(at..i).unwrap_or(""));
        // A comment (Outlook's conditional ones among them) goes whole.
        if lower.get(i..).is_some_and(|r| r.starts_with("<!--")) {
            at = lower.get(i..).and_then(|r| work::scanned(r, r.find("-->"))).map_or(inner.len(), |e| i + e + 3);
            continue;
        }
        // A `<` no name follows is text to a browser, and none of a tag here: read as one
        // (`<3 a="> <script>…">`), the script a browser finds in it went out with it.
        if !lower.as_bytes().get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'/' | b'!' | b'?')) {
            out.push('<');
            at = i + 1;
            continue;
        }
        // Its end as a browser finds it: a `>` in a quoted value isn't it (`<img alt="a>b"
        // onerror=…>` read as a tag ending at `a>` let its handler through as text).
        let Some(end) = inner.get(i..).and_then(tag_end).map(|e| i + e) else {
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
        // `<noscript>` goes too: a browser that runs scripts reads what's in it as text, one that
        // doesn't as tags, and no one reading is safe for both.
        if dropped.contains(&name.as_str()) || name == "noscript" {
            at = end;
            // The element's content too, up to its end.
            if !tag.starts_with("</") && !tag.ends_with("/>") && !matches!(name.as_str(), "meta" | "link" | "embed") {
                let close = format!("</{name}");
                at = lower
                    .get(end..)
                    .and_then(|r| work::scanned(r, r.find(&close)))
                    .and_then(|c| lower.get(end + c..).and_then(|r| work::scanned(r, r.find('>'))).map(|e| end + c + e + 1))
                    .unwrap_or(inner.len());
            }
            continue;
        }
        out.push_str(&harmless_tag(tag));
        at = end;
        // What such an element holds is text to a browser, up to its end tag: read as tags
        // (`<style><p a="</style><script>…">`), the script after that end tag went out inside
        // a quoted value. It goes as written, less the `<` of anything a tag could start with
        // (inside an `<svg>` a browser reads tags there).
        if !tag.starts_with("</") && RAW_TEXT.contains(&name.as_str()) {
            let close = raw_text_end(&lower, end, &name);
            let text = inner.get(end..close).unwrap_or("");
            work::steps(text.len());
            let b = text.as_bytes();
            out.extend(
                text.char_indices()
                    .filter(|(n, c)| !(*c == '<' && b.get(n + 1).is_some_and(|d| d.is_ascii_alphabetic() || *d == b'/')))
                    .map(|(_, c)| c),
            );
            at = close;
        }
    }
    out.push_str(inner.get(at..).unwrap_or(""));
    out.trim().to_string()
}

/// Elements whose content a browser reads as text, not tags, up to their end tag.
const RAW_TEXT: [&str; 6] = ["style", "textarea", "xmp", "noembed", "noframes", "plaintext"];

/// Where the content of the raw-text element `name` whose tag ends at `from` ends in `lower`:
/// at its end tag as a browser finds it (`</name` and a space, `/` or `>`), or with the code.
fn raw_text_end(lower: &str, from: usize, name: &str) -> usize {
    let close = format!("</{name}");
    let mut at = from;
    while let Some(c) = lower.get(at..).and_then(|r| work::scanned(r, r.find(&close))).map(|c| at + c) {
        if lower.as_bytes().get(c + close.len()).is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>')) {
            return c;
        }
        at = c + close.len();
    }
    lower.len()
}

/// A tag without its `on…` attributes and with `javascript:` addresses emptied. Its attributes
/// are read as a browser reads them ([`crate::html::attributes`]): a handler after a quoted value with no
/// space (`src="x"onerror=…`), or after a `/` (`<img/onerror=…>`), is one; and an address whose
/// scheme comes out as `javascript:` or `vbscript:` once its character references are read and
/// the spaces a browser drops are gone (`java&#x09;script:`) goes whole. Every `javascript:`
/// written as it is is then emptied, as before.
fn harmless_tag(tag: &str) -> String {
    let (name_end, attrs) = crate::html::attributes(tag);
    let value = |a: &crate::html::Attribute| tag.get(a.value.clone()).unwrap_or("");
    let out = if tag.starts_with("</") || !attrs.iter().any(|a| a.name.starts_with("on") || hidden_script(value(a))) {
        tag.to_string()
    } else {
        let mut out = String::with_capacity(tag.len());
        out.push_str(tag.get(..name_end).unwrap_or(""));
        for a in attrs.iter().filter(|a| !a.name.starts_with("on")) {
            out.push(' ');
            if hidden_script(value(a)) {
                out.push_str(tag.get(a.span.start..a.name_end).unwrap_or(""));
                out.push_str("=\"#\"");
            } else {
                out.push_str(tag.get(a.span.clone()).unwrap_or(""));
            }
        }
        out.push_str(if tag.trim_end_matches('>').trim_end().ends_with('/') { " />" } else { ">" });
        out
    };
    // Each `javascript:` emptied in one pass (`#` can't make another): lowering the tag again and
    // moving its tail for each one was quadratic in a tag full of them.
    const JS: &str = "javascript:";
    let lower = out.to_ascii_lowercase();
    work::steps(out.len());
    let mut emptied = String::with_capacity(out.len());
    let mut at = 0;
    while let Some(j) = lower.get(at..).and_then(|r| work::scanned(r, r.find(JS))).map(|j| at + j) {
        emptied.push_str(out.get(at..j).unwrap_or(""));
        emptied.push('#');
        at = j + JS.len();
    }
    emptied.push_str(out.get(at..).unwrap_or(""));
    emptied
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
    // A file that names UTF-8 but isn't (it failed above) was saved in Windows' own.
    let charset = named
        .and_then(|n| mail_parser::Charset::from_label(n.as_bytes()))
        .filter(|c| *c != mail_parser::Charset::Utf8)
        .unwrap_or(mail_parser::Charset::Windows1252);
    charset.decode(bytes).into_owned()
}

/// `html` with its pictures beside it (`src="Riley_files/image001.png"`, as Outlook saves a
/// signature's) given as `file:` addresses, for [`local_pictures`] to write in; others stay.
pub fn pictures_beside(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let (mut last, mut from) = (0, 0);
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = crate::html::tag_end(lower.as_bytes(), start + 1) else { break };
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
        let Some(end) = crate::html::tag_end(lower.as_bytes(), start + 1) else { break };
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

/// The end of every signature picture's `cid:`.
const SIGNATURE_CID_END: &str = "@mailcraft";

/// The `cid:` of a signature's picture `n` (from 1) as the signature has it:
/// `signatureN@mailcraft`.
pub fn signature_cid(n: usize) -> String {
    format!("signature{n}{SIGNATURE_CID_END}")
}

/// A signature's picture `cid` ([`signature_cid`]) as it goes into one message, whose `tag` it
/// takes: `signatureN.{tag}@mailcraft`, a `cid:` of that message's own (RFC 2392).
pub fn message_signature_cid(cid: &str, tag: &str) -> String {
    format!("{}.{tag}{SIGNATURE_CID_END}", cid.strip_suffix(SIGNATURE_CID_END).unwrap_or(cid))
}

/// The tag of a signature's picture in a message ([`message_signature_cid`]), or `None` for any
/// other `cid:` (another sender's `signature.png@…` is theirs, not a signature's put in here).
pub fn signature_cid_tag(cid: &str) -> Option<&str> {
    let (n, tag) = cid.strip_prefix("signature")?.strip_suffix(SIGNATURE_CID_END)?.split_once('.')?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !tag.is_empty() && !tag.contains('@')).then_some(tag)
}

/// A signature's code as it goes into a message: its `data:` pictures as parts of the message,
/// each under `cid:signatureN.{tag}@mailcraft` (a `cid:` of this message's own, RFC 2392).
pub fn signature_code_parts(code: &str, tag: &str) -> (String, Vec<InlinePicture>) {
    let (mut html, mut pictures) = data_pictures(code);
    for p in &mut pictures {
        let own = message_signature_cid(&p.cid, tag);
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
        let Some(end) = crate::html::tag_end(lower.as_bytes(), i + 1).map(|e| e + 1) else {
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

/// Where the tag starting `html` ends (past its `>`), as a browser finds it
/// ([`crate::html::tag_end`]). `None`: it never ends.
fn tag_end(html: &str) -> Option<usize> {
    crate::html::tag_end(html.as_bytes(), 1).map(|e| e + 1)
}

/// Whether an address hides a script a browser would run where `javascript:` written as it is
/// doesn't show it: its scheme, once its character references are read and the spaces and
/// control characters a browser drops are gone, is `javascript:` or `vbscript:`.
fn hidden_script(value: &str) -> bool {
    let mut scheme = String::new();
    let mut rest = value;
    while scheme.len() < 16 {
        let Some(c) = rest.chars().next() else { break };
        let (c, len) = if c == '&' { reference(rest).unwrap_or(('&', 1)) } else { (c, c.len_utf8()) };
        rest = rest.get(len..).unwrap_or("");
        if c == ':' {
            let plain = value.trim_start().to_ascii_lowercase().starts_with("javascript:");
            return scheme == "vbscript" || (scheme == "javascript" && !plain);
        }
        if !(c.is_ascii_whitespace() || c.is_ascii_control()) {
            scheme.push(c.to_ascii_lowercase());
        }
    }
    false
}

/// The character a reference at the start of `s` stands for, and how long it is: numbered ones
/// (with or without their `;`, as a browser reads them) and those a scheme can hide behind.
fn reference(s: &str) -> Option<(char, usize)> {
    let after = s.get(1..)?;
    for (name, c) in [("Tab;", '\t'), ("NewLine;", '\n'), ("colon;", ':')] {
        if after.starts_with(name) {
            return Some((c, 1 + name.len()));
        }
    }
    let num = after.strip_prefix('#')?;
    let (hex, digits) = match num.strip_prefix(['x', 'X']) {
        Some(h) => (true, h),
        None => (false, num),
    };
    // Every digit there is, as a browser takes them (`&#00000000106;` is a `j`: cut at eight, it
    // hid a `javascript:`); a number too large for a character is U+FFFD, as there.
    let n = digits.bytes().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
    work::steps(n);
    if n == 0 {
        return None;
    }
    let written = digits.get(..n)?.trim_start_matches('0');
    let code = match written.len() {
        0 => 0,
        1..=8 => u32::from_str_radix(written, if hex { 16 } else { 10 }).unwrap_or(u32::MAX),
        _ => u32::MAX,
    };
    let semi = usize::from(digits.get(n..).is_some_and(|r| r.starts_with(';')));
    Some((char::from_u32(code).unwrap_or('\u{fffd}'), 2 + usize::from(hex) + n + semi))
}
