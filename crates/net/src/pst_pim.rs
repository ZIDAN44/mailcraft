//! Contacts and calendar items in an Outlook data file (MS-OXOCNTC, MS-OXOCAL).
//!
//! Most of what makes a contact or an appointment lives in *named* properties: their ids differ
//! from file to file and are found through the file's name-to-id map (MS-PST 2.4.7). The
//! recurrence of a repeating appointment is a binary pattern (MS-OXOCAL 2.2.1.44), read here
//! into an RFC 5545 rule.

use std::collections::HashMap;

use mailcraft_model::EmailAddress;
use mailcraft_model::dates::civil_from_days;
use mailcraft_model::ical::{Attendee, CalEvent};
use mailcraft_model::vcard::{Card, Typed};
use outlook_pst::ltp::prop_context::{GuidValue, PropertyValue};
use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId, PS_MAPI, PS_PUBLIC_STRINGS};
use outlook_pst::messaging::store::Store as PstStore;

type GuidKey = (u32, u16, u16, [u8; 8]);
const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const PSETID_ADDRESS: GuidKey = (0x0006_2004, 0, 0, OLE);
const PSETID_APPOINTMENT: GuidKey = (0x0006_2002, 0, 0, OLE);
const PSETID_TASK: GuidKey = (0x0006_2003, 0, 0, OLE);
const PSETID_NOTE: GuidKey = (0x0006_200E, 0, 0, OLE);
const PSETID_MEETING: GuidKey = (0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);
/// Name-to-id entries read at most.
const MAX_NAMES: usize = 200_000;
/// Deleted occurrences read at most.
const MAX_EXCEPTIONS: usize = 5_000;
/// Minutes from 1601-01-01 to 1970-01-01.
const EPOCH_MINUTES: i64 = 11_644_473_600 / 60;

fn key(g: &GuidValue) -> GuidKey {
    (g.data1(), g.data2(), g.data3(), *g.data4())
}

/// The data file's named properties: (property set, number) → property id.
#[derive(Default)]
pub(crate) struct Named(HashMap<(GuidKey, u32), u16>);

impl Named {
    pub(crate) fn of(store: &dyn PstStore) -> Named {
        let mut ids = HashMap::new();
        let Ok(map) = store.named_property_map() else { return Named(ids) };
        let props = map.properties();
        let guids = props.stream_guid().unwrap_or_default();
        for e in props.stream_entry().unwrap_or_default().into_iter().take(MAX_NAMES) {
            let NamedPropertyId::Number(lid) = e.id() else { continue };
            let set = match e.guid() {
                NamedPropertyGuid::None => continue,
                NamedPropertyGuid::Mapi => key(&PS_MAPI),
                NamedPropertyGuid::PublicStrings => key(&PS_PUBLIC_STRINGS),
                NamedPropertyGuid::GuidIndex(i) => match guids.get(usize::from(i)) {
                    Some(g) => key(g),
                    None => continue,
                },
            };
            ids.insert((set, lid), e.prop_id());
        }
        Named(ids)
    }

    fn id(&self, set: GuidKey, lid: u32) -> Option<u16> {
        self.0.get(&(set, lid)).copied()
    }
}

/// What a contact or appointment adds to a mail item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extra {
    pub display: String,
    pub given: String,
    pub family: String,
    pub company: String,
    pub title: String,
    pub emails: Vec<String>,
    pub phones: Vec<(String, &'static str)>,
    pub start: Option<i64>,
    /// A task's due date, and whether it's done.
    pub due: Option<i64>,
    pub done: bool,
    pub end: Option<i64>,
    pub location: String,
    pub all_day: bool,
    pub recurrence: Vec<u8>,
    pub uid: String,
    /// A note's colour (PidLidNoteColor: blue, green, pink, yellow, white).
    pub note_color: Option<u8>,
    /// When it last changed.
    pub modified: Option<i64>,
}

/// Read the contact and appointment properties of an item.
pub(crate) fn extract<'a>(p: &dyn Fn(u16) -> Option<&'a PropertyValue>, named: &Named) -> Extra {
    let s = |id: u16| p(id).and_then(crate::pst::text_of).unwrap_or_default();
    let n = |set, lid| named.id(set, lid).and_then(p);
    let ns = |set, lid| n(set, lid).and_then(crate::pst::text_of).unwrap_or_default();
    let time = |set, lid, fallback: u16| n(set, lid).or_else(|| p(fallback)).and_then(crate::pst::time_of);
    let mut x = Extra { display: s(0x3001), given: s(0x3A06), family: s(0x3A11), company: s(0x3A16), title: s(0x3A17), ..Default::default() };
    // Email1/2/3: the address, or (for an Exchange address) the display address.
    for (address, original) in [(0x8083, 0x8084), (0x8093, 0x8094), (0x80A3, 0x80A4)] {
        let e = [ns(PSETID_ADDRESS, address), ns(PSETID_ADDRESS, original)].into_iter().find(|e| e.contains('@'));
        if let Some(e) = e.filter(|e| !x.emails.contains(e)) {
            x.emails.push(e);
        }
    }
    for (id, kind) in [(0x3A08, "work"), (0x3A09, "home"), (0x3A1C, "cell"), (0x3A24, "fax")] {
        let v = s(id);
        if !v.trim().is_empty() {
            x.phones.push((v.trim().to_string(), kind));
        }
    }
    x.start = time(PSETID_APPOINTMENT, 0x820D, 0x0060);
    x.end = time(PSETID_APPOINTMENT, 0x820E, 0x0061);
    x.location = ns(PSETID_APPOINTMENT, 0x8208);
    x.due = n(PSETID_TASK, 0x8105).and_then(crate::pst::time_of);
    x.done = matches!(n(PSETID_TASK, 0x811C), Some(PropertyValue::Boolean(true)));
    x.all_day = matches!(n(PSETID_APPOINTMENT, 0x8215), Some(PropertyValue::Boolean(true)));
    if let Some(PropertyValue::Binary(b)) = n(PSETID_APPOINTMENT, 0x8216) {
        x.recurrence = b.buffer().to_vec();
    }
    if let Some(PropertyValue::Binary(b)) = n(PSETID_MEETING, 0x0003) {
        x.uid = b.buffer().iter().map(|b| format!("{b:02X}")).collect();
    }
    if let Some(PropertyValue::Integer32(c)) = n(PSETID_NOTE, 0x8B00) {
        x.note_color = u8::try_from(*c).ok().filter(|c| *c < mailcraft_model::pim::NOTE_COLORS);
    }
    x.modified = p(0x3008).and_then(crate::pst::time_of);
    x
}

/// The contact as a card.
pub(crate) fn card_of(x: &Extra, body: &str, uid: String) -> Card {
    let name = if x.display.trim().is_empty() { format!("{} {}", x.given, x.family).trim().to_string() } else { x.display.trim().to_string() };
    Card {
        uid,
        name,
        given: x.given.clone(),
        family: x.family.clone(),
        emails: x.emails.iter().map(|e| Typed { value: e.clone(), kind: String::new() }).collect(),
        phones: x.phones.iter().map(|(v, k)| Typed { value: v.clone(), kind: (*k).to_string() }).collect(),
        org: x.company.clone(),
        title: x.title.clone(),
        note: body.trim().to_string(),
        extra: Vec::new(),
    }
}

/// The appointment as an event (None without a start).
pub(crate) fn event_of(x: &Extra, subject: &str, body: &str, from: &EmailAddress, attendees: &[EmailAddress], uid: String) -> Option<CalEvent> {
    let start = x.start?;
    let mut end = x.end.filter(|e| *e >= start).unwrap_or(start);
    let mut start = start;
    if x.all_day {
        // Stored as local midnights: the nearest UTC midnight is the day.
        let day = |t: i64| (t + 43_200).div_euclid(86_400) * 86_400;
        start = day(start);
        end = day(end).max(start + 86_400);
    }
    let (rrule, exdates) = match recurrence(&x.recurrence, x.start.unwrap_or(start)) {
        Some((r, ex)) => (Some(r), ex),
        None => (None, Vec::new()),
    };
    let person = |a: &EmailAddress| Attendee { email: a.email.clone(), name: a.name.clone(), ..Default::default() };
    Some(CalEvent {
        uid: if x.uid.is_empty() { uid } else { x.uid.clone() },
        summary: subject.to_string(),
        description: body.trim().to_string(),
        location: x.location.clone(),
        start,
        end,
        all_day: x.all_day,
        rrule,
        exdates,
        organizer: (!from.email.is_empty() && !attendees.is_empty()).then(|| person(from)),
        attendees: attendees.iter().map(person).collect(),
        ..Default::default()
    })
}

struct Bytes<'a>(&'a [u8], usize);

impl Bytes<'_> {
    fn u16(&mut self) -> Option<u16> {
        let b = self.0.get(self.1..self.1.checked_add(2)?)?;
        self.1 += 2;
        Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.0.get(self.1..self.1.checked_add(4)?)?;
        self.1 += 4;
        Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]))
    }
}

const DAYS: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

fn byday(mask: u32, ordinal: Option<i32>) -> String {
    let pre = ordinal.map(|n| n.to_string()).unwrap_or_default();
    DAYS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, d)| format!("{pre}{d}")).collect::<Vec<_>>().join(",")
}

/// A recurrence pattern as (rule, deleted occurrences), given the first occurrence's start (UTC).
pub(crate) fn recurrence(blob: &[u8], start: i64) -> Option<(String, Vec<i64>)> {
    let mut b = Bytes(blob, 0);
    let (_reader, _writer) = (b.u16()?, b.u16()?);
    let frequency = b.u16()?;
    let pattern = b.u16()?;
    let _calendar = b.u16()?;
    let _first = b.u32()?;
    let period = i64::from(b.u32()?);
    let _sliding = b.u32()?;
    let (mut a, mut c) = (0, 0);
    match pattern {
        0x0001 | 0x0002 | 0x0004 | 0x000A | 0x000C => a = b.u32()?,
        0x0003 | 0x000B => {
            a = b.u32()?;
            c = b.u32()?;
        }
        _ => {}
    }
    let end_type = b.u32()?;
    let count = b.u32()?;
    let _first_dow = b.u32()?;
    let deleted_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let deleted: Vec<i64> = (0..deleted_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let modified_count = usize::try_from(b.u32()?).ok()?.min(MAX_EXCEPTIONS);
    let modified: Vec<i64> = (0..modified_count).map(|_| b.u32().map(i64::from)).collect::<Option<_>>()?;
    let first_day = i64::from(b.u32()?);
    let end_day = i64::from(b.u32()?);
    // The appointment part: when in the (local) day the occurrences start.
    let start_offset = (|| {
        b.u32()?;
        b.u32()?;
        b.u32().map(i64::from)
    })()
    .unwrap_or(0);
    let nth = |n: u32| if n >= 5 { -1 } else { i32::try_from(n).unwrap_or(1) };
    let mut rule = match (frequency, pattern) {
        (0x200A, 0x0000) => format!("FREQ=DAILY;INTERVAL={}", (period / 1440).max(1)),
        (0x200A, 0x0001) => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
        (0x200B, 0x0001) => format!("FREQ=WEEKLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, None)),
        (0x200C, 0x0002) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY={a}", period.max(1)),
        (0x200C, 0x0004) => format!("FREQ=MONTHLY;INTERVAL={};BYMONTHDAY=-1", period.max(1)),
        (0x200C, 0x0003) => format!("FREQ=MONTHLY;INTERVAL={};BYDAY={}", period.max(1), byday(a, Some(nth(c)))),
        (0x200D, 0x0002 | 0x0004) => format!("FREQ=YEARLY;INTERVAL={}", (period / 12).max(1)),
        (0x200D, 0x0003) => {
            let (_, month, _) = civil_from_days(start.div_euclid(86_400));
            format!("FREQ=YEARLY;INTERVAL={};BYMONTH={month};BYDAY={}", (period / 12).max(1), byday(a, Some(nth(c))))
        }
        _ => return None,
    };
    match end_type {
        0x2021 => {
            let (y, m, d) = civil_from_days((end_day - EPOCH_MINUTES).div_euclid(1440));
            rule.push_str(&format!(";UNTIL={y:04}{m:02}{d:02}T235959Z"));
        }
        0x2022 if count > 0 => rule.push_str(&format!(";COUNT={count}")),
        _ => {}
    }
    // Local day + time into it → UTC, by how far the first occurrence is from its local time.
    let local_first = (first_day - EPOCH_MINUTES + start_offset) * 60;
    let shift = start - local_first;
    let exdates = deleted.iter().filter(|d| !modified.contains(d)).map(|d| (d - EPOCH_MINUTES + start_offset) * 60 + shift).collect();
    // Sanity: a first day far from the start means the blob isn't what it says.
    if shift.abs() > 2 * 86_400 {
        return Some((rule, Vec::new()));
    }
    Some((rule, exdates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcraft_model::dates::days_from_civil;

    fn minutes(y: i64, m: u32, d: u32) -> u32 {
        u32::try_from((days_from_civil(y, m, d) * 1440) + EPOCH_MINUTES).unwrap()
    }

    fn blob(frequency: u16, pattern: u16, period: u32, specific: &[u32], end: (u32, u32), deleted: &[u32], modified: &[u32], first: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [0x3004u16, 0x3004, frequency, pattern, 0] {
            v.extend(x.to_le_bytes());
        }
        let mut words = vec![first, period, 0];
        words.extend(specific);
        words.extend([end.0, end.1, 0, u32::try_from(deleted.len()).unwrap()]);
        words.extend(deleted);
        words.push(u32::try_from(modified.len()).unwrap());
        words.extend(modified);
        words.extend([first, minutes(4500, 1, 1), 0x3006, 0x3008, 9 * 60]);
        for w in words {
            v.extend(w.to_le_bytes());
        }
        v
    }

    #[test]
    fn recurrence_patterns_become_rules() {
        // Mondays and Wednesdays at 09:00 in UTC+2 (07:00Z), ten times, one deleted, one moved.
        let first = minutes(2026, 10, 5);
        let start = days_from_civil(2026, 10, 5) * 86_400 + 7 * 3600;
        let b = blob(0x200B, 0x0001, 1, &[0b000_1010], (0x2022, 10), &[minutes(2026, 10, 7), minutes(2026, 10, 12)], &[minutes(2026, 10, 12)], first);
        let (rule, ex) = recurrence(&b, start).unwrap();
        assert_eq!(rule, "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE;COUNT=10");
        assert_eq!(ex, [days_from_civil(2026, 10, 7) * 86_400 + 7 * 3600]);
        // Every second day, until a date.
        let b = blob(0x200A, 0x0000, 2880, &[], (0x2021, 0), &[], &[], first);
        let b = {
            let mut b = b;
            let at = b.len() - 4 * 4;
            b.splice(at..at + 4, minutes(2026, 12, 31).to_le_bytes());
            b
        };
        assert_eq!(recurrence(&b, start).unwrap().0, "FREQ=DAILY;INTERVAL=2;UNTIL=20261231T235959Z");
        // Every weekday; the last Friday of each month; every year.
        assert_eq!(recurrence(&blob(0x200A, 0x0001, 1, &[0x3E], (0x2023, 0), &[], &[], first), start).unwrap().0, "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
        assert_eq!(
            recurrence(&blob(0x200C, 0x0003, 1, &[0b010_0000, 5], (0x2023, 0), &[], &[], first), start).unwrap().0,
            "FREQ=MONTHLY;INTERVAL=1;BYDAY=-1FR"
        );
        assert_eq!(recurrence(&blob(0x200D, 0x0002, 12, &[5], (0x2023, 0), &[], &[], first), start).unwrap().0, "FREQ=YEARLY;INTERVAL=1");
        // Junk is no rule, and never a crash.
        assert!(recurrence(&[], start).is_none());
        assert!(recurrence(&b[..20], start).is_none());
        assert!(recurrence(&blob(0x7777, 0x0001, 1, &[1], (0x2023, 0), &[], &[], first), start).is_none());
        let mut huge = blob(0x200B, 0x0001, 1, &[1], (0x2023, 0), &[], &[], first);
        huge.splice(38..42, u32::MAX.to_le_bytes());
        assert!(recurrence(&huge, start).is_none());
    }

    #[test]
    fn contacts_and_appointments_map() {
        let x = Extra {
            given: "Ana".into(),
            family: "Lima".into(),
            company: "Contoso".into(),
            emails: vec!["ana@contoso.example".into()],
            phones: vec![("+1 555 0100".into(), "cell")],
            ..Default::default()
        };
        let c = card_of(&x, "Met at the fair.\r\n", "u1".into());
        assert_eq!((c.name.as_str(), c.org.as_str(), c.note.as_str()), ("Ana Lima", "Contoso", "Met at the fair."));
        assert_eq!(c.phones[0], Typed { value: "+1 555 0100".into(), kind: "cell".into() });
        let day = days_from_civil(2026, 10, 20) * 86_400;
        // An all-day item stored at local midnight in UTC+6 (18:00Z the day before).
        let x =
            Extra { start: Some(day - 6 * 3600), end: Some(day + 86_400 - 6 * 3600), all_day: true, location: "Hall".into(), ..Default::default() };
        let e = event_of(&x, "Fair", "", &EmailAddress::default(), &[], "u2".into()).unwrap();
        assert_eq!((e.start, e.end, e.all_day, e.uid.as_str(), e.location.as_str()), (day, day + 86_400, true, "u2", "Hall"));
        assert!(e.organizer.is_none());
        let x = Extra { start: Some(day + 3600), end: Some(day), uid: "0400".into(), ..Default::default() };
        let ana = EmailAddress::new("Ana", "ana@contoso.example");
        let e = event_of(&x, "Call", "", &EmailAddress::new("Me", "me@x.example"), std::slice::from_ref(&ana), "u3".into()).unwrap();
        assert_eq!((e.end, e.uid.as_str()), (day + 3600, "0400"));
        assert_eq!(e.organizer.unwrap().email, "me@x.example");
        assert!(event_of(&Extra::default(), "", "", &EmailAddress::default(), &[], "u".into()).is_none());
    }
}
