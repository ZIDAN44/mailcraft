//! A task as an Outlook task (`IPM.Task`, MS-OXOTASK 2.2.2): its title and notes, status, how far
//! along it is, its start, due and done days, its priority (the item's importance), reminder and
//! categories, how it repeats (PidLidTaskRecurrence, a RecurrencePattern: [`super::pattern`]) and
//! its files. What Outlook has no place for (the to-do's properties MailCraft doesn't read, a rule
//! no pattern says the same as) stays in its text as a to-do of its own (`pst_pim::rest`), which
//! the import reads back.

use mailcraft_model::pim::{Task, TaskPriority, TaskStatus};

use super::contact::{param, parts};
use super::item::{PSETID_COMMON, PSETID_TASK, filetime};
use super::pim::{File, Item, Value, crlf, keywords, lid, reminder, tag};
use super::*;

/// The task as an item; `now` stands in for when it last changed when it doesn't say.
pub(super) fn item(t: &Task, now: i64) -> Item {
    let status = TaskStatus::ALL.iter().position(|s| *s == t.status).and_then(|i| i32::try_from(i).ok()).unwrap_or(0);
    let mut props = vec![
        lid(PSETID_TASK, 0x8101, Value::Int(status)),
        lid(PSETID_TASK, 0x8102, Value::Float(f64::from(t.percent.min(100)) / 100.0)),
        lid(PSETID_TASK, 0x811C, Value::Bool(t.is_done())),
        // PidLidTaskMode: a task of the user's own, not assigned.
        lid(PSETID_COMMON, 0x8518, Value::Int(0)),
        tag(
            0x0017,
            Value::Int(match t.priority {
                TaskPriority::Low => 0,
                TaskPriority::Normal => 1,
                TaskPriority::High => 2,
            }),
        ),
    ];
    // Its start and due days, also as PidLidCommonStart and PidLidCommonEnd, which Outlook sorts by.
    for (day, own, common) in [(t.start, 0x8104, 0x8516), (t.due, 0x8105, 0x8517)] {
        if let Some(day) = day {
            props.extend([lid(PSETID_TASK, own, Value::Time(filetime(day))), lid(PSETID_COMMON, common, Value::Time(filetime(day)))]);
        }
    }
    if let Some(done) = t.completed.filter(|_| t.is_done()) {
        props.push(lid(PSETID_TASK, 0x810F, Value::Time(filetime(done))));
    }
    if let Some(at) = t.reminder {
        props.extend(reminder(at));
    }
    props.extend(keywords(&t.categories));
    let mut rest: Vec<String> = Vec::new();
    if let Some(rule) = t.rrule.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        // The pattern counts from the task's first day, as the import reads it.
        let pattern = t.start.or(t.due).and_then(|from| super::pattern::write(rule, from).map(|p| (from, p)));
        match pattern {
            Some((from, p)) => {
                if !super::pattern::says(&p, from, rule) {
                    rest.push(format!("RRULE:{rule}"));
                }
                props.extend([lid(PSETID_TASK, 0x8116, Value::Bin(p)), lid(PSETID_TASK, 0x8126, Value::Bool(true))]);
            }
            None => rest.push(format!("RRULE:{rule}")),
        }
    }
    let mut files = Vec::new();
    for line in &t.extra {
        match file(line) {
            Some(f) => files.push(f),
            None => rest.push(line.clone()),
        }
    }
    Item {
        class: "IPM.Task",
        subject: t.title.clone(),
        body: crlf(&crate::pst_pim::with_rest(&t.notes, "VTODO", &rest)),
        modified: filetime(if t.modified > 0 { t.modified } else { now }),
        props,
        files,
    }
}

/// A file the task carries as its line holds it: `ATTACH` with its bytes inline (RFC 5545
/// 3.8.1.1, as the import writes one); `None` for a link or a line that isn't one.
pub(super) fn file(line: &str) -> Option<File> {
    let (name, params, value) = parts(line);
    let inline = param(params, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("BINARY"))
        && param(params, "ENCODING").is_some_and(|e| e.eq_ignore_ascii_case("BASE64"));
    if name != "ATTACH" || !inline {
        return None;
    }
    let data = mime::decode_base64(value)?;
    let filename = param(params, "X-FILENAME").filter(|n| !n.trim().is_empty()).unwrap_or("attachment").to_string();
    let mime_type = param(params, "FMTTYPE").map(str::to_string).unwrap_or_else(|| mime::mime_from_name(&filename).to_string());
    Some(File { name: filename, mime: mime_type, data, picture: false })
}
