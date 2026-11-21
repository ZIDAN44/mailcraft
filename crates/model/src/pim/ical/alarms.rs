//! Reminders: an event's alarms (`VALARM`) read as minutes before it starts, and set.

use std::ops::Range;

use super::*;

/// The longest reminder taken (four weeks before), as Outlook's own list goes.
pub const REMINDER_MAX_MINUTES: i64 = 4 * 7 * 24 * 60;

/// What a line kept as written does to the components around it.
enum Mark {
    /// Begins one; whether it's an alarm.
    Begin { alarm: bool },
    /// Ends the innermost one open (whatever it names, as reading takes it).
    End,
    /// Neither.
    Other,
}

fn mark(raw: &str) -> Mark {
    let s = raw.trim_start();
    let starts = |p: &str| s.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
    if !starts("BEGIN") && !starts("END") {
        return Mark::Other;
    }
    match parse_line(s) {
        Some(l) if l.name == "BEGIN" => Mark::Begin { alarm: l.value.trim().eq_ignore_ascii_case("VALARM") },
        Some(l) if l.name == "END" => Mark::End,
        _ => Mark::Other,
    }
}

/// An alarm among lines kept as written: its lines, `BEGIN` to `END`, and whether it ends (one
/// left open, in a damaged event, runs to the last line).
pub(super) struct Alarm {
    pub(super) lines: Range<usize>,
    pub(super) closed: bool,
}

/// The alarms of the component these lines are kept for (`extra`): its own, as reading takes them,
/// not one nested in another component it holds.
pub(super) fn alarms(extra: &[String]) -> impl Iterator<Item = Alarm> + '_ {
    let mut depth = 0usize;
    let mut from: Option<usize> = None;
    let mut lines = extra.iter().enumerate();
    std::iter::from_fn(move || {
        for (i, raw) in lines.by_ref() {
            match mark(raw) {
                Mark::Begin { alarm } => {
                    if depth == 0 && alarm {
                        from = Some(i);
                    }
                    depth = depth.saturating_add(1);
                }
                Mark::End => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && let Some(f) = from.take()
                    {
                        return Some(Alarm { lines: f..i.saturating_add(1), closed: true });
                    }
                }
                Mark::Other => {}
            }
        }
        from.take().map(|f| Alarm { lines: f..extra.len(), closed: false })
    })
}

/// The properties of the alarms in these lines, read (the components' own lines among them).
fn alarm_lines(extra: &[String]) -> impl Iterator<Item = Line> + '_ {
    alarms(extra).flat_map(move |a| extra.get(a.lines).unwrap_or_default()).filter_map(|raw| parse_line(raw.trim()))
}

/// An alarm that shows a reminder at `trigger` (a whole `TRIGGER` line).
pub(super) fn display_alarm(trigger: String) -> [String; 5] {
    ["BEGIN:VALARM".to_string(), "ACTION:DISPLAY".to_string(), "DESCRIPTION:Reminder".to_string(), trigger, "END:VALARM".to_string()]
}

/// An event's reminder, in minutes before it starts, from its first alarm (`VALARM`) with a
/// trigger relative to the start (`TRIGGER:-PT15M`, `TRIGGER;RELATED=START:-P1D`) or at a set
/// time (`TRIGGER;VALUE=DATE-TIME:…Z`). `None`: no reminder. A trigger after the start counts as
/// "at the start".
pub fn reminder_minutes(e: &CalEvent) -> Option<i64> {
    alarm_lines(&e.extra).filter(|l| l.name == "TRIGGER").find_map(|l| trigger_minutes(e, &l))
}

/// The most reminders an event's alarms give that are read (Google's calendar takes five).
pub const REMINDERS_MAX: usize = 10;

/// Every reminder an event's alarms give, in minutes before it starts, the earliest first, each
/// once (issue #224: only the first was read, so "an hour before and 2 minutes before" reminded
/// once). Read as [`reminder_minutes`] reads its first.
pub fn reminders_minutes(e: &CalEvent) -> Vec<i64> {
    let mut out: Vec<i64> = alarms(&e.extra)
        .filter_map(|a| {
            let lines = e.extra.get(a.lines)?;
            lines.iter().filter_map(|raw| parse_line(raw.trim())).filter(|l| l.name == "TRIGGER").find_map(|l| trigger_minutes(e, &l))
        })
        .collect();
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
    out.truncate(REMINDERS_MAX);
    out
}

/// An alarm's `TRIGGER` as minutes before the event starts: relative to the start, or at a set
/// time; `None` for one relative to the end, or one that can't be read.
fn trigger_minutes(e: &CalEvent, l: &Line) -> Option<i64> {
    let is = |p: &str, v: &str| l.param(p).is_some_and(|x| x.trim().eq_ignore_ascii_case(v));
    if is("RELATED", "END") {
        return None;
    }
    let before = if is("VALUE", "DATE-TIME") {
        let (t, _, _) = date_time(&l.value)?;
        e.start.saturating_sub(t)
    } else {
        duration(&l.value)?.saturating_neg()
    };
    Some((before / 60).clamp(0, REMINDER_MAX_MINUTES))
}

/// Give an event a display alarm for each of `minutes` before it starts (none: no reminder), in
/// place of the alarms it had; left as it is when it reminds at those times already (written
/// again, its alarms would move in its lines, and an event saved unchanged was a change).
pub fn set_reminders(e: &mut CalEvent, minutes: &[i64]) {
    let mut wanted: Vec<i64> = minutes.iter().map(|m| (*m).clamp(0, REMINDER_MAX_MINUTES)).collect();
    wanted.sort_unstable_by(|a, b| b.cmp(a));
    wanted.dedup();
    wanted.truncate(REMINDERS_MAX);
    if reminders_minutes(e) == wanted {
        return;
    }
    set_reminder(e, None);
    for m in wanted {
        e.extra.extend(display_alarm(format!("TRIGGER:-PT{m}M")));
    }
}

/// Give an event one display alarm `minutes` before it starts, in place of any it had, or none
/// (`None`).
pub fn set_reminder(e: &mut CalEvent, minutes: Option<i64>) {
    // Every alarm goes, nested lines included.
    let gone: Vec<Range<usize>> = alarms(&e.extra).map(|a| a.lines).collect();
    let mut next = gone.iter().peekable();
    let mut i = 0usize;
    e.extra.retain(|_| {
        while next.next_if(|r| r.end <= i).is_some() {}
        let keep = !next.peek().is_some_and(|r| r.contains(&i));
        i = i.saturating_add(1);
        keep
    });
    if let Some(m) = minutes {
        let m = m.clamp(0, REMINDER_MAX_MINUTES);
        e.extra.extend(display_alarm(format!("TRIGGER:-PT{m}M")));
    }
}

/// When the event's alarms were last answered (dismissed) by a calendar program: the latest
/// `ACKNOWLEDGED` of its alarms (RFC 9074 §6). An occurrence whose reminder came at or before it
/// has been dealt with.
pub fn acknowledged(e: &CalEvent) -> Option<i64> {
    alarm_lines(&e.extra)
        .filter(|l| l.name == "ACKNOWLEDGED")
        .filter_map(|l| match date_time(&l.value) {
            Some((t, false, true)) => Some(t),
            _ => None,
        })
        .max()
}

/// Mark the event's alarms answered at `at` (UTC seconds): each gets `ACKNOWLEDGED`, in place of
/// the one it had (RFC 9074 §6), so every calendar program reading it knows the reminder was
/// dismissed. Snooze alarms (`RELATED-TO;RELTYPE=SNOOZE`) go: they were answered too.
pub fn acknowledge(e: &mut CalEvent, at: i64) {
    let stamp = format!("ACKNOWLEDGED:{}", stamp(at.max(0)));
    let found: Vec<Alarm> = alarms(&e.extra).collect();
    let mut out = Vec::with_capacity(e.extra.len().saturating_add(found.len()));
    let mut lines = std::mem::take(&mut e.extra).into_iter();
    let mut placed = 0usize;
    for a in found {
        out.extend(lines.by_ref().take(a.lines.start.saturating_sub(placed)));
        placed = a.lines.end;
        let mut alarm: Vec<String> = lines.by_ref().take(a.lines.len()).collect();
        // An alarm left open (a damaged event) is kept as it was.
        if !a.closed {
            out.append(&mut alarm);
            continue;
        }
        let read: Vec<Option<Line>> = alarm.iter().map(|raw| parse_line(raw.trim())).collect();
        let snooze =
            read.iter().flatten().any(|l| l.name == "RELATED-TO" && l.param("RELTYPE").is_some_and(|v| v.trim().eq_ignore_ascii_case("SNOOZE")));
        if snooze {
            continue;
        }
        let end = alarm.pop();
        out.extend(alarm.into_iter().zip(read).filter(|(_, l)| !l.as_ref().is_some_and(|l| l.name == "ACKNOWLEDGED")).map(|(raw, _)| raw));
        out.push(stamp.clone());
        out.extend(end);
    }
    out.extend(lines);
    e.extra = out;
}
