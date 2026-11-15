//! An outgoing message: the draft being written and the RFC 5322 bytes it's built into, and
//! what a saved one asks of its sending (its importance, a read receipt, the message it
//! answers or forwards).

pub mod reply;

use super::*;

/// The header a draft's copy names the message it answers or forwards in ([`Draft::source`]).
pub const SOURCE_HEADER: &str = "X-MailCraft-Source";

/// The header on each copy of a draft MailCraft files on an IMAP server: a random mark of that
/// copy's own, which tells it from another program's copy of the draft. Never in a draft opened
/// again to be written, nor in what's sent: both are built anew from what the draft says.
pub const COPY_HEADER: &str = "X-MailCraft-Copy";

/// What a saved message asks of its sending: its importance (high 1, low -1), whether it asks
/// for a read receipt, and the message it answers or forwards ([`SOURCE_HEADER`]). For a draft
/// opened to carry on writing it.
pub fn sending_options(raw: &[u8]) -> (i8, bool, String) {
    let Some(m) = MessageParser::default().parse(raw) else { return (0, false, String::new()) };
    let receipt = header_raw(m.root(), "Disposition-Notification-To").is_some_and(|v| !v.trim().is_empty());
    (importance(&m), receipt, named_source(&m))
}

/// What a message's [`SOURCE_HEADER`] says (`reply KEY`, `forward KEY`): unfolded, its encoded
/// words read (a folder's name in it may be in any script). Nothing for a field longer than
/// [`build`] writes, with room for its encoding: a message from anywhere can carry one.
fn named_source(m: &Message<'_>) -> String {
    let Some(field) = m.root().headers().get(SOURCE_HEADER) else { return String::new() };
    let raw = field.raw_value();
    if raw.len() > 8 << 10 {
        return String::new();
    }
    let parsed = mail_parser::HeaderForm::Text.parse(raw);
    parsed.value().as_text().map(|v| v.trim().to_string()).unwrap_or_default()
}

/// The message a draft of a forward forwards, as its [`SOURCE_HEADER`] names it (a key's id,
/// [`MessageKey::to_id`]); empty for any other message.
pub(super) fn forward_of(m: &Message<'_>) -> String {
    let source = named_source(m);
    source.strip_prefix("forward ").and_then(|k| MessageKey::from_id(k.trim())).map(|k| k.to_id()).unwrap_or_default()
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

/// A meeting's invitation, answer or cancellation (iTIP, RFC 5546) carried in a message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutCalendar {
    /// The iTIP method (`REQUEST`, `REPLY`, `CANCEL`…), as in the calendar's `METHOD`.
    pub method: String,
    /// The iCalendar text (RFC 5545).
    pub data: String,
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
    /// A meeting's calendar, sent as iMIP says (RFC 6047 §2.4): a `text/calendar` part with its
    /// `method` and `charset`, inline beside the text in `multipart/alternative`, as Outlook and
    /// Thunderbird send it. Exchange takes a calendar only as a file for a file, not a meeting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<OutCalendar>,
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

pub(super) fn b_addr(a: &EmailAddress) -> mail_builder::headers::address::Address<'static> {
    if a.name.trim().is_empty() { a.email.clone().into() } else { (a.name.trim().to_string(), a.email.clone()).into() }
}

pub(super) fn b_list(v: &[EmailAddress]) -> mail_builder::headers::address::Address<'static> {
    v.iter().map(b_addr).collect::<Vec<_>>().into()
}

/// A fresh Message-ID: `<time.seed@domain>`. The caller supplies the randomness (`seed`), so this
/// stays deterministic and wasm-safe.
pub fn new_message_id(from_email: &str, time: i64, seed: u64) -> String {
    let domain = from_email.rsplit_once('@').map(|(_, d)| d).filter(|d| !d.is_empty()).unwrap_or("mailcraft.invalid");
    format!("{:x}.{:016x}@{}", time.max(0), seed, domain)
}

/// The message with its `Date:` set to `date` (a message held back to be sent later is dated
/// when it goes, as Outlook dates delayed mail). Messages without a header block come back as
/// they were.
pub fn with_date(raw: &[u8], date: i64) -> Vec<u8> {
    let Some(blank) = header_end(raw) else { return raw.to_vec() };
    let mut out = raw.to_vec();
    if let Some(field) = field_span(raw.get(..blank.start).unwrap_or_default(), "Date") {
        out.splice(field, format!("Date: {}", mail_builder::headers::date::Date::new(date.max(0)).to_rfc822()).into_bytes());
    }
    out
}

/// Serialise a draft to RFC 5322 bytes. Bcc is written into a draft's copy only
/// ([`Draft::keep_bcc`]); a message sent carries it in the SMTP envelope alone.
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
                // Its lines end CR LF as it goes: a file saved with bare line feeds went with
                // them, which mail servers refuse outright (and a line of one dot in it wasn't
                // doubled, so the message ended there for a server that takes them).
                let part = if crate::smime::bare_line_ends(&a.data) {
                    MimePart::new(mime, crate::smime::crlf(&a.data))
                } else {
                    MimePart::new(mime, a.data.as_slice())
                };
                return part.attachment(sanitize_filename(&a.name)).header("Content-Transfer-Encoding", mail_builder::headers::raw::Raw::new(cte));
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
    // A meeting's calendar: the last alternative, the one a calendar program reads (RFC 6047 §2.4).
    let calendar = d.calendar.as_ref().map(calendar_part);
    let root = match &d.html {
        None => {
            let body = match calendar {
                Some(c) => multipart("multipart/alternative", 3, vec![text, c]),
                None => text,
            };
            if files.is_empty() { body } else { multipart("multipart/mixed", 1, std::iter::once(body).chain(files).collect()) }
        }
        Some(html) => {
            // mixed( related( alternative(text, html), pictures… ), attachments… ): the pictures
            // belong with the HTML that shows them (RFC 2387), the files with the message.
            let alt = multipart(
                "multipart/alternative",
                3,
                std::iter::once(text).chain(std::iter::once(MimePart::new("text/html", html.as_str()))).chain(calendar).collect(),
            );
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

/// A meeting's calendar as a message part: `text/calendar; charset=utf-8; method=…`, inline (no
/// `Content-Disposition`), its line ends left to mail-builder (which writes CR LF).
fn calendar_part(c: &OutCalendar) -> MimePart<'_> {
    // The method is a token (RFC 5546 §1.4); anything else in it is left out of the header.
    let method: String =
        c.method.trim().chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-').take(64).collect::<String>().to_ascii_uppercase();
    let ct = ContentType::new("text/calendar").attribute("charset", "utf-8");
    let ct = if method.is_empty() { ct } else { ct.attribute("method", method) };
    let data: std::borrow::Cow<'_, str> = if c.data.contains("\r\n") { c.data.replace("\r\n", "\n").into() } else { c.data.as_str().into() };
    MimePart::new(ct, data)
}

/// Longest `Content-ID` an attachment keeps, in bytes: its header line stays within RFC 5322's
/// 998 characters.
pub(super) const MAX_CID: usize = 900;

/// The machine-readable part of a `multipart/report` (RFC 6522): a delivery status (RFC 3464),
/// a read receipt (RFC 8098) or a feedback report (RFC 5965).
pub const REPORT_STATUS: [&str; 3] = ["message/delivery-status", "message/disposition-notification", "message/feedback-report"];

/// Whether `data` is 7bit data as RFC 2045 §2.7 says: ASCII without NUL, CR and LF only together,
/// lines of at most 998 characters. A line that starts with `--` could close the message's
/// multipart early (RFC 2046 §5.1.1), so data with one isn't sent as it is either.
pub(super) fn seven_bit(data: &[u8]) -> bool {
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
pub(super) fn fnv(s: &str) -> u64 {
    mailcraft_model::hash::fnv1a(s.as_bytes())
}
