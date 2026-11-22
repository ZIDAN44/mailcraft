//! Tasks in calendar files (`VTODO`, RFC 5545 §3.6.2): its title and notes, the days it starts
//! and is due, when it was done, its status and how far along it is, its priority, categories,
//! how it repeats and its reminder (its first alarm, as a time). What MailCraft doesn't read is
//! kept as written, its files (`ATTACH`) among it. Outlook's Waiting and Deferred have no word in
//! iCalendar: they're written as needing action, with an `X-` property that says which.

use super::*;
use crate::pim::{Task, TaskPriority, TaskSource, TaskStatus};

/// The most lines of a to-do's alarms kept as written.
const ALARM_LINES_MAX: usize = 200;

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
    /// What it says that the task keeps less of ([`TaskSource`]): `DTSTART` and `DUE` with a
    /// time, its `STATUS` line, its alarms as written.
    start_at: Option<i64>,
    due_at: Option<i64>,
    status_line: Option<String>,
    alarms: Vec<String>,
}

/// The time a `DTSTART` or `DUE` with a time of day is at (UTC); `None` for a date.
fn time_of(zones: &FileZones, l: &Line) -> Option<i64> {
    zones.time(l, "").filter(|(_, all_day, _)| !all_day).map(|(t, _, _)| t)
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
    // always ends what it begins), and the depth of one nested in a kept one and left out so.
    let mut outer = 0usize;
    let mut depth = 0usize;
    let mut in_alarm = false;
    let mut keep = false;
    let mut dropped_from: Option<usize> = None;
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
                            dropped_from = None;
                        }
                        "VCALENDAR" => {}
                        _ => outer = outer.saturating_add(1),
                    }
                    continue;
                };
                depth = depth.saturating_add(1);
                if (in_alarm || (depth == 1 && what() == "VALARM")) && r.alarms.len() < ALARM_LINES_MAX {
                    r.alarms.push(l.raw.clone());
                }
                if depth == 1 {
                    in_alarm = what() == "VALARM";
                    keep = !in_alarm && r.task.extra.len() < EXTRA_MAX;
                    dropped_from = None;
                } else if keep && dropped_from.is_none() && r.task.extra.len() >= EXTRA_MAX {
                    dropped_from = Some(depth);
                }
                if keep && dropped_from.is_none() {
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
                    if in_alarm && r.alarms.len() < ALARM_LINES_MAX {
                        r.alarms.push(l.raw.clone());
                    }
                    match dropped_from {
                        None if keep => r.task.extra.push(l.raw.clone()),
                        Some(d) if d == depth => dropped_from = None,
                        _ => {}
                    }
                    depth -= 1;
                    if depth == 0 {
                        in_alarm = false;
                        keep = false;
                    }
                    continue;
                }
                if what() == "VTODO"
                    && let Some(r) = cur.take()
                    && out.len() < MAX_ITEMS
                {
                    out.push(finish(r, zones));
                }
                continue;
            }
            _ => {}
        }
        let Some(r) = cur.as_mut() else { continue };
        if depth > 0 {
            if in_alarm && r.alarms.len() < ALARM_LINES_MAX {
                r.alarms.push(l.raw.clone());
            }
            if in_alarm && l.name == "TRIGGER" && r.trigger.is_none() {
                r.trigger = Some(l);
            } else if keep && dropped_from.is_none() && r.task.extra.len() < EXTRA_MAX {
                r.task.extra.push(l.raw.clone());
            }
            continue;
        }
        let t = &mut r.task;
        match l.name.as_str() {
            "UID" => t.uid = stored_uid(&l.value),
            "SUMMARY" => t.title = unescape(&l.value),
            "DESCRIPTION" => t.notes = unescape(&l.value),
            "DTSTART" => {
                t.start = day_of(zones, l);
                r.start_at = time_of(zones, l);
            }
            "DUE" => {
                t.due = day_of(zones, l);
                r.due_at = time_of(zones, l);
            }
            "DURATION" => r.duration = duration(&l.value),
            "COMPLETED" => t.completed = zones.time(l, "").map(|(t, _, _)| t),
            "STATUS" => {
                r.status_line = Some(l.raw.clone());
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
            "CATEGORIES" => add_categories(&mut t.categories, &l.value),
            "RRULE" => t.rrule = Some(l.value.trim().to_string()).filter(|r| !r.is_empty()),
            "LAST-MODIFIED" => t.modified = zones.time(l, "").map_or(0, |(t, _, _)| t),
            // Written new each time.
            "DTSTAMP" => {}
            _ => {
                if t.extra.len() < EXTRA_MAX {
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
    // What it says beyond what the task keeps, with what was read of it.
    let source = TaskSource {
        start: t.start.zip(r.start_at),
        due: t.due.zip(r.due_at),
        // Only one MailCraft wouldn't write as it was (`CANCELLED`, needing action yet begun).
        status: r
            .status_line
            .filter(|line| line.split_once(':').is_none_or(|(_, v)| !v.trim().eq_ignore_ascii_case(status_word(&t))))
            .map(|line| (t.status, line)),
        // Alarms cut off at the cap aren't kept: what's written back is whole.
        reminder: if r.alarms.is_empty() || r.alarms.len() >= ALARM_LINES_MAX { None } else { t.reminder },
        alarms: if r.alarms.len() >= ALARM_LINES_MAX { Vec::new() } else { r.alarms },
    };
    if source != TaskSource::default() {
        t.source = Some(Box::new(source));
    }
    t
}

/// The `STATUS` a task is written with.
fn status_word(t: &Task) -> &'static str {
    match t.status {
        TaskStatus::Completed => "COMPLETED",
        TaskStatus::InProgress => "IN-PROCESS",
        TaskStatus::NotStarted | TaskStatus::Waiting | TaskStatus::Deferred if t.percent > 0 => "IN-PROCESS",
        TaskStatus::NotStarted | TaskStatus::Waiting | TaskStatus::Deferred => "NEEDS-ACTION",
    }
}

/// One `VTODO`, folded; `now` is its `DTSTAMP`.
pub(super) fn write_task(t: &Task, now: i64, out: &mut String) {
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
    let src = t.source.as_deref();
    // A day as read with its time of day (unchanged since): at that time again.
    let as_read = |kept: Option<(i64, i64)>, day_now: i64| kept.filter(|(d, _)| *d == day_now).map(|(_, at)| stamp(at));
    // A rule needs a start to repeat from: the due day when there's none.
    let start = t.start.or_else(|| t.rrule.as_ref().and(t.due));
    if let Some(s) = start {
        match as_read(src.and_then(|x| x.start), s) {
            Some(at) => lines.push(format!("DTSTART:{at}")),
            None => lines.push(format!("DTSTART;VALUE=DATE:{}", day(s))),
        }
    }
    // Due no earlier than it starts (RFC 5545 §3.8.2.3).
    if let Some(d) = t.due {
        let d = start.map_or(d, |s| d.max(s));
        match as_read(src.and_then(|x| x.due), d) {
            Some(at) => lines.push(format!("DUE:{at}")),
            None => lines.push(format!("DUE;VALUE=DATE:{}", day(d))),
        }
    }
    if let Some(c) = t.completed.filter(|_| t.is_done()) {
        lines.push(format!("COMPLETED:{}", stamp(c)));
    }
    let status = status_word(t);
    // The status as written, while it's the one read (`CANCELLED` is Deferred here).
    match src.and_then(|x| x.status.as_ref()).filter(|(s, _)| *s == t.status) {
        Some((_, line)) => lines.push(clean(line)),
        None => lines.push(format!("STATUS:{status}")),
    }
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
    // Its alarms as written, while the reminder is the one read from them.
    match src.filter(|x| !x.alarms.is_empty() && x.reminder == t.reminder) {
        Some(x) => lines.extend(whole(&x.alarms, &["VEVENT", "VTODO"])),
        None => {
            if let Some(at) = t.reminder {
                lines.extend(display_alarm(format!("TRIGGER;VALUE=DATE-TIME:{}", stamp(at))));
            }
        }
    }
    lines.push("END:VTODO".into());
    for l in lines {
        fold(&l, out);
    }
}
