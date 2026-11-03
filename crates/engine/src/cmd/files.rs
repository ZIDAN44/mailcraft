//! Commands: what the system hands MailCraft as its mail app (`mailto:` links, and `.eml`,
//! `.ics` and `.vcf` files opened with it).

use mailcraft_model::Job;

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
            .params(r#"{"path": string (.eml and .msg show the message; .ics adds its events to the calendar, .vcf its people to the contacts)}"#),
        CommandSpec::new("file.addToFolder", "Add message files to a folder", "Folder pane (files dropped on a folder)", add_to_folder)
            .params(r#"{"paths": [string] (.eml and Outlook .msg files), "folder"?: string (the folder open), "account"?: string}"#),
    ]
}

/// Whether a file is a message file by its name: what a folder takes when it's dropped there.
pub fn is_message_file(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "eml" | "msg")
}

/// Message files put into a folder (issue #38): the open one unless another is named. The files
/// are read and filed by the network layer ([`Job::FileMessages`]); the folder shows them once
/// it's read again, and [`Session::filed`] says how many went in.
fn add_to_folder(s: &mut Session, v: &Value) -> CmdResult {
    let paths: Vec<String> = v
        .get("paths")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).take(500).collect())
        .unwrap_or_default();
    if paths.is_empty() {
        return Err(CmdError::Params("`paths` names no files".into()));
    }
    let (account, folder) = match (p::str(v, "account"), p::str(v, "folder")) {
        (a, Some(f)) => (a.map(str::to_string).or_else(|| s.current_account().map(|x| x.config.id.clone())), f.to_string()),
        (None, None) => match &s.view {
            View::Folder { account, folder } => (Some(account.clone()), folder.clone()),
            _ => return Err(CmdError::Params("open a folder to add them to".into())),
        },
        (Some(_), None) => return Err(CmdError::Params("`folder` is required with `account`".into())),
    };
    let account = account.ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
    let Some(a) = s.account(&account) else { return Err(CmdError::Params(format!("no account `{account}`"))) };
    if !a.folders.iter().any(|f| f.path == folder && f.selectable) {
        return Err(CmdError::Params(format!("no folder `{folder}` to add them to")));
    }
    let n = paths.len();
    s.jobs.push(Job::FileMessages { account: account.clone(), folder: folder.clone(), paths });
    Ok(json!({"account": account, "folder": folder, "files": n}))
}

/// What kind of file `path` holds: by its extension, else by its first line.
fn kind(path: &str, bytes: &[u8]) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "ics" | "ical" | "icalendar" | "ifb" => "calendar",
        "vcf" | "vcard" => "contacts",
        "eml" => "message",
        "msg" => "outlook",
        _ if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) => "outlook",
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
/// calendar's events and tasks and a contacts file's people are added to the account on screen.
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
            // Its to-dos go to the tasks (in place of those with their uids, as events do).
            let mut tasks = 0usize;
            for t in cal.tasks {
                match s.open_task(None, t) {
                    Ok(_) => tasks += 1,
                    Err(e @ CmdError::Disabled(_)) => return Err(e),
                    Err(_) => skipped += 1,
                }
            }
            // The calendar goes to the first one's day, to show it.
            if let Some(start) = first {
                s.cal_day = Some(start.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400));
            }
            Ok(json!({"kind": kind, "name": name, "added": added, "tasks": tasks, "skipped": skipped, "start": first}))
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
        // An Outlook message file is made a standard message by the network layer, and shown
        // when it comes ([`Session::message_file_read`]).
        "outlook" => {
            s.jobs.push(Job::ReadMessageFile { path: path.to_string() });
            Ok(json!({"kind": "reading", "name": name}))
        }
        _ => {
            let bytes = match head {
                Some(b) => b,
                None => read_capped(path, MAX_MESSAGE)?,
            };
            message_preview(s, &name, &bytes)
        }
    }
}

/// What to show of a message file: as `attachment.preview` answers.
pub(crate) fn message_preview(s: &mut Session, name: &str, bytes: &[u8]) -> CmdResult {
    let m = mailcraft_mime::parse(bytes).ok_or_else(|| CmdError::Failed(format!("{name} can't be read as a message")))?;
    let attachments: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
    let warning = s.sender_warning(&m.from);
    Ok(json!({
        "kind": "message", "name": name, "subject": m.subject, "from": m.from.to_string(),
        "to": mailcraft_model::addr::format_list(&m.to), "cc": mailcraft_model::addr::format_list(&m.cc),
        "date": m.date, "text": m.text, "attachments": attachments, "warning": warning,
    }))
}
