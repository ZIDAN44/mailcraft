//! A message carrying a meeting's invitation, cancellation or answer (a calendar with a `METHOD`,
//! RFC 6047) as Outlook's meeting mail (MS-OXOCAL 2.2.6, 2.2.7): `IPM.Schedule.Meeting.Request`,
//! `.Canceled`, or `.Resp.Pos`, `.Neg` or `.Tent` by the answer, carrying the meeting's own
//! properties as its appointment has them ([`super::calendar`]: its times and zone, place, how it
//! shows its time, reminder, global object id, its pattern and changed occurrences when it
//! repeats, whether the user organizes it or was invited, with the user's answer), and its times,
//! place and when it last changed for its guests and its organizer, so Outlook can take it from
//! the file as it takes one it received. Its calendar stays among its files, which the import
//! reads it back from.

use mailcraft_model::ical::{self, CalEvent};

use super::calendar::{Owner, PSETID_MEETING};
use super::item::filetime;
use super::pim::{Key, Value, lid};
use super::*;

/// The longest calendar read from a message for its meeting.
const MAX_CALENDAR: usize = 4 << 20;

/// What a message is when it carries a meeting: its class and the meeting's properties.
#[derive(Default)]
pub(super) struct Meeting {
    pub(super) class: &'static str,
    pub(super) props: Vec<(Key, Value)>,
}

/// The class of meeting mail for a calendar's `method`, and the answer it carries (`partstat`).
fn class(method: &str, partstat: &str) -> Option<&'static str> {
    match (method.trim().to_ascii_uppercase().as_str(), partstat.trim().to_ascii_uppercase().as_str()) {
        ("REQUEST", _) => Some("IPM.Schedule.Meeting.Request"),
        ("CANCEL", _) => Some("IPM.Schedule.Meeting.Canceled"),
        ("REPLY", "ACCEPTED") => Some("IPM.Schedule.Meeting.Resp.Pos"),
        ("REPLY", "DECLINED") => Some("IPM.Schedule.Meeting.Resp.Neg"),
        ("REPLY", "TENTATIVE") => Some("IPM.Schedule.Meeting.Resp.Tent"),
        _ => None,
    }
}

/// The meeting a message (`raw`, parsed as `p`) carries, when its calendar asks or answers one;
/// `owner`: whose mail it is.
pub(super) fn of(raw: &[u8], p: &mime::Parsed, owner: &Owner) -> Option<Meeting> {
    let part =
        p.attachments.iter().find(|a| a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))?;
    if part.size > MAX_CALENDAR {
        return None;
    }
    let cal = ical::parse(&mime::attachment_text(raw, part.index)?, 0);
    let method = cal.method.as_deref()?;
    let master = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first())?;
    let answer = master.attendees.first().map(|a| a.partstat.as_str()).unwrap_or_default();
    let class = class(method, answer)?;
    // A request or cancellation is one the user received, unless the account's addresses say
    // the user organizes it; an answer is to the meeting's organizer.
    let invited = owner.invited(master, matches!(class, "IPM.Schedule.Meeting.Request" | "IPM.Schedule.Meeting.Canceled"));
    let changed = if master.dtstamp > 0 { master.dtstamp } else { p.date };
    Some(Meeting { class, props: properties(master, &cal.events, owner, invited, changed) })
}

/// The properties of the meeting `e` (with the rest of its calendar's `events`, its changed
/// occurrences among them) as its appointment has them (the named ones and its times; the rest
/// are the message's own), the class of that appointment, its place (PidLidWhere), and when it
/// last changed for its guests and its organizer (`changed`, Unix seconds; 0: not known).
pub(super) fn properties(e: &CalEvent, events: &[CalEvent], owner: &Owner, invited: bool, changed: i64) -> Vec<(Key, Value)> {
    let own: Vec<&CalEvent> = events.iter().filter(|o| o.uid == e.uid && o.recurrence_id.is_some()).collect();
    let repeats = e.rrule.as_deref().is_some_and(|r| !r.trim().is_empty());
    let series = repeats.then(|| super::calendar::series(e, &own, owner, invited)).flatten();
    let recurring = series.is_some();
    let mut props: Vec<(Key, Value)> = super::calendar::properties(e, recurring, owner, invited)
        .into_iter()
        .filter(|(k, _)| matches!(k, Key::Lid(..) | Key::Tag(0x0060 | 0x0061 | 0x0063)))
        .collect();
    props.extend(series.into_iter().flatten());
    // PidLidAppointmentMessageClass: what it becomes in the calendar.
    props.push(lid(PSETID_MEETING, 0x0024, Value::Text("IPM.Appointment".into())));
    if !e.location.is_empty() {
        props.push(lid(PSETID_MEETING, 0x0002, Value::Text(e.location.clone())));
    }
    if changed > 0 {
        // PidLidAttendeeCriticalChange and PidLidOwnerCriticalChange.
        props.extend([lid(PSETID_MEETING, 0x0001, Value::Time(filetime(changed))), lid(PSETID_MEETING, 0x001A, Value::Time(filetime(changed)))]);
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_by_method_and_answer() {
        assert_eq!(class("request", ""), Some("IPM.Schedule.Meeting.Request"));
        assert_eq!(class("CANCEL", "ACCEPTED"), Some("IPM.Schedule.Meeting.Canceled"));
        assert_eq!(class("REPLY", "accepted"), Some("IPM.Schedule.Meeting.Resp.Pos"));
        assert_eq!(class("REPLY", "DECLINED"), Some("IPM.Schedule.Meeting.Resp.Neg"));
        assert_eq!(class("REPLY", "TENTATIVE"), Some("IPM.Schedule.Meeting.Resp.Tent"));
        assert_eq!(class("REPLY", "DELEGATED"), None);
        assert_eq!(class("PUBLISH", ""), None);
    }
}
