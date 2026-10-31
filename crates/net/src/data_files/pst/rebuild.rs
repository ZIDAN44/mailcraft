//! An item written out again as a standard message (RFC 5322 / MIME), and its summary.

use super::item::{File, Item};
use super::*;

/// The item as a standard message.
pub(super) fn to_raw(it: &Item, fallback_id: &str) -> Result<Vec<u8>, String> {
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
    for f in &it.files {
        match f {
            File::Data { name, mime: m, cid, data } => {
                let cid = cid.trim().trim_matches(|c| c == '<' || c == '>');
                if !cid.is_empty() && html.as_deref().is_some_and(|h| h.contains(&format!("cid:{cid}"))) {
                    d.inline.push(InlinePicture { cid: cid.to_string(), name: name.clone(), mime: m.clone(), data: data.clone() });
                } else {
                    d.attachments.push(OutAttachment { name: name.clone(), mime: m.clone(), data: data.clone() });
                }
            }
            File::Message(inner) => {
                let raw = to_raw(inner, &format!("{fallback_id}.1"))?;
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                d.attachments.push(OutAttachment { name: format!("{subject}.eml"), mime: "message/rfc822".into(), data: raw });
            }
        }
    }
    let id = it.message_id.trim().trim_matches(|c| c == '<' || c == '>');
    let id = if id.is_empty() { fallback_id } else { id };
    let built = mime::build(&d, &it.from, it.date, id)?;
    Ok(with_headers(built, &it.headers, &it.bcc))
}

/// Keep the item's original headers (when it came in by mail) above the rebuilt body; a sent
/// item's Bcc goes back in.
fn with_headers(built: Vec<u8>, original: &str, bcc: &[EmailAddress]) -> Vec<u8> {
    let Some(split) = built.windows(4).position(|w| w == b"\r\n\r\n") else { return built };
    let (head, body) = built.split_at(split);
    let head = String::from_utf8_lossy(head);
    let mime_header = |line: &str| {
        let l = line.to_ascii_lowercase();
        l.starts_with("content-") || l.starts_with("mime-version:")
    };
    let mut out = String::new();
    let original = original.trim();
    if !original.is_empty() && original.len() <= MAX_HEADERS && original.contains(':') {
        // The original's own headers except its body's description, then the rebuilt body's.
        let mut keep = false;
        for line in original.lines() {
            if !line.starts_with([' ', '\t']) {
                keep = !mime_header(line);
            }
            if keep {
                out.push_str(line.trim_end_matches('\r'));
                out.push_str("\r\n");
            }
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
        if !bcc.is_empty() {
            out.push_str(&format!("Bcc: {}\r\n", mailcraft_model::addr::format_list(bcc)));
        }
        out.push_str(&head);
        out.push_str("\r\n");
    }
    let mut raw = out.into_bytes();
    // `body` starts with the blank line ending the header.
    raw.extend_from_slice(body.get(2..).unwrap_or_default());
    raw
}

/// A message's summary from its rebuilt source's headers, with its preview and attachments
/// from the item itself (parsing the whole source again, attachments and all, only to find them
/// cost more than reading it).
pub(super) fn summary_of(raw: &[u8], it: &Item, key: MessageKey, size: u32) -> mailcraft_model::Summary {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |i| i + 4);
    let mut s = mime::summary_from_header(raw.get(..end).unwrap_or(raw), key, it.flags.clone(), size, it.date);
    s.has_attachments = !it.files.is_empty();
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
