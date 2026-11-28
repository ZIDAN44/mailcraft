//! What MailCraft reads of mail-parser's parts: the files a message carries, a part's bytes, and
//! its header fields as written.

use std::borrow::Cow;

use mail_parser::{Charset, DecodeProblems, Encoding, MessagePart, MessageRef, PartKind, PartRole};

/// The parts of `m` (not those of the messages attached to it) the attachment bar lists, in the
/// order they come: mail-parser's attachments (RFC 8621), and also a picture, sound or video the
/// body shows in both its text and HTML versions, which RFC 8621 leaves out of the attachments
/// but which the reading pane finds its `cid:` pictures among, and a part in no list at all, so
/// nothing a message carries goes unseen.
pub(crate) fn files_of<'m>(m: MessageRef<'m>) -> impl Iterator<Item = MessagePart<'m>> {
    m.parts().filter(|p| p.is_attachment() || matches!(p.role(), PartRole::MediaBody | PartRole::Dropped))
}

/// A part's bytes as the reading pane takes them: a text part's text (read in its charset, as
/// UTF-8), any other's bytes with the transfer encoding undone (an attached message's source),
/// and nothing for a multipart.
pub(crate) fn contents<'m>(p: MessagePart<'m>) -> Cow<'m, [u8]> {
    match p.kind() {
        PartKind::Text | PartKind::Html => {
            let charset = p.content_type().and_then(|c| c.attribute("charset")).and_then(|c| Charset::from_label(c.as_bytes())).unwrap_or_default();
            match text_bytes(p) {
                Cow::Borrowed(b) => match charset.decode(b) {
                    Cow::Borrowed(t) => Cow::Borrowed(t.as_bytes()),
                    Cow::Owned(t) => Cow::Owned(t.into_bytes()),
                },
                Cow::Owned(b) => Cow::Owned(charset.decode_owned(b).into_bytes()),
            }
        }
        PartKind::Multipart => Cow::Borrowed(&[]),
        PartKind::Binary | PartKind::InlineBinary => padded_base64(p).map_or_else(|| p.decoded(), Cow::Owned),
        PartKind::Message(_) => p.decoded(),
    }
}

/// A base64 part's bytes when something follows its padding: base64 ends there (RFC 2045: "="
/// is used only at the end of the data), and what follows isn't the part's (a footer a mail
/// gateway writes into the message's last part, "Scanned by …"). mail-parser decoded on past
/// it, and the footer's letters came out at the end of the file (#260). `None` for any other
/// part, which mail-parser decodes as it is.
pub(crate) fn padded_base64(p: MessagePart<'_>) -> Option<Vec<u8>> {
    if p.encoding() != Encoding::Base64 {
        return None;
    }
    let raw = p.raw_body();
    let pad = raw.iter().position(|&b| b == b'=')?;
    let end = pad + raw.get(pad..)?.iter().take(2).take_while(|&&b| b == b'=').count();
    // Nothing after the padding but line ends, or more base64 (a program that writes it in
    // padded pieces, one after another): mail-parser's own reading is right. Words, with their
    // spaces and stops, aren't base64.
    let base64_line = |line: &[u8]| line.trim_ascii().iter().all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
    if raw.get(end..)?.split(|&b| b == b'\n').all(base64_line) {
        return None;
    }
    // Words before it too: never base64 (text sent as base64 by mistake, which mail-parser's
    // reading, or its source, shows best).
    if !raw.get(..end)?.split(|&b| b == b'\n').all(base64_line) {
        return None;
    }
    let clean: Vec<u8> = raw.get(..end)?.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    encodify::base64::LENIENT.decode(&clean).ok()
}

/// A text part's bytes, the transfer encoding undone. One whose base64 can't be read is its
/// source as it came: text sent as base64 by mistake stays readable (mail-parser decodes what
/// it can of it, passing over what isn't base64, which turns such text into noise).
fn text_bytes(p: MessagePart<'_>) -> Cow<'_, [u8]> {
    if p.encoding() != Encoding::Base64 {
        return p.decoded();
    }
    // A footer after its padding isn't the text's, nor a sign it was never base64.
    if let Some(bytes) = padded_base64(p) {
        return Cow::Owned(bytes);
    }
    match p.decoded_checked() {
        (_, problems) if problems.contains(DecodeProblems::MALFORMED_TRANSFER_ENCODING) => Cow::Borrowed(p.raw_body()),
        (bytes, _) => bytes,
    }
}

/// A text file's bytes as they were sent, only the transfer encoding undone: [`contents`] gives
/// a `text/*` part as text, read in its charset or, with none, as UTF-8 with any other byte
/// replaced, so a file in Latin-1 or Windows-1252 would be saved changed. `None` for other
/// parts, whose bytes are already these.
pub(crate) fn sent_bytes(p: MessagePart<'_>) -> Option<Vec<u8>> {
    p.is_text().then(|| text_bytes(p).into_owned())
}

/// The value of a message's last `name` header field as written (folding and all), when it is
/// UTF-8.
pub(crate) fn header_raw<'m>(m: MessageRef<'m>, name: &str) -> Option<&'m str> {
    m.headers().get(name).and_then(|h| std::str::from_utf8(h.raw_value()).ok())
}

/// The value of a message's first (topmost) `name` header field as written, when it is UTF-8:
/// the one the receiving server added, where a lower one may be the sender's own.
pub(crate) fn header_top<'m>(m: MessageRef<'m>, name: &str) -> Option<&'m str> {
    m.headers().all(name).next().and_then(|h| std::str::from_utf8(h.raw_value()).ok())
}
