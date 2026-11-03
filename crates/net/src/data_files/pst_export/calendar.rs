//! The account's calendar as Outlook appointments (`IPM.Appointment`, MS-OXOCAL 2.2.1) in the
//! data file's Calendar folder: an event's times (and the zone they were set in, as Outlook keeps
//! a zone: [`zone`]), whether it lasts all day, how it shows its time, its place, reminder (the
//! first alarm: Outlook has one), privacy and categories, its uid as the meeting's global object
//! id, its organizer and guests with their answers as the item's recipients, and its files. A
//! repeating event is one appointment with its pattern, its deleted occurrences and its changed
//! ones, each also an item attached to it ([`recur`]). A rule the pattern can't say (hourly, a
//! `BYSETPOS` past what "the nth weekday" says, several months, a rule whose read-back gives other
//! days…) goes in as its occurrences, each an appointment of its own, up to
//! [`ical::MAX_OCCURRENCES`]: a limit of the format, counted in the export's result.

use std::collections::{BTreeMap, HashSet};

use mailcraft_model::ical::{self, CalEvent, Privacy, ShowAs};

use super::item::{PSETID_COMMON, filetime};
use super::pim::{Attached, File, Item, Key, Person, Value, crlf, keywords, lid, tag};
use super::*;
use outlook_pst::ltp::prop_context::GuidValue;

mod recur;
mod zone;

/// PSETID_Appointment and PSETID_Meeting (MS-OXPROPS 1.3.2).
pub(super) const PSETID_APPOINTMENT: GuidValue = GuidValue::new(0x0006_2002, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub(super) const PSETID_MEETING: GuidValue = GuidValue::new(0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);
/// PidLidGlobalObjectId's first bytes (MS-OXOCAL 2.2.1.27), and those of its Data for a meeting
/// whose uid isn't Outlook's, the uid following.
const GLOBAL_ID: [u8; 16] = [0x04, 0, 0, 0, 0x82, 0, 0xE0, 0, 0x74, 0xC5, 0xB7, 0x10, 0x1A, 0x82, 0xE0, 0x08];
const VCAL_UID: &[u8; 12] = b"vCal-Uid\x01\0\0\0";
/// PidLidAppointmentStateFlags (MS-OXOCAL 2.2.1.10): a meeting, one received, one called off.
const ASF_MEETING: i32 = 0x1;
const ASF_RECEIVED: i32 = 0x2;
const ASF_CANCELED: i32 = 0x4;
/// PidTagRecipientFlags (MS-OXOCAL 2.2.4.10.1): one mail can go to, its organizer.
const RECIP_SENDABLE: i32 = 0x1;
const RECIP_ORGANIZER: i32 = 0x2;
/// PidLidResponseStatus: the meeting is the user's own (respOrganized).
const RESPONSE_ORGANIZED: i32 = 1;
/// 100 ns intervals in a second.
const TICKS: i64 = 10_000_000;

/// The appointments for `events` (as many as they make), each with the uid it is marked with;
/// `now` (Unix seconds) stands in for when one was last changed when it doesn't say.
pub(super) fn items(events: &[&CalEvent], now: i64) -> Vec<(String, Item)> {
    let mut overrides: BTreeMap<&str, Vec<&CalEvent>> = BTreeMap::new();
    for e in events.iter().filter(|e| e.recurrence_id.is_some()) {
        overrides.entry(e.uid.as_str()).or_default().push(e);
    }
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for e in events.iter().filter(|e| e.recurrence_id.is_none()) {
        // One series per uid: a second event with the uid is one of its own.
        let own = if seen.insert(e.uid.as_str()) { overrides.remove(e.uid.as_str()).unwrap_or_default() } else { Vec::new() };
        let rule = e.rrule.as_deref().map(str::trim).filter(|r| !r.is_empty());
        if rule.is_none() {
            out.push((e.uid.clone(), appointment(e, None, now)));
            out.extend(own.into_iter().map(|o| alone(o, now)));
            continue;
        }
        match recur::series(e, &own, now) {
            Some(s) => {
                let left: Vec<&CalEvent> = own.iter().filter(|o| !s.taken.contains(&o.recurrence_id.unwrap_or(i64::MIN))).copied().collect();
                out.push((e.uid.clone(), appointment(e, Some(s), now)));
                out.extend(left.into_iter().map(|o| alone(o, now)));
            }
            None => out.extend(one_by_one(e, &own, now)),
        }
    }
    // Changed occurrences of a series that isn't here (an invitation to one of them).
    out.extend(overrides.into_values().flatten().map(|o| alone(o, now)));
    out
}

/// A changed occurrence on its own: marked by its uid and the occurrence it replaces.
fn alone(o: &CalEvent, now: i64) -> (String, Item) {
    (format!("{}/{}", o.uid, o.recurrence_id.unwrap_or(o.start)), appointment(o, None, now))
}

/// A series whose rule the pattern can't say, as its occurrences, each an appointment of its own
/// with a uid of its own (`<uid>/<start>`, as MailCraft names one of several sharing a uid):
/// deleted ones left out, changed ones as they were changed (called off: left out).
fn one_by_one(e: &CalEvent, own: &[&CalEvent], now: i64) -> Vec<(String, Item)> {
    let len = e.end.saturating_sub(e.start);
    let mut out = Vec::new();
    let all = ical::starts(e, i64::MAX);
    if all.is_empty() {
        // A rule that makes no day (a frequency nobody knows): the event as it starts.
        let one = CalEvent { rrule: None, exdates: Vec::new(), ..e.clone() };
        return vec![(e.uid.clone(), appointment(&one, None, now))];
    }
    for s in all.into_iter().filter(|s| !e.exdates.contains(s)).take(ical::MAX_OCCURRENCES) {
        let changed = own.iter().find(|o| o.recurrence_id == Some(s));
        if changed.is_some_and(|o| o.status.eq_ignore_ascii_case("CANCELLED")) {
            continue;
        }
        let mut one = changed.map_or_else(|| CalEvent { start: s, end: s.saturating_add(len), ..e.clone() }, |o| (*o).clone());
        (one.rrule, one.exdates, one.recurrence_id) = (None, Vec::new(), None);
        one.uid = format!("{}/{s}", e.uid);
        let mut item = appointment(&one, None, now);
        item.occurrence = true;
        out.push((one.uid, item));
    }
    out
}

/// PidLidBusyStatus for how an event shows its time.
fn busy(e: &CalEvent) -> i32 {
    match ical::show_as(e) {
        ShowAs::Free => 0,
        ShowAs::Tentative => 1,
        ShowAs::Busy => 2,
        ShowAs::OutOfOffice => 3,
        ShowAs::WorkingElsewhere => 4,
    }
}

/// PidTagRecipientTrackStatus for a guest's `PARTSTAT` (none when it says none Outlook has).
fn track(partstat: &str) -> Option<i32> {
    match partstat.trim().to_ascii_uppercase().as_str() {
        "TENTATIVE" => Some(2),
        "ACCEPTED" => Some(3),
        "DECLINED" => Some(4),
        "NEEDS-ACTION" => Some(5),
        _ => None,
    }
}

/// The event's organizer and guests as recipients (the organizer first, flagged as such; none
/// without guests).
fn people(e: &CalEvent) -> Vec<Person> {
    if e.attendees.is_empty() {
        return Vec::new();
    }
    let organizer = e.organizer.iter().filter(|o| !o.email.trim().is_empty()).map(|o| Person {
        name: o.name.clone(),
        email: o.email.clone(),
        kind: UnicodePstRecipientType::To,
        flags: Some(RECIP_SENDABLE | RECIP_ORGANIZER),
        track: Some(0),
    });
    let guests = e.attendees.iter().filter(|a| !a.email.trim().is_empty()).map(|a| Person {
        name: a.name.clone(),
        email: a.email.clone(),
        kind: UnicodePstRecipientType::To,
        flags: Some(RECIP_SENDABLE),
        track: track(&a.partstat),
    });
    organizer.chain(guests).collect()
}

/// A meeting's global object id for `uid` (MS-OXOCAL 2.2.1.27): the id itself when the uid is
/// one in hexadecimal (as Outlook writes it in a calendar file), else one carrying the uid; with
/// the day of the occurrence it is, for a changed one.
pub(super) fn global_id(uid: &str, day: Option<(i64, u32, u32)>) -> Vec<u8> {
    let hex = (uid.len().is_multiple_of(2) && uid.len() >= 80)
        .then(|| (0..uid.len()).step_by(2).map(|i| uid.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect::<Option<Vec<u8>>>())
        .flatten()
        .filter(|b| b.starts_with(&GLOBAL_ID));
    let mut id = match hex {
        Some(b) => b,
        None => {
            let data = [&VCAL_UID[..], uid.as_bytes(), &[0]].concat();
            let size = u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes();
            [&GLOBAL_ID[..], &[0; 4], &[0; 8], &[0; 8], &size, &data].concat()
        }
    };
    if let Some((y, m, d)) = day {
        let y = u16::try_from(y).unwrap_or(0).to_be_bytes();
        for (at, b) in [(16, y[0]), (17, y[1]), (18, u8::try_from(m).unwrap_or(0)), (19, u8::try_from(d).unwrap_or(0))] {
            if let Some(slot) = id.get_mut(at) {
                *slot = b;
            }
        }
    }
    id
}

/// The properties of an appointment for `e`, but its recurrence's: its times, zone, how it shows
/// them, place, reminder, privacy, categories, global object id, state and version. Those a
/// meeting's mail carries too are the named ones (`super::meeting`).
pub(super) fn properties(e: &CalEvent, recurring: bool) -> Vec<(Key, Value)> {
    let (start, end) = (filetime(e.start), filetime(e.end.max(e.start)));
    let appt = |id, v| lid(PSETID_APPOINTMENT, id, v);
    let mut props = vec![
        appt(0x820D, Value::Time(start)),
        appt(0x820E, Value::Time(end)),
        tag(0x0060, Value::Time(start)),
        tag(0x0061, Value::Time(end)),
        lid(PSETID_COMMON, 0x8516, Value::Time(start)),
        lid(PSETID_COMMON, 0x8517, Value::Time(end)),
        appt(0x8213, Value::Int(i32::try_from(e.end.saturating_sub(e.start).max(0) / 60).unwrap_or(i32::MAX))),
        appt(0x8215, Value::Bool(e.all_day)),
        appt(0x8205, Value::Int(busy(e))),
        appt(0x8223, Value::Bool(recurring)),
        lid(PSETID_MEETING, 0x0005, Value::Bool(recurring)),
        appt(0x8201, Value::Int(i32::try_from(e.sequence).unwrap_or(i32::MAX))),
    ];
    if !e.location.is_empty() {
        props.push(appt(0x8208, Value::Text(e.location.clone())));
    }
    if !recurring {
        props.extend([appt(0x8235, Value::Time(start)), appt(0x8236, Value::Time(end))]);
    }
    props.extend(zone::properties(e, recurring));
    if let Some(m) = ical::reminder_minutes(e) {
        props.extend([
            lid(PSETID_COMMON, 0x8503, Value::Bool(true)),
            lid(PSETID_COMMON, 0x8501, Value::Int(i32::try_from(m).unwrap_or(0))),
            lid(PSETID_COMMON, 0x8502, Value::Time(start)),
            lid(PSETID_COMMON, 0x8560, Value::Time(filetime(e.start.saturating_sub(m.saturating_mul(60))))),
        ]);
    }
    let sensitivity = match ical::privacy(e) {
        _ if ical::personal(e) => Some(1),
        Privacy::Private => Some(2),
        Privacy::Confidential => Some(3),
        Privacy::Public => None,
    };
    props.extend(sensitivity.map(|s| tag(0x0036, Value::Int(s))));
    props.extend(keywords(&e.categories));
    let id = global_id(&e.uid, None);
    props.extend([lid(PSETID_MEETING, 0x0003, Value::Bin(id.clone())), lid(PSETID_MEETING, 0x0023, Value::Bin(id))]);
    let meeting = !e.attendees.is_empty();
    let received = meeting && ical::is_guest_copy(e);
    let cancelled = e.status.eq_ignore_ascii_case("CANCELLED");
    let state = if meeting { ASF_MEETING } else { 0 } | if received { ASF_RECEIVED } else { 0 } | if cancelled { ASF_CANCELED } else { 0 };
    props.push(appt(0x8217, Value::Int(state)));
    if meeting && !received {
        props.push(appt(0x8218, Value::Int(RESPONSE_ORGANIZED)));
    }
    if e.attendees.iter().any(|a| a.rsvp) {
        props.push(tag(0x0063, Value::Bool(true)));
    }
    props
}

/// A series' own properties: its pattern (PidLidAppointmentRecur), what kind it is, its first and
/// last days.
fn recurrence(s: &mut recur::Series) -> Vec<(Key, Value)> {
    vec![
        lid(PSETID_APPOINTMENT, 0x8216, Value::Bin(std::mem::take(&mut s.pattern))),
        lid(PSETID_APPOINTMENT, 0x8231, Value::Int(s.kind)),
        lid(PSETID_APPOINTMENT, 0x8235, Value::Time(s.clip.0)),
        lid(PSETID_APPOINTMENT, 0x8236, Value::Time(s.clip.1)),
    ]
}

/// The recurrence properties of the series `e` with its changed occurrences `own`, as its
/// meeting's mail carries them; none when the pattern can't say it.
pub(super) fn series(e: &CalEvent, own: &[&CalEvent]) -> Option<Vec<(Key, Value)>> {
    recur::series(e, own, 0).map(|mut s| recurrence(&mut s))
}

/// The appointment for `e`, with its pattern and changed occurrences when it is a series (`s`).
fn appointment(e: &CalEvent, s: Option<recur::Series>, now: i64) -> Item {
    let mut props = properties(e, s.is_some());
    let mut attached = Vec::new();
    if let Some(mut s) = s {
        props.extend(recurrence(&mut s));
        attached = s.changed;
    }
    let organizer = e.organizer.as_ref().map(|o| (o.name.clone(), o.email.clone())).unwrap_or_default();
    Item {
        class: "IPM.Appointment",
        subject: e.summary.clone(),
        body: crlf(&e.description),
        modified: filetime(if e.dtstamp > 0 { e.dtstamp } else { now }),
        props,
        files: files(e),
        sender: organizer,
        people: people(e),
        attached,
        occurrence: false,
    }
}

/// The files an event carries (its `ATTACH` lines with their bytes).
fn files(e: &CalEvent) -> Vec<File> {
    e.extra.iter().filter_map(|l| super::task::file(l)).collect()
}

/// A changed occurrence `o` of the series `e` as the item attached to it (MS-OXOCAL 2.2.10.1):
/// its own text, times, place, how it shows them, reminder and guests, the occurrence it
/// replaces (PidLidExceptionReplaceTime), and hidden as an exception (afException), with its
/// times in the series' zone (`local`) as the attachment says them.
fn exception(e: &CalEvent, o: &CalEvent, replaces: i64, local: &dyn Fn(i64) -> i64, now: i64) -> Attached {
    // Its own global object id names the day of the occurrence it is.
    let mut props: Vec<(Key, Value)> = properties(o, false).into_iter().filter(|(k, _)| !matches!(k, Key::Lid(_, 0x0003 | 0x0023))).collect();
    let (y, m, d) = mailcraft_model::dates::civil_from_days(local(replaces).div_euclid(86_400));
    props.extend([
        lid(PSETID_APPOINTMENT, 0x8228, Value::Time(filetime(replaces))),
        lid(PSETID_APPOINTMENT, 0x8206, Value::Bool(o.description != e.description)),
        lid(PSETID_APPOINTMENT, 0x822B, Value::Bool(o.attendees != e.attendees)),
        lid(PSETID_MEETING, 0x0003, Value::Bin(global_id(&e.uid, Some((y, m, d))))),
        lid(PSETID_MEETING, 0x0023, Value::Bin(global_id(&e.uid, None))),
    ]);
    let organizer = o.organizer.as_ref().or(e.organizer.as_ref()).map(|p| (p.name.clone(), p.email.clone())).unwrap_or_default();
    let item = Item {
        class: recur::EXCEPTION_CLASS,
        subject: o.summary.clone(),
        body: crlf(&o.description),
        modified: filetime(if o.dtstamp > 0 { o.dtstamp } else { now }),
        props,
        files: files(o),
        sender: organizer,
        // Its guests when they aren't the series'.
        people: if o.attendees != e.attendees { people(o) } else { Vec::new() },
        attached: Vec::new(),
        occurrence: false,
    };
    // As local times, written as if they were UTC (PidTagExceptionStartTime, …EndTime).
    let at = |t: i64| Value::Time(local(t).saturating_add(11_644_473_600).saturating_mul(TICKS).max(0));
    let attachment = vec![
        (0x7FFE, Value::Bool(true)),
        (0x7FFD, Value::Int(0x2)),
        (0x7FFA, Value::Int(0)),
        (0x7FFB, at(o.start)),
        (0x7FFC, at(o.end.max(o.start))),
        (0x7FF9, Value::Time(filetime(replaces))),
    ];
    Attached { item, props: attachment }
}
