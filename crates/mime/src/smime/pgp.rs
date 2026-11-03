//! OpenPGP's MIME (RFC 3156, PGP/MIME): finding what protects a message (`multipart/signed` with
//! an `application/pgp-signature`, `multipart/encrypted` with `application/pgp-encrypted`, or a
//! PGP block in a text alone), writing them for sending, and the keys a message carries (its
//! `Autocrypt:` header, `application/pgp-keys` parts). The cryptography is `mailcraft-pgp`'s;
//! taking a message apart and putting it back together is S/MIME's ([`super`]).
//!
//! As with S/MIME, only a message protected as a whole counts (EFAIL): a signed or encrypted
//! part inside some other multipart is an attachment, and inline PGP only in a message that is
//! one text.

use mail_parser::{MessageParser, MimeHeaders};

use super::{RAW_MAX, crlf, entity_of, fields, find, first_part, protect_headers, safe_to_sign, split, with_entity};

/// What protects a whole message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protection {
    None,
    /// `multipart/signed` (`application/pgp-signature`): the first part exactly as it was signed
    /// (CRLF line ends), and the signature (armored, as sent).
    Signed {
        content: Vec<u8>,
        signature: Vec<u8>,
    },
    /// `multipart/encrypted` (`application/pgp-encrypted`): the encrypted message (armored).
    Encrypted(Vec<u8>),
    /// A message that is one text holding a PGP block (`-----BEGIN PGP MESSAGE-----` or a
    /// cleartext-signed text): its text as sent (transfer encoding undone, not converted), and
    /// its charset.
    Inline {
        body: Vec<u8>,
        charset: String,
    },
}

fn content_type_of(p: &mail_parser::MessagePart<'_>) -> String {
    p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or_default()).to_ascii_lowercase()).unwrap_or_default()
}

/// Whether `text` has a line that starts a PGP message or a cleartext-signed text.
fn has_block(text: &[u8]) -> bool {
    [b"-----BEGIN PGP MESSAGE-----".as_slice(), b"-----BEGIN PGP SIGNED MESSAGE-----"].iter().any(|marker| {
        let mut from = 0;
        while let Some(at) = find(text, marker, from) {
            if at == 0 || matches!(text.get(at - 1), Some(b'\n' | b'\r')) {
                return true;
            }
            from = at + marker.len();
        }
        false
    })
}

/// The most a multipart message is looked through for attached keys before it's opened (a
/// message of pictures isn't).
const KEYS_LOOKED_MAX: usize = 2 << 20;

/// Whether `raw` may be protected with OpenPGP or carry keys for it, from its header (and the
/// text of a message that is one text, or a smaller multipart's bytes): PGP/MIME, an
/// `Autocrypt:` header, a PGP block, an attached key. When it says no, [`protection`] is `None`
/// and [`autocrypt`] and [`key_parts`] are empty.
pub fn may_carry(raw: &[u8]) -> bool {
    if raw.len() > RAW_MAX {
        return false;
    }
    let (head, body) = split(raw);
    if fields(head).iter().any(|(n, _)| n == "autocrypt") {
        return true;
    }
    let Some(m) = MessageParser::default().parse_headers(raw) else { return false };
    let Some(root) = m.parts.first() else { return false };
    let (kind, sub) = match root.content_type() {
        Some(ct) => (ct.ctype().to_ascii_lowercase(), ct.subtype().unwrap_or_default().to_ascii_lowercase()),
        None => ("text".to_string(), "plain".to_string()),
    };
    let protocol = root.content_type().and_then(|c| c.attribute("protocol")).unwrap_or_default().trim().to_ascii_lowercase();
    match (kind.as_str(), sub.as_str()) {
        ("multipart", "signed") => protocol == "application/pgp-signature",
        ("multipart", "encrypted") => protocol == "application/pgp-encrypted",
        ("multipart", _) => raw.len() <= KEYS_LOOKED_MAX && find_ignoring_case(body, b"application/pgp-keys"),
        // Quoted-printable leaves an armor line as it is (a text in base64 isn't looked into).
        ("text", "plain") => find(body, b"-----BEGIN PGP ", 0).is_some(),
        _ => false,
    }
}

/// Whether `needle` (lower-case) is in `hay`, in any case.
fn find_ignoring_case(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// What protects `raw` as a whole (OpenPGP).
pub fn protection(raw: &[u8]) -> Protection {
    if raw.len() > RAW_MAX {
        return Protection::None;
    }
    let Some(m) = MessageParser::default().parse(raw) else { return Protection::None };
    let (kind, sub) = match m.content_type() {
        Some(ct) => (ct.ctype().to_ascii_lowercase(), ct.subtype().unwrap_or_default().to_ascii_lowercase()),
        None => ("text".to_string(), "plain".to_string()),
    };
    let protocol = m.content_type().and_then(|c| c.attribute("protocol")).unwrap_or_default().trim().to_ascii_lowercase();
    if kind == "multipart" && sub == "signed" && protocol == "application/pgp-signature" {
        let Some(boundary) = m.content_type().and_then(|c| c.attribute("boundary")) else { return Protection::None };
        let Some(content) = first_part(raw, boundary) else { return Protection::None };
        let signature = m.parts.iter().skip(1).find(|p| content_type_of(p) == "application/pgp-signature").map(|p| p.contents().to_vec());
        return match signature {
            Some(signature) if !signature.is_empty() => Protection::Signed { content, signature },
            _ => Protection::None,
        };
    }
    if kind == "multipart" && sub == "encrypted" && protocol == "application/pgp-encrypted" {
        // The version part first, then the message (RFC 3156 §4).
        let body = m.parts.iter().skip(1).find(|p| content_type_of(p) == "application/octet-stream").map(|p| p.contents().to_vec());
        return match body {
            Some(b) if !b.is_empty() => Protection::Encrypted(b),
            _ => Protection::None,
        };
    }
    if kind == "text" && sub == "plain" {
        let Some(part) = m.parts.first() else { return Protection::None };
        let body = part.contents();
        if has_block(body) {
            let charset = m.content_type().and_then(|c| c.attribute("charset")).unwrap_or("utf-8").trim().to_ascii_lowercase();
            return Protection::Inline { body: body.to_vec(), charset };
        }
    }
    Protection::None
}

/// A text as a MIME entity to put back in its message (what an inline PGP message held).
pub fn text_entity(text: &[u8], charset: &str) -> Vec<u8> {
    let charset = if std::str::from_utf8(text).is_ok() { "utf-8" } else { charset };
    let charset: String = charset.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.:".contains(*c)).take(40).collect();
    let mut out = format!("Content-Type: text/plain; charset={}\r\n", if charset.is_empty() { "utf-8" } else { &charset }).into_bytes();
    if !text.is_ascii() {
        out.extend_from_slice(b"Content-Transfer-Encoding: 8bit\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&crlf(text));
    out
}

/// The `Autocrypt:` header fields' values in `raw`'s own header (unfolded), at most a few.
pub fn autocrypt(raw: &[u8]) -> Vec<String> {
    let (head, _) = split(raw.get(..raw.len().min(RAW_MAX)).unwrap_or_default());
    fields(head)
        .into_iter()
        .filter(|(n, _)| n == "autocrypt")
        .take(4)
        .filter_map(|(_, f)| {
            let text = String::from_utf8_lossy(&f).replace("\r\n", "");
            text.split_once(':').map(|(_, v)| v.trim().to_string())
        })
        .collect()
}

/// The `application/pgp-keys` parts of `raw` (keys someone sent), at most a few, each at most a
/// megabyte.
pub fn key_parts(raw: &[u8]) -> Vec<Vec<u8>> {
    if raw.len() > RAW_MAX {
        return Vec::new();
    }
    let Some(m) = MessageParser::default().parse(raw) else { return Vec::new() };
    m.parts
        .iter()
        .filter(|p| content_type_of(p) == "application/pgp-keys")
        .map(|p| p.contents())
        .filter(|c| !c.is_empty() && c.len() <= 1 << 20)
        .take(8)
        .map(<[u8]>::to_vec)
        .collect()
}

/// Whether an attachment is PGP/MIME's own (its signature, version part or encrypted message).
pub fn is_pgp_part(mime: &str, name: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    m.contains("pgp-signature") || m.contains("pgp-encrypted") || n == "encrypted.asc"
}

/// A signed message: `outer` header fields, then `multipart/signed` with `entity` exactly as
/// signed and `signature` (armored) as `signature.asc`.
pub fn signed_message(outer: &[u8], entity: &[u8], signature: &[u8], boundary: &str) -> Vec<u8> {
    let mut out = outer.to_vec();
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/signed; micalg=pgp-sha256; protocol=\"application/pgp-signature\";\r\n boundary=\"{boundary}\"\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(b"This is an OpenPGP/MIME signed message (RFC 4880 and 3156)\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(entity);
    out.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/pgp-signature; name=\"signature.asc\"\r\n");
    out.extend_from_slice(b"Content-Description: OpenPGP digital signature\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n");
    out.extend(crlf(signature.trim_ascii_end()));
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// An encrypted message: `outer` header fields, then `multipart/encrypted` with the version part
/// and `armored` (the encrypted message).
pub fn encrypted_message(outer: &[u8], armored: &[u8], boundary: &str) -> Vec<u8> {
    let mut out = outer.to_vec();
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\";\r\n boundary=\"{boundary}\"\r\n\r\n").as_bytes(),
    );
    out.extend_from_slice(b"This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156)\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(
        b"Content-Type: application/pgp-encrypted\r\nContent-Description: PGP/MIME version identification\r\n\r\nVersion: 1\r\n\r\n",
    );
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/octet-stream; name=\"encrypted.asc\"\r\n");
    out.extend_from_slice(b"Content-Description: OpenPGP encrypted message\r\n");
    out.extend_from_slice(b"Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n");
    out.extend(crlf(armored.trim_ascii_end()));
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// `raw` signed (`sign`: the MIME entity in, an armored detached signature out) and/or encrypted
/// (`encrypt`: the entity in, an armored message out) as PGP/MIME, as it will be sent: signed
/// first, then encrypted (RFC 3156 §6.1), its header fields inside too (RFC 9788), the subject
/// outside `[...]` when it's encrypted with `hide_subject`.
pub fn protect(
    raw: &[u8],
    sign: Option<super::Crypto<'_>>,
    encrypt: Option<super::Crypto<'_>>,
    hide_subject: bool,
    boundary: &str,
) -> Result<Vec<u8>, String> {
    let (outer, entity) = entity_of(raw);
    let (outer, mut entity) = protect_headers(&outer, &entity, encrypt.is_some(), hide_subject);
    let mut message = with_entity(raw, &entity);
    if let Some(sign) = sign {
        if !safe_to_sign(&entity) {
            return Err("a part of it couldn't be sent byte for byte, so its signature wouldn't hold".into());
        }
        let signature = sign(&entity)?;
        message = signed_message(&outer, &entity, &signature, boundary);
        entity = entity_of(&message).1;
    }
    if let Some(encrypt) = encrypt {
        let armored = encrypt(&entity)?;
        message = encrypted_message(&outer, &armored, &format!("{boundary}e"));
    }
    Ok(message)
}

#[cfg(test)]
mod tests;
