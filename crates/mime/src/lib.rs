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
mod fonts;
pub mod layout;
pub mod markup;
pub mod office;
pub mod rich;
pub use rich::reply::{ReplyMode, quoted_body, reply, reply_doc, reply_head_doc};

use mail_builder::MessageBuilder;
use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::MimePart;
use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders};
use mailcraft_model::thread::norm_id;
use mailcraft_model::{EmailAddress, Flags, MessageKey, Summary};
use serde::{Deserialize, Serialize};

mod attachments;
mod files;
mod hidden;
mod html;
mod photos;
pub mod smime;
pub use smime::pgp;
mod tnef;
mod trackers;
pub use attachments::{
    AttachmentInfo, AttachmentKind, HtmlRefs, MAX_WANTED, PendingFile, TEXT_FIRST, attachment_data, attachment_data_moved, attachment_out,
    attachment_text, attachments_data, attachments_moved, craft_app_for, draft_carries_files, index_doc, is_text_first, kind_of, part_name,
    quoted_pictures, quoted_pictures_given, text_first, text_preview_kind,
};
#[doc(hidden)]
pub use attachments::{IndexStats, index_doc_stats};
pub use attachments::{REACTIONS, is_reaction_text, reaction};
use attachments::{attached, has_real_attachments, reaction_of};
use files::attachment_name;
pub use files::{alt_code_char, alt_code_pages, harmless_to_open, mime_from_name, runs_code, sanitize_filename};
pub use hidden::{HIDDEN_WORDS_MIN, hidden_text};
use html::*;
pub use html::*;
pub use photos::without_location;
pub use tnef::{
    Tnef, TnefBody, TnefFile, byte_code_page, decode_code_page, decompress_rtf, known_code_page, read_tnef, rtf_body, rtf_wraps_html,
    unsupported_code_page, utf16_code_page,
};
pub use trackers::{Trackers, clean_link, trackers, tracks_click};

/// `bytes` read in the character set `charset` names (`iso-8859-1`, `shift_jis`), when it's one
/// known: for showing a source's text that isn't UTF-8 (View message source, issue #27).
pub fn decode_in(charset: &str, bytes: &[u8]) -> Option<String> {
    mail_parser::decoders::charsets::map::charset_decoder(charset.trim().as_bytes()).map(|d| d(bytes))
}

/// Longest preview kept in a summary, in characters.
pub const PREVIEW_CHARS: usize = 200;
/// The most of an HTML body read for its preview, in characters.
const PREVIEW_HTML: usize = 64 * 1024;
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
    // Mail in HTML only: its text as the reading pane's plain text makes it (mail-parser's own
    // runs one table cell into the next: "BANNERFirst block", issue #41), from its start.
    let html_only = m.text_bodies().next().is_some_and(|p| p.is_text_html());
    let text = match m.body_html(0).filter(|_| html_only) {
        Some(h) => html::html_to_words(h.get(..h.char_indices().nth(PREVIEW_HTML).map_or(h.len(), |(i, _)| i)).unwrap_or(&h)),
        None => m.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
    };
    let mut s = Summary {
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
        smime: m.root_part().content_type().map_or(mailcraft_model::Smime::None, |c| {
            mailcraft_model::Smime::from_content_type(
                c.ctype(),
                c.subtype().unwrap_or_default(),
                c.attribute("protocol"),
                c.attribute("smime-type"),
                c.attribute("name"),
            )
        }),
        importance: importance(m),
        reaction: match reaction_of(m) {
            r if r.is_empty() && m.root_part().content_disposition().is_some_and(|d| d.ctype().eq_ignore_ascii_case("reaction")) => {
                mailcraft_model::summary::REACTION_TO_READ.to_string()
            }
            r => r,
        },
        ..Default::default()
    };
    if let Some(index) = thread_index(m) {
        s.set_thread_index(&index);
    }
    s
}

/// The conversation index a message's `Thread-Index` header holds (base64 of 22 + 5·k bytes,
/// k ≤ [`mailcraft_model::thread::MAX_INDEX_BLOCKS`]); a longer header isn't decoded.
fn thread_index(m: &Message<'_>) -> Option<Vec<u8>> {
    let raw = m.header_raw("Thread-Index")?;
    let longest = (22 + 5 * mailcraft_model::thread::MAX_INDEX_BLOCKS).div_ceil(3) * 4;
    let clean: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).take(longest + 1).collect();
    if clean.is_empty() || clean.len() > longest {
        return None;
    }
    mail_parser::decoders::base64::base64_decode(clean.as_bytes())
}

/// Base64 text as bytes, its spaces and line breaks passed over (`None` when it isn't base64):
/// a card's picture and files (`data:` URIs) as an export writes them into a data file.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() {
        return Some(Vec::new());
    }
    mail_parser::decoders::base64::base64_decode(&clean)
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

/// A message ready for the reading pane.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parsed {
    pub subject: String,
    pub from: EmailAddress,
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    /// A sent or draft copy's Bcc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<EmailAddress>,
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
    /// Text the HTML body hides from its reader, when there's a lot of it ([`hidden_text`]), and
    /// how many words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hidden_text: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub hidden_words: usize,
    /// Tracking pictures and links that report clicks, in the HTML body.
    #[serde(default, skip_serializing_if = "Trackers::is_empty")]
    pub trackers: Trackers,
    /// `List-Unsubscribe` target, when the sender offers one.
    pub unsubscribe: Option<String>,
    /// Sent in bulk (a list, a newsletter, an automatic message), as [`Summary::bulk`] says,
    /// from this message's own header: whatever account it came through.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bulk: bool,
    /// A reaction to the message it answers (RFC 9078): its emoji, as [`Summary::reaction`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reaction: String,
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
    /// A picture's size as the message sets it (issue #10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_size: Option<ImageSize>,
}

fn is_none(v: &usize) -> bool {
    *v == 0
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// Lines of a rich message, each a list of spans.
pub type RichText = Vec<Vec<Span>>;

/// Parse a full message for display. `None` only for input that isn't a message at all.
pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let m = MessageParser::default().parse(raw)?;
    // mail-parser lists text parts among the HTML bodies too (converted); only real HTML counts.
    let real_html = m.html_bodies().any(|p| p.is_text_html());
    // A `winmail.dat` gives its files, and its rich body when the message has no HTML (issue #18).
    let (attached, tnef_body) = attached(&m);
    let tnef_html = match &tnef_body {
        TnefBody::Html(h) if !real_html => Some(h.as_str()),
        _ => None,
    };
    let html = m.body_html(0).filter(|_| real_html).or(tnef_html.map(std::borrow::Cow::Borrowed));
    let tracking = html.as_ref().map(|h| trackers(h)).unwrap_or_default();
    let mut hidden = None;
    let (rich, images, layout) = match &html {
        Some(h) => {
            // Style sheet rules as inline styles, so cells keep their colours and titles their size.
            let h = layout::inline_css(h);
            hidden = hidden_text(&h);
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
        _ if let Some(h) = tnef_html => (html_to_text(h), true),
        // Prefer the HTML version when the text part is an auto-generated stub.
        (_, Some(t), Some(h)) if t.trim().is_empty() => (html_to_text(&h), true),
        (true, Some(t), _) if !m.text_part(0).is_some_and(|p| p.is_text_html()) => (tidy_text(&t), real_html),
        (_, _, Some(h)) => (html_to_text(&h), true),
        (_, Some(t), None) => (tidy_text(&t), false),
        _ => (String::new(), false),
    };
    // Plain text Outlook wrapped, when the message has none of its own.
    let text = match &tnef_body {
        TnefBody::Text(t) if text.trim().is_empty() => tidy_text(t),
        _ => text,
    };
    let attachments = attached.into_iter().map(|a| a.info).collect();
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
        bcc: addrs(m.bcc()),
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
        hidden_words: hidden.as_ref().map_or(0, |(_, n)| *n),
        hidden_text: hidden.map(|(t, _)| t).unwrap_or_default(),
        trackers: tracking,
        unsubscribe,
        bulk: is_bulk(&m),
        reaction: reaction_of(&m),
    })
}

/// A message's words for search: its text (HTML as text) and its files' names, as one text
/// ([`index_doc`], which never decodes the files themselves).
pub fn search_text(raw: &[u8]) -> String {
    index_doc(raw).joined()
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

/// The header a draft's copy names the message it answers or forwards in ([`Draft::source`]).
pub const SOURCE_HEADER: &str = "X-MailCraft-Source";

/// What a saved message asks of its sending: its importance (high 1, low -1), whether it asks
/// for a read receipt, and the message it answers or forwards ([`SOURCE_HEADER`]). For a draft
/// opened to carry on writing it.
pub fn sending_options(raw: &[u8]) -> (i8, bool, String) {
    let Some(m) = MessageParser::default().parse(raw) else { return (0, false, String::new()) };
    let receipt = m.header_raw("Disposition-Notification-To").is_some_and(|v| !v.trim().is_empty());
    let source = m.header_raw(SOURCE_HEADER).map(|v| v.trim().to_string()).unwrap_or_default();
    (importance(&m), receipt, source)
}

/// The HTML body as written, for a message that has one: its own, or the one Outlook wrapped in
/// a `winmail.dat` (issue #18).
pub fn html_body(raw: &[u8]) -> Option<String> {
    let m = MessageParser::default().parse(raw)?;
    if !m.html_bodies().any(|p| p.is_text_html()) {
        return match attached(&m).1 {
            TnefBody::Html(h) => Some(h),
            _ => None,
        };
    }
    m.body_html(0).map(|h| h.into_owned())
}

/// A file attached to an outgoing message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutAttachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
    /// The `Content-ID` it had where it came from (a data file's attachment), kept on the file:
    /// it stays an attachment, listed, and whatever refers to it by that id still finds it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_id: String,
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
    /// Write Bcc into it: a draft's copy, which keeps who it goes to (a message sent never
    /// carries it; it goes in the envelope).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_bcc: bool,
    /// In a draft's copy too: the message it answers or forwards (`reply KEY`, `forward KEY`),
    /// which gets its flag once it's sent ([`SOURCE_HEADER`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Every part 7-bit (an attached message in base64 when it has 8-bit text), as a signed
    /// message must be to arrive byte for byte (RFC 8551 §3.1.2).
    #[serde(skip)]
    pub seven_bit: bool,
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

/// Serialise a draft to RFC 5322 bytes. Bcc is written into a draft's copy only
/// ([`Draft::keep_bcc`]); a message sent carries it in the SMTP envelope alone.
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

pub fn build<'d>(d: &'d Draft, from: &EmailAddress, date: i64, message_id: &str) -> Result<Vec<u8>, String> {
    let mut b = MessageBuilder::new().from(b_addr(from)).subject(d.subject.clone()).date(date.max(0)).message_id(message_id.to_string());
    if !d.to.is_empty() {
        b = b.to(b_list(&d.to));
    }
    if !d.cc.is_empty() {
        b = b.cc(b_list(&d.cc));
    }
    if d.keep_bcc && !d.bcc.is_empty() {
        b = b.bcc(b_list(&d.bcc));
    }
    // One line, as ids are (a key holds a folder's name, which could hold anything).
    let source: String = d.source.chars().filter(|c| !c.is_control()).take(900).collect();
    if d.keep_bcc && !source.trim().is_empty() {
        b = b.header(SOURCE_HEADER, mail_builder::headers::text::Text::new(source));
    }
    if !d.reply_to.is_empty() {
        b = b.reply_to(b_list(&d.reply_to));
    }
    // Ids written as they're kept (a draft's could come from anywhere): no line breaks.
    let in_reply_to = mailcraft_model::thread::norm_id(&d.in_reply_to);
    if !in_reply_to.is_empty() {
        b = b.in_reply_to(in_reply_to);
    }
    let references: Vec<String> = d.references.iter().map(|r| mailcraft_model::thread::norm_id(r)).filter(|r| !r.is_empty()).collect();
    if !references.is_empty() {
        b = b.references(references);
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
    let multipart = |kind: &str, n: u32, parts: Vec<MimePart<'d>>| {
        MimePart::new(ContentType::new(kind.to_string()).attribute("boundary", format!("=_mc{seed}_{n}")), parts)
    };
    // The parts borrow the draft's files and texts: each is read where it is, not copied first
    // (an imported message's attachments can be tens of megabytes).
    let files: Vec<MimePart<'d>> = d
        .attachments
        .iter()
        .map(|a| {
            let mime = if a.mime.is_empty() { mime_from_name(&a.name).to_string() } else { a.mime.clone() };
            // An attached message goes as it is (RFC 2046 §5.2.1: 7bit or 8bit, never base64).
            if mime.eq_ignore_ascii_case("message/rfc822") {
                // In a signed message: encoded (base64) by mail-builder, as binary.
                if d.seven_bit && !a.data.is_ascii() {
                    return MimePart::new(mime, a.data.as_slice()).attachment(sanitize_filename(&a.name));
                }
                let cte = if a.data.is_ascii() { "7bit" } else { "8bit" };
                return MimePart::new(mime, a.data.as_slice())
                    .attachment(sanitize_filename(&a.name))
                    .header("Content-Transfer-Encoding", mail_builder::headers::raw::Raw::new(cte));
            }
            // A report's status goes as it is when it can (RFC 3464 §2.1: 7bit, so it stays
            // readable); one that can't is encoded as any other file.
            let as_is = REPORT_STATUS.iter().any(|s| mime.eq_ignore_ascii_case(s)) && seven_bit(&a.data);
            let part = MimePart::new(mime, a.data.as_slice()).attachment(sanitize_filename(&a.name));
            let part = if as_is { part.header("Content-Transfer-Encoding", mail_builder::headers::raw::Raw::new("7bit")) } else { part };
            // A kept Content-ID goes into a header line as it is: one with spaces, line breaks or
            // brackets inside, or too long for the line, is left off.
            let cid = a.content_id.trim().trim_start_matches('<').trim_end_matches('>');
            let fits = !cid.is_empty() && cid.len() <= MAX_CID && !cid.chars().any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>');
            if fits { part.cid(cid.to_string()) } else { part }
        })
        .collect();
    // The text with bare line ends (mail-builder writes CR LF): copied only when it has others.
    let body: std::borrow::Cow<'d, str> = if d.body.contains("\r\n") { d.body.replace("\r\n", "\n").into() } else { d.body.as_str().into() };
    let text = MimePart::new("text/plain", body);
    let root = match &d.html {
        None if files.is_empty() => text,
        None => multipart("multipart/mixed", 1, std::iter::once(text).chain(files).collect()),
        Some(html) => {
            // mixed( related( alternative(text, html), pictures… ), attachments… ): the pictures
            // belong with the HTML that shows them (RFC 2387), the files with the message.
            let alt = multipart("multipart/alternative", 3, vec![text, MimePart::new("text/html", html.as_str())]);
            let mut related = vec![alt];
            for p in &d.inline {
                let mime = if p.mime.is_empty() { mime_from_name(&p.name).to_string() } else { p.mime.clone() };
                // A named picture keeps its name (`filename=` and `name=`, as Outlook writes them), so a
                // saved copy or another client's list shows it: still inline, and not a file while the
                // HTML shows it.
                let part = match files::picture_name(&p.name, &mime) {
                    Some(name) => MimePart::new(ContentType::new(mime).attribute("name", name.clone()), p.data.as_slice())
                        .header("Content-Disposition", ContentType::new("inline").attribute("filename", name)),
                    None => MimePart::new(mime, p.data.as_slice()).inline(),
                };
                related.push(part.cid(p.cid.clone()));
            }
            let root = if related.len() == 1 { related.remove(0) } else { multipart("multipart/related", 2, related) };
            if files.is_empty() { root } else { multipart("multipart/mixed", 1, std::iter::once(root).chain(files).collect()) }
        }
    };
    b.body(root).write_to_vec().map_err(|e| e.to_string())
}

/// Longest `Content-ID` an attachment keeps, in bytes: its header line stays within RFC 5322's
/// 998 characters.
const MAX_CID: usize = 900;

/// The machine-readable part of a `multipart/report` (RFC 6522): a delivery status (RFC 3464),
/// a read receipt (RFC 8098) or a feedback report (RFC 5965).
pub const REPORT_STATUS: [&str; 3] = ["message/delivery-status", "message/disposition-notification", "message/feedback-report"];

/// Whether `data` is 7bit data as RFC 2045 §2.7 says: ASCII without NUL, CR and LF only together,
/// lines of at most 998 characters. A line that starts with `--` could close the message's
/// multipart early (RFC 2046 §5.1.1), so data with one isn't sent as it is either.
fn seven_bit(data: &[u8]) -> bool {
    let mut line = 0usize;
    for (i, &b) in data.iter().enumerate() {
        match b {
            b'\r' if data.get(i.saturating_add(1)) != Some(&b'\n') => return false,
            b'\n' if i.checked_sub(1).and_then(|j| data.get(j)) != Some(&b'\r') => return false,
            b'\r' | b'\n' => line = 0,
            0 | 0x80.. => return false,
            b'-' if line == 1 && i.checked_sub(1).and_then(|j| data.get(j)) == Some(&b'-') => return false,
            _ => {
                line += 1;
                if line > 998 {
                    return false;
                }
            }
        }
    }
    true
}

/// FNV-1a, for boundaries.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests;
