//! A task (`IPM.Task`, MS-OXOTASK) as MailCraft keeps one: its title and notes, the days it
//! starts and is due, its status and how far along it is, when it was done, its priority (the
//! item's importance), categories and reminder, and how it repeats, its recurrence pattern
//! (PidLidTaskRecurrence, the same RecurrencePattern a repeating appointment's starts with) read
//! into an RFC 5545 rule. A task with no day at all comes in as one. What an export kept in its
//! text (`rest`: the to-do's properties MailCraft doesn't read, and a rule Outlook's pattern can't
//! say) comes back where it was.

use mailcraft_model::pim::{TASK_NOTES_KEPT, Task, TaskPriority, TaskStatus, task_day};

use super::*;

/// The task, `importance` its item's (-1 low, 0 normal, 1 high).
pub(crate) fn task_of(x: &Extra, subject: &str, body: &str, importance: i8, uid: String) -> Task {
    let start = x.task_start.map(task_day);
    let due = x.due.map(task_day);
    let (notes, rest) = split_rest(&body.replace("\r\n", "\n"), "VTODO");
    let (said, rest) = kept(&rest);
    // The pattern counts from the task's first day: its start, else the day it's due.
    let rrule = match (said, x.task_recurrence.is_empty(), start.or(due)) {
        (Some(rule), ..) => Some(rule),
        (None, false, Some(from)) => recurrence(&x.task_recurrence, from).map(|(rule, _)| rule),
        _ => None,
    };
    let mut status = x.task_status.and_then(TaskStatus::from_number).unwrap_or_default();
    if x.done {
        status = TaskStatus::Completed;
    }
    let mut t = Task {
        uid,
        title: subject.trim().to_string(),
        notes: notes.chars().take(TASK_NOTES_KEPT).collect(),
        start,
        due,
        completed: x.completed.filter(|_| status == TaskStatus::Completed),
        status,
        percent: x.percent.unwrap_or(0),
        priority: match importance {
            i8::MIN..=-1 => TaskPriority::Low,
            0 => TaskPriority::Normal,
            1..=i8::MAX => TaskPriority::High,
        },
        categories: x.categories.clone(),
        reminder: x.reminder_at,
        rrule,
        modified: x.modified.unwrap_or(0),
        extra: rest,
    };
    // A start after the day it's due isn't one MailCraft can keep: the due day stands.
    if let (Some(s), Some(d)) = (t.start, t.due)
        && s > d
    {
        t.start = Some(d);
    }
    let at = t.completed.unwrap_or(t.modified);
    t.settle(at);
    t
}

/// What a task's text kept (`lines`, a to-do's content lines) read as a to-do is: its rule, and
/// the lines MailCraft keeps as they are. Only those: a line that would say the task's uid, title
/// or days, or end the to-do and begin something else, isn't one the task takes, and those kept
/// are held to what a to-do keeps, each component they begin ended.
fn kept(lines: &[String]) -> (Option<String>, Vec<String>) {
    if lines.is_empty() {
        return (None, Vec::new());
    }
    let text = format!("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\n{}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n", lines.join("\r\n"));
    match mailcraft_model::ical::parse(&text, 0).tasks.into_iter().next() {
        Some(t) => (t.rrule, nested_well(t.extra)),
        None => (None, Vec::new()),
    }
}

/// `lines` when each component one begins is ended by its own name (`BEGIN:X` … `END:X`); else
/// only those outside any component, so an `END:VTODO` can't close the to-do early for a reader
/// that goes by names.
fn nested_well(lines: Vec<String>) -> Vec<String> {
    let edge = |l: &str, word: &str| {
        let (head, value) = l.split_once(':').unwrap_or((l, ""));
        let name = head.split(';').next().unwrap_or_default();
        name.trim().eq_ignore_ascii_case(word).then(|| value.trim().to_ascii_uppercase())
    };
    let mut open: Vec<String> = Vec::new();
    let mut well = true;
    for l in &lines {
        if let Some(name) = edge(l, "BEGIN") {
            open.push(name);
        } else if let Some(name) = edge(l, "END") {
            well &= open.pop().is_some_and(|o| o == name);
        }
    }
    if well && open.is_empty() {
        return lines;
    }
    let mut depth = 0usize;
    let mut out = Vec::new();
    for l in lines {
        if edge(&l, "BEGIN").is_some() {
            depth = depth.saturating_add(1);
        } else if edge(&l, "END").is_some() {
            depth = depth.saturating_sub(1);
        } else if depth == 0 {
            out.push(l);
        }
    }
    out
}

/// A repeating task's pattern (PidLidTaskRecurrence) as the import reads it, the task due on
/// `due` (Unix seconds): the task. For the fuzz target (`pst::repeating_task`).
#[doc(hidden)]
pub fn repeating_task(pattern: &[u8], due: i64) -> Task {
    let x = Extra { due: Some(due), task_recurrence: pattern.to_vec(), ..Default::default() };
    task_of(&x, "Task", "", 0, "u".into())
}
