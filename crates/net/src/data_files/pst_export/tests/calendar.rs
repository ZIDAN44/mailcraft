//! The calendar through a data file: exported as Outlook appointments in its Calendar folder, and
//! read back by MailCraft's import, field by field and occurrence by occurrence.

use super::*;
use mailcraft_model::dates::days_from_civil;
use mailcraft_model::ical::{self, Attendee, CalEvent, Privacy, ShowAs};
use mailcraft_model::pim::{Pim, StoredEvent};
use mailcraft_model::zone::Zone;
use mailcraft_pst::write;

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-cal-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Local wall-clock `y-m-d h:mi` in `zone`, as an instant.
fn at(zone: &str, y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
    let local = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60;
    Zone::named(zone).unwrap().to_utc(local)
}

fn person(email: &str, name: &str, partstat: &str, rsvp: bool) -> Attendee {
    Attendee { email: email.into(), name: name.into(), partstat: partstat.into(), rsvp }
}

/// The account's calendar: a single event with all Outlook keeps of one, an all-day event, a
/// weekly series in New York (across the change of summer time) with a moved, a retitled and a
/// deleted occurrence and one called off, an event in Tokyo and a daily series in Berlin, a
/// meeting with its organizer and guests, and an hourly series no pattern says.
fn calendar() -> Vec<CalEvent> {
    let ny = "America/New_York";
    let mut single = CalEvent {
        uid: "single@studio.example".into(),
        summary: "Screening — café".into(),
        description: "Bring the reels.\nAnd the notes.".into(),
        location: "Room 4".into(),
        start: 1_792_000_800,
        end: 1_792_006_200,
        categories: vec!["Film".into(), "Red".into()],
        sequence: 2,
        extra: vec![crate::pst_pim::event_file("agenda.txt", "text/plain", b"1. reels")],
        ..Default::default()
    };
    ical::set_reminder(&mut single, Some(30));
    ical::set_show_as(&mut single, ShowAs::Tentative);
    ical::set_privacy(&mut single, Privacy::Private);
    let all_day = CalEvent {
        uid: "fair@studio.example".into(),
        summary: "Book fair".into(),
        start: days_from_civil(2026, 11, 20) * 86_400,
        end: days_from_civil(2026, 11, 22) * 86_400,
        all_day: true,
        ..Default::default()
    };
    let mut weekly = CalEvent {
        uid: "weekly@studio.example".into(),
        summary: "Edit review".into(),
        location: "Suite B".into(),
        description: "Weekly.".into(),
        start: at(ny, 2026, 10, 6, 9, 30),
        end: at(ny, 2026, 10, 6, 10, 30),
        tzid: ny.into(),
        rrule: Some("FREQ=WEEKLY;BYDAY=TU,TH;COUNT=16".into()),
        exdates: vec![at(ny, 2026, 11, 10, 9, 30)],
        ..Default::default()
    };
    ical::set_reminder(&mut weekly, Some(10));
    let occurrence = |y, m, d| CalEvent { recurrence_id: Some(at(ny, y, m, d, 9, 30)), rrule: None, exdates: Vec::new(), ..weekly.clone() };
    // Moved by a day and an hour, after the clocks went back.
    let moved = CalEvent { start: at(ny, 2026, 11, 6, 14, 0), end: at(ny, 2026, 11, 6, 15, 30), ..occurrence(2026, 11, 5) };
    // Retitled, somewhere else, its own notes, no reminder, out of office.
    let mut retitled = CalEvent {
        summary: "Edit review — final cut".into(),
        location: "Theatre".into(),
        description: "Bring popcorn.".into(),
        ..occurrence(2026, 10, 15)
    };
    ical::set_reminder(&mut retitled, None);
    ical::set_show_as(&mut retitled, ShowAs::OutOfOffice);
    let called_off = CalEvent { status: "CANCELLED".into(), ..occurrence(2026, 11, 19) };
    let tokyo = CalEvent {
        uid: "tokyo@studio.example".into(),
        summary: "Call with Tokyo".into(),
        start: at("Asia/Tokyo", 2026, 12, 1, 18, 0),
        end: at("Asia/Tokyo", 2026, 12, 1, 19, 0),
        tzid: "Asia/Tokyo".into(),
        ..Default::default()
    };
    let berlin = CalEvent {
        uid: "berlin@studio.example".into(),
        summary: "Stand-up".into(),
        start: at("Europe/Berlin", 2026, 10, 20, 9, 0),
        end: at("Europe/Berlin", 2026, 10, 20, 9, 15),
        tzid: "Europe/Berlin".into(),
        rrule: Some("FREQ=DAILY;INTERVAL=2;UNTIL=20261115T235959Z".into()),
        ..Default::default()
    };
    let meeting = CalEvent {
        uid: "040000008200E00074C5B7101A82E00800000000A0B1C2D3E4F5A6010000000000000000100000004F1A2B3C4D5E6F708192A3B4C5D6E7F8".into(),
        summary: "Budget".into(),
        start: 1_793_000_000,
        end: 1_793_003_600,
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![
            person("ana@studio.example", "Ana Lima", "ACCEPTED", false),
            person("ben@studio.example", "Ben", "DECLINED", false),
            person("cleo@posteo.example", "", "NEEDS-ACTION", true),
        ],
        ..Default::default()
    };
    let hourly = CalEvent {
        uid: "hourly@studio.example".into(),
        summary: "Render check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;INTERVAL=2;COUNT=5".into()),
        exdates: vec![1_794_000_000 + 2 * 7200],
        ..Default::default()
    };
    vec![single, all_day, weekly, moved, retitled, called_off, tokyo, berlin, meeting, hourly]
}

/// What an event is, as the import can give it back: its uid, times, zone, text, place, people,
/// how it shows its time, privacy, reminder, categories, files, and its occurrences.
fn shape(e: &CalEvent) -> String {
    let files: Vec<&String> = e.extra.iter().filter(|l| crate::pst_pim::is_file_line(l)).collect();
    format!(
        "{} {:?} {}–{} all_day={} tz={} {:?} {:?} {:?} org={:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        e.uid,
        e.recurrence_id,
        e.start,
        e.end,
        e.all_day,
        e.tzid,
        e.summary,
        e.description,
        e.location,
        e.organizer.as_ref().map(|o| &o.email),
        e.attendees,
        ical::show_as(e),
        ical::privacy(e),
        ical::reminder_minutes(e),
        e.categories,
        files,
        e.status,
        e.sequence,
    )
}

/// Every occurrence of `events` over three years from the first, as (uid, start, end, title).
fn occurrences(events: &[CalEvent]) -> Vec<(String, i64, i64, String)> {
    let from = 1_780_000_000;
    let mut out: Vec<_> = ical::occurrences(events, from, from + 3 * 366 * 86_400)
        .into_iter()
        .map(|o| (events[o.event].uid.clone(), o.start, o.end, events[o.event].summary.clone()))
        .collect();
    out.sort();
    out
}

#[test]
fn the_calendar_round_trips_inside_a_data_file() {
    use mailcraft_pst::PropertyValue as V;
    let dir = scratch("round-trip");
    let store = Store::open(dir.join("cache")).unwrap();
    let events = calendar();
    let pim = Pim { events: events.iter().map(|e| StoredEvent { event: e.clone(), ..Default::default() }).collect(), ..Default::default() };
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Calendar.pst");
    let done = export(&store, "a1", "Calendar", &file, NONE, &mut |_| {}).unwrap();
    // Six appointments (a series one, its moved and retitled occurrences inside it, the one
    // called off deleted), and the hourly series' four occurrences, each one of its own.
    assert_eq!((done.events, done.occurrences), (10, 4), "{done:?}");
    assert!(!dir.join("Calendar Calendar.ics").exists());
    assert_eq!(write::special_folder(&file, Special::Calendar).unwrap(), Some(vec!["Calendar".to_string()]));

    // As Outlook keeps them.
    let appt = |lid| named_id(&file, 0x0006_2002, lid).unwrap();
    let got = props_of(&file, &["Calendar"], "Edit review", &[0x001A, appt(0x8223), appt(0x8231), appt(0x8234), appt(0x8216), 0x0E1B]);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Appointment"));
    assert!(matches!(got[1], Some(V::Boolean(true))) && matches!(got[2], Some(V::Integer32(2))), "{got:?}");
    assert_eq!(got[3].as_ref().and_then(crate::pst::text_of).as_deref(), Some("America/New_York"));
    // Writer version 0x3009, two changed occurrences.
    let Some(V::Binary(p)) = &got[4] else { panic!("{got:?}") };
    assert!(p.buffer().windows(4).any(|w| w == 0x3009u32.to_le_bytes()));
    assert!(matches!(got[5], Some(V::Boolean(true))), "has its changed occurrences attached");
    let meeting = |lid| named_id(&file, 0x6ED8_DA90, lid).unwrap();
    let got = props_of(&file, &["Calendar"], "Screening — café", &[meeting(0x0003), 0x0036, appt(0x8205), appt(0x8215)]);
    let Some(V::Binary(id)) = &got[0] else { panic!("{got:?}") };
    assert!(id.buffer().windows(8).any(|w| w == b"vCal-Uid"));
    assert!(matches!(got[1..], [Some(V::Integer32(2)), Some(V::Integer32(1)), Some(V::Boolean(false))]), "{got:?}");

    // Read back by the import.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.skipped, imported.skips.len()), (0, 0), "{imported:?}");
    let got: Pim = back.load_account_doc("b1", mailcraft_model::pim::PIM_DOC).unwrap();
    let have: Vec<CalEvent> = got.events.iter().map(|e| e.event.clone()).collect();
    assert_eq!(have.len(), 12, "{have:#?}");
    // Each event as it went out (the occurrence called off is a deleted one now, as Outlook keeps
    // it; the hourly series is its occurrences, each with a uid of its own).
    let by_uid = |uid: &str, rid: Option<i64>| {
        have.iter().find(|e| e.uid == uid && e.recurrence_id == rid).unwrap_or_else(|| panic!("{uid} {rid:?}: {have:#?}"))
    };
    for e in events.iter().filter(|e| !e.uid.starts_with("hourly") && e.status != "CANCELLED") {
        let h = by_uid(&e.uid, e.recurrence_id);
        let mut want = e.clone();
        if want.uid.starts_with("weekly") && want.recurrence_id.is_none() {
            want.exdates.push(at("America/New_York", 2026, 11, 19, 9, 30));
        }
        assert_eq!(shape(h), shape(&want), "{}", e.summary);
        if let Some(rule) = &e.rrule {
            let mut ex = want.exdates.clone();
            let mut back_ex = h.exdates.clone();
            ex.sort_unstable();
            back_ex.sort_unstable();
            assert_eq!(back_ex, ex, "{rule}");
            assert_eq!(ical::starts(h, i64::MAX), ical::starts(e, i64::MAX), "{rule} → {:?}", h.rrule);
        }
    }
    // Every occurrence, over three years, where it was.
    let mut want = events.clone();
    for e in want.iter_mut().filter(|e| e.uid.starts_with("weekly") && e.recurrence_id.is_none()) {
        e.exdates.push(at("America/New_York", 2026, 11, 19, 9, 30));
    }
    want.retain(|e| e.status != "CANCELLED");
    let strip = |o: Vec<(String, i64, i64, String)>| {
        o.into_iter().map(|(uid, s, e, t)| (uid.split('/').next().unwrap_or_default().to_string(), s, e, t)).collect::<Vec<_>>()
    };
    assert_eq!(strip(occurrences(&have)), strip(occurrences(&want)));
    let hourly: Vec<&CalEvent> = have.iter().filter(|e| e.uid.starts_with("hourly@studio.example/")).collect();
    assert_eq!(hourly.len(), 4);
    assert!(hourly.iter().all(|e| e.rrule.is_none() && e.summary == "Render check"));

    // Exported again into the same file: nothing goes in twice.
    let again = export(&store, "a1", "Calendar", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.events, again.occurrences, again.pim_already), (0, 0, 10), "{again:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A meeting's request carries the meeting's own properties as its appointment has them: the
/// class Outlook gives meeting mail, the global object id, times, place and pattern, and, the
/// account being a guest, that it was received and isn't answered yet.
#[test]
fn meeting_mail_carries_its_meeting() {
    use mailcraft_pst::PropertyValue as V;
    let dir = scratch("meeting-mail");
    let store = Store::open(dir.join("cache")).unwrap();
    let mut invite = CalEvent {
        uid: "review@studio.example".into(),
        summary: "Review".into(),
        location: "Suite B".into(),
        start: at("Europe/Berlin", 2026, 11, 3, 10, 0),
        end: at("Europe/Berlin", 2026, 11, 3, 11, 0),
        tzid: "Europe/Berlin".into(),
        rrule: Some("FREQ=WEEKLY;COUNT=4".into()),
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![person("ana@studio.example", "Ana", "NEEDS-ACTION", true)],
        ..Default::default()
    };
    ical::set_reminder(&mut invite, Some(15));
    let moved = CalEvent {
        recurrence_id: Some(at("Europe/Berlin", 2026, 11, 10, 10, 0)),
        start: at("Europe/Berlin", 2026, 11, 10, 12, 0),
        end: at("Europe/Berlin", 2026, 11, 10, 13, 0),
        rrule: None,
        ..invite.clone()
    };
    let ics = ical::write(&[invite.clone(), moved], Some("REQUEST"), 1_790_000_000);
    let reply = CalEvent { attendees: vec![person("ana@studio.example", "Ana", "ACCEPTED", false)], ..invite.clone() };
    let answer = ical::write(&[reply], Some("REPLY"), 1_790_000_000);
    let mk = |uid: u32, subject: &str, ics: &str, method: &str| {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@studio.example")],
            subject: subject.into(),
            body: "Join us.".into(),
            attachments: vec![mime::OutAttachment {
                name: "invite.ics".into(),
                mime: format!("text/calendar; method={method}"),
                data: ics.as_bytes().to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@studio.example"), 1_790_000_000, &format!("m{uid}@studio.example")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let inbox = FolderState {
        validity: 1,
        uid_next: 4,
        messages: vec![mk(1, "Review", &ics, "REQUEST").into(), mk(2, "Accepted: Review", &answer, "REPLY").into(), mk(3, "Plain", "", "x").into()],
        ..Default::default()
    };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let folder =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    let account = mailcraft_model::AccountConfig { id: "a1".into(), email: "ana@studio.example".into(), ..Default::default() };
    store.save_accounts(&[account]).unwrap();
    let file = dir.join("Mail.pst");
    let done = export(&store, "a1", "Mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 3);
    let appt = |lid| named_id(&file, 0x0006_2002, lid).unwrap();
    let meeting = |lid| named_id(&file, 0x6ED8_DA90, lid).unwrap();
    let common = |lid| named_id(&file, 0x0006_2008, lid).unwrap();
    let ids = [0x001A, meeting(0x0003), appt(0x820D), appt(0x8208), appt(0x8216), appt(0x8234), common(0x8501), meeting(0x0024)];
    let got = props_of(&file, &["Inbox"], "Review", &ids);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Schedule.Meeting.Request"));
    let Some(V::Binary(id)) = &got[1] else { panic!("{got:?}") };
    assert_eq!(crate::pst_pim::uid_of(id.buffer()), "review@studio.example");
    assert!(matches!(got[2], Some(V::Time(t)) if t == item::filetime(invite.start)), "{got:?}");
    assert_eq!(got[3].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Suite B"));
    let Some(V::Binary(p)) = &got[4] else { panic!("{got:?}") };
    let (rule, _) = crate::pst_pim::recurrence_in(p.buffer(), invite.start, "Europe/Berlin").unwrap();
    assert_eq!(rule, "FREQ=WEEKLY;INTERVAL=1;BYDAY=TU;COUNT=4");
    assert_eq!(got[5].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Europe/Berlin"));
    assert!(matches!(got[6], Some(V::Integer32(15))));
    assert_eq!(got[7].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Appointment"));
    let class = |subject: &str| props_of(&file, &["Inbox"], subject, &[0x001A]).remove(0).as_ref().and_then(crate::pst::text_of);
    assert_eq!(class("Accepted: Review").as_deref(), Some("IPM.Schedule.Meeting.Resp.Pos"));
    assert_eq!(class("Plain").as_deref(), Some("IPM.Note"));
    // Received (asfMeeting | asfReceived), not answered yet; its times, place, and when it changed.
    let ids = [appt(0x8217), appt(0x8218), 0x0060, 0x0061, 0x0063, meeting(0x0002), meeting(0x0001), meeting(0x001A)];
    let got = props_of(&file, &["Inbox"], "Review", &ids);
    assert!(matches!(got[..2], [Some(V::Integer32(3)), Some(V::Integer32(5))]), "{got:?}");
    assert!(matches!(got[2], Some(V::Time(t)) if t == item::filetime(invite.start)), "{got:?}");
    assert!(matches!(got[3], Some(V::Time(t)) if t == item::filetime(invite.end)), "{got:?}");
    assert!(matches!(got[4], Some(V::Boolean(true))), "{got:?}");
    assert_eq!(got[5].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Suite B"));
    assert!(matches!(got[6..], [Some(V::Time(a)), Some(V::Time(b))] if a == b && a == item::filetime(1_790_000_000)), "{got:?}");
    // The guest's answer to it: the guest's own, accepted.
    let got = props_of(&file, &["Inbox"], "Accepted: Review", &[appt(0x8217), appt(0x8218)]);
    assert!(matches!(got[..], [Some(V::Integer32(3)), Some(V::Integer32(3))]), "{got:?}");

    // Read back, the request still carries its calendar.
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    let review = inbox.messages.iter().find(|m| m.subject == "Review").unwrap();
    let raw = back.read_body(&review.key).unwrap();
    let p = mime::parse(&raw).unwrap();
    let part = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar")).unwrap();
    let cal = ical::parse(&mime::attachment_text(&raw, part.index).unwrap(), 0);
    assert_eq!(cal.method.as_deref(), Some("REQUEST"));
    assert_eq!(cal.events.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

use crate::data_files::pst_export::calendar::{self as cal, Owner};
use crate::data_files::pst_export::pim::{Item, Key, Value};

/// A named property of an item, by its number.
fn lid_of(item: &Item, id: u32) -> Option<&Value> {
    item.props.iter().find_map(|(k, v)| matches!(k, Key::Lid(_, i) if *i == id).then_some(v))
}

fn int_of(item: &Item, id: u32) -> Option<i32> {
    match lid_of(item, id) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// A series no pattern says goes in one occurrence at a time: its files on the first only, not
/// copied into each (a 100 kB file on 2,000 hours was 200 MB held at once, and in the file).
#[test]
fn a_series_one_by_one_carries_its_files_once() {
    let data = vec![7u8; 100_000];
    let e = CalEvent {
        uid: "render@studio.example".into(),
        summary: "Render check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;COUNT=2000".into()),
        extra: vec![crate::pst_pim::event_file("cut.bin", "application/octet-stream", &data), "X-KEPT:1".into()],
        ..Default::default()
    };
    let items = cal::items(&[&e], 0, &Owner::default());
    assert_eq!(items.len(), 2000);
    assert!(items.iter().all(|(_, i)| i.occurrence));
    let held: usize = items.iter().flat_map(|(_, i)| i.files.iter().map(|f| f.data.len())).sum();
    assert_eq!(held, data.len());
    assert_eq!(items[0].1.files.len(), 1);
}

/// Changed occurrences of a series written one occurrence at a time that no start written takes
/// (one the rule doesn't make, or of a rule that makes none) go in on their own, as the pattern's
/// leftovers do; one called off adds nothing.
#[test]
fn changed_occurrences_no_start_takes_go_in_on_their_own() {
    let e = CalEvent {
        uid: "check@studio.example".into(),
        summary: "Check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;COUNT=3".into()),
        ..Default::default()
    };
    let changed = |rid: i64, summary: &str, status: &str| CalEvent {
        recurrence_id: Some(rid),
        start: rid + 600,
        end: rid + 1200,
        summary: summary.into(),
        status: status.into(),
        rrule: None,
        ..e.clone()
    };
    let moved = changed(e.start + 1800, "Moved", "");
    let taken = changed(e.start + 3600, "Later", "");
    let off = changed(e.start + 5400, "Off", "CANCELLED");
    let items = cal::items(&[&e, &moved, &taken, &off], 0, &Owner::default());
    let titles: Vec<(&str, &str)> = items.iter().map(|(u, i)| (u.as_str(), i.subject.as_str())).collect();
    let uid = |t: i64| format!("check@studio.example/{t}");
    assert_eq!(titles.len(), 4, "{titles:?}");
    assert!(titles.contains(&(uid(e.start + 3600).as_str(), "Later")), "{titles:?}");
    assert!(titles.contains(&(uid(e.start + 1800).as_str(), "Moved")), "{titles:?}");
    assert!(!titles.iter().any(|(_, t)| *t == "Off"));
    // A rule that makes no day: the event as it starts, and its changed occurrence.
    let never = CalEvent { rrule: Some("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30".into()), ..e.clone() };
    let items = cal::items(&[&never, &moved], 0, &Owner::default());
    let titles: Vec<&str> = items.iter().map(|(_, i)| i.subject.as_str()).collect();
    assert_eq!(titles, ["Check", "Moved"]);
}

/// A meeting someone else organizes is one the user received (asfReceived), with the user's own
/// answer; one the account's address organizes is the user's own (respOrganized); a guest's
/// copy is received whoever's addresses are known.
#[test]
fn a_meeting_someone_else_organizes_is_received() {
    let meeting = CalEvent {
        uid: "budget@studio.example".into(),
        summary: "Budget".into(),
        start: 1_793_000_000,
        end: 1_793_003_600,
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![person("ana@studio.example", "Ana", "ACCEPTED", false), person("ben@studio.example", "Ben", "NEEDS-ACTION", true)],
        ..Default::default()
    };
    let state = |owner: &Owner, e: &CalEvent| {
        let items = cal::items(&[e], 0, owner);
        (int_of(&items[0].1, 0x8217), int_of(&items[0].1, 0x8218))
    };
    let me = |a: &str| Owner { me: vec![a.into()], ..Default::default() };
    assert_eq!(state(&me("ANA@studio.example"), &meeting), (Some(3), Some(3)));
    assert_eq!(state(&me("ben@studio.example"), &meeting), (Some(3), Some(5)));
    // Not among the guests either: not answered yet.
    assert_eq!(state(&me("zoe@studio.example"), &meeting), (Some(3), Some(5)));
    assert_eq!(state(&me("riley@studio.example"), &meeting), (Some(1), Some(1)));
    // Nothing known of the account: as the user's own, unless the calendar keeps it as a guest.
    assert_eq!(state(&Owner::default(), &meeting), (Some(1), Some(1)));
    let guest = Owner { guest: vec!["budget@studio.example".into()], ..Default::default() };
    assert_eq!(state(&guest, &meeting), (Some(3), Some(5)));
    let mut copy = meeting.clone();
    ical::mark_guest_copy(&mut copy);
    assert_eq!(state(&Owner::default(), &copy), (Some(3), Some(5)));
    // Not a meeting: neither.
    let alone = CalEvent { attendees: Vec::new(), ..meeting.clone() };
    assert_eq!(state(&me("ana@studio.example"), &alone), (Some(0), None));
}

/// An all-day event starts and ends at this computer's midnights, in its zone, as Outlook keeps
/// one; read back by the import, it and a series of them keep their days and deleted days.
#[test]
fn all_day_at_this_computers_midnights() {
    let ny = Zone::named("America/New_York").unwrap();
    let owner = Owner { zone: ny, ..Default::default() };
    let day = days_from_civil(2026, 11, 20) * 86_400;
    let e = CalEvent {
        uid: "fair@studio.example".into(),
        summary: "Fair".into(),
        start: day,
        end: day + 86_400,
        all_day: true,
        rrule: Some("FREQ=DAILY;COUNT=4".into()),
        exdates: vec![day + 86_400],
        ..Default::default()
    };
    let items = cal::items(&[&e], 0, &owner);
    let item = &items[0].1;
    let time = |id| match lid_of(item, id) {
        Some(Value::Time(t)) => *t,
        _ => panic!("{id:x}"),
    };
    assert_eq!(time(0x820D), item::filetime(ny.to_utc(day)));
    assert_eq!(time(0x820E), item::filetime(ny.to_utc(day + 86_400)));
    assert_eq!(time(0x8235), item::filetime(ny.to_utc(day)), "the series' first day, from its local midnight");
    assert!(matches!(lid_of(item, 0x8234), Some(Value::Text(t)) if t == "America/New_York"));
    let Some(Value::Bin(blob)) = lid_of(item, 0x8216) else { panic!("a pattern") };
    // Read back as Outlook's own would be.
    let x = crate::pst_pim::Extra {
        start: Some(ny.to_utc(day)),
        end: Some(ny.to_utc(day + 86_400)),
        all_day: true,
        recurrence: blob.clone(),
        ..Default::default()
    };
    let back = crate::pst_pim::event_of(&x, "Fair", "", &EmailAddress::default(), &[], "fair@studio.example".into()).unwrap();
    assert_eq!((back.start, back.end, back.all_day), (day, day + 86_400, true));
    assert_eq!(back.exdates, [day + 86_400]);
    assert_eq!(ical::starts(&back, i64::MAX), ical::starts(&e, i64::MAX));
    // With no zone known, UTC as before.
    let utc = cal::items(&[&e], 0, &Owner::default());
    assert!(matches!(lid_of(&utc[0].1, 0x820D), Some(Value::Time(t)) if *t == item::filetime(day)));
}

/// A rule at set hours and one with added dates go in one occurrence at a time, at the hours and
/// dates they make (three a day for three hours, not one every hour).
#[test]
fn hours_and_added_dates_go_in_as_their_occurrences() {
    let day = days_from_civil(2026, 11, 2) * 86_400;
    let hours = CalEvent {
        uid: "h@studio.example".into(),
        start: day + 9 * 3600,
        end: day + 9 * 3600 + 600,
        rrule: Some("FREQ=HOURLY;BYHOUR=9,10,11;COUNT=6".into()),
        ..Default::default()
    };
    let starts = |e: &CalEvent| {
        cal::items(&[e], 0, &Owner::default())
            .iter()
            .map(|(_, i)| match lid_of(i, 0x820D) {
                Some(Value::Time(t)) => (*t - item::filetime(day)) / 36_000_000_000,
                _ => -1,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(starts(&hours), [9, 10, 11, 33, 34, 35]);
    let added = CalEvent { uid: "r@studio.example".into(), rrule: None, rdates: vec![day + 30 * 3600, day + 50 * 3600], ..hours.clone() };
    assert_eq!(starts(&added), [9, 30, 50]);
}

/// The pattern the export writes for `e` (none when it goes in one occurrence at a time).
fn written(e: &CalEvent) -> Option<Vec<u8>> {
    crate::data_files::pst_export::calendar::series(e, &[], &Default::default(), false)?.into_iter().find_map(|(k, v)| match (k, v) {
        (crate::data_files::pst_export::pim::Key::Lid(_, 0x8216), crate::data_files::pst_export::pim::Value::Bin(b)) => Some(b),
        _ => None,
    })
}

/// A rule of a kind Outlook's pattern says, as RFC 5545 writes it.
fn rule_strategy() -> impl proptest::strategy::Strategy<Value = (String, bool)> {
    use proptest::prelude::*;
    let days = proptest::sample::subsequence(vec!["SU", "MO", "TU", "WE", "TH", "FR", "SA"], 1..=7).prop_map(|d| d.join(","));
    let nth = prop_oneof![Just(-1i32), 1..=4i32];
    let weekday = proptest::sample::select(vec!["SU", "MO", "TU", "WE", "TH", "FR", "SA"]);
    let end = prop_oneof![Just(String::new()), (1u32..40).prop_map(|n| format!(";COUNT={n}")), (1i64..700).prop_map(|d| format!(";UNTIL=+{d}"))];
    let kind = prop_oneof![
        (1u32..5).prop_map(|i| (format!("FREQ=DAILY;INTERVAL={i}"), true)),
        (1u32..4, days, proptest::sample::select(vec!["", ";WKST=MO", ";WKST=SU"]))
            .prop_map(|(i, d, w)| (format!("FREQ=WEEKLY;INTERVAL={i};BYDAY={d}{w}"), true)),
        (1u32..4, prop_oneof![Just(-1i32), 1..=28i32]).prop_map(|(i, d)| (format!("FREQ=MONTHLY;INTERVAL={i};BYMONTHDAY={d}"), false)),
        (1u32..4, nth.clone(), weekday.clone()).prop_map(|(i, n, w)| (format!("FREQ=MONTHLY;INTERVAL={i};BYDAY={n}{w}"), false)),
        (1u32..3).prop_map(|i| (format!("FREQ=YEARLY;INTERVAL={i}"), false)),
        (1u32..3, nth, weekday).prop_map(|(i, n, w)| (format!("FREQ=YEARLY;INTERVAL={i};BYMONTH=M;BYDAY={n}{w}"), false)),
    ];
    (kind, end).prop_map(|((k, by_day), e)| (format!("{k}{e}"), by_day))
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(400))]
    /// Any rule Outlook's pattern says, in a zone with summer time or none, all day or not, with
    /// occurrences deleted: written as the export writes it and read back by the import
    /// (`pst_pim::recur`), the series has the same occurrences over three years.
    #[test]
    fn a_rule_written_and_read_back_keeps_its_occurrences(
        (rule, _) in rule_strategy(),
        day in 0i64..1000,
        minute in 0i64..(24 * 4),
        zone in proptest::sample::select(vec!["", "America/New_York", "Europe/Berlin", "Australia/Sydney", "Asia/Kolkata"]),
        all_day in proptest::prelude::any::<bool>(),
        deleted in proptest::collection::vec(0usize..40, 0..4),
    ) {
        let first = days_from_civil(2025, 1, 1) + day;
        let (_, month, _) = mailcraft_model::dates::civil_from_days(first);
        let rule = rule.replace("BYMONTH=M", &format!("BYMONTH={month}"));
        // An end on a day after the first.
        let rule = match rule.split_once(";UNTIL=+") {
            Some((head, days)) => {
                let (y, m, d) = mailcraft_model::dates::civil_from_days(first + days.parse::<i64>().unwrap());
                format!("{head};UNTIL={y:04}{m:02}{d:02}T235959Z")
            }
            None => rule,
        };
        let (start, tzid) = if all_day || zone.is_empty() {
            (first * 86_400 + if all_day { 0 } else { minute * 900 }, String::new())
        } else {
            (Zone::named(zone).unwrap().to_utc(first * 86_400 + minute * 900), zone.to_string())
        };
        let mut e = CalEvent { uid: "p".into(), start, end: start + if all_day { 86_400 } else { 1800 }, all_day, tzid, rrule: Some(rule.clone()), ..Default::default() };
        let all = ical::starts(&e, i64::MAX);
        e.exdates = deleted.iter().filter_map(|i| all.get(*i).copied()).collect();
        // A day Outlook keeps every month (the 1st to the 28th, or the last) and the rules above
        // are all said by the pattern.
        let blob = written(&e);
        proptest::prop_assert!(blob.is_some(), "{} at {}", rule, e.start);
        let blob = blob.unwrap();
        let tz = if e.all_day { String::new() } else { e.tzid.clone() };
        let (back_rule, exdates) = crate::pst_pim::recurrence_in(&blob, e.start, &tz).unwrap();
        let back = CalEvent { rrule: Some(back_rule.clone()), exdates, ..e.clone() };
        let window = |x: &CalEvent| ical::occurrences(std::slice::from_ref(x), e.start, e.start + 3 * 366 * 86_400).iter().map(|o| o.start).collect::<Vec<_>>();
        proptest::prop_assert_eq!(window(&back), window(&e), "{} → {}", rule, back_rule);
    }
}
