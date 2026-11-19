//! A message's flags as Graph keeps them: read, flagged or complete (one status for both),
//! a flag's days, and its reminder (gap 9). Graph's flag has no reminder: Outlook keeps it on
//! the message (`PidLidReminderSet`, `PidLidReminderSignalTime`, `PidLidReminderTime`), which
//! Graph reads and writes as single-value extended properties.

use mailcraft_model::{Flag, Flags, Op};
use serde_json::{Value, json};

/// The reminder's properties, as Graph names them (PSETID_Common).
const REMINDER_SET: &str = "Boolean {00062008-0000-0000-C000-000000000046} Id 0x8503";
const REMINDER_SIGNAL: &str = "SystemTime {00062008-0000-0000-C000-000000000046} Id 0x8560";
const REMINDER_TIME: &str = "SystemTime {00062008-0000-0000-C000-000000000046} Id 0x8502";

/// The `$expand` that has a message come with its reminder's properties.
pub(super) fn reminder_expand() -> String {
    let ask = [REMINDER_SET, REMINDER_SIGNAL, REMINDER_TIME].map(|id| format!("id eq '{id}'")).join(" or ");
    format!("$expand=singleValueExtendedProperties($filter={})", crate::http::pct(&ask))
}

/// A single-value extended property's value, by its id (as Graph names it back: any case).
fn extended<'a>(m: &'a Value, id: &str) -> Option<&'a str> {
    m.get("singleValueExtendedProperties")?
        .as_array()?
        .iter()
        .take(50)
        .find(|p| p.get("id").and_then(Value::as_str).is_some_and(|i| i.split_whitespace().eq(id.split_whitespace()) || i.eq_ignore_ascii_case(id)))
        .and_then(|p| p.get("value"))
        .and_then(Value::as_str)
}

/// A time Graph gives (`2026-11-20T09:00:00Z`, with fractions of a second or none, in UTC).
fn utc(t: &str) -> Option<i64> {
    let t = t.trim();
    mailcraft_jmap::utc_date(&format!("{}Z", t.get(..19).unwrap_or(t)))
}

/// A message's flags as the model knows them.
pub(super) fn flags(m: &Value) -> Flags {
    let mut names: Vec<&str> = Vec::new();
    if m.get("isRead").and_then(Value::as_bool) == Some(true) {
        names.push("\\Seen");
    }
    if m.get("isDraft").and_then(Value::as_bool) == Some(true) {
        names.push("\\Draft");
    }
    match m.get("flag").and_then(|f| f.get("flagStatus")).and_then(Value::as_str) {
        Some("flagged") => names.push("\\Flagged"),
        // Marked complete: Graph keeps one status for both.
        Some("complete") => names.push(mailcraft_model::FOLLOW_UP_COMPLETE),
        _ => {}
    }
    let mut f = Flags::from_imap(names);
    // The day it's due (gap 9): an instant in the zone it names, read as the nearest midnight
    // (Outlook keeps the user's local midnight; Graph says it in UTC unless asked otherwise).
    f.follow_up_due = m
        .get("flag")
        .and_then(|f| f.get("dueDateTime"))
        .filter(|d| d.get("timeZone").and_then(Value::as_str).is_none_or(|z| z.eq_ignore_ascii_case("UTC")))
        .and_then(|d| d.get("dateTime"))
        .and_then(Value::as_str)
        .and_then(utc)
        .map(|t| mailcraft_model::pim::task_day(t).div_euclid(86_400));
    // Its reminder, when set: when it reminds (a snooze moves that), else its time.
    if extended(m, REMINDER_SET).is_some_and(|v| v.trim().eq_ignore_ascii_case("true")) {
        f.reminder = extended(m, REMINDER_SIGNAL).and_then(utc).or_else(|| extended(m, REMINDER_TIME).and_then(utc));
    }
    f
}

/// The change a flag or follow-up op makes to a message on the server (`None`: nothing Graph
/// keeps). Graph's one `flagStatus` holds both the flag and the follow-up marked complete: the
/// engine sends the keyword taken off before the flag is set, and put on after it.
pub(super) fn flag_patch(op: &Op) -> Option<Value> {
    match op {
        Op::SetFlag { flag: Flag::Seen, on, .. } => Some(json!({"isRead": on})),
        Op::SetFlag { flag: Flag::Flagged, on, .. } => Some(json!({"flag": {"flagStatus": if *on { "flagged" } else { "notFlagged" }}})),
        Op::SetKeyword { keyword, on, .. } if keyword.eq_ignore_ascii_case(mailcraft_model::FOLLOW_UP_COMPLETE) => {
            Some(json!({"flag": {"flagStatus": if *on { "complete" } else { "notFlagged" }}}))
        }
        // A flagged message's days (gap 9), as the instants they start where the user is: sent
        // only for a flagged one (Graph flags one given days).
        Op::SetFollowUp { due, .. } => {
            let at = |d: &mailcraft_model::FollowUpDay| json!({"dateTime": mailcraft_model::dates::utc_date_time(d.at), "timeZone": "UTC"});
            let day = due.as_ref().map(at).unwrap_or(Value::Null);
            Some(json!({"flag": {"flagStatus": "flagged", "startDateTime": day, "dueDateTime": day}}))
        }
        // A flag's reminder (gap 9): set, at both times (Outlook shows the one, reminds at the
        // other); or no longer set (Graph can't take a property off).
        Op::SetReminder { at: Some(at), .. } => {
            let at = mailcraft_model::dates::utc_stamp(*at);
            Some(json!({"singleValueExtendedProperties": [
                {"id": REMINDER_SET, "value": "true"},
                {"id": REMINDER_TIME, "value": at},
                {"id": REMINDER_SIGNAL, "value": at},
            ]}))
        }
        Op::SetReminder { at: None, .. } => Some(json!({"singleValueExtendedProperties": [{"id": REMINDER_SET, "value": "false"}]})),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_model::MessageKey;

    /// Mark Complete, its Undo and a flag set again reach Graph's one flag status in the order
    /// the engine sends them; other keywords (categories) stay on this computer.
    #[test]
    fn follow_up_changes_as_graph_keeps_them() {
        let keys = vec![MessageKey::default()];
        let status = |op: Op| flag_patch(&op).and_then(|p| p["flag"]["flagStatus"].as_str().map(str::to_string));
        let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on };
        let complete = |on| Op::SetKeyword { keys: keys.clone(), keyword: mailcraft_model::FOLLOW_UP_COMPLETE.into(), on };
        // Mark Complete: the flag off, then complete.
        assert_eq!([status(flag(false)), status(complete(true))], [Some("notFlagged".into()), Some("complete".into())]);
        // Its Undo, or flagged again: the keyword off, then the flag on.
        assert_eq!([status(complete(false)), status(flag(true))], [Some("notFlagged".into()), Some("flagged".into())]);
        assert_eq!(flag_patch(&Op::SetKeyword { keys: keys.clone(), keyword: "work".into(), on: true }), None);
        assert_eq!(flag_patch(&Op::SetFlag { keys: keys.clone(), flag: Flag::Seen, on: true }), Some(json!({"isRead": true})));
        // A day (gap 9): the instants it starts where the user is, in UTC; none, taken off.
        let day = mailcraft_model::dates::days_from_civil(2026, 11, 25);
        let due = mailcraft_model::FollowUpDay { day, at: day * 86_400 - 3600 };
        let at = json!({"dateTime": "2026-11-24T23:00:00", "timeZone": "UTC"});
        assert_eq!(
            flag_patch(&Op::SetFollowUp { keys: keys.clone(), due: Some(due) }),
            Some(json!({"flag": {"flagStatus": "flagged", "startDateTime": at, "dueDateTime": at}}))
        );
        assert_eq!(
            flag_patch(&Op::SetFollowUp { keys, due: None }),
            Some(json!({"flag": {"flagStatus": "flagged", "startDateTime": null, "dueDateTime": null}}))
        );
    }

    /// Gap 9: a flag's due day, as Graph gives it (the instant the day starts where it was set,
    /// in UTC), is that day; one in another zone isn't guessed at.
    #[test]
    fn a_flags_due_day_is_read() {
        let due = |d: Value| flags(&json!({"flag": {"flagStatus": "flagged", "dueDateTime": d}})).follow_up_due;
        let day = mailcraft_model::dates::days_from_civil(2026, 11, 20);
        assert_eq!(due(json!({"dateTime": "2026-11-19T18:00:00.0000000", "timeZone": "UTC"})), Some(day), "set six hours east");
        assert_eq!(due(json!({"dateTime": "2026-11-20T05:00:00.0000000", "timeZone": "UTC"})), Some(day), "five hours west");
        assert_eq!(due(json!({"dateTime": "2026-11-20T00:00:00", "timeZone": "Pacific Standard Time"})), None);
        assert_eq!(due(json!("nonsense")), None);
        assert_eq!(flags(&json!({"flag": {"flagStatus": "flagged"}})).follow_up_due, None);
    }

    /// Gap 9: a flag's reminder goes to Graph as Outlook keeps it, and is read back so: when
    /// it's set, at its signal time (or its time); not set, none.
    #[test]
    fn a_flags_reminder_goes_both_ways() {
        let keys = vec![MessageKey::default()];
        let at = mailcraft_model::dates::days_from_civil(2026, 11, 20) * 86_400 + 9 * 3600;
        let sent = flag_patch(&Op::SetReminder { keys: keys.clone(), at: Some(at) }).unwrap();
        assert_eq!(
            sent,
            json!({"singleValueExtendedProperties": [
                {"id": "Boolean {00062008-0000-0000-C000-000000000046} Id 0x8503", "value": "true"},
                {"id": "SystemTime {00062008-0000-0000-C000-000000000046} Id 0x8502", "value": "2026-11-20T09:00:00Z"},
                {"id": "SystemTime {00062008-0000-0000-C000-000000000046} Id 0x8560", "value": "2026-11-20T09:00:00Z"},
            ]})
        );
        let off = flag_patch(&Op::SetReminder { keys, at: None }).unwrap();
        assert_eq!(off["singleValueExtendedProperties"][0]["value"], "false");
        // As Graph answers: its ids in its own case, times with fractions.
        let read = |props: Value| flags(&json!({"flag": {"flagStatus": "flagged"}, "singleValueExtendedProperties": props})).reminder;
        let set = |v: &str| json!({"id": "Boolean {00062008-0000-0000-c000-000000000046} Id 0x8503", "value": v});
        let time = |id: &str, v: &str| json!({"id": format!("SystemTime {{00062008-0000-0000-c000-000000000046}} Id {id}"), "value": v});
        assert_eq!(read(json!([set("true"), time("0x8560", "2026-11-20T09:00:00.0000000Z")])), Some(at));
        assert_eq!(read(json!([set("true"), time("0x8502", "2026-11-20T09:00:00Z")])), Some(at), "no signal time: its time");
        assert_eq!(
            read(json!([set("true"), time("0x8502", "2026-11-20T08:00:00Z"), time("0x8560", "2026-11-20T09:00:00Z")])),
            Some(at),
            "snoozed: the signal time"
        );
        assert_eq!(read(json!([set("false"), time("0x8560", "2026-11-20T09:00:00Z")])), None, "dismissed");
        assert_eq!(read(json!([time("0x8560", "2026-11-20T09:00:00Z")])), None, "never set");
        assert_eq!(read(json!("junk")), None);
        // Asked for by Graph's own ids, the filter's words encoded.
        let e = reminder_expand();
        assert!(e.starts_with("$expand=singleValueExtendedProperties($filter=id%20eq%20%27Boolean%20%7B00062008-"), "{e}");
        assert!(e.ends_with("%27)") && e.matches("%20or%20").count() == 2, "{e}");
    }
}
