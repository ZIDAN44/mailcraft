//! A message's text first over IMAP (issue #24): its structure (`BODYSTRUCTURE`, RFC 3501
//! §7.4.2), then only its header and text parts, so a message with a 20 MB attachment reads at
//! once while its whole source comes. The files are listed, named and sized, without their bytes
//! ([`mime::text_first`]), as Exchange's are.

use std::collections::HashMap;

use imap_proto::{BodyStructure, ContentDisposition, MessageSection, SectionPath};
use mailcraft_mime as mime;

use super::*;

/// Messages smaller than this come whole at once: their text first would save little and cost
/// two more round trips.
pub const TEXT_FIRST_OVER: u32 = 256 << 10;

/// The most text fetched first (all text parts together); a message with more comes whole.
const MAX_TEXT: u64 = 8 << 20;

/// Parts looked at, and how deep, in a structure from the server.
const MAX_PARTS: usize = 500;
const MAX_DEPTH: usize = 16;

/// What a message's structure says to fetch first.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outline {
    /// The text parts, HTML or plain, in order: each its section (`1.1`, `2`) and the MIME
    /// header its structure gives (servers differ on `BODY[1.1.MIME]`: GreenMail answers with
    /// the part above).
    pub texts: Vec<(String, Vec<u8>)>,
    pub files: Vec<mime::PendingFile>,
    /// Signed or encrypted: only its whole source shows it (and checks it).
    pub protected: bool,
    text_bytes: u64,
    parts: usize,
}

fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn params(p: &imap_proto::BodyParams<'_>) -> Vec<(String, String)> {
    p.as_ref().map(|l| l.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap_or_default()
}

/// A part's file name: the disposition's `filename`, else the type's `name` (RFC 2183 §2.3), as
/// the server gave them (RFC 2231 or RFC 2047 encoded).
fn file_name(disposition: Option<&ContentDisposition<'_>>, ty: &imap_proto::ContentType<'_>) -> Option<String> {
    mime::part_name(&disposition.map(|d| params(&d.params)).unwrap_or_default(), &params(&ty.params))
}

/// A text part's MIME header, from its structure: its type, character set and transfer encoding.
fn text_header(sub: &str, ty: &imap_proto::ContentType<'_>, encoding: &imap_proto::ContentEncoding<'_>) -> Vec<u8> {
    let token = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')).take(64).collect::<String>();
    let charset = ty.params.as_ref().and_then(|l| l.iter().find(|(k, _)| k.eq_ignore_ascii_case("charset"))).map(|(_, v)| token(v));
    let encoding = match encoding {
        imap_proto::ContentEncoding::SevenBit => "7bit".to_string(),
        imap_proto::ContentEncoding::EightBit => "8bit".into(),
        imap_proto::ContentEncoding::Binary => "binary".into(),
        imap_proto::ContentEncoding::Base64 => "base64".into(),
        imap_proto::ContentEncoding::QuotedPrintable => "quoted-printable".into(),
        imap_proto::ContentEncoding::Other(o) => token(o),
    };
    let charset = charset.filter(|c| !c.is_empty()).map(|c| format!("; charset=\"{c}\"")).unwrap_or_default();
    format!("Content-Type: text/{}{charset}\r\nContent-Transfer-Encoding: {encoding}\r\n", token(sub)).into_bytes()
}

impl Outline {
    /// The outline of a structure from the server: `None` for a message of one part (its text is
    /// all of it).
    pub(crate) fn of(bs: &BodyStructure<'_>) -> Option<Outline> {
        if !matches!(bs, BodyStructure::Multipart { .. }) {
            return None;
        }
        let mut o = Outline::default();
        o.walk(bs, &mut Vec::new());
        Some(o)
    }

    fn walk(&mut self, bs: &BodyStructure<'_>, path: &mut Vec<u32>) {
        self.parts = self.parts.saturating_add(1);
        if self.parts > MAX_PARTS || path.len() > MAX_DEPTH {
            self.protected = true;
            return;
        }
        let section = || path.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        match bs {
            BodyStructure::Multipart { common, bodies, .. } => {
                let sub = lower(&common.ty.subtype);
                if sub == "signed" || sub == "encrypted" {
                    self.protected = true;
                    return;
                }
                for (i, b) in bodies.iter().enumerate() {
                    path.push(u32::try_from(i).unwrap_or(u32::MAX).saturating_add(1));
                    self.walk(b, path);
                    path.pop();
                }
            }
            BodyStructure::Text { common, other, .. } => {
                let sub = lower(&common.ty.subtype);
                let attached = common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
                let named = file_name(common.disposition.as_ref(), &common.ty);
                if !attached && named.is_none() && (sub == "plain" || sub == "html") {
                    self.texts.push((section(), text_header(&sub, &common.ty, &other.transfer_encoding)));
                    self.text_bytes = self.text_bytes.saturating_add(u64::from(other.octets));
                } else {
                    self.file(common, other, named);
                }
            }
            BodyStructure::Basic { common, other, .. } => {
                let kind = format!("{}/{}", lower(&common.ty.ty), lower(&common.ty.subtype));
                if matches!(kind.as_str(), "application/pkcs7-mime" | "application/x-pkcs7-mime" | "application/pgp-encrypted") {
                    self.protected = true;
                    return;
                }
                let named = file_name(common.disposition.as_ref(), &common.ty);
                self.file(common, other, named);
            }
            // An attached message is one file; nothing inside it is this message's text.
            BodyStructure::Message { common, other, .. } => {
                let named = file_name(common.disposition.as_ref(), &common.ty);
                self.file(common, other, named);
            }
        }
    }

    fn file(&mut self, common: &imap_proto::BodyContentCommon<'_>, other: &imap_proto::BodyContentSinglePart<'_>, name: Option<String>) {
        let mime = format!("{}/{}", lower(&common.ty.ty), lower(&common.ty.subtype));
        let content_id = other.id.as_deref().map(|c| c.trim().trim_matches(['<', '>']).to_string()).filter(|c| !c.is_empty());
        let inline = common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("inline")) || (content_id.is_some() && name.is_none());
        // Base64 is a third larger than the file it carries.
        let octets = u64::from(other.octets);
        let size = if matches!(other.transfer_encoding, imap_proto::ContentEncoding::Base64) { octets.saturating_mul(3) / 4 } else { octets };
        let name = name.unwrap_or_else(|| if mime == "message/rfc822" { "message.eml".into() } else { "attachment".into() });
        self.files.push(mime::PendingFile { name, mime, size, content_id, inline });
    }

    /// Whether the text first is worth it: there are files to leave for later, and text to show.
    pub(crate) fn worth_it(&self) -> bool {
        !self.protected && !self.files.is_empty() && !self.texts.is_empty() && self.text_bytes <= MAX_TEXT
    }
}

/// A part's MIME header and content as a message of its own, for the parser to decode.
fn part_message(mime_header: &[u8], content: &[u8]) -> Vec<u8> {
    let mut head = mime_header.to_vec();
    while head.last().is_some_and(|b| *b == b'\r' || *b == b'\n') {
        head.pop();
    }
    let mut out = Vec::with_capacity(head.len().saturating_add(content.len()).saturating_add(4));
    out.extend_from_slice(&head);
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(content);
    out
}

/// The text first, as MIME, from the message's header, its text parts (each its MIME header and
/// content) and its outline's files.
pub(crate) fn assemble(header: &[u8], texts: &[(Vec<u8>, Vec<u8>)], files: &[mime::PendingFile]) -> Option<Vec<u8>> {
    let mut head = header.to_vec();
    head.extend_from_slice(b"\r\n");
    let p = mime::parse(&head)?;
    let mut d = mime::Draft {
        to: p.to,
        cc: p.cc,
        reply_to: p.reply_to,
        subject: p.subject,
        in_reply_to: p.in_reply_to,
        importance: importance(header),
        ..Default::default()
    };
    for (mime_header, content) in texts {
        let part = part_message(mime_header, content);
        match mime::html_body(&part) {
            Some(h) if d.html.is_none() => d.html = Some(h),
            Some(_) => {}
            None if d.body.is_empty() => d.body = mime::parse(&part).map(|p| p.text).unwrap_or_default(),
            None => {}
        }
    }
    mime::text_first(&d, &p.from, p.date, &p.message_id, files).ok()
}

/// `Importance:` (or `X-Priority:`) as the draft writes it.
fn importance(header: &[u8]) -> i8 {
    let text = String::from_utf8_lossy(header).to_ascii_lowercase();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("importance:") {
            return match v.trim() {
                "high" => 1,
                "low" => -1,
                _ => 0,
            };
        }
        if let Some(v) = line.strip_prefix("x-priority:") {
            return match v.trim().chars().next() {
                Some('1' | '2') => 1,
                Some('4' | '5') => -1,
                _ => 0,
            };
        }
    }
    0
}

impl<S: Read + Write> Client<S> {
    /// Message `uid`'s text first (in the selected folder), as MIME: `None` when it has no
    /// files to leave for later, is signed or encrypted, or is gone.
    pub fn text_first(&mut self, uid: u32) -> Result<Option<Vec<u8>>> {
        let rs = self.cmd(&[Part::Text(&format!("UID FETCH {uid} (UID BODYSTRUCTURE)"))])?;
        let outline = rs.iter().find_map(|r| match r {
            Response::Fetch(_, attrs) if attrs.iter().any(|a| matches!(a, AttributeValue::Uid(u) if *u == uid)) => {
                attrs.iter().find_map(|a| match a {
                    AttributeValue::BodyStructure(bs) => Outline::of(bs),
                    _ => None,
                })
            }
            _ => None,
        });
        let Some(outline) = outline.filter(Outline::worth_it) else { return Ok(None) };
        let mut items = vec!["UID".to_string(), "BODY.PEEK[HEADER]".to_string()];
        for (s, _) in &outline.texts {
            items.push(format!("BODY.PEEK[{s}]"));
        }
        let rs = self.cmd(&[Part::Text(&format!("UID FETCH {uid} ({})", items.join(" ")))])?;
        let mut header = None;
        let mut contents: HashMap<String, Vec<u8>> = HashMap::new();
        for r in rs {
            let Response::Fetch(_, attrs) = r else { continue };
            if !attrs.iter().any(|a| matches!(a, AttributeValue::Uid(u) if *u == uid)) {
                continue;
            }
            for a in attrs {
                let AttributeValue::BodySection { section: Some(section), data: Some(data), .. } = a else { continue };
                match section {
                    SectionPath::Full(MessageSection::Header) => header = Some(data.into_owned()),
                    SectionPath::Part(path, part) => {
                        let key = path.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
                        if part.is_none() {
                            contents.insert(key, data.into_owned());
                        }
                    }
                    SectionPath::Full(_) => {}
                }
            }
        }
        let Some(header) = header else { return Ok(None) };
        let texts: Vec<(Vec<u8>, Vec<u8>)> = outline.texts.iter().filter_map(|(s, head)| Some((head.clone(), contents.remove(s)?))).collect();
        if texts.is_empty() {
            return Ok(None);
        }
        Ok(assemble(&header, &texts, &outline.files))
    }
}
