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

use mail_builder::MessageBuilder;
use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders};
use mailcraft_model::dates::{Clock, full_time};
use mailcraft_model::thread::norm_id;
use mailcraft_model::{EmailAddress, Flags, MessageKey, Summary};
use serde::{Deserialize, Serialize};

/// Longest preview kept in a summary, in characters.
pub const PREVIEW_CHARS: usize = 200;
/// HTML larger than this is cut before conversion (the rest is rarely more than markup).
const HTML_MAX: usize = 4 << 20;

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
    }
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
    pub attachments: Vec<AttachmentInfo>,
    /// `List-Unsubscribe` target, when the sender offers one.
    pub unsubscribe: Option<String>,
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
            AttachmentInfo { index, name, mime, size: p.len(), inline: p.is_inline_part() }
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
        attachments,
        unsubscribe,
    })
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
pub fn build(d: &Draft, from: &EmailAddress, date: i64, message_id: &str) -> Result<Vec<u8>, String> {
    let mut b = MessageBuilder::new().from(b_addr(from)).subject(d.subject.clone()).date(date.max(0)).message_id(message_id.to_string());
    if !d.to.is_empty() {
        b = b.to(b_list(&d.to));
    }
    if !d.cc.is_empty() {
        b = b.cc(b_list(&d.cc));
    }
    if !d.in_reply_to.is_empty() {
        b = b.in_reply_to(d.in_reply_to.clone());
    }
    if !d.references.is_empty() {
        b = b.references(d.references.clone());
    }
    b = b.header("X-Mailer", mail_builder::headers::raw::Raw::new(format!("MailCraft {}", env!("CARGO_PKG_VERSION"))));
    b = b.text_body(d.body.replace("\r\n", "\n"));
    for a in &d.attachments {
        let mime = if a.mime.is_empty() { mime_from_name(&a.name).to_string() } else { a.mime.clone() };
        b = b.attachment(mime, sanitize_filename(&a.name), a.data.clone());
    }
    b.write_to_vec().map_err(|e| e.to_string())
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

fn header_block(p: &Parsed, clock: &Clock) -> String {
    let mut s = String::new();
    s.push_str(&format!("From: {}\n", p.from));
    s.push_str(&format!("Date: {}\n", full_time(p.date, clock)));
    if !p.to.is_empty() {
        s.push_str(&format!("To: {}\n", mailcraft_model::addr::format_list(&p.to)));
    }
    if !p.cc.is_empty() {
        s.push_str(&format!("Cc: {}\n", mailcraft_model::addr::format_list(&p.cc)));
    }
    s.push_str(&format!("Subject: {}\n", p.subject));
    s
}

/// A reply or forward of `p`. `me` are the user's own addresses, left out of Reply All.
pub fn reply(p: &Parsed, mode: ReplyMode, me: &[String], clock: &Clock, signature: &str) -> Draft {
    let mine = |a: &EmailAddress| me.iter().any(|m| m.eq_ignore_ascii_case(a.email.trim()));
    let sig = if signature.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", signature.trim_end()) };
    let mut d = Draft::default();
    match mode {
        ReplyMode::Forward => {
            d.subject = prefixed("Fwd:", &p.subject);
            d.body = format!("{sig}\n\n----- Forwarded message -----\n{}\n{}", header_block(p, clock), p.text);
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
            let quoted: String = p.text.lines().map(|l| if l.is_empty() { ">\n".to_string() } else { format!("> {l}\n") }).collect();
            d.body = format!("{sig}\n\nOn {}, {} wrote:\n{}", full_time(p.date, clock), p.from.display(), quoted);
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
    d
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(r.body.contains("> Hello team,"));
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
