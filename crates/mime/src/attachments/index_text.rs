//! A message's text for search, read without decoding its files: a walk over the MIME structure
//! that finds boundaries with `memchr` and hands mail-parser only header blocks and the text parts
//! themselves. A 20 MB attachment costs one pass over its bytes, not a base64 decode.

use std::borrow::Cow;

use mail_parser::{MessageParser, MessagePart, MimeHeaders};
use mailcraft_model::search::{IndexDoc, MAX_INDEX_CHARS};
use memchr::memmem::Finder;

use super::{HtmlRefs, is_smime, may_be_shown, mime_of};
use crate::html_to_text;
use crate::tnef::{TnefBody, is_tnef, read_tnef};

/// Deepest nesting of multiparts and attached messages read.
const MAX_DEPTH: usize = 8;
/// Most parts read in one message, attached messages' included.
const MAX_PARTS: usize = 1000;
/// Most bytes of one text part decoded (or of an attached message sent encoded).
const MAX_LEAF: usize = 16 << 20;
/// Most bytes of a `winmail.dat` read for its body and its files' names.
const MAX_TNEF: usize = 64 << 20;
/// Most bytes of a part's header block.
const MAX_HEADER: usize = 1 << 20;
/// Longest name kept, in characters.
const MAX_NAME: usize = 1024;

/// What a walk cost, for tests: how many bytes went to a decoder.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub decoded_bytes: usize,
    pub parts: usize,
}

/// A message's text and files' names for search ([`IndexDoc`]). Never panics; its time is one
/// pass over `raw` plus decoding the text parts, its memory bounded by the caps above.
pub fn index_doc(raw: &[u8]) -> IndexDoc {
    index_doc_stats(raw).0
}

/// [`index_doc`], and what it cost.
#[doc(hidden)]
pub fn index_doc_stats(raw: &[u8]) -> (IndexDoc, IndexStats) {
    let mut w = Walk::default();
    w.message(raw, 0, true, false);
    if w.doc.text.trim().is_empty() {
        // What Outlook wrapped in a `winmail.dat`, when the message has no text of its own.
        match std::mem::replace(&mut w.tnef_body, TnefBody::None) {
            TnefBody::Html(h) => w.push_text(&html_to_text(&h)),
            TnefBody::Text(t) => w.push_text(&t),
            TnefBody::None => {}
        }
    }
    w.doc.has_text = !w.doc.text.trim().is_empty();
    (w.doc, w.stats)
}

/// One message's parts that wait on its HTML: pictures it may show.
#[derive(Default)]
struct Scope<'b> {
    /// HTML parts not decoded yet.
    html_raw: Vec<&'b [u8]>,
    /// HTML parts already decoded (for the text).
    html: String,
    /// Inline parts with a `Content-ID` or `Content-Location`, and their names.
    maybe_shown: Vec<(Option<String>, Option<String>, Option<String>)>,
}

#[derive(Default)]
struct Walk {
    doc: IndexDoc,
    stats: IndexStats,
    chars: usize,
    tnef_body: TnefBody,
}

impl Walk {
    /// A message (the one indexed, or one attached to it): its subject when attached, its parts,
    /// then the pictures its own HTML shows told from its files.
    fn message(&mut self, raw: &[u8], depth: usize, text_on: bool, attached: bool) {
        let (head, body) = split(raw);
        let parsed = parse_head(head);
        let part = parsed.as_ref().and_then(|m| m.parts.first());
        if attached
            && text_on
            && let Some(s) = parsed.as_ref().and_then(|m| m.subject())
        {
            self.push_text(s);
        }
        let mut scope = Scope::default();
        self.entity(part, raw, body, depth, text_on, "text/plain", &mut scope);
        if scope.maybe_shown.is_empty() {
            return;
        }
        let mut html = std::mem::take(&mut scope.html);
        for leaf in &scope.html_raw {
            if let Some(h) = self.decode(leaf, MAX_LEAF) {
                html.push('\n');
                html.push_str(&h);
            }
        }
        let make = move || html.clone();
        let refs = HtmlRefs::new(&make);
        for (cid, location, name) in scope.maybe_shown {
            if !refs.shows(cid.as_deref(), location.as_deref()) {
                self.file(name);
            }
        }
    }

    /// One MIME entity: `whole` is its header and body, `body` the body alone.
    #[allow(clippy::too_many_arguments)]
    fn entity<'b>(
        &mut self,
        part: Option<&MessagePart<'_>>,
        whole: &'b [u8],
        body: &'b [u8],
        depth: usize,
        text_on: bool,
        default: &str,
        scope: &mut Scope<'b>,
    ) {
        self.stats.parts += 1;
        let mime = part.and_then(mime_of).unwrap_or_else(|| default.to_string());
        let attachment = part.and_then(|p| p.content_disposition()).is_some_and(|d| d.ctype().eq_ignore_ascii_case("attachment"));
        let name = part.and_then(|p| p.attachment_name()).map(|n| n.chars().take(MAX_NAME).collect::<String>()).filter(|n| !n.trim().is_empty());
        if mime.starts_with("multipart/") {
            let boundary = part.and_then(|p| p.content_type()).and_then(|c| c.attribute("boundary")).unwrap_or_default();
            self.multipart(&mime, boundary.as_bytes(), body, depth, text_on, scope);
        } else if (mime == "text/plain" || mime == "text/html") && !attachment {
            let html = mime == "text/html";
            if text_on {
                let Some(text) = self.text_leaf(part.is_some(), whole, body) else { return };
                if html {
                    self.push_text(&html_to_text(&text));
                    scope.html.push('\n');
                    scope.html.push_str(&text);
                } else {
                    self.push_text(&text);
                }
            } else if html && part.is_some() {
                scope.html_raw.push(whole);
            }
        } else if mime == "message/rfc822" || mime == "message/global" {
            self.attached_message(part, body, name, depth, text_on);
        } else if part.is_some_and(is_smime) {
            // S/MIME's signature or encrypted content: the message itself, not a file in it.
        } else if is_tnef(&mime, name.as_deref()) {
            self.tnef(whole, name);
        } else if let Some((cid, location)) = part.and_then(may_be_shown) {
            scope.maybe_shown.push((cid.map(str::to_string), location.map(str::to_string), name));
        } else {
            self.file(name);
        }
    }

    /// A multipart's parts, in order; of an alternative's, only the one read as text gives text
    /// (its plain text, as the reading pane takes it), the others their files and pictures.
    fn multipart<'b>(&mut self, mime: &str, boundary: &[u8], body: &'b [u8], depth: usize, text_on: bool, scope: &mut Scope<'b>) {
        if depth >= MAX_DEPTH {
            self.doc.truncated = true;
            return;
        }
        let (parts, cut) = split_parts(body, boundary, MAX_PARTS.saturating_sub(self.stats.parts));
        self.doc.truncated |= cut;
        let default = if mime == "multipart/digest" { "message/rfc822" } else { "text/plain" };
        let heads: Vec<(&[u8], &[u8], Option<mail_parser::Message<'_>>)> = parts
            .iter()
            .map(|&p| {
                let (head, body) = split(p);
                (p, body, parse_head(head))
            })
            .collect();
        let chosen = (mime == "multipart/alternative").then(|| {
            let plain =
                heads.iter().position(|(_, _, h)| h.as_ref().and_then(|m| m.parts.first()).and_then(mime_of).is_none_or(|m| m == "text/plain"));
            plain.unwrap_or(heads.len().saturating_sub(1))
        });
        for (i, (whole, body, head)) in heads.iter().enumerate() {
            if self.stats.parts >= MAX_PARTS {
                self.doc.truncated = true;
                return;
            }
            let on = text_on && chosen.is_none_or(|c| c == i);
            self.entity(head.as_ref().and_then(|m| m.parts.first()), whole, body, depth + 1, on, default, scope);
        }
    }

    /// An attached message: a file named by its subject, whose text joins the message's.
    fn attached_message(&mut self, part: Option<&MessagePart<'_>>, body: &[u8], name: Option<String>, depth: usize, text_on: bool) {
        let encoding = part.and_then(|p| p.content_transfer_encoding()).map(str::to_ascii_lowercase).unwrap_or_default();
        let inner: Option<Cow<'_, [u8]>> = match encoding.trim() {
            "base64" | "quoted-printable" if body.len() > MAX_LEAF => None,
            "base64" => {
                self.stats.decoded_bytes = self.stats.decoded_bytes.saturating_add(body.len());
                mail_parser::decoders::base64::base64_decode(body).map(Cow::Owned)
            }
            "quoted-printable" => {
                self.stats.decoded_bytes = self.stats.decoded_bytes.saturating_add(body.len());
                mail_parser::decoders::quoted_printable::quoted_printable_decode(body).map(Cow::Owned)
            }
            _ => Some(Cow::Borrowed(body)),
        };
        let subject = inner.as_deref().and_then(|m| parse_head(split(m).0)).and_then(|m| m.subject().map(str::to_string));
        let name = name.or_else(|| subject.filter(|s| !s.trim().is_empty()).map(|s| format!("{}.eml", s.chars().take(MAX_NAME).collect::<String>())));
        self.file(name);
        match inner {
            Some(m) if depth < MAX_DEPTH => self.message(&m, depth + 1, text_on, true),
            _ => self.doc.truncated = true,
        }
    }

    /// A `winmail.dat`: the files in it (pictures its body shows left out), and its body for
    /// a message with no text of its own. One that can't be read is a file.
    fn tnef(&mut self, whole: &[u8], name: Option<String>) {
        let Some(data) = self.decode_bytes(whole, MAX_TNEF) else { return self.file(name) };
        let Some(t) = read_tnef(&data).filter(|t| !t.attachments.is_empty() || !t.body.is_empty()) else { return self.file(name) };
        let shown = |cid: &str| matches!(&t.body, TnefBody::Html(h) if h.contains(&format!("cid:{cid}")));
        for f in &t.attachments {
            if !f.content_id.as_deref().is_some_and(|c| f.hidden || shown(c)) {
                self.file(Some(f.name.chars().take(MAX_NAME).collect()));
            }
        }
        if self.tnef_body.is_empty() {
            self.tnef_body = t.body;
        }
    }

    /// One more file, named when it has a name.
    fn file(&mut self, name: Option<String>) {
        self.doc.files = self.doc.files.saturating_add(1);
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            self.doc.names.push(n);
        }
    }

    /// A text part's text: decoded (transfer encoding and character set) by mail-parser when it
    /// has headers, read as it is when it has none.
    fn text_leaf(&mut self, has_head: bool, whole: &[u8], body: &[u8]) -> Option<String> {
        if !has_head {
            let body = self.cut(body, MAX_LEAF);
            return Some(String::from_utf8_lossy(body).into_owned());
        }
        self.decode(whole, MAX_LEAF)
    }

    /// A single-part entity's text, decoded, its first `cap` bytes only.
    fn decode(&mut self, whole: &[u8], cap: usize) -> Option<String> {
        let whole = self.cut(whole, cap);
        self.stats.decoded_bytes = self.stats.decoded_bytes.saturating_add(whole.len());
        let m = MessageParser::default().parse(whole)?;
        m.parts.first().and_then(|p| p.text_contents()).map(str::to_string)
    }

    /// A single-part entity's bytes, decoded, its first `cap` bytes only.
    fn decode_bytes(&mut self, whole: &[u8], cap: usize) -> Option<Vec<u8>> {
        let whole = self.cut(whole, cap);
        self.stats.decoded_bytes = self.stats.decoded_bytes.saturating_add(whole.len());
        let m = MessageParser::default().parse(whole)?;
        m.parts.first().map(|p| p.contents().to_vec())
    }

    fn cut<'b>(&mut self, bytes: &'b [u8], cap: usize) -> &'b [u8] {
        if bytes.len() > cap {
            self.doc.truncated = true;
        }
        bytes.get(..cap).unwrap_or(bytes)
    }

    /// Text joins the message's, a line each, up to [`MAX_INDEX_CHARS`] in all.
    fn push_text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let room = MAX_INDEX_CHARS.saturating_sub(self.chars);
        if room == 0 {
            self.doc.truncated = true;
            return;
        }
        if !self.doc.text.is_empty() {
            self.doc.text.push('\n');
        }
        let kept = match text.char_indices().nth(room) {
            Some((at, _)) => {
                self.doc.truncated = true;
                text.get(..at).unwrap_or_default()
            }
            None => text,
        };
        self.chars = self.chars.saturating_add(kept.chars().count());
        self.doc.text.push_str(kept);
    }
}

/// A header block parsed (`None`: no headers).
fn parse_head(head: &[u8]) -> Option<mail_parser::Message<'_>> {
    let head = head.get(..MAX_HEADER).unwrap_or(head);
    if head.is_empty() {
        return None;
    }
    MessageParser::default().parse_headers(head).filter(|m| !m.parts.is_empty())
}

/// An entity's header block (with its last line end) and body, split at the first empty line;
/// with none it's all header. One that starts with an empty line has no headers.
pub(super) fn split(raw: &[u8]) -> (&[u8], &[u8]) {
    if let [b'\n', rest @ ..] | [b'\r', b'\n', rest @ ..] = raw {
        return (&[], rest);
    }
    let mut from = 0;
    while let Some(at) = memchr::memchr(b'\n', raw.get(from..).unwrap_or_default()).map(|i| i + from) {
        let next = at + 1;
        let blank = match raw.get(next..) {
            Some([b'\n', ..]) => Some(1),
            Some([b'\r', b'\n', ..]) => Some(2),
            _ => None,
        };
        if let Some(len) = blank {
            return (raw.get(..next).unwrap_or_default(), raw.get(next + len..).unwrap_or_default());
        }
        from = next;
    }
    (raw, &[])
}

/// A multipart body's parts, by its boundary (RFC 2046 §5.1.1): a delimiter is `--boundary` at
/// the start of a line (the body's first byte included), followed by the line end, spaces or
/// tabs (transport padding), or `--` that closes it; `--boundary-2` isn't one. The line end
/// before a delimiter belongs to it. A body never closed runs to its end. At most `max` parts;
/// `true` when some were left out.
pub(super) fn split_parts<'b>(body: &'b [u8], boundary: &[u8], max: usize) -> (Vec<&'b [u8]>, bool) {
    let mut parts = Vec::new();
    if boundary.is_empty() || boundary.len() > 200 {
        return (parts, false);
    }
    let mut needle = b"--".to_vec();
    needle.extend_from_slice(boundary);
    let finder = Finder::new(&needle);
    let mut start: Option<usize> = None;
    for at in finder.find_iter(body) {
        if at > 0 && body.get(at - 1) != Some(&b'\n') {
            continue;
        }
        let Some((close, next)) = delimiter_end(body, at + needle.len()) else { continue };
        if let Some(s) = start {
            if parts.len() >= max {
                return (parts, true);
            }
            parts.push(body.get(s..line_start(body, at)).unwrap_or_default());
        }
        if close {
            return (parts, false);
        }
        start = Some(next);
    }
    if let Some(s) = start {
        if parts.len() >= max {
            return (parts, true);
        }
        parts.push(body.get(s..).unwrap_or_default());
    }
    (parts, false)
}

/// Where a delimiter's line ends (after `--boundary` at `at`): whether it closes, and where the
/// next line starts; `None` when this isn't a delimiter.
fn delimiter_end(body: &[u8], at: usize) -> Option<(bool, usize)> {
    let mut i = at;
    let close = body.get(i..i + 2) == Some(b"--");
    if close {
        i += 2;
    }
    while matches!(body.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    match body.get(i..) {
        None | Some([]) => Some((close, body.len())),
        Some([b'\n', ..]) => Some((close, i + 1)),
        Some([b'\r', b'\n', ..]) => Some((close, i + 2)),
        // A close delimiter is followed by an epilogue, whatever is on its line.
        Some(_) if close => Some((true, i)),
        Some(_) => None,
    }
}

/// `at` moved back over the line end before it.
fn line_start(body: &[u8], at: usize) -> usize {
    let mut end = at;
    if end > 0 && body.get(end - 1) == Some(&b'\n') {
        end -= 1;
        if end > 0 && body.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
    }
    end
}
