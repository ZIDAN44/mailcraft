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
    let (notes, mut rest) = split_rest(&body.replace("\r\n", "\n"), "VTODO");
    let said = rest.iter().position(|l| l.get(..6).is_some_and(|h| h.eq_ignore_ascii_case("RRULE:"))).map(|i| rest.remove(i));
    // The pattern counts from the task's first day: its start, else the day it's due.
    let rrule = match (said, x.task_recurrence.is_empty(), start.or(due)) {
        (Some(line), ..) => line.get(6..).map(|r| r.trim().to_string()).filter(|r| !r.is_empty()),
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

/// A repeating task's pattern (PidLidTaskRecurrence) as the import reads it, the task due on
/// `due` (Unix seconds): the task. For the fuzz target (`pst::repeating_task`).
#[doc(hidden)]
pub fn repeating_task(pattern: &[u8], due: i64) -> Task {
    let x = Extra { due: Some(due), task_recurrence: pattern.to_vec(), ..Default::default() };
    task_of(&x, "Task", "", 0, "u".into())
}
