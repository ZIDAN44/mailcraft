//! A message's attachments: the MIME parts that are files, and the files inside an Outlook
//! `winmail.dat` (issue #18), listed and read by one index.

use std::borrow::Cow;

use super::*;
use crate::tnef::{TnefBody, is_tnef, read_tnef};
use mailcraft_model::{file_name, work};

pub(crate) mod fields;
pub(super) mod files;
mod index_text;
pub mod office;
pub(super) mod photos;
mod reactions;
mod text_first;
pub(super) mod tnef;
use crate::parts::sent_bytes;
pub(crate) use crate::parts::{contents, files_of, header_raw, header_top};
pub use index_text::index_doc;
#[doc(hidden)]
pub use index_text::{IndexStats, index_doc_stats};
pub(crate) use reactions::reaction_of;
pub use reactions::{REACTIONS, is_reaction_text, reaction};
pub use text_first::{PendingFile, TEXT_FIRST, is_text_first, part_name, text_first};

/// A part's size: its bytes (`data`), or, with none yet (a message's text that came first, issue
/// #24), the size its `Content-Disposition` gives (RFC 2183's `size`).
fn shown_size(p: mail_parser::MessagePart<'_>, data: &[u8]) -> usize {
    if !data.is_empty() {
        return data.len();
    }
    p.content_disposition().and_then(|d| d.attribute("size")).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0)
}

/// One of a message's attachments, with its bytes.
pub(super) struct Attached<'a> {
    pub(super) info: AttachmentInfo,
    pub(super) data: Cow<'a, [u8]>,
    /// The MIME part it is (none for a file in a `winmail.dat`), for [`sent_bytes`].
    pub(super) part: Option<mail_parser::MessagePart<'a>>,
}

/// A message's attachments in order, a `winmail.dat` given as the files in it (it stays one
/// when it can't be read, or holds nothing), and the body the first such file carries.
pub(super) fn attached<'a>(m: &'a Message<'_>) -> (Vec<Attached<'a>>, TnefBody) {
    let mut out = Vec::new();
    let mut body = TnefBody::None;
    for p in files_of(m.root()) {
        let mime = p
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
            .unwrap_or_else(|| "application/octet-stream".into());
        let data = contents(p);
        if is_tnef(&mime, p.attachment_name())
            && let Some(t) = read_tnef(&data).filter(|t| !t.attachments.is_empty() || !t.body.is_empty())
        {
            let shown = tnef_inline(&t);
            for (f, inline) in t.attachments.into_iter().zip(shown) {
                let index = out.len();
                let name = attachment_name(Some(&f.name), &f.mime, &f.data, index);
                let info = AttachmentInfo { index, name, mime: f.mime, size: f.data.len(), inline, content_id: f.content_id, content_location: None };
                out.push(Attached { info, data: Cow::Owned(f.data), part: None });
            }
            if body.is_empty() {
                body = t.body;
            }
            continue;
        }
        let index = out.len();
        let name = attachment_name(p.attachment_name(), &mime, &data, index);
        // A picture the text may show. A file that merely has a Content-ID and says nothing of
        // how it's to be shown (a PDF from some invoicing programs) is a file: taken for
        // inline, it was counted (the paperclip) and then listed nowhere, nor saved with the rest.
        let inline = p.is_inline_part() && mime.starts_with("image/");
        let info = AttachmentInfo {
            index,
            name,
            mime,
            size: shown_size(p, &data),
            inline,
            content_id: p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string()),
            content_location: p.content_location().map(|c| c.trim().trim_matches(['<', '>', '"']).to_string()).filter(|c| !c.is_empty()),
        };
        out.push(Attached { info, data, part: Some(p) });
    }
    (out, body)
}

/// The body the first `winmail.dat` with one carries, as [`attached`] gives it, without the
/// message's other files read.
pub(super) fn tnef_body(m: &Message<'_>) -> TnefBody {
    let mut body = TnefBody::None;
    for p in files_of(m.root()) {
        if !is_tnef(&mime_of(p).unwrap_or_default(), p.attachment_name()) {
            continue;
        }
        if let Some(t) = read_tnef(&contents(p)).filter(|t| !t.attachments.is_empty() || !t.body.is_empty())
            && body.is_empty()
        {
            body = t.body;
        }
    }
    body
}

/// Whether a message carries a file: the paperclip, "Has files" and `has:attachment` agree
/// with search's count ([`index_doc`]), and with a data file's import ([`draft_carries_files`]),
/// by sharing [`carries_file`].
pub(super) fn has_real_attachments(m: &Message<'_>) -> bool {
    let html = || m.html_body().filter(|p| matches!(p.kind(), PartKind::Html)).filter_map(|p| p.text()).collect::<Vec<_>>().join("\n");
    let refs = HtmlRefs::new(&html);
    files_of(m.root()).any(|p| {
        let shown = || may_be_shown(p).is_some_and(|(cid, location)| refs.shows(cid, location));
        carries_file(&mime_of(p).unwrap_or_default(), p.attachment_name(), &contents(p), &shown)
    })
}

/// For each file in a `winmail.dat`, whether it's a picture in its body: one Outlook hid, or
/// one the body shows by `cid:`.
pub(crate) fn tnef_inline(t: &crate::Tnef) -> Vec<bool> {
    let make = || match &t.body {
        TnefBody::Html(h) => h.clone(),
        _ => String::new(),
    };
    let refs = HtmlRefs::new(&make);
    t.attachments.iter().map(|f| f.content_id.as_deref().is_some_and(|c| f.hidden || refs.shows(Some(c), None))).collect()
}

/// Most references kept from one message's HTML.
const MAX_REFS: usize = 10_000;

/// The pictures a message's HTML shows, by `cid:` or by the address a part's
/// `Content-Location` gives it (RFC 2557): read from the HTML only when a part asks.
pub struct HtmlRefs<'a> {
    refs: std::cell::OnceCell<Refs>,
    make: &'a dyn Fn() -> String,
}

/// What an HTML body refers to, lower case: every `cid:` (up to its closing quote, bracket or
/// space), and every `src`, `href` and `background` address, whole and by its last segment.
#[derive(Default)]
struct Refs {
    cids: std::collections::HashSet<String>,
    paths: std::collections::HashSet<String>,
    files: std::collections::HashSet<String>,
}

impl Refs {
    fn read(html: &str) -> Refs {
        let lower = html.to_ascii_lowercase();
        let mut r = Refs::default();
        let mut from = 0;
        while let Some(at) = lower.get(from..).and_then(|s| s.find("cid:")).map(|i| i + from + 4) {
            from = at;
            let rest = lower.get(at..).unwrap_or_default().trim_start_matches('<');
            let end = rest.find(|c: char| c.is_ascii_whitespace() || "\"'()<>&;,".contains(c)).unwrap_or(rest.len());
            let cid = rest.get(..end).unwrap_or_default();
            if !cid.is_empty() && r.cids.len() < MAX_REFS {
                r.cids.insert(cid.to_string());
                r.cids.insert(percent_decoded(cid));
            }
        }
        // Every `src`, `href` and `background` of a tag, read as every other pass reads a tag's
        // attributes ([`crate::html::attributes`]): not a name in another's quoted value
        // (`alt="… src=plan.pdf"` took a file of that name for shown), nor in the text.
        let mut from = 0;
        while let Some(at) = lower.get(from..).and_then(|s| work::scanned(s, s.find('<'))).map(|i| i + from) {
            let Some(end) = crate::html::tag_end(lower.as_bytes(), at + 1) else { break };
            from = end + 1;
            let tag = lower.get(at..=end).unwrap_or_default();
            for a in crate::html::attributes(tag).1.into_iter().filter(|a| matches!(a.name.as_str(), "src" | "href" | "background")) {
                let value = tag.get(a.value).unwrap_or_default();
                let path = percent_decoded(value.split(['?', '#']).next().unwrap_or(value).trim());
                if !path.is_empty() && r.paths.len() < MAX_REFS {
                    // The same last part as the reading pane's (`picture_attachment`), so that
                    // they find the same picture.
                    if !file_name(&path).is_empty() {
                        r.files.insert(file_name(&path).to_string());
                    }
                    r.paths.insert(path);
                }
            }
        }
        r
    }
}

/// `%xx` escapes decoded (RFC 3986), as the reading pane decodes a picture's address
/// ([`picture_attachment`]).
pub(crate) fn percent_decoded(s: &str) -> String {
    mailcraft_model::mailto::percent_decode(s)
}

impl<'a> HtmlRefs<'a> {
    /// The references of the HTML `make` gives, read the first time a part asks.
    pub fn new(make: &'a dyn Fn() -> String) -> HtmlRefs<'a> {
        HtmlRefs { refs: std::cell::OnceCell::new(), make }
    }

    /// Whether the HTML refers to this `Content-ID` by a `cid:` (whatever its case). A file a
    /// message is built with goes in its body only then: an address whose last segment is the
    /// same (a link to `plan.pdf`, and a file whose Content-ID is `plan.pdf`) doesn't show it.
    pub fn cites(&self, cid: &str) -> bool {
        let r = self.refs.get_or_init(|| Refs::read(&(self.make)()));
        let cid = cid.trim().trim_matches(['<', '>']).to_ascii_lowercase();
        !cid.is_empty() && r.cids.contains(&cid)
    }

    /// Whether the HTML shows the part with this `Content-ID` or `Content-Location`, matched as
    /// the reading pane finds a picture ([`picture_attachment`]): the `cid:` exactly, the address
    /// exactly or by its last segment.
    pub fn shows(&self, cid: Option<&str>, location: Option<&str>) -> bool {
        let r = self.refs.get_or_init(|| Refs::read(&(self.make)()));
        let cid = cid.map(|c| c.trim().trim_matches(['<', '>']).to_ascii_lowercase()).filter(|c| !c.is_empty());
        let location = location.map(|l| l.trim().trim_matches(['<', '>', '"']).to_ascii_lowercase()).filter(|l| !l.is_empty());
        cid.is_some_and(|c| r.cids.contains(&c) || r.files.contains(&c))
            || location.is_some_and(|l| r.paths.contains(&l) || r.files.contains(file_name(&l)) || r.cids.contains(&l))
    }
}

/// A part's type as `type/subtype`, lower case.
pub(crate) fn mime_of(p: mail_parser::MessagePart<'_>) -> Option<String> {
    p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or_default()).to_ascii_lowercase())
}

/// S/MIME's signature and encrypted content are the message itself, not files in it.
pub(crate) fn is_smime(p: mail_parser::MessagePart<'_>) -> bool {
    smime::is_smime_part(&mime_of(p).unwrap_or_default(), p.attachment_name().unwrap_or_default())
}

/// The `Content-ID` and `Content-Location` of a part the HTML may show in its place: one inline
/// or without a disposition, that has either.
pub(crate) fn may_be_shown<'p>(p: mail_parser::MessagePart<'p>) -> Option<(Option<&'p str>, Option<&'p str>)> {
    let inline = p.content_disposition().is_none_or(|d| d.ctype().eq_ignore_ascii_case("inline"));
    let (cid, location) = (p.content_id(), p.content_location());
    (inline && (cid.is_some() || location.is_some())).then_some((cid, location))
}

/// Whether one of a message's non-text parts, of type `mime` and named `name`, is a file it
/// carries: not S/MIME's own part, and not a picture the HTML shows (`shown`: an inline part, or
/// one with no disposition, whose `Content-ID` or `Content-Location` the real `text/html` bodies
/// refer to). Whatever its name says: Outlook names its pictures `image001.png`, and Apple Mail's
/// inline PDF has no `Content-ID` and is a file. A `winmail.dat` is the files in it, as the
/// reading pane and search list them.
fn carries_file(mime: &str, name: Option<&str>, data: &[u8], shown: &dyn Fn() -> bool) -> bool {
    if is_tnef(mime, name)
        && let Some(t) = read_tnef(data).filter(|t| !t.attachments.is_empty() || !t.body.is_empty())
    {
        return tnef_inline(&t).contains(&false);
    }
    !smime::is_smime_part(mime, name.unwrap_or_default()) && !shown()
}

/// Whether the message [`build`](crate::build) makes of `d` carries a file, as the paperclip,
/// "Has files" and `has:attachment` read it back from the source ([`has_real_attachments`], by
/// the same [`carries_file`]): a data file's import knows it before it writes the source, and
/// doesn't parse it again. The files are files unless they are S/MIME's own part or a
/// `winmail.dat` of pictures its body shows; the pictures go in only beside HTML, and are files
/// when it doesn't show them.
pub fn draft_carries_files(d: &Draft) -> bool {
    let html = || d.html.clone().unwrap_or_default();
    let refs = HtmlRefs::new(&html);
    // A type as `build` gives it, the name the part is read back with, the bytes.
    let file = carries_file;
    let typed = |mime: &str, name: &str| if mime.is_empty() { mime_from_name(name).to_string() } else { mime.to_string() };
    d.attachments.iter().any(|a| file(&typed(&a.mime, &a.name), Some(&sanitize_filename(&a.name)), &a.data, &|| false))
        || (d.html.is_some()
            && d.inline.iter().any(|p| {
                let mime = typed(&p.mime, &p.name);
                file(&mime, crate::files::picture_name(&p.name, &mime).as_deref(), &p.data, &|| refs.shows(Some(&p.cid), None))
            }))
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

/// Most pictures a reply or forward takes from the message it quotes, and bytes each.
const QUOTED_PICTURES: usize = 100;
const QUOTED_PICTURE_MAX: usize = 25 << 20;

/// The pictures of a message as a reply or forward quotes it (issue #10: they came out as
/// "[picture]"): each one in the message (by `cid:`, by its `Content-Location` or name, or written
/// into the HTML as `data:`) given by `cid:`, with its bytes for the reply to send; one on the web
/// kept by its address, as Outlook keeps it (never loaded here); tracking pictures left out.
/// Also the indexes of the attachments the quote shows, and each picture's address in the
/// message with the `cid:` it was given (for [`quote_html`]).
pub fn quoted_pictures_given(p: &mut Parsed, raw: &[u8]) -> (Vec<InlinePicture>, Vec<usize>, Vec<(String, String)>) {
    let mut out: Vec<InlinePicture> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    let found: Vec<(String, Option<usize>)> = p
        .rich
        .iter()
        .flatten()
        .filter_map(|s| s.image.clone())
        // And those behind its boxes (a banner behind a title), which a quote sent as it came shows.
        .chain(crate::layout::background_pictures(&p.layout))
        .map(|src| {
            let at = picture_attachment(p, &src);
            (src, at)
        })
        .collect();
    let trackers = p.trackers.pictures.clone();
    // The pictures in the message, read from one parse (each was the whole message parsed and
    // every file in it decoded again: 40 ms for 30 pictures of 200 KB).
    let wanted: Vec<usize> =
        found.iter().filter_map(|(_, at)| p.attachments.get((*at)?)).filter(|a| a.size <= QUOTED_PICTURE_MAX).map(|a| a.index).collect();
    let datas: std::collections::HashMap<usize, Vec<u8>> = attachments_data(raw, &wanted, usize::MAX, usize::MAX).into_iter().collect();
    let mut given: Vec<(String, String)> = Vec::new();
    for (src, at) in found {
        if given.iter().any(|(s, _)| *s == src) || out.len() >= QUOTED_PICTURES {
            continue;
        }
        let picture = match at.and_then(|i| p.attachments.get(i)) {
            Some(a) if a.size <= QUOTED_PICTURE_MAX => datas.get(&a.index).cloned().map(|data| {
                let cid = a
                    .content_id
                    .clone()
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| format!("quoted-{}-{:012x}@mailcraft", a.index, crate::fnv(&src) & 0xFFFF_FFFF_FFFF));
                InlinePicture { cid, name: a.name.clone(), mime: a.mime.clone(), data }
            }),
            Some(_) => None,
            None => data_url(&src, QUOTED_PICTURE_MAX).map(|(mime, data)| {
                let n = out.len() + 1;
                let ext = Some(crate::files::ext_for_mime(&mime)).filter(|e| !e.is_empty()).unwrap_or(".png");
                InlinePicture {
                    cid: format!("quoted{n}-{:012x}@mailcraft", crate::fnv(&src) & 0xFFFF_FFFF_FFFF),
                    name: format!("image{n:03}{ext}"),
                    mime,
                    data,
                }
            }),
        };
        if let Some(pic) = picture {
            used.extend(at);
            given.push((src, pic.cid.clone()));
            if !out.iter().any(|q| q.cid == pic.cid) {
                out.push(pic);
            }
        }
    }
    // In the text and in its layout (which a reply quotes when there is one).
    let mut quote = |span: &mut Span| {
        let Some(src) = span.image.clone() else { return };
        if let Some((_, cid)) = given.iter().find(|(s, _)| *s == src) {
            span.image = Some(format!("cid:{cid}"));
        } else if trackers.contains(&src) {
            span.image = None;
            span.text.clear();
        }
    };
    p.rich.iter_mut().flatten().for_each(&mut quote);
    crate::layout::each_span(&mut p.layout, &mut quote);
    (out, used, given)
}

/// A picture written into HTML as `data:image/…;base64,…` (PNG, JPEG, GIF, WebP or BMP) of at
/// most `max` bytes: its type (`image/jpg` as `image/jpeg`) and bytes. The one reader of them, for
/// a reply's quoted pictures and a signature's.
pub(crate) fn data_url(src: &str, max: usize) -> Option<(String, Vec<u8>)> {
    let src = src.trim();
    let rest = src.get(..11).filter(|p| p.eq_ignore_ascii_case("data:image/")).and_then(|_| src.get(5..))?;
    let (kind, b64) = rest.split_once(',')?;
    let kind = kind.to_ascii_lowercase();
    let mime = kind.strip_suffix(";base64")?;
    if !["image/png", "image/jpeg", "image/jpg", "image/gif", "image/webp", "image/bmp"].contains(&mime) || b64.len() > max / 3 * 4 + 8 {
        return None;
    }
    let clean: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = encodify::base64::LENIENT.decode(&clean).ok().filter(|d| !d.is_empty() && d.len() <= max)?;
    Some((if mime == "image/jpg" { "image/jpeg" } else { mime }.to_string(), data))
}

/// The most a picture written into a message's HTML may hold, decoded.
const DATA_PICTURE_MAX: usize = 16 << 20;

/// A picture written into a message's HTML as `data:image/…;base64,…`, read to show it (issue
/// #225: an Apple Mail rich link card's picture is one): its bytes, when it's one of the kinds
/// [`data_url`] reads and not too big.
pub fn data_picture(src: &str) -> Option<Vec<u8>> {
    data_url(src, DATA_PICTURE_MAX).map(|(_, data)| data)
}

/// The bytes of attachment `index`.
pub fn attachment_data(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    attachments_data(raw, &[index], usize::MAX, usize::MAX).pop().map(|(_, d)| d)
}

/// [`attachment_data`] for big files, from one parse: `each` is handed each of `wanted` the
/// message has (each once, in no set order), one at a time, its bytes decoded once and handed
/// over, not copied again, so a message with 30 big files is parsed once, not 30 times, and
/// holds no file twice.
pub fn attachments_moved(raw: &[u8], wanted: &[usize], each: &mut dyn FnMut(usize, Vec<u8>)) {
    let mut wanted = wanted.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return;
    }
    let Some(m) = MessageParser::default().parse(raw) else { return };
    // mail-parser decodes a part when it's read, into bytes of its own: those are moved out (a
    // part sent as it is, unencoded, is copied out of the source, as `attachment_data` does).
    for a in attached(&m).0 {
        // Each once.
        let Ok(w) = wanted.binary_search(&a.info.index) else { continue };
        wanted.remove(w);
        each(a.info.index, a.part.and_then(sent_bytes).unwrap_or_else(|| a.data.into_owned()));
    }
}

/// Attachment `index` as text to show: a text file read in the charset it names (its bytes,
/// saved as they came, are [`attachment_data`]'s).
pub fn attachment_text(raw: &[u8], index: usize) -> Option<String> {
    let m = MessageParser::default().parse(raw)?;
    let a = attached(&m).0.into_iter().find(|a| a.info.index == index)?;
    Some(String::from_utf8_lossy(&a.data).into_owned())
}

/// Attachment `index` to send again (a forward, an Outbox message taken back to change): its
/// bytes as they came ([`attachment_data`]'s), and a text file's type with the charset it names
/// (`text/plain; charset=windows-1252`), so the copy is read as the original was; without it, a
/// file in Latin-1 went out as bare `text/plain` and was read as UTF-8.
pub fn attachment_out(raw: &[u8], index: usize) -> Option<crate::OutAttachment> {
    attachments_out(raw, &[index]).pop().map(|(_, out)| out)
}

/// [`attachment_out`] for several files from one parse (a draft reopened, a message forwarded:
/// each of its files was the whole message parsed and all its files decoded again). Each of
/// `wanted` the message has, once, in the message's order.
pub fn attachments_out(raw: &[u8], wanted: &[usize]) -> Vec<(usize, crate::OutAttachment)> {
    let Some(m) = MessageParser::default().parse(raw) else { return Vec::new() };
    let mut out: Vec<(usize, crate::OutAttachment)> = Vec::new();
    for a in attached(&m).0.into_iter().filter(|a| wanted.contains(&a.info.index)) {
        if out.iter().any(|(i, _)| *i == a.info.index) {
            continue;
        }
        let sent = a.part.and_then(sent_bytes);
        let charset = a
            .part
            .filter(|_| sent.is_some() && a.info.mime.starts_with("text/"))
            .and_then(|p| charset_token(p.content_type()?.attribute("charset")?));
        let mime = match charset {
            Some(c) => format!("{}; charset={c}", a.info.mime),
            None => a.info.mime.clone(),
        };
        let data = sent.unwrap_or_else(|| a.data.into_owned());
        out.push((a.info.index, crate::OutAttachment { name: a.info.name, mime, data, ..Default::default() }));
    }
    out
}

/// A charset's name as it can go into a `Content-Type`: a token (RFC 2045 §5.1) of at most 64
/// bytes, so it can't add to the header it goes into.
fn charset_token(c: &str) -> Option<&str> {
    let c = c.trim().trim_matches('"');
    let ok = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`{|}~".contains(&b);
    (!c.is_empty() && c.len() <= 64 && c.bytes().all(ok)).then_some(c)
}

/// Most attachments [`attachments_data`] looks for at once.
pub const MAX_WANTED: usize = 256;

/// The bytes of several attachments from one parse (a message with 36 pictures was read and
/// parsed 36 times to show their thumbnails). Indexes are [`attachment_data`]'s; they're taken
/// in the order `wanted` lists them (the first [`MAX_WANTED`], each once), each at most
/// `max_each` bytes and all together at most `max_total`: one past either cap is left out, and
/// so is an index the message doesn't have.
pub fn attachments_data(raw: &[u8], wanted: &[usize], max_each: usize, max_total: usize) -> Vec<(usize, Vec<u8>)> {
    let mut order: Vec<usize> = Vec::new();
    for i in wanted {
        if order.len() >= MAX_WANTED {
            break;
        }
        if !order.contains(i) {
            order.push(*i);
        }
    }
    if order.is_empty() {
        return Vec::new();
    }
    let Some(m) = MessageParser::default().parse(raw) else { return Vec::new() };
    let mut sorted = order.clone();
    sorted.sort_unstable();
    let mut found: Vec<(usize, Cow<'_, [u8]>)> = attached(&m)
        .0
        .into_iter()
        .filter(|a| sorted.binary_search(&a.info.index).is_ok())
        .map(|a| (a.info.index, a.part.and_then(sent_bytes).map_or(a.data, Cow::Owned)))
        .collect();
    let mut out = Vec::new();
    let mut total = 0usize;
    for i in order {
        let Some(at) = found.iter().position(|(j, _)| *j == i) else { continue };
        let (_, data) = found.swap_remove(at);
        if data.len() > max_each {
            continue;
        }
        match total.checked_add(data.len()) {
            Some(t) if t <= max_total => total = t,
            _ => continue,
        }
        out.push((i, data.into_owned()));
    }
    out
}
