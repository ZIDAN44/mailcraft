//! Outlook's `winmail.dat` (`application/ms-tnef`, MS-OXTNEF): the files and the rich body
//! Outlook wraps in it when it sends in Rich Text Format (issue #18). Without reading it, the
//! files and the formatting were one `winmail.dat` attachment nothing could open.
//!
//! The stream is a signature and a run of attributes, each `level id length data checksum`; the
//! message's own properties and each attachment's come as MAPI property lists inside
//! `attMsgProps` and `attAttachment`. The body is HTML (`PidTagHtml`), compressed RTF
//! (`PidTagRtfCompressed`, MS-OXRTFCP, often HTML wrapped in RTF, MS-OXRTFEX) or plain text. An
//! attached Outlook message is a TNEF stream of its own, made into an `.eml`.
//!
//! Everything here is hostile input: lengths are checked against what's there, counts and
//! nesting are capped, and a damaged stream gives what was read before the damage (or `None`).

use mail_builder::MessageBuilder;
use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::MimePart;

use crate::files::mime_from_name;

mod rtf;

pub use rtf::{decompress_rtf, rtf_body};

/// The signature a TNEF stream starts with (little-endian `0x223E9F78`).
const SIGNATURE: [u8; 4] = [0x78, 0x9F, 0x3E, 0x22];
/// Most attachments read from one stream.
const MAX_FILES: usize = 500;
/// Most properties read from one list.
const MAX_PROPS: usize = 4096;
/// How deep attached messages are read (each one a stream of its own).
const MAX_DEPTH: u8 = 3;

// Attribute ids (level 1: the message; level 2: an attachment).
const ATT_OEM_CODEPAGE: u32 = 0x0006_9007;
const ATT_SUBJECT: u32 = 0x0001_8004;
const ATT_BODY: u32 = 0x0002_800C;
const ATT_MSG_PROPS: u32 = 0x0006_9003;
const ATT_REND_DATA: u32 = 0x0006_9002;
const ATT_TITLE: u32 = 0x0001_8010;
const ATT_DATA: u32 = 0x0006_800F;
const ATT_ATTACHMENT: u32 = 0x0006_9005;
const ATT_TRANSPORT_NAME: u32 = 0x0006_9001;

// MAPI property ids.
const PR_SUBJECT: u16 = 0x0037;
const PR_CLIENT_SUBMIT_TIME: u16 = 0x0039;
const PR_SENDER_NAME: u16 = 0x0C1A;
const PR_SENDER_EMAIL: u16 = 0x0C1F;
const PR_BODY: u16 = 0x1000;
const PR_RTF_COMPRESSED: u16 = 0x1009;
const PR_HTML: u16 = 0x1013;
const PR_DISPLAY_NAME: u16 = 0x3001;
const PR_ATTACH_DATA: u16 = 0x3701;
const PR_ATTACH_FILENAME: u16 = 0x3704;
const PR_ATTACH_METHOD: u16 = 0x3705;
const PR_ATTACH_LONG_FILENAME: u16 = 0x3707;
const PR_ATTACH_MIME_TAG: u16 = 0x370E;
const PR_ATTACH_CONTENT_ID: u16 = 0x3712;
const PR_INTERNET_CPID: u16 = 0x3FDE;
const PR_MESSAGE_CODEPAGE: u16 = 0x3FFD;
const PR_ATTACHMENT_HIDDEN: u16 = 0x7FFE;

/// `attach method`: an attached message.
const METHOD_EMBEDDED: i64 = 5;

/// What a `winmail.dat` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tnef {
    pub subject: String,
    /// Who sent it and when, as an attached message says (empty and 0 when it doesn't).
    pub from_name: String,
    pub from_email: String,
    pub date: i64,
    pub body: TnefBody,
    pub attachments: Vec<TnefFile>,
}

/// A TNEF body, as the reading pane takes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TnefBody {
    #[default]
    None,
    Html(String),
    Text(String),
}

impl TnefBody {
    pub fn is_empty(&self) -> bool {
        match self {
            TnefBody::None => true,
            TnefBody::Html(s) | TnefBody::Text(s) => s.trim().is_empty(),
        }
    }
}

/// A file inside a `winmail.dat`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TnefFile {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
    /// For a picture the body shows (`cid:`).
    pub content_id: Option<String>,
    /// Outlook keeps pictures in the body out of the attachment list.
    pub hidden: bool,
}

/// One MAPI property value (only the kinds this reads).
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Int(i64),
    Bool(bool),
    /// An 8-bit string in the stream's code page.
    Str8(Vec<u8>),
    Unicode(String),
    Binary(Vec<u8>),
    /// An object: its interface id, then its data.
    Object(Vec<u8>),
    /// A time (FILETIME: 100 ns since 1601).
    Time(u64),
    Other,
}

type Props = Vec<(u16, Value)>;

fn prop(props: &Props, id: u16) -> Option<&Value> {
    props.iter().find(|(i, _)| *i == id).map(|(_, v)| v)
}

/// The files and body in a `winmail.dat`; `None` for bytes that aren't one.
pub fn read_tnef(data: &[u8]) -> Option<Tnef> {
    read_at(data, 0)
}

/// Whether an attachment is a `winmail.dat` (by its type, or its name when sent as plain bytes).
pub(crate) fn is_tnef(mime: &str, name: Option<&str>) -> bool {
    let mime = mime.to_ascii_lowercase();
    mime == "application/ms-tnef" || mime == "application/vnd.ms-tnef" || name.is_some_and(|n| n.trim().eq_ignore_ascii_case("winmail.dat"))
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let piece = self.data.get(self.at..end)?;
        self.at = end;
        Some(piece)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
    }
    /// Past the padding that brings `n` bytes to a multiple of four.
    fn pad(&mut self, n: usize) -> Option<()> {
        self.take((4 - n % 4) % 4).map(|_| ())
    }
    fn left(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }
}

/// An attachment as the stream builds it up.
#[derive(Default)]
struct Building {
    title: Option<Vec<u8>>,
    transport: Option<Vec<u8>>,
    data: Option<Vec<u8>>,
    props: Props,
}

fn read_at(data: &[u8], depth: u8) -> Option<Tnef> {
    let mut r = Reader { data, at: 0 };
    if r.take(4)? != SIGNATURE {
        return None;
    }
    r.u16()?; // the legacy key
    let mut oem = 1252;
    let mut subject = None;
    let mut body_attr = None;
    let mut msg = Props::new();
    let mut files: Vec<Building> = Vec::new();
    // A damaged or cut-short stream keeps what came before.
    while r.left() >= 9 {
        let Some(level) = r.take(1).and_then(|b| b.first().copied()) else { break };
        let (Some(id), Some(len)) = (r.u32(), r.u32()) else { break };
        let Some(value) = r.take(len as usize) else { break };
        if r.u16().is_none() {
            break;
        }
        match (level, id) {
            (1, ATT_OEM_CODEPAGE) => oem = Reader { data: value, at: 0 }.u32().unwrap_or(1252),
            (1, ATT_SUBJECT) => subject = Some(value.to_vec()),
            (1, ATT_BODY) => body_attr = Some(value.to_vec()),
            (1, ATT_MSG_PROPS) => msg.extend(props(value)),
            (2, ATT_REND_DATA) => {
                if files.len() >= MAX_FILES {
                    break;
                }
                files.push(Building::default());
            }
            (2, ATT_TITLE | ATT_DATA | ATT_ATTACHMENT | ATT_TRANSPORT_NAME) => {
                if files.is_empty() {
                    files.push(Building::default());
                }
                let Some(f) = files.last_mut() else { break };
                match id {
                    ATT_TITLE => f.title = Some(value.to_vec()),
                    ATT_DATA => f.data = Some(value.to_vec()),
                    ATT_TRANSPORT_NAME => f.transport = Some(value.to_vec()),
                    _ => f.props.extend(props(value)),
                }
            }
            _ => {}
        }
    }
    // Strings in the properties are in the message's code page; the attributes' in the stream's.
    let cp = match prop(&msg, PR_INTERNET_CPID).or(prop(&msg, PR_MESSAGE_CODEPAGE)) {
        Some(Value::Int(n)) if *n > 0 => *n as u32,
        _ => oem,
    };
    let subject = text(prop(&msg, PR_SUBJECT), cp).or_else(|| subject.map(|s| decode(trim_nul(&s), oem))).unwrap_or_default();
    let body = body_of(&msg, cp, body_attr.as_deref().map(|b| decode(trim_nul(b), oem)));
    let attachments = files.into_iter().enumerate().filter_map(|(i, f)| file(f, i, oem, cp, depth)).collect();
    let (from_name, from_email, date) = sender_and_date(&msg, cp);
    Some(Tnef {
        subject: subject.trim().to_string(),
        from_name: from_name.unwrap_or_default(),
        from_email: from_email.unwrap_or_default(),
        date: date.unwrap_or(0),
        body,
        attachments,
    })
}

/// The body: the HTML when there is one, else the compressed RTF (the HTML or text it carries,
/// or itself as simple HTML), else the plain text.
fn body_of(msg: &Props, cp: u32, attr: Option<String>) -> TnefBody {
    match prop(msg, PR_HTML) {
        Some(Value::Binary(b)) => return TnefBody::Html(decode(b, cp)),
        Some(Value::Str8(b)) => return TnefBody::Html(decode(trim_nul(b), cp)),
        Some(Value::Unicode(s)) => return TnefBody::Html(s.clone()),
        _ => {}
    }
    if let Some(Value::Binary(b)) = prop(msg, PR_RTF_COMPRESSED)
        && let Some(rtf) = decompress_rtf(b)
    {
        let body = rtf_body(&rtf);
        if !body.is_empty() {
            return body;
        }
    }
    match text(prop(msg, PR_BODY), cp).or(attr) {
        Some(t) => TnefBody::Text(t.replace("\r\n", "\n")),
        None => TnefBody::None,
    }
}

fn file(f: Building, index: usize, oem: u32, cp: u32, depth: u8) -> Option<TnefFile> {
    let p = &f.props;
    let name = text(prop(p, PR_ATTACH_LONG_FILENAME), cp)
        .or_else(|| text(prop(p, PR_DISPLAY_NAME), cp))
        .or_else(|| f.title.as_deref().map(|t| decode(trim_nul(t), oem)))
        .or_else(|| text(prop(p, PR_ATTACH_FILENAME), cp))
        .or_else(|| f.transport.as_deref().map(|t| decode(trim_nul(t), oem)))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let embedded = matches!(prop(p, PR_ATTACH_METHOD), Some(Value::Int(METHOD_EMBEDDED)));
    let (name, mime, data) = match (prop(p, PR_ATTACH_DATA), embedded) {
        // An attached Outlook message: a stream of its own, after the object's interface id.
        (Some(Value::Object(o)), true) => {
            let inner = o.get(16..).filter(|_| depth < MAX_DEPTH).and_then(|t| read_at(t, depth + 1))?;
            let eml = to_eml(&inner)?;
            let title = name.unwrap_or_else(|| if inner.subject.is_empty() { format!("message {}", index + 1) } else { inner.subject.clone() });
            (Some(format!("{}.eml", title.trim_end_matches(".msg"))), "message/rfc822".to_string(), eml)
        }
        (Some(Value::Binary(b)), _) => (name, String::new(), b.clone()),
        // An OLE object: its data after the interface id.
        (Some(Value::Object(o)), false) => (name, String::new(), o.get(16..).unwrap_or_default().to_vec()),
        _ => (name, String::new(), f.data?),
    };
    let name = name.unwrap_or_else(|| format!("attachment {}", index + 1));
    let mime = match (mime.is_empty(), text(prop(p, PR_ATTACH_MIME_TAG), cp)) {
        (false, _) => mime,
        (true, Some(m)) if m.contains('/') && !m.contains(char::is_whitespace) => m.to_ascii_lowercase(),
        _ => mime_from_name(&name).to_string(),
    };
    let content_id = text(prop(p, PR_ATTACH_CONTENT_ID), cp).map(|c| c.trim().trim_matches(['<', '>']).to_string()).filter(|c| !c.is_empty());
    let hidden = matches!(prop(p, PR_ATTACHMENT_HIDDEN), Some(Value::Bool(true)));
    Some(TnefFile { name, mime, data, content_id, hidden })
}

/// An attached Outlook message as an RFC 5322 message, so the reading pane opens it as one.
fn to_eml(t: &Tnef) -> Option<Vec<u8>> {
    let mut b = MessageBuilder::new().subject(t.subject.clone());
    if !t.from_email.is_empty() {
        b = b.from((t.from_name.clone(), t.from_email.clone()));
    }
    if t.date > 0 {
        b = b.date(t.date);
    }
    let body = match &t.body {
        TnefBody::Html(h) => MimePart::new(ContentType::new("text/html").attribute("charset", "utf-8"), h.clone()),
        TnefBody::Text(s) => MimePart::new(ContentType::new("text/plain").attribute("charset", "utf-8"), s.clone()),
        TnefBody::None => MimePart::new(ContentType::new("text/plain").attribute("charset", "utf-8"), String::new()),
    };
    let mut parts = vec![body];
    for f in &t.attachments {
        let mut part = MimePart::new(ContentType::new(f.mime.clone()), f.data.clone());
        part = match &f.content_id {
            Some(cid) if f.hidden || matches!(&t.body, TnefBody::Html(h) if h.contains(&format!("cid:{cid}"))) => part.inline().cid(cid.clone()),
            _ => part.attachment(f.name.clone()),
        };
        parts.push(part);
    }
    b = if parts.len() == 1 { b.body(parts.remove(0)) } else { b.body(MimePart::new(ContentType::new("multipart/mixed"), parts)) };
    b.write_to_vec().ok()
}

fn trim_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().rposition(|c| *c != 0).map_or(0, |i| i + 1);
    b.get(..end).unwrap_or_default()
}

/// A string property as text.
fn text(v: Option<&Value>, cp: u32) -> Option<String> {
    match v? {
        Value::Str8(b) => Some(decode(trim_nul(b), cp)),
        Value::Unicode(s) => Some(s.clone()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

/// Bytes in a Windows code page as text (Windows-1252 for pages this doesn't know).
pub(crate) fn decode(b: &[u8], cp: u32) -> String {
    let name: &[u8] = match cp {
        65001 => return String::from_utf8_lossy(b).into_owned(),
        1200 => return utf16(b),
        874 => b"windows-874",
        932 => b"shift_jis",
        936 | 54936 => b"gbk",
        949 => b"euc-kr",
        950 => b"big5",
        1250 => b"windows-1250",
        1251 => b"windows-1251",
        1253 => b"windows-1253",
        1254 => b"windows-1254",
        1255 => b"windows-1255",
        1256 => b"windows-1256",
        1257 => b"windows-1257",
        1258 => b"windows-1258",
        20866 => b"koi8-r",
        28591 => b"iso-8859-1",
        28592 => b"iso-8859-2",
        28595 => b"iso-8859-5",
        28597 => b"iso-8859-7",
        28605 => b"iso-8859-15",
        10000 => b"macintosh",
        20127 => return b.iter().map(|c| if c.is_ascii() { char::from(*c) } else { '\u{FFFD}' }).collect(),
        _ => b"windows-1252",
    };
    // Plain ASCII reads the same in all of them.
    if b.is_ascii() {
        return b.iter().map(|c| char::from(*c)).collect();
    }
    match mail_parser::decoders::charsets::map::charset_decoder(name) {
        Some(f) => f(b),
        None => String::from_utf8_lossy(b).into_owned(),
    }
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).filter(|c| *c != '\0').collect()
}

/// A MAPI property list (`attMsgProps`, `attAttachment`). A damaged list gives what came before.
fn props(data: &[u8]) -> Props {
    let mut r = Reader { data, at: 0 };
    let mut out = Props::new();
    let Some(count) = r.u32() else { return out };
    for _ in 0..(count as usize).min(MAX_PROPS) {
        match one_prop(&mut r) {
            Some(p) => out.push(p),
            None => break,
        }
    }
    out
}

fn one_prop(r: &mut Reader<'_>) -> Option<(u16, Value)> {
    let kind = r.u16()?;
    let id = r.u16()?;
    if id >= 0x8000 {
        // A named property: its namespace and name, which this doesn't need.
        r.take(16)?;
        match r.u32()? {
            0 => {
                r.u32()?;
            }
            1 => {
                let n = r.u32()? as usize;
                r.take(n)?;
                r.pad(n)?;
            }
            _ => return None,
        }
    }
    let base = kind & 0x0FFF;
    let multi = kind & 0x1000 != 0;
    let fixed = match base {
        0x0002 | 0x000B => Some(2),
        0x0003 | 0x0004 | 0x000A => Some(4),
        0x0005..=0x0007 | 0x0014 | 0x0040 => Some(8),
        0x0048 => Some(16),
        0x001E | 0x001F | 0x0102 | 0x000D => None,
        // A type this can't size: the rest of the list can't be read.
        _ => return None,
    };
    let value = match fixed {
        Some(size) => {
            let values = if multi { r.u32()? as usize } else { 1 };
            let mut first = Value::Other;
            for i in 0..values {
                if r.left() < size {
                    return None;
                }
                let v = match (base, size) {
                    (0x000B, _) => Value::Bool(r.u16()? != 0),
                    (0x0002, _) => Value::Int(i64::from(r.u16()? as i16)),
                    (0x0003, _) => Value::Int(i64::from(r.u32()? as i32)),
                    (0x0040, _) => Value::Time(r.u64()?),
                    (0x0014, _) => Value::Int(r.u64()? as i64),
                    _ => {
                        r.take(size)?;
                        Value::Other
                    }
                };
                r.pad(size)?;
                if i == 0 && !multi {
                    first = v;
                }
            }
            first
        }
        None => {
            let values = r.u32()? as usize;
            if values > r.left() / 4 {
                return None;
            }
            let mut first = Value::Other;
            for i in 0..values {
                let n = r.u32()? as usize;
                let b = r.take(n)?;
                r.pad(n)?;
                if i == 0 && !multi {
                    first = match base {
                        0x001E => Value::Str8(b.to_vec()),
                        0x001F => Value::Unicode(utf16(b)),
                        0x0102 => Value::Binary(b.to_vec()),
                        _ => Value::Object(b.to_vec()),
                    };
                }
            }
            first
        }
    };
    Some((id, value))
}

/// A FILETIME as Unix seconds.
fn unix_time(t: u64) -> i64 {
    (t / 10_000_000) as i64 - 11_644_473_600
}

/// Who sent an attached message, and when, for its `.eml`.
fn sender_and_date(msg: &Props, cp: u32) -> (Option<String>, Option<String>, Option<i64>) {
    let name = text(prop(msg, PR_SENDER_NAME), cp);
    let email = text(prop(msg, PR_SENDER_EMAIL), cp).filter(|e| e.contains('@'));
    let date = match prop(msg, PR_CLIENT_SUBMIT_TIME) {
        Some(Value::Time(t)) if *t > 0 => Some(unix_time(*t)),
        _ => None,
    };
    (name, email, date)
}

#[cfg(test)]
mod tests;
