//! A report that came in by mail (RFC 6522: a bounce, a read receipt, a feedback report) goes
//! back out as one. The item keeps the report's text, its status and the message it returns as
//! files; rebuilt, they are a `multipart/mixed` again unless the original header says
//! `multipart/report` and the files are just what a report holds: then the status goes first
//! among them and the message is a `multipart/report` of its `report-type` again. Anything else
//! stays mixed, every part kept.

use super::item::File;
use super::*;

/// What a returned message can be, beside the status: the whole of it or its header.
const RETURNED: [&str; 2] = ["message/rfc822", "text/rfc822-headers"];

/// Longest `report-type` kept; the ones in use are a few words.
const MAX_TYPE: usize = 64;

/// The `report-type` of an item whose original header (`0x007D`) says `multipart/report`, when
/// its files are one status and at most one returned message; those are then put status first.
/// `None` (and the files as they were) otherwise.
pub(super) fn report_type(original: &str, files: &mut [File]) -> Option<String> {
    if !rebuild::usable_headers(original) {
        return None;
    }
    let kind = report_kind(&content_type(original)?)?;
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

/// The original header's `Content-Type`, its folded lines joined.
fn content_type(original: &str) -> Option<String> {
    let mut value: Option<String> = None;
    for line in original.lines().map(|l| l.trim_end_matches('\r')) {
        if line.starts_with([' ', '\t']) {
            if let Some(v) = value.as_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
            continue;
        }
        if value.is_some() {
            break;
        }
        if let Some((name, v)) = line.split_once(':')
            && name.trim_end().eq_ignore_ascii_case("content-type")
        {
            value = Some(v.trim().to_string());
        }
    }
    value
}

/// A `multipart/report` Content-Type's `report-type`: a token, lower case, or `None` when it
/// isn't one or has none.
fn report_kind(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    if !parts.next()?.trim().eq_ignore_ascii_case("multipart/report") {
        return None;
    }
    let kind = parts.filter_map(|p| p.split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("report-type"))?.1;
    let kind = kind.trim().trim_matches('"').trim().to_ascii_lowercase();
    let token = !kind.is_empty() && kind.len() <= MAX_TYPE && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._+".contains(&b));
    token.then_some(kind)
}

/// The built message as a report of this kind: its top-level `multipart/mixed` becomes
/// `multipart/report; report-type=…` (its boundary as it was). A message whose top isn't mixed
/// is left as it is.
pub(super) fn as_report(mut built: Vec<u8>, kind: &str) -> Vec<u8> {
    let Some(end) = built.windows(4).position(|w| w == b"\r\n\r\n") else { return built };
    let head = built.get(..end).unwrap_or_default();
    let mut start = 0usize;
    let mut at = None;
    for line in head.split(|&b| b == b'\n') {
        let name = b"content-type:";
        if line.len() >= name.len() && line.get(..name.len()).is_some_and(|n| n.eq_ignore_ascii_case(name)) {
            let skip = line.iter().skip(name.len()).take_while(|b| **b == b' ' || **b == b'\t').count();
            at = Some(start.saturating_add(name.len()).saturating_add(skip));
            break;
        }
        start = start.saturating_add(line.len()).saturating_add(1);
    }
    let mixed = b"multipart/mixed";
    let Some(at) = at.filter(|&at| built.get(at..at.saturating_add(mixed.len())).is_some_and(|v| v.eq_ignore_ascii_case(mixed))) else {
        return built;
    };
    built.splice(at..at.saturating_add(mixed.len()), format!("multipart/report; report-type={kind}").into_bytes());
    built
}
