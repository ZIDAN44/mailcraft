//! Contacts, contact groups, calendar items, meeting requests and journal entries in an Outlook
//! data file (MS-OXOCNTC, MS-OXOCAL, MS-OXOJRNL).
//!
//! Most of what makes a contact or an appointment lives in *named* properties: their ids differ
//! from file to file and are found through the file's name-to-id map (MS-PST 2.4.7). The
//! recurrence of a repeating appointment is a binary pattern (MS-OXOCAL 2.2.1.44), read here
//! into an RFC 5545 rule.

use std::collections::HashMap;

use mailcraft_model::EmailAddress;
use mailcraft_model::dates::civil_from_days;
use mailcraft_model::ical::{self, Attendee, CalEvent, Privacy, ShowAs};
use mailcraft_model::vcard::{Card, PostalAddress, Typed};
use outlook_pst::ltp::prop_context::{GuidValue, PropertyValue};
use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId, PS_MAPI, PS_PUBLIC_STRINGS};
use outlook_pst::messaging::store::Store as PstStore;

use crate::pst::Val;

type GuidKey = (u32, u16, u16, [u8; 8]);
const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const PSETID_ADDRESS: GuidKey = (0x0006_2004, 0, 0, OLE);
const PSETID_APPOINTMENT: GuidKey = (0x0006_2002, 0, 0, OLE);
const PSETID_TASK: GuidKey = (0x0006_2003, 0, 0, OLE);
const PSETID_NOTE: GuidKey = (0x0006_200E, 0, 0, OLE);
const PSETID_COMMON: GuidKey = (0x0006_2008, 0, 0, OLE);
const PSETID_LOG: GuidKey = (0x0006_200A, 0, 0, OLE);
const PSETID_MEETING: GuidKey = (0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);
/// Name-to-id entries read at most.
const MAX_NAMES: usize = 200_000;
/// Deleted occurrences read at most.
const MAX_EXCEPTIONS: usize = 5_000;
/// A contact group's members, and an event's categories, kept at most.
const MAX_MEMBERS: usize = 10_000;
const MAX_CATEGORIES: usize = 64;
/// A reminder further ahead than a year (in minutes) isn't one; nor a journal entry longer.
const MAX_MINUTES: i64 = 366 * 24 * 60;
/// The provider of a wrapped entry id (a contact group's member, MS-OXOCNTC 2.2.2.2.1).
const WRAPPED: [u8; 16] = [0xC0, 0x91, 0xAD, 0xD3, 0x51, 0x9D, 0xCF, 0x11, 0xA4, 0xA9, 0x00, 0xAA, 0x00, 0x47, 0xFA, 0xA4];
/// Minutes from 1601-01-01 to 1970-01-01.
const EPOCH_MINUTES: i64 = 11_644_473_600 / 60;

fn key(g: &GuidValue) -> GuidKey {
    (g.data1(), g.data2(), g.data3(), *g.data4())
}

/// The data file's named properties: (property set, number) → property id; and the one named
/// by a string that the import reads, the categories (`Keywords` in PS_PUBLIC_STRINGS).
#[derive(Default)]
pub(crate) struct Named {
    ids: HashMap<(GuidKey, u32), u16>,
    keywords: Option<u16>,
}

impl Named {
    pub(crate) fn of(store: &dyn PstStore) -> Named {
        let mut named = Named::default();
        let Ok(map) = store.named_property_map() else { return named };
        let props = map.properties();
        let guids = props.stream_guid().unwrap_or_default();
        let strings = match props.get(0x0004) {
            Some(PropertyValue::Binary(b)) => b.buffer(),
            _ => &[],
        };
        for e in props.stream_entry().unwrap_or_default().into_iter().take(MAX_NAMES) {
            let set = match e.guid() {
                NamedPropertyGuid::None => continue,
                NamedPropertyGuid::Mapi => key(&PS_MAPI),
                NamedPropertyGuid::PublicStrings => key(&PS_PUBLIC_STRINGS),
                NamedPropertyGuid::GuidIndex(i) => match guids.get(usize::from(i)) {
                    Some(g) => key(g),
                    None => continue,
                },
            };
            match e.id() {
                NamedPropertyId::Number(lid) => {
                    named.ids.insert((set, lid), e.prop_id());
                }
                NamedPropertyId::StringOffset(at) => {
                    if set == key(&PS_PUBLIC_STRINGS) && name_at(strings, at).is_some_and(|n| n.eq_ignore_ascii_case("Keywords")) {
                        named.keywords = Some(e.prop_id());
                    }
                }
            }
        }
        named
    }

    /// Only the categories' property known, at `id` (for tests and the fuzz target).
    pub(crate) fn with_keywords(id: u16) -> Named {
        Named { keywords: Some(id), ..Default::default() }
    }

    /// The property holding an item's categories.
    pub(crate) fn keywords(&self) -> Option<u16> {
        self.keywords
    }

    fn id(&self, set: GuidKey, lid: u32) -> Option<u16> {
        self.ids.get(&(set, lid)).copied()
    }
}

/// A property's name in the name-to-id map's string stream (MS-PST 2.4.7.4): at byte `at`, a
/// little-endian length in bytes, then the name in UTF-16. Names longer than 512 bytes aren't
/// read.
fn name_at(strings: &[u8], at: u32) -> Option<String> {
    let at = usize::try_from(at).ok()?;
    let len = strings.get(at..at.checked_add(4)?)?;
    let len = usize::try_from(u32::from_le_bytes([*len.first()?, *len.get(1)?, *len.get(2)?, *len.get(3)?])).ok()?;
    if len > 512 {
        return None;
    }
    let start = at.checked_add(4)?;
    let bytes = strings.get(start..start.checked_add(len)?)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(String::from_utf16_lossy(&units))
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
    /// Business, home and other addresses.
    pub addresses: Vec<PostalAddress>,
    pub birthday: Option<i64>,
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
    /// A meeting's version (PidLidAppointmentSequence).
    pub sequence: u32,
    /// An event's categories, how it shows its time (PidLidBusyStatus: 0 free, 1 tentative, 2
    /// busy, 3 away, 4 working elsewhere), its sensitivity (1 personal, 2 private, 3
    /// confidential) and its reminder, in minutes before it starts.
    pub categories: Vec<String>,
    pub busy: Option<i64>,
    pub sensitivity: u8,
    pub reminder: Option<i64>,
    /// A contact group's members that carry their address: (name, address).
    pub members: Vec<(String, String)>,
    /// A journal entry's start and end.
    pub log_start: Option<i64>,
    pub log_end: Option<i64>,
}

/// Read the contact and appointment properties of an item.
pub(crate) fn extract<'a>(p: &dyn Fn(u16) -> Option<Val<'a>>, named: &Named) -> Extra {
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    let n = |set, lid| named.id(set, lid).and_then(p);
    let ns = |set, lid| n(set, lid).and_then(Val::into_text).unwrap_or_default();
    let time = |set, lid, fallback: u16| n(set, lid).or_else(|| p(fallback)).and_then(|v| v.time());
    let mut x = Extra { display: s(0x3001), given: s(0x3A06), family: s(0x3A11), company: s(0x3A16), title: s(0x3A17), ..Default::default() };
    // Email1/2/3: the address, or (for an Exchange address) the display address.
    for (address, original) in [(0x8083, 0x8084), (0x8093, 0x8094), (0x80A3, 0x80A4)] {
        let e = [ns(PSETID_ADDRESS, address), ns(PSETID_ADDRESS, original)].into_iter().find(|e| e.contains('@'));
        if let Some(e) = e.filter(|e| !x.emails.contains(e)) {
            x.emails.push(e);
        }
    }
    // MS-OXOCNTC's telephone numbers: business (and its second), home (and its second),
    // mobile, fax, pager, other.
    for (id, kind) in
        [(0x3A08, "work"), (0x3A1B, "work"), (0x3A09, "home"), (0x3A2F, "home"), (0x3A1C, "cell"), (0x3A24, "fax"), (0x3A21, "pager"), (0x3A1F, "")]
    {
        let v = s(id);
        if !v.trim().is_empty() {
            x.phones.push((v.trim().to_string(), kind));
        }
    }
    // Its addresses: (street, city, state, postal code, country) of business, home and other.
    for (kind, [street, city, state, code, country]) in [
        ("work", [0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26]),
        ("home", [0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A]),
        ("", [0x3A63, 0x3A5F, 0x3A62, 0x3A61, 0x3A60]),
    ] {
        let a = PostalAddress {
            kind: kind.into(),
            street: s(street).trim().replace("\r\n", "\n"),
            city: s(city).trim().to_string(),
            region: s(state).trim().to_string(),
            postcode: s(code).trim().to_string(),
            country: s(country).trim().to_string(),
        };
        if !a.is_empty() {
            x.addresses.push(a);
        }
    }
    x.birthday = p(0x3A42).and_then(|v| v.time());
    x.start = time(PSETID_APPOINTMENT, 0x820D, 0x0060);
    x.end = time(PSETID_APPOINTMENT, 0x820E, 0x0061);
    x.location = ns(PSETID_APPOINTMENT, 0x8208);
    x.due = n(PSETID_TASK, 0x8105).and_then(|v| v.time());
    x.done = matches!(n(PSETID_TASK, 0x811C), Some(Val::Bool(true)));
    x.all_day = matches!(n(PSETID_APPOINTMENT, 0x8215), Some(Val::Bool(true)));
    if let Some(Val::Bin(b)) = n(PSETID_APPOINTMENT, 0x8216) {
        x.recurrence = b.to_vec();
    }
    if let Some(Val::Bin(b)) = n(PSETID_MEETING, 0x0003) {
        x.uid = b.iter().map(|b| format!("{b:02X}")).collect();
    }
    if let Some(Val::Int(c)) = n(PSETID_NOTE, 0x8B00) {
        x.note_color = u8::try_from(c).ok().filter(|c| *c < mailcraft_model::pim::NOTE_COLORS);
    }
    x.modified = p(0x3008).and_then(|v| v.time());
    x.sequence = match n(PSETID_APPOINTMENT, 0x8201) {
        Some(Val::Int(s)) => u32::try_from(s).unwrap_or(0),
        _ => 0,
    };
    if let Some(Val::Texts(names)) = named.keywords().and_then(p) {
        for name in names.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
            if x.categories.len() < MAX_CATEGORIES && !x.categories.iter().any(|c| c == name) {
                x.categories.push(name.to_string());
            }
        }
    }
    x.busy = match n(PSETID_APPOINTMENT, 0x8205) {
        Some(Val::Int(b)) => Some(b),
        _ => None,
    };
    x.sensitivity = match p(0x0036) {
        Some(Val::Int(s @ 1..=3)) => u8::try_from(s).unwrap_or(0),
        _ => 0,
    };
    // A reminder that's set: so many minutes ahead, or Outlook's 15 when it doesn't say.
    if matches!(n(PSETID_COMMON, 0x8503), Some(Val::Bool(true))) {
        x.reminder = match n(PSETID_COMMON, 0x8501) {
            Some(Val::Int(m)) if (0..=MAX_MINUTES).contains(&m) => Some(m),
            _ => Some(15),
        };
    }
    x.members = members(n(PSETID_ADDRESS, 0x8054), n(PSETID_ADDRESS, 0x8055));
    x.log_start = n(PSETID_LOG, 0x8706).and_then(|v| v.time());
    x.log_end = n(PSETID_LOG, 0x8708).and_then(|v| v.time()).or_else(|| match (x.log_start, n(PSETID_LOG, 0x8707)) {
        (Some(start), Some(Val::Int(minutes))) if (0..=MAX_MINUTES).contains(&minutes) => Some(start.saturating_add(minutes * 60)),
        _ => None,
    });
    x
}

/// A contact group's members that carry their address: each of its one-off members
/// (PidLidDistributionListOneOffMembers), and those of its members (PidLidDistributionListMembers)
/// whose wrapped entry id holds one; once each. A member that is only a contact of the file is
/// left out (its address is in that contact).
fn members(one_offs: Option<Val<'_>>, wrapped: Option<Val<'_>>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |found: Option<(String, String)>| {
        if let Some((name, address)) = found
            && out.len() < MAX_MEMBERS
            && !out.iter().any(|(_, a)| a.eq_ignore_ascii_case(&address))
        {
            out.push((name, address));
        }
    };
    if let Some(Val::Bins(list)) = one_offs {
        list.iter().for_each(|b| add(crate::pst::one_off_address(b)));
    }
    if let Some(Val::Bins(list)) = wrapped {
        // Flags, the provider, then a type whose low bits say what's wrapped (0: a one-off).
        for b in list {
            if b.get(4..20) == Some(&WRAPPED[..]) && b.get(20).is_some_and(|t| t & 0x0F == 0) {
                add(b.get(21..).and_then(crate::pst::one_off_address));
            }
        }
    }
    out
}

/// The contact group as a card (vCard 4's `KIND:group`, a `MEMBER` for each member) whose note
/// lists its members, above its own text.
pub(crate) fn group_of(x: &Extra, subject: &str, body: &str, uid: String) -> Card {
    let mut card = card_of(x, body, uid);
    if card.name.is_empty() {
        card.name = subject.trim().to_string();
    }
    let list: Vec<String> = x.members.iter().map(|(n, a)| if n.is_empty() { a.clone() } else { format!("{n} <{a}>") }).collect();
    card.note = [list.join("\n"), card.note].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
    card.extra.push("KIND:group".into());
    card.extra.extend(x.members.iter().map(|(_, a)| format!("MEMBER:mailto:{a}")));
    card
}

/// A journal entry as a note's text: its subject, when it was (in UTC), then its own text.
pub(crate) fn journal_text(x: &Extra, subject: &str, body: &str) -> String {
    let at = |t: i64| {
        let (y, m, d) = civil_from_days(t.div_euclid(86_400));
        let s = t.rem_euclid(86_400);
        (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}", s / 3600, s % 3600 / 60))
    };
    let when = x.log_start.map(|start| {
        let (day, time) = at(start);
        match x.log_end.filter(|e| *e > start).map(at) {
            Some((end_day, end)) if end_day == day => format!("{day} {time}–{end} UTC"),
            Some((end_day, end)) => format!("{day} {time} – {end_day} {end} UTC"),
            None => format!("{day} {time} UTC"),
        }
    });
    let body = body.replace("\r\n", "\n");
    [subject.trim().to_string(), when.unwrap_or_default(), body.trim().to_string()]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
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
        addresses: x.addresses.clone(),
        // Kept as the start of the day where it was set, in UTC.
        birthday: x
            .birthday
            .map(|t| {
                let (y, m, d) = civil_from_days((t + 12 * 3600 - 1).div_euclid(86_400));
                format!("{y:04}-{m:02}-{d:02}")
            })
            .unwrap_or_default(),
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
        sequence: x.sequence,
        categories: x.categories.clone(),
        ..Default::default()
    })
    .map(|mut e| {
        let show = match x.busy {
            Some(0) => Some(ShowAs::Free),
            Some(1) => Some(ShowAs::Tentative),
            Some(3) => Some(ShowAs::OutOfOffice),
            Some(4) => Some(ShowAs::WorkingElsewhere),
            _ => None,
        };
        if let Some(show) = show {
            ical::set_show_as(&mut e, show);
        }
        match x.sensitivity {
            1 => ical::set_personal(&mut e),
            2 => ical::set_privacy(&mut e, Privacy::Private),
            3 => ical::set_privacy(&mut e, Privacy::Confidential),
            _ => {}
        }
        if x.reminder.is_some() {
            ical::set_reminder(&mut e, x.reminder);
        }
        e
    })
}

/// A meeting request, cancellation or answer (`IPM.Schedule.Meeting.*`, of class `class`) as
/// the calendar it carried (RFC 6047): its method and its text, stamped when it was sent
/// (`date`). A request and a cancellation go from their organizer (`from`) to `guests`; an
/// answer from its guest (`from`) to the organizer (the first of `guests`). `None` for another
/// kind (a forward's notice) or one with no time.
pub(crate) fn invitation(
    x: &Extra,
    class: &str,
    subject: &str,
    body: &str,
    from: &EmailAddress,
    guests: &[EmailAddress],
    date: i64,
    uid: String,
) -> Option<(&'static str, String)> {
    let kind = class.to_ascii_lowercase();
    let kind = kind.strip_prefix("ipm.schedule.meeting.")?;
    let (method, answer) = if kind.starts_with("request") {
        ("REQUEST", None)
    } else if kind.starts_with("canceled") {
        ("CANCEL", None)
    } else if kind.starts_with("resp.pos") {
        ("REPLY", Some("ACCEPTED"))
    } else if kind.starts_with("resp.neg") {
        ("REPLY", Some("DECLINED"))
    } else if kind.starts_with("resp.tent") {
        ("REPLY", Some("TENTATIVE"))
    } else {
        return None;
    };
    let mut e = event_of(x, subject, body, from, guests, uid)?;
    e.dtstamp = date;
    if let Some(partstat) = answer {
        let person = |a: &EmailAddress| Attendee { email: a.email.clone(), name: a.name.clone(), ..Default::default() };
        e.organizer = guests.first().map(person);
        e.attendees = vec![Attendee { partstat: partstat.into(), ..person(from) }];
    } else if method == "CANCEL" {
        e.status = "CANCELLED".into();
    }
    Some((method, ical::write(&[e], Some(method), date)))
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
mod tests;
