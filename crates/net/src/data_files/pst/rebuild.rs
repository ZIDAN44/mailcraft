//! An item written out again as a standard message (RFC 5322 / MIME), and its summary.

use super::item::{File, Item};
use super::*;

/// What a message's summary takes from its item, read before the item goes into its rebuilt
/// source (which takes the item apart).
pub(super) struct Glance {
    flags: Flags,
    date: i64,
    received: i64,
    has_files: bool,
    preview: String,
}

/// The item as a standard message.
pub(super) fn to_raw(mut it: Item, fallback_id: &str) -> Result<Vec<u8>, String> {
    let text = take_text(&mut it);
    Ok(build(it, text, fallback_id)?.0)
}

/// The item as a standard message, and what its summary takes from the item (whether the message
/// carries a file is read from what it is built of: a picture the HTML shows isn't one). Its
/// text is made once, for the message's text part and the preview both. The preview reads all of
/// it, as it read a plain body before: a reply's quoted lines are skipped, so the first words of
/// its own can come well after the start, and it stops once it has its words.
pub(super) fn message(mut it: Item, fallback_id: &str) -> Result<(Vec<u8>, Glance), String> {
    let text = take_text(&mut it);
    let mut glance =
        Glance { flags: std::mem::take(&mut it.flags), date: it.date, received: it.received, has_files: false, preview: mime::preview(&text) };
    let (raw, files) = build(it, text, fallback_id)?;
    glance.has_files = files;
    Ok((raw, glance))
}

/// The item's text, taken out of it: its own, or its HTML's made text when it has none.
fn take_text(it: &mut Item) -> String {
    if it.body.trim().is_empty() {
        it.html.as_deref().filter(|h| !h.trim().is_empty()).map(mime::html_to_text).unwrap_or_default()
    } else {
        std::mem::take(&mut it.body)
    }
}

/// The message, from an item whose text is `text`. The item's parts move into the message: its
/// files are not copied, and are let go of once the message is written. With it, whether the
/// message carries a file.
fn build(mut it: Item, text: String, fallback_id: &str) -> Result<(Vec<u8>, bool), String> {
    // An S/MIME item's content, kept aside to go back in once its header fields are made.
    let protected = super::protected::kind(&it.class).and_then(|kind| Some((kind, super::protected::content(&it.files)?.to_vec())));
    let html = it.html.take().filter(|h| !h.trim().is_empty());
    let mut d = Draft {
        to: std::mem::take(&mut it.to),
        cc: std::mem::take(&mut it.cc),
        subject: std::mem::take(&mut it.subject),
        body: text,
        in_reply_to: std::mem::take(&mut it.in_reply_to),
        references: std::mem::take(&mut it.references),
        importance: it.importance,
        ..Default::default()
    };
    // A file goes in the body when the HTML has its whole `cid:`, whatever its case, as the
    // reading pane matches a picture; a link whose file name is its Content-ID doesn't show it,
    // and it stays a named file.
    let make = || html.clone().unwrap_or_default();
    let refs = mime::HtmlRefs::new(&make);
    let report = super::report::report_type(&it.headers, &mut it.files);
    for f in std::mem::take(&mut it.files) {
        match f {
            File::Data { name, mime, cid, data } => {
                let cid = cid.trim().trim_matches(|c| c == '<' || c == '>');
                if !cid.is_empty() && html.is_some() && refs.cites(cid) {
                    d.inline.push(InlinePicture { cid: cid.to_string(), name, mime, data });
                } else {
                    // One the HTML doesn't show stays a file, with its Content-ID.
                    d.attachments.push(OutAttachment { name, mime, data, content_id: cid.to_string() });
                }
            }
            File::Message(inner) => {
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                let raw = to_raw(*inner, &format!("{fallback_id}.1"))?;
                d.attachments.push(OutAttachment { name: format!("{subject}.eml"), mime: "message/rfc822".into(), data: raw, ..Default::default() });
            }
        }
    }
    d.html = html;
    // An id from the file goes into a header line: no spaces or line breaks.
    let id = mailcraft_model::thread::norm_id(&it.message_id);
    let id = if id.is_empty() { fallback_id } else { &id };
    let mut built = mime::build(&d, &it.from, it.date, id)?;
    if let Some(kind) = report {
        built = super::report::as_report(built, &kind);
    }
    let files = mime::draft_carries_files(&d);
    drop(d);
    let raw = with_headers(built, &it.headers, &it.bcc, &extra_headers(&it));
    match protected.and_then(|(kind, content)| Some((kind, super::protected::message(&raw, kind, &content)?))) {
        // What a signed message carries is inside it; an encrypted one's isn't known.
        Some((kind, raw)) => {
            let files = kind == super::protected::Kind::Signed
                && mime::summary_from_full(&raw, MessageKey::default(), Flags::default(), 0, 0).has_attachments;
            Ok((raw, files))
        }
        None => Ok((raw, files)),
    }
}

/// What the item's properties say that a header carries: Outlook's conversation index and
/// topic, and its sensitivity (RFC 2156), each as one line of safe text.
fn extra_headers(it: &Item) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if !it.thread_index.is_empty() {
        out.push(("Thread-Index", crate::imap::base64(&it.thread_index)));
    }
    let topic = header_text(&it.thread_topic, MAX_TOPIC);
    if !topic.is_empty() {
        out.push(("Thread-Topic", topic));
    }
    let sensitivity = match it.sensitivity {
        1 => "Personal",
        2 => "Private",
        3 => "Company-Confidential",
        _ => "",
    };
    if !sensitivity.is_empty() {
        out.push(("Sensitivity", sensitivity.to_string()));
    }
    out
}

/// Text from the file as a header's value: control characters and line breaks become spaces,
/// runs of spaces one, at most `cap` characters, and RFC 2047 words (UTF-8, base64, folded)
/// when it isn't plain ASCII or could be read as such words.
pub(super) fn header_text(s: &str, cap: usize) -> String {
    let mut text = String::new();
    for c in s.chars().map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c }).take(cap) {
        if c != ' ' || !(text.is_empty() || text.ends_with(' ')) {
            text.push(c);
        }
    }
    let text = text.trim_end();
    if text.is_ascii() && !text.contains("=?") {
        return text.to_string();
    }
    // Words of at most 45 bytes: 60 in base64, 72 with `=?utf-8?B?…?=`.
    let word = |w: &str| format!("=?utf-8?B?{}?=", crate::imap::base64(w.as_bytes()));
    let mut words = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if i + c.len_utf8() - start > 45 {
            words.push(word(text.get(start..i).unwrap_or_default()));
            start = i;
        }
    }
    words.push(word(text.get(start..).unwrap_or_default()));
    words.join("\r\n ")
}

/// Whether a header block has a line named `name` (a folded line's continuation names nothing).
fn has_header(head: &str, name: &str) -> bool {
    head.lines().filter(|l| !l.starts_with([' ', '\t'])).any(|l| l.split_once(':').is_some_and(|(n, _)| n.trim_end().eq_ignore_ascii_case(name)))
}

/// Whether an item's own header (`0x007D`) is one the rebuilt message keeps.
pub(super) fn usable_headers(original: &str) -> bool {
    let original = original.trim();
    !original.is_empty() && original.len() <= MAX_HEADERS && original.contains(':')
}

/// Keep the item's original headers (when it came in by mail) above the rebuilt body; a sent
/// item's Bcc goes back in, and the `extra` lines the header lacks are added. The new header
/// takes the built one's place in `built` itself: the body (attachments and all) is moved along
/// in place, not copied into a second message.
fn with_headers(mut built: Vec<u8>, original: &str, bcc: &[EmailAddress], extra: &[(&str, String)]) -> Vec<u8> {
    let Some(split) = built.windows(4).position(|w| w == b"\r\n\r\n") else { return built };
    let head = String::from_utf8_lossy(built.get(..split).unwrap_or_default()).into_owned();
    let mime_header = |line: &str| {
        let l = line.to_ascii_lowercase();
        l.starts_with("content-") || l.starts_with("mime-version:")
    };
    let mut out = String::new();
    let original = original.trim();
    if usable_headers(original) {
        // The original's own headers except its body's description, then the rebuilt body's.
        let mut keep = false;
        // A blank line would end the header there.
        for line in original.lines().filter(|l| !l.trim_end_matches('\r').is_empty()) {
            if !line.starts_with([' ', '\t']) {
                keep = !mime_header(line);
            }
            if keep {
                out.push_str(line.trim_end_matches('\r'));
                out.push_str("\r\n");
            }
        }
        for (name, value) in extra.iter().filter(|(name, _)| !has_header(original, name)) {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        let mut keep = false;
        for line in head.split("\r\n") {
            if !line.starts_with([' ', '\t']) {
                keep = mime_header(line);
            }
            if keep {
                out.push_str(line);
                out.push_str("\r\n");
            }
        }
    } else {
        let bcc = bcc_line(bcc);
        if !bcc.is_empty() {
            out.push_str(&format!("Bcc: {bcc}\r\n"));
        }
        for (name, value) in extra.iter().filter(|(name, _)| !has_header(&head, name)) {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str(&head);
        out.push_str("\r\n");
    }
    // The built header and the line end after it go; the blank line ending the header stays.
    let end = split.saturating_add(2).min(built.len());
    // Room made exactly first: splice would grow the buffer to twice its size for a header longer
    // than the built one (most received mail's), and the message waits in its batch in it.
    built.reserve_exact(out.len().saturating_sub(end));
    built.splice(..end, out.into_bytes());
    built
}

/// A Bcc header's value from the item's: the addresses that are one, each name cleaned and
/// encoded as other header text (quoted when it holds a comma or such), one address per line.
fn bcc_line(bcc: &[EmailAddress]) -> String {
    let one = |a: &EmailAddress| {
        let name = header_text(&a.name, MAX_NAME);
        if name.is_empty() {
            a.email.clone()
        } else if name.contains(['"', '\\', ',', ';', ':', '<', '>', '@', '(', ')', '[', ']', '.']) {
            format!("\"{}\" <{}>", name.replace(['"', '\\'], "'"), a.email)
        } else {
            format!("{name} <{}>", a.email)
        }
    };
    let list: Vec<String> = bcc.iter().filter(|a| super::props::is_address(&a.email)).map(one).collect();
    list.join(",\r\n ")
}

/// A message's summary from its rebuilt source's headers (its conversation index among them),
/// with its preview, attachments and the time it was delivered from the item it was built of
/// (parsing the whole source again, attachments and all, only to find them cost more than
/// reading it). The list places it by that time, as Outlook does; the reading pane shows its
/// `Date:`.
pub(super) fn summary_of(raw: &[u8], glance: Glance, key: MessageKey, size: u32) -> mailcraft_model::Summary {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let mut s = mime::summary_from_header(raw.get(..end).unwrap_or(raw), key, glance.flags, size, glance.date);
    s.has_attachments = glance.has_files;
    s.preview = glance.preview;
    s.received = if glance.received == s.date { 0 } else { glance.received };
    s
}
