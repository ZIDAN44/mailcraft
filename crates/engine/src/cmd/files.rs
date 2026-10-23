//! Commands: what the system hands MailCraft as its mail app (`mailto:` links, and `.eml`,
//! `.ics` and `.vcf` files opened with it).

use super::*;

/// Largest message file read (`.eml`).
const MAX_MESSAGE: u64 = 64 << 20;
/// Largest calendar or contacts file read (`.ics`, `.vcf`).
const MAX_PIM: u64 = 16 << 20;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.mailto", "Write to a mailto: link", "Another app › mailto: link", |s, v| {
            let link = p::req_str(v, "url")?;
            let m = mailcraft_model::mailto::parse(link).ok_or_else(|| CmdError::Params("not a mailto: link".into()))?;
            s.mailto_compose(&m)
        })
        .params(r#"{"url": "mailto:…" (RFC 6068: to, cc, bcc, subject and body; nothing is attached)}"#),
        CommandSpec::new("file.open", "Open a mail file", "Another app › Open with MailCraft", open_mail_file)
            .params(r#"{"path": string (.eml shows the message; .ics adds its events to the calendar, .vcf its people to the contacts)}"#),
    ]
}

/// What kind of file `path` holds: by its extension, else by its first line.
fn kind(path: &str, bytes: &[u8]) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "ics" | "ical" | "icalendar" | "ifb" => "calendar",
        "vcf" | "vcard" => "contacts",
        "eml" => "message",
        _ => sniff(bytes),
    }
}

/// A file with no telling extension: calendars and cards say what they are on their first line
/// (after a byte-order mark and blank space, if any); anything else is taken for a message.
fn sniff(bytes: &[u8]) -> &'static str {
    let opening = String::from_utf8_lossy(bytes.get(..64).unwrap_or(bytes));
    let opening = opening.trim_start_matches('\u{feff}').trim_start();
    let opens_with = |tag: &str| opening.get(..tag.len()).is_some_and(|h| h.eq_ignore_ascii_case(tag));
    if opens_with("BEGIN:VCALENDAR") {
        "calendar"
    } else if opens_with("BEGIN:VCARD") {
        "contacts"
    } else {
        "message"
    }
}

/// Open `path`: a message is answered with what to show (as `attachment.preview` answers); a
/// calendar's events and a contacts file's people are added to the account on screen.
fn open_mail_file(s: &mut Session, v: &Value) -> CmdResult {
    let given = p::req_str(v, "path")?;
    let path = given.trim();
    if path.is_empty() {
        return Err(CmdError::Params("`path` is required".into()));
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_string();
    // Most files are small: read once (up to the smaller cap), again only for a bigger message.
    let head = read_capped(path, MAX_PIM).ok();
    let kind = kind(path, head.as_deref().unwrap_or_default());
    match kind {
        "calendar" => {
            let bytes = match head {
                Some(b) => b,
                None => read_capped(path, MAX_PIM)?,
            };
            let cal = mailcraft_model::ical::parse(&String::from_utf8_lossy(&bytes), i64::from(s.clock.offset));
            let (mut added, mut skipped, mut first) = (0usize, 0usize, None);
            for e in cal.events {
                let start = e.start;
                match s.create_event(None, None, e) {
                    Ok(_) => {
                        added += 1;
                        first = first.or(Some(start));
                    }
                    Err(e @ CmdError::Disabled(_)) => return Err(e),
                    Err(_) => skipped += 1,
                }
            }
            // The calendar goes to the first one's day, to show it.
            if let Some(start) = first {
                s.cal_day = Some(start.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400));
            }
            Ok(json!({"kind": kind, "name": name, "added": added, "skipped": skipped, "start": first}))
        }
        "contacts" => {
            let bytes = match head {
                Some(b) => b,
                None => read_capped(path, MAX_PIM)?,
            };
            let (mut added, mut skipped) = (0usize, 0usize);
            for c in mailcraft_model::vcard::parse(&String::from_utf8_lossy(&bytes)) {
                match s.create_card(None, c) {
                    Ok(_) => added += 1,
                    Err(e @ CmdError::Disabled(_)) => return Err(e),
                    Err(_) => skipped += 1,
                }
            }
            Ok(json!({"kind": kind, "name": name, "added": added, "skipped": skipped}))
        }
        _ => {
            let bytes = match head {
                Some(b) => b,
                None => read_capped(path, MAX_MESSAGE)?,
            };
            let m = mailcraft_mime::parse(&bytes).ok_or_else(|| CmdError::Failed(format!("{name} can't be read as a message")))?;
            let attachments: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
            Ok(json!({
                "kind": "message", "name": name, "subject": m.subject, "from": m.from.to_string(),
                "to": mailcraft_model::addr::format_list(&m.to), "cc": mailcraft_model::addr::format_list(&m.cc),
                "date": m.date, "text": m.text, "attachments": attachments,
            }))
        }
    }
}
