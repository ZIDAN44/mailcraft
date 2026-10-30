//! A message's text first (issue #24): from a server that gives its body and the list of its
//! files apart from its source (Exchange), the message as MIME with its files named and sized
//! but empty, to read while the whole source (a 20 MB attachment) comes.

use super::*;

/// The header that marks a message as its text first, its files to come.
pub const TEXT_FIRST: &str = "X-MailCraft-Text-First";

/// A file of a message whose text came first: what it is, not its bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingFile {
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// `Content-ID`, for a picture the HTML shows.
    pub content_id: Option<String>,
    pub inline: bool,
}

/// The message as MIME, its text and `files` (named and sized, no bytes): `d`'s headers and
/// body (HTML in `d.html` when there is some), from `from` at `date`, under `message_id`
/// (empty: none). Marked with [`TEXT_FIRST`].
pub fn text_first(d: &Draft, from: &EmailAddress, date: i64, message_id: &str, files: &[PendingFile]) -> Result<Vec<u8>, String> {
    let mut b = MessageBuilder::new().from(b_addr(from)).subject(d.subject.clone()).date(date.max(0));
    if !message_id.trim().is_empty() {
        b = b.message_id(mailcraft_model::thread::norm_id(message_id));
    }
    if !d.to.is_empty() {
        b = b.to(b_list(&d.to));
    }
    if !d.cc.is_empty() {
        b = b.cc(b_list(&d.cc));
    }
    if !d.reply_to.is_empty() {
        b = b.reply_to(b_list(&d.reply_to));
    }
    let in_reply_to = mailcraft_model::thread::norm_id(&d.in_reply_to);
    if !in_reply_to.is_empty() {
        b = b.in_reply_to(in_reply_to);
    }
    let raw = mail_builder::headers::raw::Raw::new;
    match d.importance.signum() {
        1 => b = b.header("Importance", raw("high")),
        -1 => b = b.header("Importance", raw("low")),
        _ => {}
    }
    b = b.header(TEXT_FIRST, raw("its files are on their way"));
    let text = match &d.html {
        Some(h) => MimePart::new(ContentType::new("text/html").attribute("charset", "utf-8"), h.clone()),
        None => MimePart::new(ContentType::new("text/plain").attribute("charset", "utf-8"), d.body.clone()),
    };
    let mut parts = vec![text];
    for f in files.iter().take(500) {
        let kind =
            if f.mime.contains('/') && !f.mime.contains(['\r', '\n']) { f.mime.to_ascii_lowercase() } else { "application/octet-stream".into() };
        let disposition = ContentType::new(if f.inline { "inline" } else { "attachment" })
            .attribute("filename", f.name.clone())
            .attribute("size", f.size.to_string());
        let mut p =
            MimePart::new(ContentType::new(kind).attribute("name", f.name.clone()), Vec::<u8>::new()).header("Content-Disposition", disposition);
        if let Some(cid) = f.content_id.as_deref().map(|c| c.trim().trim_matches(['<', '>'])).filter(|c| !c.is_empty() && !c.contains(['\r', '\n'])) {
            p = p.header("Content-ID", mail_builder::headers::raw::Raw::new(format!("<{cid}>")));
        }
        parts.push(p);
    }
    let mut out = Vec::new();
    b.body(MimePart::new("multipart/mixed", parts)).write_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Whether a message is its text first, its files still to come ([`text_first`]).
pub fn is_text_first(raw: &[u8]) -> bool {
    // The header block only (a body could quote the name).
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len().min(64 * 1024));
    let head = raw.get(..end).unwrap_or(raw);
    let needle = format!("\n{TEXT_FIRST}:").to_ascii_lowercase();
    String::from_utf8_lossy(head).to_ascii_lowercase().contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_text_now_its_files_named_and_sized() {
        let d = Draft {
            to: vec![EmailAddress::new("Riley", "riley@studio.example")],
            subject: "Rushes reel 2 — räder".into(),
            html: Some("<p>Here's reel 2.</p><img src=\"cid:still@x\">".into()),
            importance: 1,
            ..Default::default()
        };
        let files = [
            PendingFile { name: "reel2.mov".into(), mime: "video/quicktime".into(), size: 20_971_586, ..Default::default() },
            PendingFile { name: "still.png".into(), mime: "image/png".into(), size: 2048, content_id: Some("<still@x>".into()), inline: true },
            PendingFile { name: "bad\r\nX: y".into(), mime: "x\r\nEvil: 1".into(), size: 1, ..Default::default() },
        ];
        let raw = text_first(&d, &EmailAddress::new("Sam", "sam@studio.example"), 1_793_300_000, "<m1@studio.example>", &files).unwrap();
        assert!(is_text_first(&raw));
        assert!(!String::from_utf8_lossy(&raw).contains("\nEvil:") && !String::from_utf8_lossy(&raw).contains("\nX: y"));
        let p = parse(&raw).unwrap();
        assert_eq!(p.subject, "Rushes reel 2 — räder");
        assert_eq!(p.message_id, "m1@studio.example");
        let sizes: Vec<(String, usize, bool)> = p.attachments.iter().map(|a| (a.name.clone(), a.size, a.inline)).collect();
        assert_eq!(sizes[0], ("reel2.mov".to_string(), 20_971_586, false));
        assert_eq!((sizes[1].1, sizes[1].2), (2048, true));
        assert!(p.text.contains("reel 2"));
        // An ordinary message isn't; nor one whose text names the header.
        assert!(!is_text_first(b"Subject: x\r\n\r\nX-MailCraft-Text-First: no\r\n"));
    }
}
