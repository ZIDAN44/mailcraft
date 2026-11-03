//! Tasks in calendar files (`VTODO`, RFC 5545 §3.6.2): its title and notes, the days it starts
//! and is due, when it was done, its status and how far along it is, its priority, categories,
//! how it repeats and its reminder (its first alarm, as a time). What MailCraft doesn't read is
//! kept as written, its files (`ATTACH`) among it. Outlook's Waiting and Deferred have no word in
//! iCalendar: they're written as needing action, with an `X-` property that says which.

use super::*;
use crate::pim::{Task, TaskPriority, TaskStatus};

/// The most tasks read from one file, lines kept of one, and categories.
const MAX_TASKS: usize = 20_000;
const MAX_EXTRA: usize = 200;
const MAX_CATEGORIES: usize = 20;
/// Outlook's statuses iCalendar has no word for.
pub const TASK_STATUS: &str = "X-MAILCRAFT-TASK-STATUS";

/// A task being read, with what's settled once it's whole.
#[derive(Default)]
struct Reading<'a> {
    task: Task,
    /// Its first alarm's trigger.
    trigger: Option<&'a Line>,
    duration: Option<i64>,
    own_status: Option<TaskStatus>,
}

/// The day (UTC midnight) a `DTSTART` or `DUE` falls on: a date as it is, a time on the viewer's
/// day (or its zone's, where it has one).
fn day_of(zones: &FileZones, l: &Line) -> Option<i64> {
    let (t, all_day, _) = zones.time(l, "")?;
    if all_day {
        return Some(t);
    }
    Some(zones.local.local(t).div_euclid(86_400).saturating_mul(86_400))
}

/// The tasks in a calendar file's lines outside its events (those at its top level: one nested
/// in another component isn't).
pub(super) fn read_tasks(all: &[&Line], zones: &FileZones) -> Vec<Task> {
    let mut out = Vec::new();
    let mut cur: Option<Reading<'_>> = None;
    // Components open around the line: outside a task, those besides the calendar itself; in
    // one, those nested in it, whether the outermost of them is an alarm, and whether it's kept
    // (one that begins once the task's kept lines are full is left out whole, so what's kept
    // always ends what it begins).
    let mut outer = 0usize;
    let mut depth = 0usize;
    let mut in_alarm = false;
    let mut keep = false;
    for &l in all {
        let what = || l.value.trim().to_ascii_uppercase();
        match l.name.as_str() {
            "BEGIN" => {
                let Some(r) = cur.as_mut() else {
                    match what().as_str() {
                        "VTODO" if outer == 0 => {
                            cur = Some(Reading::default());
                            depth = 0;
                            in_alarm = false;
                            keep = false;
                        }
                        "VCALENDAR" => {}
                        _ => outer = outer.saturating_add(1),
                    }
                    continue;
                };
                depth = depth.saturating_add(1);
                if depth == 1 {
                    in_alarm = what() == "VALARM";
                    keep = !in_alarm && r.task.extra.len() < MAX_EXTRA;
                }
                if keep {
                    r.task.extra.push(l.raw.clone());
                }
                continue;
            }
            "END" => {
                let Some(r) = cur.as_mut() else {
                    if what() != "VCALENDAR" {
                        outer = outer.saturating_sub(1);
                    }
                    continue;
                };
                if depth > 0 {
                    depth -= 1;
                    if keep {
                        r.task.extra.push(l.raw.clone());
                    }
                    if depth == 0 {
                        in_alarm = false;
                        keep = false;
                    }
                    continue;
                }
                if what() == "VTODO"
                    && let Some(r) = cur.take()
                    && out.len() < MAX_TASKS
                {
                    out.push(finish(r, zones));
                }
                continue;
            }
            _ => {}
        }
        let Some(r) = cur.as_mut() else { continue };
        if depth > 0 {
            if in_alarm && l.name == "TRIGGER" && r.trigger.is_none() {
                r.trigger = Some(l);
            } else if keep && r.task.extra.len() < MAX_EXTRA {
                r.task.extra.push(l.raw.clone());
            }
            continue;
        }
        let t = &mut r.task;
        match l.name.as_str() {
            "UID" => t.uid = l.value.trim().to_string(),
            "SUMMARY" => t.title = unescape(&l.value),
            "DESCRIPTION" => t.notes = unescape(&l.value),
            "DTSTART" => t.start = day_of(zones, l),
            "DUE" => t.due = day_of(zones, l),
            "DURATION" => r.duration = duration(&l.value),
            "COMPLETED" => t.completed = zones.time(l, "").map(|(t, _, _)| t),
            "STATUS" => {
                t.status = match l.value.trim().to_ascii_uppercase().as_str() {
                    "IN-PROCESS" => TaskStatus::InProgress,
                    "COMPLETED" => TaskStatus::Completed,
                    "CANCELLED" => TaskStatus::Deferred,
                    _ => TaskStatus::NotStarted,
                }
            }
            TASK_STATUS => r.own_status = TaskStatus::from_code(&l.value),
            "PERCENT-COMPLETE" => t.percent = l.value.trim().parse::<i64>().map_or(0, |p| u8::try_from(p.clamp(0, 100)).unwrap_or(0)),
            "PRIORITY" => t.priority = TaskPriority::from_ical(l.value.trim().parse().unwrap_or(0)),
            "CATEGORIES" => {
                for c in split_text_list(&l.value) {
                    if !t.categories.contains(&c) && t.categories.len() < MAX_CATEGORIES {
                        t.categories.push(c);
                    }
                }
            }
            "RRULE" => t.rrule = Some(l.value.trim().to_string()).filter(|r| !r.is_empty()),
            "LAST-MODIFIED" => t.modified = zones.time(l, "").map_or(0, |(t, _, _)| t),
            "DTSTAMP" | "CREATED" | "SEQUENCE" => {}
            _ => {
                if t.extra.len() < MAX_EXTRA {
                    t.extra.push(l.raw.clone());
                }
            }
        }
    }
    out
}

/// A task read whole: its due day from its length when it says that instead, its own status,
/// its reminder, and status, percent and completion in step.
fn finish(r: Reading<'_>, zones: &FileZones) -> Task {
    let mut t = r.task;
    if t.due.is_none()
        && let (Some(start), Some(d)) = (t.start, r.duration)
    {
        t.due = Some(start.saturating_add(d.max(0)).div_euclid(86_400).saturating_mul(86_400));
    }
    if let Some(s) = r.own_status.filter(|s| t.status != TaskStatus::Completed && matches!(s, TaskStatus::Waiting | TaskStatus::Deferred)) {
        t.status = s;
    }
    t.reminder = r.trigger.and_then(|l| {
        if l.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME")) {
            return zones.time(l, "").map(|(t, _, _)| t);
        }
        let d = duration(&l.value)?;
        // Relative to its start (RFC 5545 §3.8.6.3), or its due day when it says so or has no
        // start; a day as its local midnight.
        let related_end = l.param("RELATED").is_some_and(|v| v.eq_ignore_ascii_case("END"));
        let base = if related_end { t.due.or(t.start) } else { t.start.or(t.due) }?;
        Some(zones.local.to_utc(base).saturating_add(d))
    });
    let at = t.completed.unwrap_or(t.modified);
    t.settle(at);
    t
}

/// One `VTODO`, folded; `now` is its `DTSTAMP`.
pub fn write_task(t: &Task, now: i64, out: &mut String) {
    let clean = |s: &str| s.replace(['\r', '\n'], "");
    let mut lines: Vec<String> = vec!["BEGIN:VTODO".into(), format!("UID:{}", clean(&t.uid)), format!("DTSTAMP:{}", stamp(now))];
    if t.modified > 0 {
        lines.push(format!("LAST-MODIFIED:{}", stamp(t.modified)));
    }
    if !t.title.is_empty() {
        lines.push(format!("SUMMARY:{}", escape(&t.title)));
    }
    if !t.notes.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&t.notes)));
    }
    // A rule needs a start to repeat from: the due day when there's none.
    let start = t.start.or_else(|| t.rrule.as_ref().and(t.due));
    if let Some(s) = start {
        lines.push(format!("DTSTART;VALUE=DATE:{}", day(s)));
    }
    // Due no earlier than it starts (RFC 5545 §3.8.2.3).
    if let Some(d) = t.due {
        lines.push(format!("DUE;VALUE=DATE:{}", day(start.map_or(d, |s| d.max(s)))));
    }
    if let Some(c) = t.completed.filter(|_| t.is_done()) {
        lines.push(format!("COMPLETED:{}", stamp(c)));
    }
    let status = match t.status {
        TaskStatus::Completed => "COMPLETED",
        TaskStatus::InProgress => "IN-PROCESS",
        TaskStatus::NotStarted | TaskStatus::Waiting | TaskStatus::Deferred if t.percent > 0 => "IN-PROCESS",
        TaskStatus::NotStarted | TaskStatus::Waiting | TaskStatus::Deferred => "NEEDS-ACTION",
    };
    lines.push(format!("STATUS:{status}"));
    if matches!(t.status, TaskStatus::Waiting | TaskStatus::Deferred) {
        lines.push(format!("{TASK_STATUS}:{}", t.status.code()));
    }
    if t.percent > 0 {
        lines.push(format!("PERCENT-COMPLETE:{}", t.percent.min(100)));
    }
    if let Some(p) = t.priority.to_ical() {
        lines.push(format!("PRIORITY:{p}"));
    }
    if !t.categories.is_empty() {
        lines.push(format!("CATEGORIES:{}", t.categories.iter().map(|c| escape(c)).collect::<Vec<_>>().join(",")));
    }
    if let Some(r) = &t.rrule {
        lines.push(format!("RRULE:{}", clean(r)));
    }
    lines.extend(whole(&t.extra, &["VEVENT"]));
    if let Some(at) = t.reminder {
        lines.extend([
            "BEGIN:VALARM".to_string(),
            "ACTION:DISPLAY".to_string(),
            "DESCRIPTION:Reminder".to_string(),
            format!("TRIGGER;VALUE=DATE-TIME:{}", stamp(at)),
            "END:VALARM".to_string(),
        ]);
    }
    lines.push("END:VTODO".into());
    for l in lines {
        fold(&l, out);
    }
}
