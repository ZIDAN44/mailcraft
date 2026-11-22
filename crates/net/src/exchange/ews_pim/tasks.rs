//! Tasks in the mailbox's Tasks folder (issue #161), as Outlook keeps them: a task made here is
//! created there, one changed here updated field by field (it keeps its item, as Outlook knows
//! it), one deleted here deleted there; then the folder is read again.
//!
//! A task's start and due days are written at UTC midnight, as an all-day appointment's are,
//! and read back as the nearest midnight ([`task_day`]): Outlook keeps them at the user's local
//! midnight, half a day either way of it.

use mailcraft_model::pim::{StoredTask, Task, TaskPriority, TaskStatus, task_day};

use super::*;

/// Status as EWS names it, in [`TaskStatus`]'s order.
const STATUS: [&str; 5] = ["NotStarted", "InProgress", "Completed", "WaitingOnOthers", "Deferred"];

fn status_name(s: TaskStatus) -> &'static str {
    STATUS.get(TaskStatus::ALL.iter().position(|x| *x == s).unwrap_or(0)).copied().unwrap_or("NotStarted")
}

fn importance(p: TaskPriority) -> &'static str {
    match p {
        TaskPriority::Low => "Low",
        TaskPriority::Normal => "Normal",
        TaskPriority::High => "High",
    }
}

/// A task in Exchange's Tasks folder (`IPM.Task`; a task request, sent to someone, isn't one).
pub(super) fn task_of(it: &El) -> Option<StoredTask> {
    let class = it.text_of("ItemClass").unwrap_or("IPM.Task").trim();
    if it.name != "Task" || !class.starts_with("IPM.Task") || class.starts_with("IPM.TaskRequest") {
        return None;
    }
    let id = it.child("ItemId")?;
    let href = format!("task:{}", id.attr("Id")?);
    let time = |name: &str| it.text_of(name).and_then(mailcraft_jmap::utc_date);
    let status = it
        .text_of("Status")
        .and_then(|s| STATUS.iter().position(|x| x.eq_ignore_ascii_case(s.trim())))
        .and_then(|i| TaskStatus::ALL.get(i).copied())
        .unwrap_or_default();
    let percent = it.text_of("PercentComplete").and_then(|p| p.trim().parse::<f64>().ok()).map_or(0, |p| p.clamp(0.0, 100.0) as u8);
    let priority = match it.text_of("Importance").map(str::trim) {
        Some("High") => TaskPriority::High,
        Some("Low") => TaskPriority::Low,
        _ => TaskPriority::Normal,
    };
    let categories =
        it.child("Categories").map(|c| c.kids.iter().map(|k| k.text.trim().to_string()).filter(|k| !k.is_empty()).collect()).unwrap_or_default();
    let outlooks = it
        .children("ExtendedProperty")
        .find(|p| p.find("ExtendedFieldURI").and_then(|f| f.attr("PropertyId")) == Some("34050"))
        .and_then(|p| p.text_of("Value"))
        .and_then(mailcraft_jmap::utc_date);
    let reminder = if it.text_of("ReminderIsSet").is_some_and(|r| r.trim() == "true") { time("ReminderDueBy").or(outlooks) } else { None };
    let rrule = it.child("Recurrence").and_then(recurrence_json).and_then(|r| rrule_of(&r));
    let notes: String =
        it.text_of("Body").unwrap_or("").replace("\r\n", "\n").trim_end().chars().take(mailcraft_model::pim::TASK_NOTES_KEPT).collect();
    let mut task = Task {
        uid: href.clone(),
        title: text(it, "Subject"),
        notes,
        start: time("StartDate").map(task_day),
        due: time("DueDate").map(task_day),
        completed: time("CompleteDate"),
        status,
        percent,
        priority,
        categories,
        reminder,
        rrule,
        modified: time("LastModifiedTime").unwrap_or(0),
        extra: Vec::new(),
        source: None,
    };
    task.settle(task.completed.unwrap_or(task.modified));
    Some(StoredTask { task, href, etag: id.attr("ChangeKey").unwrap_or("").to_string(), ..Default::default() })
}

/// A task's reminder as Outlook keeps it (MS-OXORMDR: PidLidReminderTime and
/// PidLidReminderSignalTime). Exchange keeps `ReminderDueBy` in them; a server that doesn't
/// (Gromox) keeps the reminder this way, as Outlook writes it.
const REMINDER_TIME: &str =
    r#"<t:ExtendedFieldURI PropertySetId="00062008-0000-0000-C000-000000000046" PropertyId="34050" PropertyType="SystemTime"/>"#;
const REMINDER_SIGNAL: &str =
    r#"<t:ExtendedFieldURI PropertySetId="00062008-0000-0000-C000-000000000046" PropertyId="34144" PropertyType="SystemTime"/>"#;

/// What a task is read with besides its properties: those some servers leave out of all of
/// them (Gromox: its categories and reminder), and how it repeats.
pub(super) const TASK_FIELDS: &str = concat!(
    "<t:FieldURI FieldURI=\"item:Categories\"/><t:FieldURI FieldURI=\"item:ReminderIsSet\"/>",
    "<t:FieldURI FieldURI=\"item:ReminderDueBy\"/><t:FieldURI FieldURI=\"task:Recurrence\"/>",
    r#"<t:ExtendedFieldURI PropertySetId="00062008-0000-0000-C000-000000000046" PropertyId="34050" PropertyType="SystemTime"/>"#,
);

fn field(uri: &str) -> String {
    format!("<t:FieldURI FieldURI=\"{uri}\"/>")
}

/// A task's fields, in the order EWS wants them (the item's, its extended properties, then the
/// task's), each with its field path (as a `SetItemField` names it); those it hasn't are left
/// out ([`unset`] says which).
fn task_fields(t: &Task) -> Vec<(String, String)> {
    let mut f = vec![
        (field("item:Subject"), format!("<t:Subject>{}</t:Subject>", esc(&t.title))),
        (field("item:Body"), format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&t.notes))),
    ];
    if !t.categories.is_empty() {
        let list: String = t.categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
        f.push((field("item:Categories"), format!("<t:Categories>{list}</t:Categories>")));
    }
    f.push((field("item:Importance"), format!("<t:Importance>{}</t:Importance>", importance(t.priority))));
    if let Some(at) = t.reminder {
        f.push((field("item:ReminderDueBy"), format!("<t:ReminderDueBy>{}</t:ReminderDueBy>", utc_stamp(at))));
    }
    f.push((field("item:ReminderIsSet"), format!("<t:ReminderIsSet>{}</t:ReminderIsSet>", t.reminder.is_some())));
    if let Some(at) = t.reminder {
        for uri in [REMINDER_TIME, REMINDER_SIGNAL] {
            f.push((uri.to_string(), format!("<t:ExtendedProperty>{uri}<t:Value>{}</t:Value></t:ExtendedProperty>", utc_stamp(at))));
        }
    }
    if let Some(d) = t.due {
        f.push((field("task:DueDate"), format!("<t:DueDate>{}</t:DueDate>", utc_stamp(d))));
    }
    f.push((field("task:PercentComplete"), format!("<t:PercentComplete>{}</t:PercentComplete>", t.percent.min(100))));
    if let Some(r) = t.rrule.as_deref().and_then(|r| recurrence_xml(r, t.due.or(t.start).unwrap_or(t.modified))) {
        f.push((field("task:Recurrence"), r));
    }
    if let Some(d) = t.start {
        f.push((field("task:StartDate"), format!("<t:StartDate>{}</t:StartDate>", utc_stamp(d))));
    }
    f.push((field("task:Status"), format!("<t:Status>{}</t:Status>", status_name(t.status))));
    f
}

/// The fields a task hasn't, to clear on the server (as their field paths): a day or a reminder
/// taken off, categories all gone, no longer repeating.
fn unset(t: &Task) -> Vec<String> {
    let mut out = Vec::new();
    if t.categories.is_empty() {
        out.push(field("item:Categories"));
    }
    if t.reminder.is_none() {
        out.extend([field("item:ReminderDueBy"), REMINDER_TIME.to_string(), REMINDER_SIGNAL.to_string()]);
    }
    if t.due.is_none() {
        out.push(field("task:DueDate"));
    }
    if t.rrule.is_none() {
        out.push(field("task:Recurrence"));
    }
    if t.start.is_none() {
        out.push(field("task:StartDate"));
    }
    out
}

/// The tasks made or changed here: made in the Tasks folder, or changed field by field.
pub(super) fn push_tasks(c: &Client, pim: &Pim, out: &mut DavSync) {
    for s in pim.tasks.iter().filter(|s| s.dirty) {
        let fields = task_fields(&s.task);
        let r = match s.href.strip_prefix("task:") {
            None => {
                let body: String = fields.iter().map(|(_, x)| x.as_str()).collect();
                c.call_one(&format!(
                    "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"tasks\"/></m:SavedItemFolderId>\
                     <m:Items><t:Task>{body}</t:Task></m:Items></m:CreateItem>"
                ))
                .map(|m| id_and_key(m.find("ItemId")))
            }
            Some(id) => {
                let mut updates: String =
                    fields.iter().map(|(uri, x)| format!("<t:SetItemField>{uri}<t:Task>{x}</t:Task></t:SetItemField>")).collect();
                for uri in unset(&s.task) {
                    updates.push_str(&format!("<t:DeleteItemField>{uri}</t:DeleteItemField>"));
                }
                c.call_one(&format!(
                    "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges><t:ItemChange><t:ItemId Id=\"{}\"/>\
                     <t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
                    esc(id)
                ))
                .map(|m| {
                    let (new, key) = id_and_key(m.find("ItemId"));
                    (if new.is_empty() { id.to_string() } else { new }, key)
                })
            }
        };
        match r {
            Ok((id, etag)) => out.pushed_tasks.push(StoredTask { task: s.task.clone(), href: format!("task:{id}"), etag, ..Default::default() }),
            Err(x) => out.errors.push(format!("{}: {x}", s.task.title)),
        }
    }
}

/// The tasks, read again. A mailbox without a Tasks folder keeps its tasks on this computer.
pub(super) fn read_tasks(c: &Client, out: &mut DavSync) {
    match super::reading::items(c, "tasks") {
        Ok(read) => {
            read.say(out, "Tasks");
            out.tasks = Some(read.items.iter().filter_map(task_of).collect());
        }
        Err(e) => log::info!("no Tasks folder: {e}"),
    }
}
