//! What a message is beyond a note, as Outlook keeps it: the reverse of what the import reads
//! (`pst/protected.rs`, `pst/report.rs`).
//!
//! Signed and encrypted mail (MS-OXOSMIME 2.1): a clear-signed message is
//! `IPM.Note.SMIME.MultipartSigned` with one attachment, `smime.p7m` (`multipart/signed`),
//! holding the whole signed entity, its `Content-*` header fields and all; an opaque-signed or
//! encrypted one is `IPM.Note.SMIME` with `smime.p7m` (`application/pkcs7-mime`) holding the CMS.
//! The body is the text MailCraft shows: what was signed (for an opaque signature, the content it
//! carries), nothing for encrypted mail.
//!
//! Reports (RFC 6522): a delivery report is `REPORT.IPM.Note.NDR`, `.DelayedDR` or `.DR` by its
//! recipients' `Action`, a read receipt `REPORT.IPM.Note.IPNRN` or `.IPNNRN` by its
//! `Disposition`. Its status and the message it returns stay among its files (the message as an
//! attached message), and its header block says `multipart/report`, which the import makes it
//! again.

use mailcraft_mime::smime::{self, Protection};

use super::files::File;
use super::*;

const SIGNED: &str = "IPM.Note.SMIME.MultipartSigned";
const OPAQUE: &str = "IPM.Note.SMIME";

/// The longest delivery status or disposition read for a report's class; they are a few lines
/// for each recipient.
const MAX_STATUS: usize = 256 * 1024;

/// A signed or encrypted message as Outlook keeps it.
pub(super) struct Protected {
    pub(super) class: &'static str,
    /// Its `smime.p7m`.
    pub(super) file: File,
    pub(super) body: String,
    pub(super) html: Option<String>,
}

/// `raw` (parsed as `p`) as Outlook keeps it when it is signed or encrypted as a whole.
pub(super) fn protected(raw: &[u8], p: &mime::Parsed) -> Option<Protected> {
    if !smime::is_protected(raw) {
        return None;
    }
    let p7m = |mime: &str, data: Vec<u8>| File::Data { name: "smime.p7m".into(), mime: mime.into(), cid: None, data };
    match smime::protection(raw) {
        Protection::None => None,
        Protection::Signed { .. } => {
            let (_, entity) = smime::entity_of(raw);
            Some(Protected { class: SIGNED, file: p7m("multipart/signed", entity), body: p.text.clone(), html: mime::html_body(raw) })
        }
        Protection::Opaque(cms) => {
            // What the signature carries, as the reading pane shows it.
            let shown = mailcraft_smime::carried(&cms).ok().map(|content| smime::with_entity(raw, &content));
            let body = shown.as_deref().and_then(mime::parse).map(|s| s.text).unwrap_or_default();
            let html = shown.as_deref().and_then(mime::html_body);
            Some(Protected { class: OPAQUE, file: p7m("application/pkcs7-mime", cms), body, html })
        }
        Protection::Encrypted(cms) => Some(Protected { class: OPAQUE, file: p7m("application/pkcs7-mime", cms), body: String::new(), html: None }),
    }
}

/// The class of a report (`raw`, parsed as `p`, `headers` its header block), when it is a
/// delivery report or a read receipt Outlook has one for.
pub(super) fn report(headers: &str, raw: &[u8], p: &mime::Parsed) -> Option<&'static str> {
    let content_type = item::header_value(headers, "Content-Type")?;
    let mut params = content_type.split(';');
    if !params.next()?.trim().eq_ignore_ascii_case("multipart/report") {
        return None;
    }
    let kind = params.filter_map(|x| x.split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("report-type"))?.1;
    let kind = kind.trim().trim_matches('"').to_ascii_lowercase();
    let status_type = match kind.as_str() {
        "delivery-status" => "message/delivery-status",
        "disposition-notification" => "message/disposition-notification",
        _ => return None,
    };
    let status = p.attachments.iter().find(|a| a.mime.eq_ignore_ascii_case(status_type) && a.size <= MAX_STATUS)?;
    let status = mime::attachment_data(raw, status.index)?;
    class_of(&kind, &String::from_utf8_lossy(status.get(..MAX_STATUS.min(status.len()))?))
}

/// A report's class from its `report-type` (lower case) and its status's text.
fn class_of(kind: &str, status: &str) -> Option<&'static str> {
    // The status's fields (one block for the message, one for each recipient), unfolded.
    let values = |name: &str| {
        let mut out = Vec::new();
        for block in status.replace("\r\n", "\n").split("\n\n") {
            if let Some(v) = item::header_value(block, name) {
                out.push(v.to_ascii_lowercase());
            }
        }
        out
    };
    if kind == "delivery-status" {
        let actions = values("Action");
        let any = |a: &[&str]| actions.iter().any(|x| a.contains(&x.split_whitespace().next().unwrap_or_default()));
        return if any(&["failed"]) {
            Some("REPORT.IPM.Note.NDR")
        } else if any(&["delayed"]) {
            Some("REPORT.IPM.Note.DelayedDR")
        } else if any(&["delivered", "relayed", "expanded"]) {
            Some("REPORT.IPM.Note.DR")
        } else {
            None
        };
    }
    // `Disposition: mode; type/modifier` (RFC 8098 §3.2.6).
    let disposition = values("Disposition").into_iter().next()?;
    let (_, kind) = disposition.split_once(';')?;
    match kind.split('/').next().unwrap_or_default().trim() {
        "displayed" => Some("REPORT.IPM.Note.IPNRN"),
        "deleted" => Some("REPORT.IPM.Note.IPNNRN"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_classes() {
        let dsn = |actions: &[&str]| {
            let mut s = "Reporting-MTA: dns; mx.example\r\n".to_string();
            for a in actions {
                s.push_str(&format!("\r\nFinal-Recipient: rfc822; a@example.com\r\nAction: {a}\r\nStatus: 5.0.0\r\n"));
            }
            class_of("delivery-status", &s)
        };
        assert_eq!(dsn(&["delivered", "failed"]), Some("REPORT.IPM.Note.NDR"));
        assert_eq!(dsn(&["Delayed"]), Some("REPORT.IPM.Note.DelayedDR"));
        assert_eq!(dsn(&["relayed (gatewayed)"]), Some("REPORT.IPM.Note.DR"));
        assert_eq!(dsn(&["unknown"]), None);
        assert_eq!(dsn(&[]), None);
        let mdn = |d: &str| class_of("disposition-notification", &format!("Final-Recipient: rfc822; a@b\nDisposition: {d}\n"));
        assert_eq!(mdn("manual-action/MDN-sent-manually; displayed"), Some("REPORT.IPM.Note.IPNRN"));
        assert_eq!(mdn("automatic-action/MDN-sent-automatically;\n deleted/error"), Some("REPORT.IPM.Note.IPNNRN"));
        assert_eq!(mdn("automatic-action/MDN-sent-automatically; processed"), None);
        assert_eq!(mdn("displayed"), None);
    }
}
