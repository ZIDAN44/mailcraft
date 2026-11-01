//! A signed or encrypted item (MS-OXOSMIME) as the message it came as. Outlook keeps an S/MIME
//! message's whole content as its one attachment: a clear-signed one's `multipart/signed` entity
//! (`IPM.Note.SMIME.MultipartSigned`), or an opaque-signed or encrypted one's CMS
//! (`IPM.Note.SMIME`). Rebuilt with that as a file, its signature couldn't be checked nor its
//! content decrypted; put back as the message's content, MailCraft's S/MIME reader does both.

use mailcraft_mime::smime;

use super::item::File;

/// Longest header line the entity may have, as any message's.
const MAX_LINE: usize = 998;

/// What an S/MIME item's attachment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A `multipart/signed` entity, its own header fields and all.
    Signed,
    /// A CMS `ContentInfo` (DER): signed data with its content inside, or encrypted data.
    Opaque,
}

/// What an item of class `class` keeps as its attachment, when it's S/MIME.
pub(super) fn kind(class: &str) -> Option<Kind> {
    let class = class.to_ascii_lowercase();
    if class.starts_with("ipm.note.smime.multipartsigned") {
        Some(Kind::Signed)
    } else if class.starts_with("ipm.note.smime") {
        Some(Kind::Opaque)
    } else {
        None
    }
}

/// The content an S/MIME item's files hold: its one file, when that is all it has.
pub(super) fn content(files: &[File]) -> Option<&[u8]> {
    match files {
        [File::Data { data, .. }] if !data.is_empty() => Some(data),
        _ => None,
    }
}

/// The message `built` (the item rebuilt with its content as a file) with its own header fields
/// above `content` as its content, as it was sent; `None` when `content` isn't what `kind` says
/// (the message then keeps it as a file, as before). An entity is put back as it is, since its
/// signature covers those bytes; it is taken only when its header has nothing but its own
/// `Content-*` fields, each a clean line, so the file can't add fields to the message.
pub(super) fn message(built: &[u8], kind: Kind, content: &[u8]) -> Option<Vec<u8>> {
    let mut out = smime::outer_headers(built);
    match kind {
        Kind::Signed => {
            let head = content.get(..content.windows(4).position(|w| w == b"\r\n\r\n")?)?;
            let lines: Vec<&[u8]> = head.split(|b| *b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).collect();
            let clean = |l: &&[u8]| l.len() <= MAX_LINE && l.iter().all(|b| *b == b'\t' || !b.is_ascii_control());
            let own = |l: &&[u8]| {
                let l = l.to_ascii_lowercase();
                l.starts_with(b" ") || l.starts_with(b"\t") || l.starts_with(b"content-") || l.starts_with(b"mime-version:")
            };
            if !lines.iter().all(|l| clean(l) && own(l)) {
                return None;
            }
            if !lines.iter().any(|l| l.to_ascii_lowercase().starts_with(b"mime-version:")) {
                out.extend_from_slice(b"MIME-Version: 1.0\r\n");
            }
            out.extend_from_slice(content);
            matches!(smime::protection(&out), smime::Protection::Signed { .. }).then_some(out)
        }
        Kind::Opaque => Some(smime::pkcs7_message(&out, content, smime::smime_type_of(content)?)),
    }
}
