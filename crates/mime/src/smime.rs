//! S/MIME's MIME (RFC 8551 §3): finding what protects a message, taking out exactly what was
//! signed or the encrypted content, putting a message back together around what was inside, and
//! writing `multipart/signed` and `application/pkcs7-mime` for sending. The cryptography is
//! `mailcraft-smime`'s.
//!
//! Only a message protected as a whole counts (EFAIL, and signatures spoofed by parts placed
//! around a signed one): a signed or encrypted part inside some other multipart is an attachment.

use mail_parser::{MessageParser, MessagePart};

use crate::attachments::contents;

pub mod pgp;

/// What protects a whole message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protection {
    None,
    /// `multipart/signed`: the first part exactly as it was signed (CRLF line ends), and the
    /// signature (`application/pkcs7-signature`, decoded).
    Signed {
        content: Vec<u8>,
        signature: Vec<u8>,
    },
    /// `application/pkcs7-mime; smime-type=signed-data`: the signature, its content inside.
    Opaque(Vec<u8>),
    /// `application/pkcs7-mime; smime-type=enveloped-data` (or `authEnveloped-data`).
    Encrypted(Vec<u8>),
}

/// The largest message looked at for S/MIME.
const RAW_MAX: usize = 64 << 20;

/// What a message's own header says protects it, before its body is read.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    None,
    /// `multipart/signed` with an S/MIME signature, its parts split at this boundary.
    Signed(String),
    /// `application/pkcs7-mime` (or a `.p7m` sent as `application/octet-stream`); `opaque`:
    /// `smime-type=signed-data`.
    Pkcs7 {
        opaque: bool,
    },
}

/// What the top-level header fields of a message (`head`, its root part) say protects it.
fn kind_of(head: MessagePart<'_>) -> Kind {
    let Some(ct) = head.content_type() else { return Kind::None };
    let kind = ct.ctype().to_ascii_lowercase();
    let sub = ct.subtype().unwrap_or_default().to_ascii_lowercase();
    if kind == "multipart" && sub == "signed" {
        let protocol = ct.attribute("protocol").unwrap_or_default().to_ascii_lowercase();
        if !matches!(protocol.as_str(), "application/pkcs7-signature" | "application/x-pkcs7-signature") {
            return Kind::None;
        }
        return ct.attribute("boundary").map_or(Kind::None, |b| Kind::Signed(b.to_string()));
    }
    // A `.p7m` by its Content-Type `name` or its Content-Disposition `filename`.
    let named_p7m = ct.attribute("name").or_else(|| head.attachment_name()).is_some_and(|n| n.trim().to_ascii_lowercase().ends_with(".p7m"));
    let pkcs7 = kind == "application" && (sub == "pkcs7-mime" || sub == "x-pkcs7-mime" || (sub == "octet-stream" && named_p7m));
    if !pkcs7 {
        return Kind::None;
    }
    Kind::Pkcs7 { opaque: ct.attribute("smime-type").is_some_and(|t| t.eq_ignore_ascii_case("signed-data")) }
}

/// Whether `raw` is signed or encrypted as a whole, read from its header alone (the body of a
/// message that isn't, 30 MB of pictures say, is never parsed): with S/MIME, or with OpenPGP or
/// carrying keys for it ([`pgp::may_carry`]). When it says no, [`protection`] and
/// [`pgp::protection`] are `None`.
pub fn is_protected(raw: &[u8]) -> bool {
    raw.len() <= RAW_MAX && (head_kind(raw) != Kind::None || pgp::may_carry(raw))
}

fn head_kind(raw: &[u8]) -> Kind {
    MessageParser::default().parse_headers(raw).map_or(Kind::None, |m| kind_of(m.root_part()))
}

/// What protects `raw` as a whole. Only a message whose header says so ([`is_protected`]) is
/// parsed whole.
pub fn protection(raw: &[u8]) -> Protection {
    if raw.len() > RAW_MAX {
        return Protection::None;
    }
    let kind = head_kind(raw);
    if kind == Kind::None {
        return Protection::None;
    }
    let Some(m) = MessageParser::default().parse(raw) else { return Protection::None };
    match kind {
        Kind::None => Protection::None,
        Kind::Signed(boundary) => {
            let Some(content) = first_part(raw, &boundary) else { return Protection::None };
            let signature = m
                .root()
                .parts()
                .skip(1)
                .find(|p| {
                    p.content_type().is_some_and(|c| {
                        let s = c.subtype().unwrap_or_default().to_ascii_lowercase();
                        c.ctype().eq_ignore_ascii_case("application") && (s == "pkcs7-signature" || s == "x-pkcs7-signature")
                    })
                })
                .map(|p| contents(p).into_owned());
            match signature {
                Some(signature) if !signature.is_empty() => Protection::Signed { content, signature },
                _ => Protection::None,
            }
        }
        Kind::Pkcs7 { opaque } => {
            let body = contents(m.root_part()).into_owned();
            if body.is_empty() {
                Protection::None
            } else if opaque {
                Protection::Opaque(body)
            } else {
                // Enveloped, authenticated, or unsaid (most `.p7m` mail is encrypted; the CMS says).
                Protection::Encrypted(body)
            }
        }
    }
}

/// `text` with every line ending CRLF (lone LF and lone CR become CRLF).
fn crlf(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 32);
    let mut i = 0;
    while let Some(&b) = text.get(i) {
        match b {
            b'\r' if text.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 2;
                continue;
            }
            b'\r' | b'\n' => out.extend_from_slice(b"\r\n"),
            _ => out.push(b),
        }
        i += 1;
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// The first body part of a `multipart/signed` exactly as signed: from just after the first
/// delimiter line to the CRLF before the next (RFC 1847 §2.1, RFC 8551 §3.5), line ends as CRLF
/// (a message stored with bare LFs was signed with CRLF).
fn first_part(raw: &[u8], boundary: &str) -> Option<Vec<u8>> {
    let raw = crlf(raw);
    let body_at = find(&raw, b"\r\n\r\n", 0)? + 4;
    let delimiter = format!("--{boundary}");
    let d = delimiter.as_bytes();
    // The first delimiter at the start of a line (or of the body).
    let first = if raw.get(body_at..).is_some_and(|b| b.starts_with(d)) { body_at } else { find(&raw, &[b"\r\n", d].concat(), body_at)? + 2 };
    // Past the delimiter line (and any transport padding on it).
    let start = find(&raw, b"\r\n", first + d.len())? + 2;
    let end = find(&raw, &[b"\r\n", d].concat(), start.saturating_sub(2))?;
    raw.get(start..end.max(start)).map(<[u8]>::to_vec)
}

/// The header block and body of a message (line ends as they are).
fn split(raw: &[u8]) -> (&[u8], &[u8]) {
    let (head, sep) = match (find(raw, b"\r\n\r\n", 0), find(raw, b"\n\n", 0)) {
        (Some(a), Some(b)) if b < a => (b, 2),
        (Some(a), _) => (a, 4),
        (None, Some(b)) => (b, 2),
        (None, None) => (raw.len(), 0),
    };
    (raw.get(..head).unwrap_or(raw), raw.get(head + sep..).unwrap_or_default())
}

/// Header fields, each with its folded lines, as (name, whole field).
fn fields(head: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for line in head.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
            if let Some((_, f)) = out.last_mut() {
                f.extend_from_slice(b"\r\n");
                f.extend_from_slice(line);
            }
            continue;
        }
        let name = line.split(|b| *b == b':').next().map(|n| String::from_utf8_lossy(n).trim().to_ascii_lowercase()).unwrap_or_default();
        if !name.is_empty() {
            out.push((name, line.to_vec()));
        }
    }
    out
}

/// The header fields of `raw` that describe the message, not its content (From, To, Subject,
/// Date…): everything but `Content-*` and `MIME-Version`, CRLF-ended.
pub fn outer_headers(raw: &[u8]) -> Vec<u8> {
    let (head, _) = split(raw);
    let mut out = Vec::new();
    for (name, f) in fields(head) {
        if name.starts_with("content-") || name == "mime-version" {
            continue;
        }
        out.extend_from_slice(&f);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// `raw` with its content replaced by `entity` (a MIME entity: its own `Content-*` headers, a
/// blank line, its body), as a reader shows what a signature or encryption held.
pub fn with_entity(raw: &[u8], entity: &[u8]) -> Vec<u8> {
    with_entity_protected(raw, entity).0
}

/// An entity's header block and body (an entity without headers starts with its blank line).
fn entity_parts(entity: &[u8]) -> (&[u8], &[u8]) {
    if let Some(body) = entity.strip_prefix(b"\r\n") {
        return (&[], body);
    }
    split(entity)
}

/// The value of parameter `name` in a header field (`Content-Type: text/plain; hp="cipher"`),
/// lower-case, quotes taken off.
fn parameter(field: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(field).replace("\r\n", " ");
    text.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().trim_matches('"').to_ascii_lowercase())
    })
}

/// [`with_entity`], and whether `entity` carried its message's header fields (header protection,
/// RFC 9788: `hp=` on its `Content-Type`). Those then replace the outer ones, which a server or
/// anyone on the way could have changed (or, for encrypted mail, were left `[...]`); `HP-Outer`
/// fields, and the Legacy Display part written for programs that don't read it, aren't shown.
/// Without `hp=`, header fields inside other than `Content-*` are ignored.
pub fn with_entity_protected(raw: &[u8], entity: &[u8]) -> (Vec<u8>, bool) {
    let entity = crlf(entity);
    let (head, body) = entity_parts(&entity);
    let inner = fields(head);
    let content_type = inner.iter().find(|(n, _)| n == "content-type").map(|(_, f)| f.as_slice()).unwrap_or_default();
    let protected = parameter(content_type, "hp").is_some_and(|v| v == "clear" || v == "cipher");
    let names: Vec<&str> =
        inner.iter().map(|(n, _)| n.as_str()).filter(|n| !n.starts_with("content-") && *n != "hp-outer" && *n != "mime-version").collect();
    let mut out = Vec::new();
    let raw = crlf(raw);
    for (name, f) in fields(split(&raw).0) {
        if name.starts_with("content-") || name == "mime-version" || (protected && names.contains(&name.as_str())) {
            continue;
        }
        out.extend_from_slice(&f);
        out.extend_from_slice(b"\r\n");
    }
    if protected {
        for (_, f) in inner.iter().filter(|(n, _)| !n.starts_with("content-") && n != "hp-outer" && n != "mime-version") {
            out.extend_from_slice(f);
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    for (_, f) in inner.iter().filter(|(n, _)| n.starts_with("content-")) {
        out.extend_from_slice(f);
        out.extend_from_slice(b"\r\n");
    }
    if inner.iter().all(|(n, _)| !n.starts_with("content-")) {
        out.extend_from_slice(b"Content-Type: text/plain; charset=us-ascii\r\n");
    }
    out.extend_from_slice(b"\r\n");
    let body = if protected { without_legacy_display(content_type, body) } else { body.to_vec() };
    out.extend_from_slice(&body);
    (out, protected)
}

/// A multipart body without its Legacy Display part (the first part, marked
/// `hp-legacy-display="1"`: the protected subject repeated for programs that don't read header
/// protection, RFC 9788 §4.5.3).
fn without_legacy_display(content_type: &[u8], body: &[u8]) -> Vec<u8> {
    let Some(boundary) =
        parameter(content_type, "boundary").filter(|_| String::from_utf8_lossy(content_type).to_ascii_lowercase().contains("multipart/"))
    else {
        return body.to_vec();
    };
    // The boundary as written (parameter() lower-cases it).
    let ct = String::from_utf8_lossy(content_type).to_string();
    let written = ct
        .split(';')
        .find_map(|p| {
            p.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case("boundary")).map(|(_, v)| v.trim().trim_matches('"').to_string())
        })
        .unwrap_or(boundary);
    let d = format!("--{written}");
    let starts: Vec<usize> = {
        let mut v = Vec::new();
        let mut from = 0;
        while let Some(i) = find(body, d.as_bytes(), from) {
            if i == 0 || body.get(i.saturating_sub(2)..i) == Some(b"\r\n") {
                v.push(i);
            }
            from = i + d.len();
        }
        v
    };
    let (Some(&first), Some(&second)) = (starts.first(), starts.get(1)) else { return body.to_vec() };
    let part = body.get(first..second).unwrap_or_default();
    let (part_head, _) = split(part.get(d.len()..).map(|p| p.strip_prefix(b"\r\n").unwrap_or(p)).unwrap_or_default());
    let marked = fields(part_head).iter().any(|(n, f)| n == "content-type" && parameter(f, "hp-legacy-display").as_deref() == Some("1"));
    if !marked {
        return body.to_vec();
    }
    [body.get(..first).unwrap_or_default(), body.get(second..).unwrap_or_default()].concat()
}

/// A message (as `crate::build` makes it) split for protecting: its describing header fields,
/// and its content as a MIME entity, CRLF throughout.
pub fn entity_of(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let raw = crlf(raw);
    let (head, body) = split(&raw);
    let mut entity = Vec::new();
    for (name, f) in fields(head) {
        if name.starts_with("content-") {
            entity.extend_from_slice(&f);
            entity.extend_from_slice(b"\r\n");
        }
    }
    if entity.is_empty() {
        entity.extend_from_slice(b"Content-Type: text/plain; charset=utf-8\r\n");
    }
    entity.extend_from_slice(b"\r\n");
    entity.extend_from_slice(body);
    (outer_headers(&raw), entity)
}

/// A header field's value, decoded (`=?utf-8?…?=` words as text).
fn field_value(field: &[u8]) -> String {
    let mut m = field.to_vec();
    m.extend_from_slice(b"\r\n\r\n");
    crate::parse(&m).map(|p| p.subject).unwrap_or_default()
}

/// Quoted-printable for a short text (RFC 2045 §6.7): 7-bit, lines under 76.
fn quoted_printable(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut len = 0;
        let bytes = line.trim_end_matches('\r').as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            let last = i + 1 == bytes.len();
            let plain = (b' '..=b'~').contains(&b) && b != b'=' && !(last && b == b' ');
            let piece = if plain { vec![b] } else { format!("={b:02X}").into_bytes() };
            if len + piece.len() > 75 {
                out.extend_from_slice(b"=\r\n");
                len = 0;
            }
            len += piece.len();
            out.extend(piece);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Header protection (RFC 9788) for a message about to be signed (`hp="clear"`) or encrypted
/// (`hp="cipher"`): `outer` (its describing header fields) written into `entity` too, where the
/// signature or encryption covers them. Encrypted with `hide_subject` (the RFC's default policy,
/// `hcp_baseline`), the Subject outside becomes `[...]` and Comments and Keywords go; `HP-Outer`
/// fields inside say what was left outside, and a Legacy Display part repeats the subject for
/// programs that don't read header protection. Returns the outer fields and the entity to protect.
///
/// What sent the message (`X-Mailer`, `User-Agent`) isn't copied inside: it says nothing to the
/// reader, and servers strip it for privacy (docker-mailserver's Postfix does, and Postfix applies
/// such rules to every MIME part's header too), which would break a clear signature over a copy.
/// Nor is the sender's key (`Autocrypt:`, a kilobyte or more): it's for the way, read outside.
pub fn protect_headers(outer: &[u8], entity: &[u8], encrypted: bool, hide_subject: bool) -> (Vec<u8>, Vec<u8>) {
    const NOT_PROTECTED: [&str; 3] = ["x-mailer", "user-agent", "autocrypt"];
    let entity = crlf(entity);
    let (head, body) = entity_parts(&entity);
    let content: Vec<(String, Vec<u8>)> = fields(head).into_iter().filter(|(n, _)| n.starts_with("content-")).collect();
    let given = fields(&crlf(outer));
    let mut new_outer = Vec::new();
    let mut hp_outer = Vec::new();
    let mut hidden_subject = None;
    for (name, f) in &given {
        let shown: Vec<u8> = match name.as_str() {
            "subject" if encrypted && hide_subject => {
                hidden_subject = Some(field_value(f));
                b"Subject: [...]".to_vec()
            }
            "comments" | "keywords" if encrypted && hide_subject => continue,
            _ => f.clone(),
        };
        new_outer.extend_from_slice(&shown);
        new_outer.extend_from_slice(b"\r\n");
        if encrypted && !NOT_PROTECTED.contains(&name.as_str()) {
            hp_outer.push([b"HP-Outer: ".as_slice(), &shown].concat());
        }
    }
    let hp = if encrypted { "cipher" } else { "clear" };
    let mut out = Vec::new();
    for (_, f) in given.iter().filter(|(n, _)| !NOT_PROTECTED.contains(&n.as_str())) {
        out.extend_from_slice(f);
        out.extend_from_slice(b"\r\n");
    }
    for f in &hp_outer {
        out.extend_from_slice(f);
        out.extend_from_slice(b"\r\n");
    }
    match hidden_subject.filter(|s| !s.trim().is_empty()) {
        // The content becomes the second part of a multipart/mixed, after the Legacy Display part.
        Some(subject) => {
            let seed: u64 = entity.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
            let boundary = format!("=_hp{seed:016x}");
            out.extend_from_slice(format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"; hp=\"{hp}\"\r\n\r\n").as_bytes());
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            out.extend_from_slice(
                b"Content-Type: text/plain; charset=utf-8; hp-legacy-display=\"1\"\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n",
            );
            out.extend(quoted_printable(&format!("Subject: {subject}\n")));
            out.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
            for (_, f) in &content {
                out.extend_from_slice(f);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(body);
            out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        }
        None => {
            let mut has_type = false;
            for (name, f) in &content {
                if name == "content-type" {
                    has_type = true;
                    out.extend_from_slice(f);
                    out.extend_from_slice(format!("; hp=\"{hp}\"").as_bytes());
                } else {
                    out.extend_from_slice(f);
                }
                out.extend_from_slice(b"\r\n");
            }
            if !has_type {
                out.extend_from_slice(format!("Content-Type: text/plain; charset=us-ascii; hp=\"{hp}\"\r\n").as_bytes());
            }
            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(body);
        }
    }
    (new_outer, out)
}

/// Whether `entity` goes through any mail server unchanged: 7-bit, no line over 998 bytes, no
/// trailing white space a server could strip (RFC 8551 §3.1.2). A signature over anything else
/// may not survive the way.
pub fn safe_to_sign(entity: &[u8]) -> bool {
    entity.is_ascii() && entity.split(|b| *b == b'\n').all(|l| l.len() <= 998 && !l.strip_suffix(b"\r").unwrap_or(l).ends_with(b" "))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 in lines of 76 (RFC 2045 §6.8), CRLF-ended.
/// `data` in base64 (RFC 4648), on one line.
pub(crate) fn base64(data: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(data.len() * 4 / 3 + 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            let c = if i <= chunk.len() { B64.get(((n >> (18 - 6 * i)) & 63) as usize).copied().unwrap_or(b'A') } else { b'=' };
            text.push(c);
        }
    }
    text
}

fn base64_lines(data: &[u8]) -> Vec<u8> {
    let text = base64(data);
    let mut out = Vec::with_capacity(text.len() + text.len() / 38);
    for line in text.chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A signed message: `outer` header fields, then `multipart/signed` with `entity` exactly as
/// signed and `signature` (DER `SignedData`) as `smime.p7s`.
pub fn signed_message(outer: &[u8], entity: &[u8], signature: &[u8], boundary: &str) -> Vec<u8> {
    let mut out = outer.to_vec();
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(
        format!("Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256;\r\n boundary=\"{boundary}\"\r\n\r\n")
            .as_bytes(),
    );
    out.extend_from_slice(b"This is a signed message in S/MIME format.\r\n\r\n");
    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    out.extend_from_slice(entity);
    out.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    out.extend_from_slice(b"Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n");
    out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"smime.p7s\"\r\n");
    out.extend_from_slice(b"Content-Description: S/MIME Cryptographic Signature\r\n\r\n");
    out.extend(base64_lines(signature));
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// An encrypted message: `outer` header fields, then `cms` (DER) as `smime.p7m`; `authenticated`
/// for `AuthEnvelopedData`.
pub fn encrypted_message(outer: &[u8], cms: &[u8], authenticated: bool) -> Vec<u8> {
    pkcs7_message(outer, cms, if authenticated { "authEnveloped-data" } else { "enveloped-data" })
}

/// A message that is `outer` header fields, then `cms` (DER) as `smime.p7m` of S/MIME type
/// `smime_type` (`signed-data`, `enveloped-data`, `authEnveloped-data`: [`smime_type_of`]).
pub fn pkcs7_message(outer: &[u8], cms: &[u8], smime_type: &str) -> Vec<u8> {
    let description = if smime_type == "signed-data" { "S/MIME Signed Message" } else { "S/MIME Encrypted Message" };
    let mut out = outer.to_vec();
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(format!("Content-Type: application/pkcs7-mime; smime-type={smime_type}; name=\"smime.p7m\"\r\n").as_bytes());
    out.extend_from_slice(b"Content-Transfer-Encoding: base64\r\n");
    out.extend_from_slice(b"Content-Disposition: attachment; filename=\"smime.p7m\"\r\n");
    out.extend_from_slice(format!("Content-Description: {description}\r\n\r\n").as_bytes());
    out.extend(base64_lines(cms));
    out
}

/// The S/MIME type of a CMS `ContentInfo` (RFC 5652 §3) from the content type that starts it:
/// `signed-data`, `enveloped-data`, or `authEnveloped-data` (RFC 5083); `None` for anything else.
pub fn smime_type_of(cms: &[u8]) -> Option<&'static str> {
    // pkcs7 (1.2.840.113549.1.7) and S/MIME's content types (1.2.840.113549.1.9.16.1), as DER.
    const PKCS7: [u8; 10] = [0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07];
    const SMIME: [u8; 12] = [0x06, 0x0B, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01];
    // A SEQUENCE, its length in its own byte, in up to four more, or left open (BER).
    let rest = match cms {
        [0x30, n, rest @ ..] if *n <= 0x80 => rest,
        [0x30, n @ 0x81..=0x84, rest @ ..] => rest.get(usize::from(*n - 0x80)..)?,
        _ => return None,
    };
    if let Some(kind) = rest.strip_prefix(&PKCS7) {
        return match kind.first() {
            Some(2) => Some("signed-data"),
            Some(3) => Some("enveloped-data"),
            _ => None,
        };
    }
    (rest.strip_prefix(&SMIME)?.first() == Some(&23)).then_some("authEnveloped-data")
}

/// Whether an attachment is S/MIME's own (the signature or the encrypted content), or PGP/MIME's,
/// not the sender's file: it isn't listed among a protected message's attachments.
pub fn is_smime_part(mime: &str, name: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    m.contains("pkcs7-signature") || m.contains("pkcs7-mime") || n == "smime.p7s" || n == "smime.p7m" || pgp::is_pgp_part(mime, name)
}

/// What signs and what encrypts a message for [`protect`]: each takes the MIME entity and gives
/// the CMS (a detached signature; `EnvelopedData` or, with `gcm`, `AuthEnvelopedData`).
pub type Crypto<'a> = &'a mut dyn FnMut(&[u8]) -> Result<Vec<u8>, String>;

/// `raw` signed (with `sign`) and/or encrypted (with `encrypt`, AES-GCM when `gcm`), as it will
/// be sent: its header fields inside too (RFC 9788), the subject outside `[...]` when it's
/// encrypted with `hide_subject`. `boundary` separates a signed message's parts.
pub fn protect(
    raw: &[u8],
    sign: Option<Crypto<'_>>,
    encrypt: Option<Crypto<'_>>,
    gcm: bool,
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
        let cms = encrypt(&entity)?;
        message = encrypted_message(&outer, &cms, gcm);
    }
    Ok(message)
}

#[cfg(test)]
mod tests;
