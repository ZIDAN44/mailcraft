//! Reminders: an event's alarms (`VALARM`) read as minutes before it starts, and set.

use super::*;

/// The longest reminder taken (four weeks before), as Outlook's own list goes.
pub const REMINDER_MAX_MINUTES: i64 = 4 * 7 * 24 * 60;

/// An event's reminder, in minutes before it starts, from its first alarm (`VALARM`) with a
/// trigger relative to the start (`TRIGGER:-PT15M`, `TRIGGER;RELATED=START:-P1D`) or at a set
/// time (`TRIGGER;VALUE=DATE-TIME:…Z`). `None`: no reminder. A trigger after the start counts as
/// "at the start".
pub fn reminder_minutes(e: &CalEvent) -> Option<i64> {
    let mut in_alarm = false;
    for raw in e.extra.iter().take(400) {
        let line = raw.trim();
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            in_alarm = true;
            continue;
        }
        if upper == "END:VALARM" {
            in_alarm = false;
            continue;
        }
        if !in_alarm || !upper.starts_with("TRIGGER") {
            continue;
        }
        let Some((params, value)) = line.split_once(':') else { continue };
        let params = params.to_ascii_uppercase();
        if params.contains("RELATED=END") {
            continue;
        }
        // One that can't be read is passed over for the next.
        let before = if params.contains("VALUE=DATE-TIME") {
            let Some((t, _, _)) = date_time(value) else { continue };
            e.start.saturating_sub(t)
        } else {
            let Some(d) = duration(value) else { continue };
            d.saturating_neg()
        };
        return Some((before / 60).clamp(0, REMINDER_MAX_MINUTES));
    }
    None
}

/// Give an event one display alarm `minutes` before it starts, in place of any it had, or none
/// (`None`).
pub fn set_reminder(e: &mut CalEvent, minutes: Option<i64>) {
    // Every alarm goes, nested lines included.
    let mut kept = Vec::with_capacity(e.extra.len());
    let mut depth = 0usize;
    for raw in std::mem::take(&mut e.extra) {
        let upper = raw.trim().to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            depth += 1;
            continue;
        }
        if depth > 0 {
            if upper == "END:VALARM" {
                depth -= 1;
            }
            continue;
        }
        kept.push(raw);
    }
    e.extra = kept;
    if let Some(m) = minutes {
        let m = m.clamp(0, REMINDER_MAX_MINUTES);
        e.extra.extend([
            "BEGIN:VALARM".to_string(),
            "ACTION:DISPLAY".to_string(),
            "DESCRIPTION:Reminder".to_string(),
            format!("TRIGGER:-PT{m}M"),
            "END:VALARM".to_string(),
        ]);
    }
}
