//! A message's attachments: the MIME parts that are files, and the files inside an Outlook
//! `winmail.dat` (issue #18), listed and read by one index.

use std::borrow::Cow;

use super::*;
use crate::tnef::{TnefBody, is_tnef, read_tnef};

/// One of a message's attachments, with its bytes.
pub(super) struct Attached<'a> {
    pub(super) info: AttachmentInfo,
    pub(super) data: Cow<'a, [u8]>,
}

/// A message's attachments in order, a `winmail.dat` given as the files in it (it stays one
/// when it can't be read, or holds nothing), and the body the first such file carries.
pub(super) fn attached<'a>(m: &'a Message<'a>) -> (Vec<Attached<'a>>, TnefBody) {
    let mut out = Vec::new();
    let mut body = TnefBody::None;
    for p in m.attachments() {
        let mime = p
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
            .unwrap_or_else(|| "application/octet-stream".into());
        if is_tnef(&mime, p.attachment_name())
            && let Some(t) = read_tnef(p.contents()).filter(|t| !t.attachments.is_empty() || !t.body.is_empty())
        {
            if body.is_empty() {
                body = t.body;
            }
            // A picture the body shows is in it, whether or not Outlook marked it hidden.
            let shown = |cid: &str| matches!(&body, TnefBody::Html(h) if h.contains(&format!("cid:{cid}")));
            for f in t.attachments {
                let index = out.len();
                let inline = f.content_id.as_deref().is_some_and(|c| f.hidden || shown(c));
                let name = attachment_name(Some(&f.name), &f.mime, &f.data, index);
                let info = AttachmentInfo { index, name, mime: f.mime, size: f.data.len(), inline, content_id: f.content_id, content_location: None };
                out.push(Attached { info, data: Cow::Owned(f.data) });
            }
            continue;
        }
        let index = out.len();
        let name = attachment_name(p.attachment_name(), &mime, p.contents(), index);
        let info = AttachmentInfo {
            index,
            name,
            mime,
            size: p.len(),
            inline: p.is_inline_part(),
            content_id: p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string()),
            content_location: p.content_location().map(|c| c.trim().trim_matches(['<', '>', '"']).to_string()).filter(|c| !c.is_empty()),
        };
        out.push(Attached { info, data: Cow::Borrowed(p.contents()) });
    }
    (out, body)
}

pub(super) fn has_real_attachments(m: &Message<'_>) -> bool {
    // S/MIME's signature and encrypted content are the message itself, not files in it.
    let smime = |p: &mail_parser::MessagePart<'_>| {
        let mime = p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or_default())).unwrap_or_default();
        smime::is_smime_part(&mime, p.attachment_name().unwrap_or_default())
    };
    m.attachments().any(|p| (p.attachment_name().is_some() || !p.is_inline_part()) && !smime(p))
}

pub(super) trait InlinePart {
    fn is_inline_part(&self) -> bool;
}

impl InlinePart for mail_parser::MessagePart<'_> {
    /// Inline images referenced from the HTML (`Content-ID`) don't count as attachments.
    fn is_inline_part(&self) -> bool {
        self.content_id().is_some() && self.content_disposition().is_none_or(|d| d.ctype().eq_ignore_ascii_case("inline"))
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
    // Sent as `application/octet-stream`, a picture this can show is still one (by its name).
    if mime.starts_with("image/") || ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&ext.as_str()) {
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

/// The bytes of attachment `index`.
pub fn attachment_data(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    let m = MessageParser::default().parse(raw)?;
    attached(&m).0.into_iter().nth(index).map(|a| a.data.into_owned())
}
