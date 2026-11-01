//! An item written out again as a standard message (RFC 5322 / MIME), and its summary.

use super::item::{File, Item};
use super::*;

/// The item as a standard message, and whether that message carries a file (for its summary:
/// see [`summary_of`]).
pub(super) fn to_raw(it: &Item, fallback_id: &str) -> Result<(Vec<u8>, bool), String> {
    let html = it.html.clone().filter(|h| !h.trim().is_empty());
    let body = if it.body.trim().is_empty() { html.as_deref().map(mime::html_to_text).unwrap_or_default() } else { it.body.clone() };
    let mut d = Draft {
        to: it.to.clone(),
        cc: it.cc.clone(),
        subject: it.subject.clone(),
        body,
        in_reply_to: it.in_reply_to.clone(),
        references: it.references.clone(),
        html: html.clone(),
        importance: it.importance,
        ..Default::default()
    };
    // A file goes in the body when the HTML has its whole `cid:`, whatever its case, as the
    // reading pane matches a picture; a link whose file name is its Content-ID doesn't show it,
    // and it stays a named file.
    let make = || html.clone().unwrap_or_default();
    let refs = mime::HtmlRefs::new(&make);
    for f in &it.files {
        match f {
            File::Data { name, mime: m, cid, data } => {
                let cid = cid.trim().trim_matches(|c| c == '<' || c == '>');
                if !cid.is_empty() && html.is_some() && refs.cites(cid) {
                    d.inline.push(InlinePicture { cid: cid.to_string(), name: name.clone(), mime: m.clone(), data: data.clone() });
                } else {
                    d.attachments.push(OutAttachment { name: name.clone(), mime: m.clone(), data: data.clone() });
                }
            }
            File::Message(inner) => {
                let (raw, _) = to_raw(inner, &format!("{fallback_id}.1"))?;
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                d.attachments.push(OutAttachment { name: format!("{subject}.eml"), mime: "message/rfc822".into(), data: raw });
            }
        }
    }
    // An id from the file goes into a header line: no spaces or line breaks.
    let id = mailcraft_model::thread::norm_id(&it.message_id);
    let id = if id.is_empty() { fallback_id } else { &id };
    let built = mime::build(&d, &it.from, it.date, id)?;
    Ok((with_headers(built, &it.headers, &it.bcc, &extra_headers(it)), mime::draft_carries_files(&d)))
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
/// item's Bcc goes back in, and the `extra` lines the header lacks are added.
fn with_headers(built: Vec<u8>, original: &str, bcc: &[EmailAddress], extra: &[(&str, String)]) -> Vec<u8> {
    let Some(split) = built.windows(4).position(|w| w == b"\r\n\r\n") else { return built };
    let (head, body) = built.split_at(split);
    let head = String::from_utf8_lossy(head);
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
    let mut raw = out.into_bytes();
    // `body` starts with the blank line ending the header.
    raw.extend_from_slice(body.get(2..).unwrap_or_default());
    raw
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

/// A message's summary from its rebuilt source's headers, with its preview from the item itself
/// and whether it carries a file (`files`) from what it was built of (parsing the whole source
/// again, attachments and all, only to find them cost more than reading it). A picture the HTML
/// shows isn't a file, as read from the source: counting the item's files counted those too.
pub(super) fn summary_of(raw: &[u8], it: &Item, key: MessageKey, size: u32, files: bool) -> mailcraft_model::Summary {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let mut s = mime::summary_from_header(raw.get(..end).unwrap_or(raw), key, it.flags.clone(), size, it.date);
    s.has_attachments = files;
    let text = if it.body.trim().is_empty() {
        it.html
            .as_deref()
            .map(|h| mime::html_to_text(h.get(..h.char_indices().nth(64 * 1024).map_or(h.len(), |(i, _)| i)).unwrap_or(h)))
            .unwrap_or_default()
    } else {
        it.body.replace("\r\n", "\n")
    };
    s.preview = mime::preview(&text);
    s
}
