//! MailCraft MIME handling.
//!
//! * [`summary_from_header`] turns a header block (what sync downloads for every message) into
//!   the message-list [`Summary`].
//! * [`parse`] turns a full message into what the reading pane shows: text (HTML converted to
//!   plain text, so no remote content is ever loaded), the attachment list, and the header
//!   fields needed for replies.
//! * [`build`] serialises a [`Draft`] to RFC 5322 bytes; [`reply`] and [`forward`] prepare drafts
//!   from a message.
//!
//! Parsing uses `mail-parser`, which never panics on malformed input; HTML conversion is
//! additionally fenced with `catch_unwind` and size caps, since mail is hostile input.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod css;
pub mod layout;
pub mod markup;
pub mod office;
pub mod rich;

use mail_builder::MessageBuilder;
use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::MimePart;
use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders};
use mailcraft_model::dates::{Clock, full_time};
use mailcraft_model::thread::norm_id;
use mailcraft_model::{EmailAddress, Flags, MessageKey, Summary};
use serde::{Deserialize, Serialize};

/// Longest preview kept in a summary, in characters.
pub const PREVIEW_CHARS: usize = 200;
/// HTML larger than this is cut before conversion (the rest is rarely more than markup).
pub(crate) const HTML_MAX: usize = 4 << 20;

fn addrs(a: Option<&Address<'_>>) -> Vec<EmailAddress> {
    a.map(|a| a.iter().filter_map(|x| x.address().map(|e| EmailAddress::new(x.name().unwrap_or_default().trim(), e.trim()))).collect())
        .unwrap_or_default()
}

fn ids(v: &HeaderValue<'_>) -> Vec<String> {
    let raw: Vec<String> = match v {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(ToString::to_string).collect(),
        _ => Vec::new(),
    };
    raw.into_iter().map(|s| norm_id(&s)).filter(|s| !s.is_empty()).collect()
}

/// Collapse whitespace and skip quoted lines and signatures, for the one-line preview.
pub fn preview(text: &str) -> String {
    let mut out = String::with_capacity(PREVIEW_CHARS + 8);
    let mut n = 0usize;
    for line in text.lines() {
        let t = line.trim();
        if t == "--" || t == "-- " {
            break;
        }
        if t.starts_with('>') || t.is_empty() {
            continue;
        }
        for w in t.split_whitespace() {
            if n > 0 {
                out.push(' ');
                n += 1;
            }
            for c in w.chars() {
                if n >= PREVIEW_CHARS {
                    out.push('…');
                    return out;
                }
                out.push(c);
                n += 1;
            }
        }
    }
    out
}

fn has_real_attachments(m: &Message<'_>) -> bool {
    m.attachments().any(|p| p.attachment_name().is_some() || !p.is_inline_part())
}

trait InlinePart {
    fn is_inline_part(&self) -> bool;
}

impl InlinePart for mail_parser::MessagePart<'_> {
    /// Inline images referenced from the HTML (`Content-ID`) don't count as attachments.
    fn is_inline_part(&self) -> bool {
        self.content_id().is_some() && self.content_disposition().is_none_or(|d| d.ctype().eq_ignore_ascii_case("inline"))
    }
}

/// Build a summary from a message's header (or its whole source). `fallback_date` is the
/// server's INTERNALDATE, used when the `Date` header is missing or unparseable.
pub fn summary_from_header(raw: &[u8], key: MessageKey, flags: Flags, size: u32, fallback_date: i64) -> Summary {
    let Some(m) = MessageParser::default().parse(raw) else {
        return Summary { key, flags, size, date: fallback_date, subject: String::new(), ..Default::default() };
    };
    summary_from_message(&m, key, flags, size, fallback_date)
}

fn summary_from_message(m: &Message<'_>, key: MessageKey, flags: Flags, size: u32, fallback_date: i64) -> Summary {
    let from = addrs(m.from()).into_iter().next().or_else(|| addrs(m.sender()).into_iter().next()).unwrap_or_default();
    let date = m.date().map(|d| d.to_timestamp()).filter(|t| *t > 0).unwrap_or(fallback_date);
    let content_type_multipart_mixed = m
        .root_part()
        .content_type()
        .is_some_and(|c| c.ctype().eq_ignore_ascii_case("multipart") && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("mixed")));
    let has_attachments = has_real_attachments(m) || (m.attachment_count() == 0 && content_type_multipart_mixed && m.text_body_count() == 0);
    let text = m.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
    Summary {
        key,
        message_id: m.message_id().map(norm_id).unwrap_or_default(),
        in_reply_to: ids(m.in_reply_to()).into_iter().next().unwrap_or_default(),
        references: ids(m.references()),
        subject: m.subject().unwrap_or_default().trim().to_string(),
        from,
        to: addrs(m.to()),
        cc: addrs(m.cc()),
        date,
        size,
        flags,
        has_attachments,
        preview: preview(&text),
        modseq: 0,
        bulk: is_bulk(m),
        importance: importance(m),
    }
}

/// How important the sender marked a message: 1 high, -1 low, 0 normal. `Importance` (RFC 2156),
/// else the de-facto `X-Priority` (1–2 high, 4–5 low), else `Priority` (urgent, non-urgent).
pub(crate) fn importance(m: &Message<'_>) -> i8 {
    let h = |name: &str| m.header_raw(name).map(|v| v.trim().to_ascii_lowercase()).unwrap_or_default();
    match h("Importance").as_str() {
        "high" => return 1,
        "low" => return -1,
        "normal" => return 0,
        _ => {}
    }
    match h("X-Priority").chars().find(char::is_ascii_digit) {
        Some('1' | '2') => return 1,
        Some('4' | '5') => return -1,
        Some(_) => return 0,
        None => {}
    }
    match h("Priority").as_str() {
        "urgent" => 1,
        "non-urgent" => -1,
        _ => 0,
    }
}

/// Mail sent in bulk: lists, newsletters, notifications (RFC 2369 `List-*`, RFC 2919 `List-Id`,
/// RFC 3834 `Auto-Submitted`, the de-facto `Precedence: bulk|list|junk`, and no-reply senders).
fn is_bulk(m: &Message<'_>) -> bool {
    let has = |name: &str| m.header_raw(name).is_some_and(|v| !v.trim().is_empty());
    let precedence = m.header_raw("Precedence").map(|v| v.trim().to_ascii_lowercase()).unwrap_or_default();
    let auto = m.header_raw("Auto-Submitted").map(|v| v.trim().to_ascii_lowercase()).unwrap_or_default();
    let from = m.from().and_then(|a| a.first()).and_then(|a| a.address()).unwrap_or_default().to_ascii_lowercase();
    has("List-Id")
        || has("List-Unsubscribe")
        || matches!(precedence.as_str(), "bulk" | "list" | "junk")
        || (!auto.is_empty() && auto != "no")
        || from.starts_with("noreply@")
        || from.starts_with("no-reply@")
        || from.starts_with("donotreply@")
}

/// An attachment as the reading pane lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// Position among the message's attachments (for [`attachment_data`]).
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub inline: bool,
    /// `Content-ID` without angle brackets, for pictures the HTML shows (`cid:`).
    #[serde(default)]
    pub content_id: Option<String>,
    /// `Content-Location` (RFC 2557): the address the HTML may show it by instead
    /// (`<img src="logo.png">`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_location: Option<String>,
}

impl AttachmentInfo {
    /// What kind of preview the UI can offer.
    pub fn kind(&self) -> AttachmentKind {
        kind_of(&self.mime, &self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Archive,
    Document,
    Other,
}

pub fn kind_of(mime: &str, name: &str) -> AttachmentKind {
    let mime = mime.to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if mime.starts_with("image/") {
        AttachmentKind::Image
    } else if mime.starts_with("video/") {
        AttachmentKind::Video
    } else if mime.starts_with("audio/") {
        AttachmentKind::Audio
    } else if mime == "application/pdf" || ext == "pdf" {
        AttachmentKind::Pdf
    } else if mime.starts_with("text/") {
        AttachmentKind::Text
    } else if ["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str()) {
        AttachmentKind::Archive
    } else if ["docx", "doc", "odt", "xlsx", "xls", "ods", "pptx", "ppt", "odp", "rtf", "craft"].contains(&ext.as_str()) {
        AttachmentKind::Document
    } else {
        AttachmentKind::Other
    }
}

/// The Craft app that edits files of this kind (by its command name), when there is one.
pub fn craft_app_for(mime: &str, name: &str) -> Option<(&'static str, &'static str)> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let app = match kind_of(mime, name) {
        AttachmentKind::Pdf => ("pdfcraft", "PdfCraft"),
        AttachmentKind::Image => ("photocraft", "PhotoCraft"),
        AttachmentKind::Video => ("filmcraft", "FilmCraft"),
        AttachmentKind::Audio => ("soundcraft", "SoundCraft"),
        _ => match ext.as_str() {
            "docx" | "doc" | "odt" | "rtf" | "dotx" => ("wordcraft", "WordCraft"),
            "xlsx" | "xls" | "ods" | "csv" | "xlsm" => ("gridcraft", "GridCraft"),
            "pptx" | "ppt" | "odp" | "ppsx" => ("deckcraft", "DeckCraft"),
            _ => return None,
        },
    };
    Some(app)
}

/// Whether an attachment can be shown as text (and how: `message` for an attached email).
pub fn text_preview_kind(mime: &str, name: &str) -> Option<&'static str> {
    let m = mime.to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if m == "message/rfc822" || ext == "eml" {
        return Some("message");
    }
    let texty = m.starts_with("text/")
        || ["application/json", "application/xml", "application/x-yaml", "application/javascript"].contains(&m.as_str())
        || [
            "txt", "md", "csv", "tsv", "json", "xml", "yaml", "yml", "log", "ics", "vcf", "ini", "toml", "rs", "py", "js", "ts", "html", "css", "sh",
            "srt", "vtt",
        ]
        .contains(&ext.as_str());
    texty.then_some("text")
}

/// A message ready for the reading pane.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parsed {
    pub subject: String,
    pub from: EmailAddress,
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub date: i64,
    pub message_id: String,
    #[serde(default)]
    pub in_reply_to: String,
    pub references: Vec<String>,
    /// The body as plain text (from the text part, or converted from HTML).
    pub text: String,
    /// Whether the original had an HTML body (shown converted).
    pub had_html: bool,
    /// The HTML body, formatted (empty for plain-text mail).
    #[serde(default)]
    pub rich: RichText,
    /// The HTML body's columns, for mail laid out with tables (empty otherwise: `rich` flows).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<layout::Block>,
    /// Every picture the HTML body shows (`cid:` and web addresses).
    #[serde(default)]
    pub images: Vec<String>,
    pub attachments: Vec<AttachmentInfo>,
    /// `List-Unsubscribe` target, when the sender offers one.
    pub unsubscribe: Option<String>,
}

/// A run of text in a rich (HTML) message, with its formatting.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strike: bool,
    /// Code or preformatted text (shown in a monospace font).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// A picture (`src`: `cid:…` for one in the message, `https:` for one on the web); the text
    /// is its description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Part of a heading (`1` for `<h1>` … `6`), shown larger.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heading: u8,
    /// The text's colour and its background (a highlight), as the message set them (`style`,
    /// `<font color>`, `<style>` rules). The reader adjusts them so the text stays readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[u8; 3]>,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// Lines of a rich message, each a list of spans.
pub type RichText = Vec<Vec<Span>>;

/// Most lines kept of a rich rendering (the rest is shown as text).
const RICH_MAX_LINES: usize = 20_000;

/// Word's and Outlook's paragraphs (`class=MsoNormal`) have no space between them (their style
/// sheet, which a copied fragment leaves behind, says so): as lines, not paragraphs.
fn word_paragraphs(html: &str) -> String {
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
    let mut lines = html_to_rich(&word_paragraphs(html));
    for line in &mut lines {
        line.retain(|sp| sp.image.is_none());
    }
    let blank = |l: &Vec<Span>| l.iter().all(|sp| sp.text.trim().is_empty());
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !blank(l)).unwrap_or(lines.len());
    lines.drain(..first);
    rich::from_rich(&lines)
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
const HEADING_MARK: char = '\u{E000}';

/// The description given to pictures that have none (a report's charts, a logo), which the
/// converter would otherwise leave out: removed again once they're pictures.
const PICTURE_MARK: char = '\u{E001}';

/// An attribute's value in a tag's text (`<img alt="x" …>`), as written; `Some("")` when it's
/// there and empty.
fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
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
fn mark_pictures(html: &str) -> String {
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
/// lack one, which html2text's parser would otherwise skip with the whole rule).
fn close_css_rules(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut at = 0;
    while let Some(start) = lower.get(at..).and_then(|r| r.find("<style")).map(|i| i + at) {
        let Some(end) = lower.get(start..).and_then(|r| r.find("</style")).map(|i| i + start) else { break };
        out.push_str(html.get(at..start).unwrap_or(""));
        out.push_str(&html.get(start..end).unwrap_or("").replace('}', ";}"));
        at = end;
    }
    out.push_str(html.get(at..).unwrap_or(""));
    out
}

/// `html` with a mark after every `<h1>`…`<h6>` start tag (and any mark already in it removed).
fn mark_headings(html: &str) -> String {
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

/// Trim trailing spaces and collapse runs of blank lines.
fn tidy_text(s: &str) -> String {
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

/// Parse a full message for display. `None` only for input that isn't a message at all.
pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let m = MessageParser::default().parse(raw)?;
    // mail-parser lists text parts among the HTML bodies too (converted); only real HTML counts.
    let real_html = m.html_bodies().any(|p| p.is_text_html());
    let (rich, images, layout) = match m.body_html(0).filter(|_| real_html) {
        Some(h) => {
            // Style sheet rules as inline styles, so cells keep their colours and titles their size.
            let h = layout::inline_css(&h);
            let layout = layout::html_layout(&h);
            // Pictures behind boxes are shown, or held back, like the others.
            let mut images = html_images(&h);
            for src in layout::background_pictures(&layout) {
                if !images.contains(&src) && images.len() < 200 {
                    images.push(src);
                }
            }
            (html_to_rich(&h), images, layout)
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let (text, had_html) = match (m.text_body_count() > 0, m.body_text(0), m.body_html(0)) {
        // Prefer the HTML version when the text part is an auto-generated stub.
        (_, Some(t), Some(h)) if t.trim().is_empty() => (html_to_text(&h), true),
        (true, Some(t), _) if !m.text_part(0).is_some_and(|p| p.is_text_html()) => (tidy_text(&t), real_html),
        (_, _, Some(h)) => (html_to_text(&h), true),
        (_, Some(t), None) => (tidy_text(&t), false),
        _ => (String::new(), false),
    };
    let attachments = m
        .attachments()
        .enumerate()
        .map(|(index, p)| {
            let mime = p
                .content_type()
                .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
                .unwrap_or_else(|| "application/octet-stream".into());
            let name = p.attachment_name().map(sanitize_filename).unwrap_or_else(|| format!("attachment-{}{}", index + 1, ext_for_mime(&mime)));
            AttachmentInfo {
                index,
                name,
                mime,
                size: p.len(),
                inline: p.is_inline_part(),
                content_id: p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string()),
                content_location: p.content_location().map(|c| c.trim().trim_matches(['<', '>', '"']).to_string()).filter(|c| !c.is_empty()),
            }
        })
        .collect();
    let unsubscribe = match m.list_unsubscribe() {
        HeaderValue::Address(a) => a.first().and_then(|x| x.address()).map(str::to_string),
        HeaderValue::Text(t) => Some(t.trim_matches(['<', '>']).to_string()),
        HeaderValue::TextList(l) => l.first().map(|t| t.trim_matches(['<', '>']).to_string()),
        _ => None,
    };
    Some(Parsed {
        subject: m.subject().unwrap_or_default().trim().to_string(),
        from: addrs(m.from()).into_iter().next().unwrap_or_default(),
        reply_to: addrs(m.reply_to()),
        to: addrs(m.to()),
        cc: addrs(m.cc()),
        date: m.date().map(|d| d.to_timestamp()).unwrap_or(0),
        message_id: m.message_id().map(norm_id).unwrap_or_default(),
        in_reply_to: ids(m.in_reply_to()).into_iter().next().unwrap_or_default(),
        references: ids(m.references()),
        text,
        had_html,
        rich,
        layout,
        images,
        attachments,
        unsubscribe,
    })
}

/// A message's words for search: its text (HTML as text) and its attachments' names. Lighter
/// than [`parse`] (no formatting, no layout), for indexing every downloaded message.
pub fn search_text(raw: &[u8]) -> String {
    let Some(m) = MessageParser::default().parse(raw) else { return String::new() };
    let mut out: String = m.body_text(0).map(|t| t.chars().take(50_000).collect()).unwrap_or_default();
    for a in m.attachments() {
        if let Some(n) = a.attachment_name() {
            out.push('\n');
            out.push_str(n);
        }
    }
    out
}

/// The summary for a fully downloaded message (preview from the real body, HTML included).
pub fn summary_from_full(raw: &[u8], key: MessageKey, flags: Flags, size: u32, fallback_date: i64) -> Summary {
    let mut s = summary_from_header(raw, key, flags, size, fallback_date);
    if s.preview.is_empty()
        && let Some(p) = parse(raw)
    {
        s.preview = preview(&p.text);
    }
    s
}

/// The HTML body as written, for a message that has one.
pub fn html_body(raw: &[u8]) -> Option<String> {
    let m = MessageParser::default().parse(raw)?;
    if !m.html_bodies().any(|p| p.is_text_html()) {
        return None;
    }
    m.body_html(0).map(|h| h.into_owned())
}

/// The bytes of attachment `index`.
pub fn attachment_data(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    let m = MessageParser::default().parse(raw)?;
    m.attachments().nth(index).map(|p| p.contents().to_vec())
}

/// A file name safe to write: no directories, no reserved characters, not empty.
pub fn sanitize_filename(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = leaf.chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(180).collect();
    if cleaned.is_empty() { "attachment".to_string() } else { cleaned }
}

fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "application/pdf" => ".pdf",
        "text/plain" => ".txt",
        "text/calendar" => ".ics",
        "message/rfc822" => ".eml",
        _ => "",
    }
}

/// The MIME type for an attachment, from its file name.
pub fn mime_from_name(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "ics" => "text/calendar",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
}

/// A file attached to an outgoing message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutAttachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// An outgoing message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// Where replies go, when not to the sender (Outlook's "Have replies sent to").
    #[serde(default)]
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body: String,
    #[serde(default)]
    pub in_reply_to: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<OutAttachment>,
    /// A formatted version of `body`: sent as `multipart/alternative` next to the text.
    #[serde(default)]
    pub html: Option<String>,
    /// Pictures the HTML shows by `cid:` (sent inline, `multipart/related`).
    #[serde(default)]
    pub inline: Vec<InlinePicture>,
    /// High (1) or low (-1) importance; 0 sends no header.
    #[serde(default)]
    pub importance: i8,
    /// Ask for a read receipt (RFC 8098 `Disposition-Notification-To`, to the sender).
    #[serde(default)]
    pub read_receipt: bool,
}

/// A picture inside a formatted message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlinePicture {
    /// The `Content-ID` the HTML refers to (`<img src="cid:…">`).
    pub cid: String,
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

impl Draft {
    /// Every envelope recipient (To, Cc and Bcc), deduplicated case-insensitively.
    pub fn recipients(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for a in self.to.iter().chain(&self.cc).chain(&self.bcc) {
            let e = a.email.trim().to_string();
            if !e.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&e)) {
                out.push(e);
            }
        }
        out
    }
}

fn b_addr(a: &EmailAddress) -> mail_builder::headers::address::Address<'static> {
    if a.name.trim().is_empty() { a.email.clone().into() } else { (a.name.trim().to_string(), a.email.clone()).into() }
}

fn b_list(v: &[EmailAddress]) -> mail_builder::headers::address::Address<'static> {
    v.iter().map(b_addr).collect::<Vec<_>>().into()
}

/// A fresh Message-ID: `<time.seed@domain>`. The caller supplies the randomness (`seed`), so this
/// stays deterministic and wasm-safe.
pub fn new_message_id(from_email: &str, time: i64, seed: u64) -> String {
    let domain = from_email.rsplit_once('@').map(|(_, d)| d).filter(|d| !d.is_empty()).unwrap_or("mailcraft.invalid");
    format!("{:x}.{:016x}@{}", time.max(0), seed, domain)
}

/// Serialise a draft to RFC 5322 bytes. Bcc is never written into the message (it travels only
/// in the SMTP envelope).
/// The message with its `Date:` set to `date` (a message held back to be sent later is dated
/// when it goes, as Outlook dates delayed mail). Messages without a header block come back as
/// they were.
pub fn with_date(raw: &[u8], date: i64) -> Vec<u8> {
    let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return raw.to_vec() };
    let head = &raw[..end + 2];
    let mut out = Vec::with_capacity(raw.len() + 40);
    let mut skipping = false;
    let mut replaced = false;
    for line in head.split_inclusive(|b| *b == b'\n') {
        let folded = line.first().is_some_and(|b| *b == b' ' || *b == b'\t');
        if skipping && folded {
            continue;
        }
        skipping = false;
        if !replaced && !folded && line.len() >= 5 && line[..5].eq_ignore_ascii_case(b"date:") {
            out.extend_from_slice(format!("Date: {}\r\n", mail_builder::headers::date::Date::new(date.max(0)).to_rfc822()).as_bytes());
            skipping = true;
            replaced = true;
            continue;
        }
        out.extend_from_slice(line);
    }
    out.extend_from_slice(&raw[end + 2..]);
    out
}

pub fn build(d: &Draft, from: &EmailAddress, date: i64, message_id: &str) -> Result<Vec<u8>, String> {
    let mut b = MessageBuilder::new().from(b_addr(from)).subject(d.subject.clone()).date(date.max(0)).message_id(message_id.to_string());
    if !d.to.is_empty() {
        b = b.to(b_list(&d.to));
    }
    if !d.cc.is_empty() {
        b = b.cc(b_list(&d.cc));
    }
    if !d.reply_to.is_empty() {
        b = b.reply_to(b_list(&d.reply_to));
    }
    if !d.in_reply_to.is_empty() {
        b = b.in_reply_to(d.in_reply_to.clone());
    }
    if !d.references.is_empty() {
        b = b.references(d.references.clone());
    }
    b = b.header("X-Mailer", mail_builder::headers::raw::Raw::new(format!("MailCraft {}", env!("CARGO_PKG_VERSION"))));
    // Importance as RFC 2156 says, and as X-Priority, which most mail programs read.
    let raw = mail_builder::headers::raw::Raw::new;
    match d.importance.signum() {
        1 => b = b.header("Importance", raw("high")).header("X-Priority", raw("1")),
        -1 => b = b.header("Importance", raw("low")).header("X-Priority", raw("5")),
        _ => {}
    }
    if d.read_receipt && !from.email.trim().is_empty() {
        b = b.header(
            "Disposition-Notification-To",
            mail_builder::headers::address::Address::new_address(None::<String>, from.email.trim().to_string()),
        );
    }
    // Boundaries come from the message itself: unique per message and part, the same every time
    // (mail-builder would read the clock, which the browser doesn't have).
    let seed = format!("{:016x}", fnv(&format!("{message_id}\u{0}{date}")));
    let multipart = |kind: &str, n: u32, parts: Vec<MimePart<'static>>| {
        MimePart::new(ContentType::new(kind.to_string()).attribute("boundary", format!("=_mc{seed}_{n}")), parts)
    };
    let files: Vec<MimePart<'static>> = d
        .attachments
        .iter()
        .map(|a| {
            let mime = if a.mime.is_empty() { mime_from_name(&a.name).to_string() } else { a.mime.clone() };
            // An attached message goes as it is (RFC 2046 §5.2.1: 7bit or 8bit, never base64).
            if mime.eq_ignore_ascii_case("message/rfc822") {
                let cte = if a.data.is_ascii() { "7bit" } else { "8bit" };
                return MimePart::new(mime, a.data.clone())
                    .attachment(sanitize_filename(&a.name))
                    .header("Content-Transfer-Encoding", mail_builder::headers::raw::Raw::new(cte));
            }
            MimePart::new(mime, a.data.clone()).attachment(sanitize_filename(&a.name))
        })
        .collect();
    let text = MimePart::new("text/plain", d.body.replace("\r\n", "\n"));
    let root = match &d.html {
        None if files.is_empty() => text,
        None => multipart("multipart/mixed", 1, std::iter::once(text).chain(files).collect()),
        Some(html) => {
            // mixed( related( alternative(text, html), pictures… ), attachments… ): the pictures
            // belong with the HTML that shows them (RFC 2387), the files with the message.
            let alt = multipart("multipart/alternative", 3, vec![text, MimePart::new("text/html", html.clone())]);
            let mut related = vec![alt];
            for p in &d.inline {
                let mime = if p.mime.is_empty() { mime_from_name(&p.name).to_string() } else { p.mime.clone() };
                related.push(MimePart::new(mime, p.data.clone()).inline().cid(p.cid.clone()));
            }
            let root = if related.len() == 1 { related.remove(0) } else { multipart("multipart/related", 2, related) };
            if files.is_empty() { root } else { multipart("multipart/mixed", 1, std::iter::once(root).chain(files).collect()) }
        }
    };
    b.body(root).write_to_vec().map_err(|e| e.to_string())
}

/// FNV-1a, for boundaries.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplyMode {
    Reply,
    ReplyAll,
    Forward,
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let base = mailcraft_model::thread::base_subject(subject);
    let already = subject.trim().to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase());
    if already { subject.trim().to_string() } else { format!("{prefix} {base}") }
}

/// The original under a reply or forward, the way Outlook lays it out: a rule, then who sent it
/// to whom and when (labels in bold), then the original with its own formatting.
fn quoted_original(p: &Parsed, clock: &Clock) -> (String, rich::Styles) {
    let mut lines: Vec<(&str, String)> = vec![("From:", p.from.to_string()), ("Sent:", full_time(p.date, clock))];
    if !p.to.is_empty() {
        lines.push(("To:", mailcraft_model::addr::format_list(&p.to)));
    }
    if !p.cc.is_empty() {
        lines.push(("Cc:", mailcraft_model::addr::format_list(&p.cc)));
    }
    lines.push(("Subject:", p.subject.clone()));
    // The rule: an empty paragraph drawn as a line.
    let mut text = String::new();
    let mut styles = rich::Styles::plain("");
    if let Some(first) = styles.paras.first_mut() {
        first.rule = true;
    }
    let bold = rich::Style { bold: true, ..Default::default() };
    for (label, value) in lines {
        let line = format!("\n{label} {value}");
        let mut st = rich::Styles::plain(&line);
        st.restyle(&line, 1..1 + label.chars().count(), |_| bold.clone());
        rich::append(&mut text, &mut styles, &line, &st);
    }
    let (orig, ost) = if p.had_html && !p.rich.is_empty() { rich::from_rich(&p.rich) } else { (p.text.clone(), rich::Styles::plain(&p.text)) };
    rich::append(&mut text, &mut styles, "\n\n", &rich::Styles::plain("\n\n"));
    rich::append(&mut text, &mut styles, &orig, &ost);
    (text, styles)
}

/// A reply or forward of `p`, as plain text. `me` are the user's own addresses, left out of
/// Reply All.
pub fn reply(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> Draft {
    let (mut d, styles) = reply_doc(p, mode, me, clock, signature);
    d.body = rich::to_text(&d.body, &styles);
    d
}

/// A reply or forward of `p` to edit: the draft (its body the editable text) and the text's
/// styles.
pub fn reply_doc(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> (Draft, rich::Styles) {
    let mine = |a: &EmailAddress| me.iter().any(|m| m.eq_ignore_ascii_case(a.email.trim()));
    let sig = if signature.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", signature.trim_end()) };
    // The cursor's line, the signature, a blank line, then the original under its rule.
    let mut body = sig;
    let mut styles = rich::Styles::plain(&body);
    rich::append(&mut body, &mut styles, "\n\n", &rich::Styles::plain("\n\n"));
    let (orig, ost) = quoted_original(p, clock);
    rich::append(&mut body, &mut styles, &orig, &ost);
    let mut d = Draft::default();
    match mode {
        ReplyMode::Forward => {
            d.subject = prefixed("Fwd:", &p.subject);
        }
        ReplyMode::Reply | ReplyMode::ReplyAll => {
            d.subject = prefixed("Re:", &p.subject);
            let primary = if p.reply_to.is_empty() { vec![p.from.clone()] } else { p.reply_to.clone() };
            // Replying to your own sent message goes back to its recipients.
            d.to = if primary.iter().all(&mine) && !p.to.is_empty() { p.to.clone() } else { primary };
            if mode == ReplyMode::ReplyAll {
                let mut cc: Vec<EmailAddress> = Vec::new();
                for a in p.to.iter().chain(&p.cc) {
                    let dup = d.to.iter().chain(&cc).any(|x| x.email.eq_ignore_ascii_case(&a.email));
                    if !mine(a) && !dup {
                        cc.push(a.clone());
                    }
                }
                d.cc = cc;
            }
            if !p.message_id.is_empty() {
                d.in_reply_to = p.message_id.clone();
                d.references = p.references.clone();
                d.references.push(p.message_id.clone());
                // RFC 5322 §3.6.4 allows trimming; keep the first and the last 20.
                if d.references.len() > 21 {
                    let tail = d.references.split_off(d.references.len() - 20);
                    d.references.truncate(1);
                    d.references.extend(tail);
                }
            }
        }
    }
    d.body = body;
    (d, styles)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #1: a signature copied from Outlook (Word's HTML on the clipboard, only the copied
    /// fragment) keeps its look when pasted.
    #[test]
    fn a_signature_pasted_from_outlook_keeps_its_formatting() {
        let html = "<!--StartFragment--><p class=MsoNormal><b><span style='font-size:14.0pt;font-family:\"Arial\",sans-serif;color:#BFBFBF'>Company bv<o:p></o:p></span></b></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt;font-family:\"Tahoma\",sans-serif'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt;font-family:\"Tahoma\",sans-serif'>An street 1<br>B-1000 Brussel<br>Belgium<br>VAT BE 01234 567 890<br>Mob&nbsp;&nbsp;&nbsp;&nbsp; +32 (0)490 12 34 56<br>\
<a href=\"http://www.company.com/\"><span style='color:#44546A'>www.company.com</span></a><o:p></o:p></span></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><i><span style='font-size:14.0pt;color:#BFBFBF'>\u{2018}Choose a job you like' \u{2013} Confucius<o:p></o:p></span></i></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><i><span style='font-size:10.0pt'>The information contained in this message is confidential &amp; intended for the addressee.<o:p></o:p></span></i></p>\r\n\
<p class=MsoNormal><img width=120 height=40 src=\"file:///C:/Users/x/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png\"><o:p></o:p></p><!--EndFragment-->";
        let (text, styles) = pasted_html(html);
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines.first().map(|l| l.trim()), Some("Company bv"));
        assert_eq!(lines.get(1).map(|l| l.trim()), Some(""), "one empty line, as in Outlook");
        assert_eq!(lines.get(2).map(|l| l.trim()), Some("An street 1"));
        assert!(lines.iter().any(|l| l.contains("Mob") && l.contains("+32")));
        assert!(!text.contains("[picture]") && !text.contains('\u{FFFC}'), "the other program's pictures aren't brought along");
        let at = |needle: &str| styles.style(text.find(needle).map(|b| text[..b].chars().count()).unwrap());
        let company = at("Company");
        assert!(company.bold && company.color == Some([0xBF, 0xBF, 0xBF]), "{company:?}");
        assert_eq!(at("www.company").link.as_deref(), Some("http://www.company.com/"));
        let quote = at("Choose");
        assert!(quote.italic && quote.color == Some([0xBF, 0xBF, 0xBF]), "{quote:?}");
        assert!(at("The information").italic);
        assert!(!at("Belgium").bold && !at("Belgium").italic);
        // As the message sends it: HTML with the look, and a plain-text twin.
        let out = rich::to_html(&text, &styles);
        assert!(out.contains("<b") || out.contains("font-weight"), "{out}");
        assert!(out.contains("http://www.company.com/"));
        // Into a message, between what was there.
        let (mut body, mut st) = ("Hi\n\nBye".to_string(), rich::Styles::plain("Hi\n\nBye"));
        rich::splice(&mut body, &mut st, 3..3, &text, &styles);
        assert!(body.starts_with("Hi\nCompany bv") && body.ends_with("\nBye"), "{body}");
        assert_eq!(st.chars.len(), body.chars().count());
        assert!(st.style(3).bold && !st.style(0).bold && !st.style(body.chars().count() - 1).italic);
        // Word's paragraphs are lines (one line per paragraph isn't double-spaced); a web
        // page's keep their space.
        assert_eq!(pasted_html("<p class=MsoNormal>Riley<o:p></o:p></p>\r\n<p class=MsoNormal>Studio</p>").0, "Riley\nStudio");
        assert_eq!(pasted_html("<p>Riley</p><p>Studio</p>").0, "Riley\n\nStudio");
        assert_eq!(pasted_html("<P CLASS=MsoNormal>a</P><pre class=MsoNormal>b</pre>").0.matches('a').count(), 1);
        assert!(pasted_html("").0.is_empty());
        assert!(pasted_html("<img src=\"https://x.example/a.png\">").0.is_empty());
    }

    const SIMPLE: &[u8] = b"From: Zoe Park <zoe@studio.example>\r\n\
To: Ann <ann@studio.example>, ops@studio.example\r\n\
Cc: Kim <kim@studio.example>\r\n\
Subject: =?UTF-8?Q?Caf=C3=A9_plans?=\r\n\
Date: Wed, 14 Oct 2026 09:30:00 +0200\r\n\
Message-ID: <m1@studio.example>\r\n\
References: <m0@studio.example>\r\n\
In-Reply-To: <m0@studio.example>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello team,\r\n\
\r\n\
> quoted earlier text\r\n\
Let's meet at the caf\xc3\xa9.\r\n\
-- \r\n\
Zoe\r\n";

    const MULTI: &[u8] = b"From: a@x.example\r\n\
Subject: Pictures\r\n\
Message-ID: <p@x>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"B\"\r\n\
\r\n\
--B\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>See <b>attached</b>.</p><img src=\"https://tracker.example/p.gif\"></body></html>\r\n\
--B\r\n\
Content-Type: image/png; name=\"../../evil.png\"\r\n\
Content-Disposition: attachment; filename=\"../../evil.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--B--\r\n";

    fn key() -> MessageKey {
        MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 9 }
    }

    #[test]
    fn summary_fields() {
        let s = summary_from_header(SIMPLE, key(), Flags::default(), 321, 5);
        assert_eq!(s.subject, "Café plans");
        assert_eq!(s.from, EmailAddress::new("Zoe Park", "zoe@studio.example"));
        assert_eq!(s.to.len(), 2);
        assert_eq!(s.cc[0].name, "Kim");
        assert_eq!(s.message_id, "m1@studio.example");
        assert_eq!(s.in_reply_to, "m0@studio.example");
        assert_eq!(s.references, vec!["m0@studio.example".to_string()]);
        assert_eq!(s.date, 1_791_963_000);
        assert_eq!(s.preview, "Hello team, Let's meet at the café.");
        assert!(!s.has_attachments);
        assert_eq!(s.size, 321);
    }

    #[test]
    fn bulk_mail_is_recognised() {
        let s = |head: &str| summary_from_header(format!("{head}From: a@x.example\r\nSubject: s\r\n\r\nb").as_bytes(), key(), Flags::default(), 0, 0);
        assert!(!s("").bulk);
        assert!(s("List-Id: <news.x.example>\r\n").bulk);
        assert!(s("List-Unsubscribe: <mailto:u@x.example>\r\n").bulk);
        assert!(s("Precedence: bulk\r\n").bulk);
        assert!(s("Auto-Submitted: auto-generated\r\n").bulk);
        assert!(!s("Auto-Submitted: no\r\n").bulk);
        assert!(summary_from_header(b"From: noreply@shop.example\r\n\r\nx", key(), Flags::default(), 0, 0).bulk);
    }

    #[test]
    fn garbage_never_panics() {
        for raw in [&b""[..], b"\xff\xfe\x00", b"Subject", b"Content-Type: multipart/mixed; boundary=\r\n\r\n--\r\n--", b"From: <<<@>>>\r\n\r\n"] {
            let _ = summary_from_header(raw, key(), Flags::default(), 0, 7);
            let _ = parse(raw);
            let _ = attachment_data(raw, 3);
        }
        assert_eq!(html_to_text(""), "");
        let _ = html_to_text("<table><tr><td><table><tr><td>x</td></tr></table></td></tr></table><<<>>>&#xFFFFFFFF;");
    }

    #[test]
    fn rich_html_keeps_formatting_links_and_pictures() {
        let r = html_to_rich(
            "<p>Hello <b>bold</b> and <i>slanted</i> <a href=\"https://x.example/a\">link</a>.</p><p><img src=\"cid:logo@x\" alt=\"Logo\"><img src=\"https://t.example/p.gif\" alt=\"\"></p><pre>code</pre><script>alert(1)</script>",
        );
        let all: Vec<&Span> = r.iter().flatten().collect();
        assert!(all.iter().any(|s| s.bold && s.text.contains("bold")));
        assert!(all.iter().any(|s| s.italic && s.text.contains("slanted")));
        assert!(all.iter().any(|s| s.link.as_deref() == Some("https://x.example/a") && s.text.contains("link")));
        assert!(all.iter().any(|s| s.image.as_deref() == Some("cid:logo@x")));
        assert!(all.iter().any(|s| s.code && s.text.contains("code")));
        assert!(!all.iter().any(|s| s.text.contains("alert")), "scripts never show");
        let h = html_to_rich("<h2>Big news</h2><p># not a heading</p>");
        assert_eq!((h[0][0].text.as_str(), h[0][0].heading), ("Big news", 2));
        assert!(h.iter().flatten().any(|s| s.heading == 0 && s.text.starts_with('#')), "{h:?}");
        // A picture without a description keeps its place (shown once the web is allowed).
        assert_eq!(remote_images(&r), ["https://t.example/p.gif"]);
        assert!(all.iter().filter(|s| s.image.is_some()).all(|s| !s.text.contains(PICTURE_MARK)));
        assert!(html_to_rich("").is_empty());
        let p = parse(MULTI).unwrap();
        assert!(!p.rich.is_empty());
        assert_eq!(p.images, vec!["https://tracker.example/p.gif".to_string()]);
        assert_eq!(
            html_images("<IMG alt=x SRC='https://a.example/1.png?x=1&amp;y=2'><img src=cid:k@x><img src=\"cid:k@x\"><img>"),
            vec!["https://a.example/1.png?x=1&y=2".to_string(), "cid:k@x".to_string()]
        );
        assert_eq!(remote_images(&html_to_rich("<img src=\"https://a.example/x.png\" alt=\"Map\">")), vec!["https://a.example/x.png".to_string()]);
        assert!(parse(SIMPLE).unwrap().rich.is_empty(), "plain mail has no rich body");
    }

    #[test]
    fn html_and_attachments() {
        let p = parse(MULTI).unwrap();
        assert!(p.had_html);
        assert!(p.text.contains("See") && p.text.contains("attached"));
        assert!(!p.text.contains("<b>"));
        assert_eq!(p.attachments.len(), 1);
        let a = &p.attachments[0];
        assert_eq!(a.name, "evil.png");
        assert_eq!(a.mime, "image/png");
        assert_eq!(a.kind(), AttachmentKind::Image);
        assert_eq!(attachment_data(MULTI, 0).unwrap()[..4], [0x89, b'P', b'N', b'G']);
        let s = summary_from_full(MULTI, key(), Flags::default(), 0, 0);
        assert!(s.has_attachments);
        assert!(s.preview.contains("attached"));
    }

    #[test]
    fn build_round_trips() {
        let d = Draft {
            to: vec![EmailAddress::new("Ann", "ann@studio.example")],
            cc: vec![EmailAddress::new("", "kim@studio.example")],
            bcc: vec![EmailAddress::new("", "secret@studio.example")],
            subject: "Storyboard v2 ✏️".into(),
            body: "Hi Ann,\nhere it is.\n".into(),
            in_reply_to: "m1@studio.example".into(),
            references: vec!["m0@studio.example".into(), "m1@studio.example".into()],
            attachments: vec![OutAttachment { name: "board.png".into(), mime: String::new(), data: vec![1, 2, 3, 4] }],
            ..Default::default()
        };
        let raw = build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x1@studio.example").unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("secret@"), "Bcc must not be written");
        let p = parse(&raw).unwrap();
        assert_eq!(p.subject, d.subject);
        assert_eq!(p.to, d.to);
        assert_eq!(p.cc, d.cc);
        assert_eq!(p.message_id, "x1@studio.example");
        assert_eq!(p.text.trim(), "Hi Ann,\nhere it is.");
        assert_eq!(p.attachments[0].name, "board.png");
        assert_eq!(p.attachments[0].mime, "image/png");
        assert_eq!(attachment_data(&raw, 0).unwrap(), vec![1, 2, 3, 4]);
        let s = summary_from_header(&raw, key(), Flags::default(), 0, 0);
        assert_eq!(s.references, d.references);
        assert_eq!(s.in_reply_to, "m1@studio.example");
        assert_eq!(d.recipients(), vec!["ann@studio.example", "kim@studio.example", "secret@studio.example"]);
        // The same message builds the same bytes: nothing comes from the clock (which a browser
        // doesn't have), and each message's boundaries are its own.
        assert_eq!(build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x1@studio.example").unwrap(), raw);
        let other = build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x2@studio.example").unwrap();
        assert_ne!(other, raw);
        // Replies sent elsewhere; a held-back message dated when it goes.
        let to_desk = Draft { reply_to: vec![EmailAddress::new("Desk", "desk@studio.example")], ..d.clone() };
        let raw2 = build(&to_desk, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x3@studio.example").unwrap();
        assert_eq!(parse(&raw2).unwrap().reply_to, to_desk.reply_to);
        let later = with_date(&raw2, 1_792_000_000);
        let p2 = parse(&later).unwrap();
        assert_eq!((p2.date, p2.subject.as_str(), p2.text.trim()), (1_792_000_000, d.subject.as_str(), "Hi Ann,\nhere it is."));
        assert_eq!(String::from_utf8_lossy(&later).matches("Date:").count(), 1);
        assert_eq!(with_date(b"no header block", 5), b"no header block".to_vec());
        let rich = Draft {
            html: Some("<p>Hi <b>Ann</b><img src=\"cid:pic@x\"></p>".into()),
            inline: vec![InlinePicture { cid: "pic@x".into(), name: "pic.png".into(), mime: "image/png".into(), data: vec![9, 9] }],
            ..d.clone()
        };
        let raw = build(&rich, &EmailAddress::new("", "zoe@studio.example"), 5, "x3@studio.example").unwrap();
        let text = String::from_utf8_lossy(&raw);
        for kind in ["multipart/mixed", "multipart/related", "multipart/alternative"] {
            assert!(text.contains(kind), "{kind}");
        }
        assert_eq!(text.matches("boundary=").count(), 3);
        let p = parse(&raw).unwrap();
        assert!(p.had_html && p.text.contains("Ann"));
        assert_eq!(p.attachments.iter().filter(|a| !a.inline).map(|a| a.name.as_str()).collect::<Vec<_>>(), ["board.png"]);
        // Plain text alone is a single part.
        let plain = Draft { attachments: Vec::new(), ..d };
        let raw = build(&plain, &EmailAddress::new("", "zoe@studio.example"), 5, "x4@studio.example").unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("multipart"));
    }

    #[test]
    fn formatted_mail_with_inline_pictures() {
        let d = Draft {
            to: vec![EmailAddress::new("", "ann@studio.example")],
            subject: "Look".into(),
            body: "See the picture.".into(),
            html: Some("<p>See the <b>picture</b>.</p><img src=\"cid:pic1@mailcraft\" alt=\"pic\">".into()),
            inline: vec![InlinePicture { cid: "pic1@mailcraft".into(), name: "p.png".into(), mime: "image/png".into(), data: vec![9, 9] }],
            ..Default::default()
        };
        let raw = build(&d, &EmailAddress::new("", "z@studio.example"), 0, "f@x").unwrap();
        let text = String::from_utf8_lossy(&raw).to_ascii_lowercase();
        assert!(text.contains("multipart/alternative") && text.contains("multipart/related"), "{text}");
        let p = parse(&raw).unwrap();
        assert_eq!(p.text.trim(), "See the picture.");
        assert!(p.rich.iter().flatten().any(|s| s.bold && s.text == "picture"));
        assert_eq!(p.images, vec!["cid:pic1@mailcraft".to_string()]);
        let pic = p.attachments.iter().find(|a| a.content_id.as_deref() == Some("pic1@mailcraft")).unwrap();
        assert!(pic.inline);
        assert_eq!(attachment_data(&raw, pic.index).unwrap(), vec![9, 9]);
    }

    #[test]
    fn replies() {
        let p = parse(SIMPLE).unwrap();
        assert!(!p.had_html, "plain text is not HTML");
        let clock = Clock::new(1_792_000_000, 0);
        let me = vec!["ann@studio.example".to_string()];
        let r = reply(&p, ReplyMode::Reply, &me, &clock, "");
        assert_eq!(r.subject, "Re: Café plans");
        assert_eq!(r.to, vec![p.from.clone()]);
        assert!(r.cc.is_empty());
        assert_eq!(r.in_reply_to, "m1@studio.example");
        assert_eq!(r.references, vec!["m0@studio.example".to_string(), "m1@studio.example".into()]);
        // Outlook's layout: a rule, who sent it to whom and when, then the original as it was.
        let tail = &r.body[r.body.find(markup::RULE).unwrap()..];
        assert!(tail.starts_with(&format!("{}\nFrom: Zoe Park <zoe@studio.example>\nSent: ", markup::RULE)), "{tail}");
        assert!(
            tail.contains("\nTo: Ann <ann@studio.example>; ops@studio.example\nCc: Kim <kim@studio.example>\nSubject: Café plans\n\nHello team,"),
            "{tail:?}"
        );
        assert!(!r.body.contains("wrote:"));
        // Sent formatted: the rule is a rule, the labels bold, the rest plain text.
        let html = markup::to_html(&r.body);
        assert!(
            html.contains("<hr") && html.contains("<b>From:</b> Zoe Park &lt;zoe@studio.example&gt;") && html.contains("<b>Subject:</b> Café plans")
        );
        assert!(html.contains("Hello team,") && !html.contains("<b>Hello"));
        assert!(markup::has_formatting(&r.body));
        assert_eq!(
            markup::header_lines("x\n____________\nFrom: a\n  b\nTo: c\n\nFrom: not a header"),
            [false, false, true, true, true, false, false]
        );
        let all = reply(&p, ReplyMode::ReplyAll, &me, &clock, "Ann");
        let cc: Vec<&str> = all.cc.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(cc, vec!["ops@studio.example", "kim@studio.example"]);
        assert!(all.body.starts_with("\n\n-- \nAnn"));
        let f = reply(&p, ReplyMode::Forward, &me, &clock, "");
        assert_eq!(f.subject, "Fwd: Café plans");
        assert!(f.to.is_empty() && f.in_reply_to.is_empty());
        assert!(f.body.contains("Subject: Café plans"));
        let rr =
            reply(&parse(&build(&r, &EmailAddress::new("", "ann@studio.example"), 0, "r@x").unwrap()).unwrap(), ReplyMode::Reply, &me, &clock, "");
        assert_eq!(rr.subject, "Re: Café plans");
        // Replying to my own message addresses its recipients.
        assert_eq!(rr.to[0].email, "zoe@studio.example");
    }

    #[test]
    fn html_tables() {
        let html = "<p>Before</p><table><tr><th>Item</th><th>Qty</th></tr><tr><td>Poster A2</td><td>2</td></tr></table>                    <table width=600><tr><td><p>Layout cell</p></td></tr></table><p>After</p>";
        let rich = html_to_rich(html);
        let lines: Vec<String> = rich.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect()).collect();
        let row = rich.iter().find(|l| l.iter().any(|s| s.text.contains("Poster A2"))).unwrap();
        assert!(row.iter().all(|s| s.code), "data rows line up in monospace");
        let layout = rich.iter().find(|l| l.iter().any(|s| s.text.contains("Layout cell"))).unwrap();
        assert!(!layout.iter().any(|s| s.code), "a layout table is just text");
        assert!(!lines.iter().any(|l| !l.is_empty() && l.chars().all(|c| c == '─')), "no stray rules: {lines:?}");
    }

    #[test]
    fn html_colours_and_hidden_parts() {
        let html = r#"<style>span.warn { font-weight: normal; color: #cc0000 }</style><div style="display:none">preheader text</div>
<p>Plain <span style="color:#ff0000">red</span> <span style="background-color: yellow">marked</span> <span class="warn">styled</span> <font color="blue">old</font></p>"#;
        let rich = html_to_rich(html);
        let spans: Vec<&Span> = rich.iter().flatten().collect();
        let find = |t: &str| spans.iter().find(|s| s.text.contains(t)).copied();
        assert_eq!(find("red").and_then(|s| s.color), Some([255, 0, 0]));
        assert_eq!(find("marked").and_then(|s| s.background), Some([255, 255, 0]));
        assert_eq!(find("styled").and_then(|s| s.color), Some([0xcc, 0, 0]), "<style> rules");
        assert!(find("old").and_then(|s| s.color).is_some(), "<font color>");
        assert_eq!(find("Plain").and_then(|s| s.color), None);
        assert!(find("preheader").is_none(), "display:none stays hidden");
    }

    #[test]
    fn helpers() {
        assert_eq!(sanitize_filename("..\\..\\a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize_filename("..."), "attachment");
        assert_eq!(mime_from_name("X.JPG"), "image/jpeg");
        assert_eq!(new_message_id("z@studio.example", 255, 1), "ff.0000000000000001@studio.example");
        assert_eq!(new_message_id("bad", -5, 0), "0.0000000000000000@mailcraft.invalid");
        assert_eq!(preview("a\n\n  b   c\n> q\n-- \nsig"), "a b c");
        assert_eq!(preview(&"x".repeat(500)).chars().count(), PREVIEW_CHARS + 1);
    }
}

#[cfg(test)]
mod importance_tests {
    use super::*;

    /// High and low importance, and a read receipt, go out as headers other mail programs read,
    /// and come back as the message's importance.
    #[test]
    fn importance_and_read_receipts_round_trip() {
        let from = EmailAddress::new("Riley", "riley@studio.example");
        let k = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        for (importance, header, priority) in [(1, "Importance: high", "X-Priority: 1"), (-1, "Importance: low", "X-Priority: 5")] {
            let d = Draft { subject: "Hi".into(), body: "x".into(), importance, read_receipt: true, ..Default::default() };
            let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
            let text = String::from_utf8_lossy(&raw);
            assert!(text.contains(header) && text.contains(priority), "{text}");
            assert!(text.contains("Disposition-Notification-To: <riley@studio.example>"), "{text}");
            assert_eq!(summary_from_full(&raw, k.clone(), Flags::default(), 1, 0).importance, importance);
        }
        let plain = build(&Draft { subject: "Hi".into(), body: "x".into(), ..Default::default() }, &from, 1_700_000_000, "m@x").unwrap();
        let text = String::from_utf8_lossy(&plain);
        assert!(!text.contains("Importance") && !text.contains("Disposition-Notification-To"));
        // Other mail programs' ways of saying it.
        for (h, want) in [("X-Priority: 2 (High)", 1), ("X-Priority: 3", 0), ("Priority: non-urgent", -1), ("Importance: HIGH", 1)] {
            let raw = format!("From: a@b.example\r\nSubject: s\r\n{h}\r\n\r\nbody\r\n");
            assert_eq!(summary_from_full(raw.as_bytes(), k.clone(), Flags::default(), 1, 0).importance, want, "{h}");
        }
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    pub(crate) fn report() -> Vec<u8> {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        format!("From: Reports <reports@fw.example>\r\nTo: me@studio.example\r\nSubject: Reports: Daily\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"M\"\r\n\r\n\
--M\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n\
--A\r\nContent-Type: text/plain\r\n\r\nHTML Report enclosed.\r\n\
--A\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><style>.title {{ background-color: #888888; color: #ffffff; font-size: 20px }} td.odd {{ background-color: #f0f0f0 }} .bar {{ background-color:#333333; color:#fff }}</style></head><body>\
<table width=\"100%\"><tr><td><img src=\"cid:logo@x\"></td><td><h2>Reports: Daily</h2></td></tr></table>\
<table width=\"100%\"><tr><td class=\"bar\"><img src=\"icon_config.png\"> Config</td></tr></table>\
<table width=\"100%\"><tr><td class=\"title\" colspan=\"2\">Summary</td></tr><tr><td>Hostname</td><td>untangle</td></tr><tr><td class=\"odd\">Version</td><td class=\"odd\">17.4.1</td></tr></table>\
<div style=\"text-align:center\"><img src=\"cid:chart%201.png\"><img src=\"cid:host.png\"></div></body></html>\r\n\
--A--\r\n\
--M\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=\"BrandingLogo.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-Location: icon_config.png\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-ID: <chart 1.png>\r\nContent-Disposition: inline; filename=\"chart 1.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-Disposition: attachment; filename=\"host.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M--\r\n").into_bytes()
    }

    /// Issue #1: a firewall's daily report as Outlook shows it. Its logo, icon and charts have
    /// no description and are found by `cid:`, by Content-Location and by file name; its grey
    /// bars and striped rows come from a style sheet's classes, and its titles are sized ones.
    #[test]
    fn a_generated_report_keeps_its_pictures_colours_and_titles() {
        let p = parse(&report()).unwrap();
        let found: Vec<Option<usize>> = p.images.iter().map(|i| picture_attachment(&p, i)).collect();
        assert_eq!(found, [Some(0), Some(1), Some(2), Some(3)], "{:?}", p.images);
        assert_eq!(picture_attachment(&p, "https://fw.example/logo.png"), None, "the web is never an attachment");
        assert_eq!(picture_attachment(&p, "cid:nothing@x"), None);
        assert_eq!(picture_attachment(&p, "images/host.png?v=2"), Some(3), "a path's file name");
        // Every picture has its place, in the flowing text and in the layout.
        let pictures = |r: &RichText| r.iter().flatten().filter(|s| s.image.is_some()).count();
        assert_eq!(pictures(&p.rich), 4);
        let mut cells = Vec::new();
        fn walk<'a>(b: &'a [layout::Block], out: &mut Vec<&'a layout::Cell>) {
            for x in b {
                if let layout::Block::Table(t) = x {
                    for c in t.rows.iter().flatten() {
                        out.push(c);
                        walk(&c.blocks, out);
                    }
                }
            }
        }
        walk(&p.layout, &mut cells);
        let text = |c: &layout::Cell| -> String {
            c.blocks
                .iter()
                .map(|b| if let layout::Block::Text(r) = b { r.iter().flatten().map(|s| s.text.as_str()).collect::<String>() } else { String::new() })
                .collect()
        };
        let cell = |t: &str| cells.iter().find(|c| text(c).contains(t)).copied().unwrap();
        assert_eq!(cell("Config").background, Some([0x33, 0x33, 0x33]), "a class's background");
        let summary = cell("Summary");
        assert_eq!((summary.background, summary.span), (Some([0x88, 0x88, 0x88]), 2));
        assert!(
            matches!(&summary.blocks[0], layout::Block::Text(r) if r[0][0].heading == 3 && r[0][0].color == Some([255, 255, 255])),
            "sized as a title, in the class's colour"
        );
        assert_eq!(cell("17.4.1").background, Some([0xf0, 0xf0, 0xf0]), "td.odd");
        assert_eq!(cell("untangle").background, None);
        let logo = cells.iter().find(
            |c| matches!(c.blocks.first(), Some(layout::Block::Text(r)) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:logo@x"))),
        );
        assert!(logo.is_some(), "the logo's cell stays");
        let charts = cells.iter().find(
            |c| matches!(c.blocks.first(), Some(layout::Block::Text(r)) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:host.png"))),
        );
        assert_eq!(charts.map(|c| c.align), Some(layout::Align::Center), "centred charts stay centred");
    }

    /// Tracking pixels stay out; a picture's own description is kept; `<font size>` titles.
    #[test]
    fn pictures_without_a_description_keep_their_place() {
        let r = html_to_rich(
            r#"<p><img src="cid:a@x"><img src="https://t.example/p.gif" width="1" height="1"><img src="cid:b@x" alt="Chart"><img alt="" src='cid:c@x'></p>"#,
        );
        let imgs: Vec<(&str, &str)> = r.iter().flatten().filter_map(|s| Some((s.image.as_deref()?, s.text.as_str()))).collect();
        assert_eq!(imgs, [("cid:a@x", ""), ("cid:b@x", "Chart"), ("cid:c@x", "")]);
        assert!(r.iter().flatten().all(|s| !s.text.contains(PICTURE_MARK)));
        let p = layout::inline_css(r#"<div><font size="6">Big</font></div><div style="font-size:11pt">Body</div>"#);
        let r = html_to_rich(&p);
        let level = |t: &str| r.iter().flatten().find(|s| s.text.contains(t)).map(|s| s.heading);
        assert_eq!((level("Big"), level("Body")), (Some(1), Some(0)), "{p}");
    }
}
