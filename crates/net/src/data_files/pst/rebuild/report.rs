//! A report that came in by mail (RFC 6522: a bounce, a read receipt, a feedback report) goes
//! back out as one. The item keeps the report's text, its status and the message it returns as
//! files; rebuilt, they are a `multipart/mixed` again unless the original header says
//! `multipart/report` and the files are just what a report holds: then the status goes first
//! among them and the message is a `multipart/report` of its `report-type` again. Anything else
//! stays mixed, every part kept.

use super::super::item::File;
use super::*;

/// What a returned message can be, beside the status: the whole of it or its header.
const RETURNED: [&str; 2] = ["message/rfc822", "text/rfc822-headers"];

/// The `report-type` of an item whose original header (`0x007D`) says `multipart/report`, when
/// its files are one status and at most one returned message; those are then put status first.
/// `None` (and the files as they were) otherwise.
pub(crate) fn report_type(original: &str, files: &mut [File]) -> Option<String> {
    if !rebuild::usable_headers(original) {
        return None;
    }
    let kind = mime::report_type(&mime::header_field(original.as_bytes(), "Content-Type")?)?;
    let mime = |f: &File| match f {
        File::Data { mime, .. } => mime.trim().to_ascii_lowercase(),
        File::Message(_) => "message/rfc822".to_string(),
    };
    let status = |f: &File| mime::REPORT_STATUS.contains(&mime(f).as_str());
    let returned = |f: &File| RETURNED.contains(&mime(f).as_str());
    let statuses = files.iter().filter(|f| status(f)).count();
    let returns = files.iter().filter(|f| returned(f)).count();
    if statuses != 1 || returns > 1 || statuses + returns != files.len() {
        return None;
    }
    files.sort_by_key(|f| !status(f));
    Some(kind)
}

/// The built message as a report of this kind: its top-level `multipart/mixed` becomes
/// `multipart/report; report-type=…` (its boundary as it was). A message whose top isn't mixed
/// is left as it is.
pub(crate) fn as_report(mut built: Vec<u8>, kind: &str) -> Vec<u8> {
    let Some(blank) = mime::header_end(&built) else { return built };
    let head = built.get(..blank.start).unwrap_or_default();
    // Its value, past the colon and the white space after it.
    let at = mime::field_span(head, "Content-Type").and_then(|field| {
        let colon = head.get(field.clone())?.iter().position(|b| *b == b':')?;
        let value = field.start.saturating_add(colon).saturating_add(1);
        Some(value.saturating_add(head.get(value..)?.iter().take_while(|b| **b == b' ' || **b == b'\t').count()))
    });
    let mixed = b"multipart/mixed";
    let Some(at) = at.filter(|&at| built.get(at..at.saturating_add(mixed.len())).is_some_and(|v| v.eq_ignore_ascii_case(mixed))) else {
        return built;
    };
    built.splice(at..at.saturating_add(mixed.len()), format!("multipart/report; report-type={kind}").into_bytes());
    built
}
