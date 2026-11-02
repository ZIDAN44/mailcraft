use super::recur::pattern;
use super::*;
use mailcraft_model::dates::days_from_civil;

/// Names in the string stream are found by their offset; a damaged offset or length is no name.
#[test]
fn names_in_the_string_stream() {
    let mut stream = vec![0xAA, 0xBB];
    for name in ["Keywords", "x-custom"] {
        let bytes: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        stream.extend((bytes.len() as u32).to_le_bytes());
        stream.extend(bytes);
    }
    assert_eq!(name_at(&stream, 2).as_deref(), Some("Keywords"));
    assert_eq!(name_at(&stream, 22).as_deref(), Some("x-custom"));
    for at in [0, 1, 3, 21, 23, 1000, u32::MAX] {
        let _ = name_at(&stream, at);
    }
    assert_eq!(name_at(&stream, 39), None, "past the end");
    let huge = [0xFF, 0xFF, 0xFF, 0xFF, b'a', 0];
    assert_eq!(name_at(&huge, 0), None);
}

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

/// A named start or end of a type that is not a time leaves the event without one: it does not
/// fall back to the plain start and end properties.
#[test]
fn a_mistyped_named_time_hides_the_fallback() {
    let ids = HashMap::from([((PSETID_APPOINTMENT, 0x820D), 0x8001), ((PSETID_APPOINTMENT, 0x820E), 0x8002)]);
    let named = Named { ids, keywords: None };
    let props = HashMap::from([(0x8001, Val::Other), (0x8002, Val::Time(133_485_408_000_000_000)), (0x0060, Val::Time(133_485_408_000_000_000))]);
    let x = extract(&|id| props.get(&id).cloned(), &named);
    assert_eq!((x.start, x.end), (None, Some(1_704_067_200)));
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
    let x = Extra { start: Some(day - 6 * 3600), end: Some(day + 86_400 - 6 * 3600), all_day: true, location: "Hall".into(), ..Default::default() };
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

/// A one-off entry id of `address`, in UTF-16.
fn one_off(name: &str, address: &str) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02, 0, 0, 0, 0x80];
    for s in [name, "SMTP", address] {
        b.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    }
    b
}

/// A contact group's members come from its one-off members and from the members whose wrapped
/// entry id holds one, once each; a member that is a contact of the file, or junk, is left out.
#[test]
fn a_contact_group_s_members() {
    let ana = one_off("Ana", "ana@posteo.example");
    let ben = one_off("", "ben@studio.example");
    let wrap = |kind: u8, inner: &[u8]| [&[0, 0, 0, 0][..], &WRAPPED, &[kind], inner].concat();
    let wrapped_ben = wrap(0xC0, &ben);
    let wrapped_ana = wrap(0x00, &ana);
    let contact = wrap(0xC3, &[1, 2, 3, 4]);
    let ids = HashMap::from([((PSETID_ADDRESS, 0x8054), 0x8001), ((PSETID_ADDRESS, 0x8055), 0x8002)]);
    let named = Named { ids, keywords: None };
    let props = HashMap::from([
        (0x8001, Val::Bins(vec![&ana[..], &[0xFF; 3][..]])),
        (0x8002, Val::Bins(vec![&wrapped_ana[..], &wrapped_ben[..], &contact[..], &[][..]])),
    ]);
    let x = extract(&|id| props.get(&id).cloned(), &named);
    assert_eq!(x.members, [("Ana".to_string(), "ana@posteo.example".to_string()), (String::new(), "ben@studio.example".to_string())]);
    // Cut anywhere, no crash: before its one-off's address, no member.
    for n in 0..wrapped_ben.len() {
        let got = members(None, Some(Val::Bins(vec![&wrapped_ben[..n]])));
        assert!(n >= 21 + 24 || got.is_empty());
    }
    assert!(members(Some(Val::Bin(&ana)), Some(Val::Other)).is_empty(), "single values aren't lists");
}

/// An appointment keeps its categories, how it shows its time, its privacy and its reminder.
#[test]
fn an_event_keeps_its_categories_show_as_privacy_and_reminder() {
    let ids = HashMap::from([
        ((PSETID_APPOINTMENT, 0x820D), 0x8001),
        ((PSETID_APPOINTMENT, 0x8205), 0x8002),
        ((PSETID_COMMON, 0x8503), 0x8003),
        ((PSETID_COMMON, 0x8501), 0x8004),
        ((PSETID_APPOINTMENT, 0x8201), 0x8005),
    ]);
    let named = Named { ids, keywords: Some(0x8006) };
    let event = |busy: i64, sensitivity: i64, reminder: Option<i64>| {
        let mut props = HashMap::from([
            (0x8001, Val::Time(133_485_408_000_000_000)),
            (0x8002, Val::Int(busy)),
            (0x0036, Val::Int(sensitivity)),
            (0x8005, Val::Int(3)),
            (0x8006, Val::Texts(vec!["Print".into(), " ".into(), "Print".into(), "Studio".into()])),
        ]);
        if let Some(m) = reminder {
            props.insert(0x8003, Val::Bool(true));
            props.insert(0x8004, Val::Int(m));
        }
        let x = extract(&|id| props.get(&id).cloned(), &named);
        event_of(&x, "Press check", "", &EmailAddress::default(), &[], "u".into()).unwrap()
    };
    let e = event(0, 2, Some(30));
    assert_eq!(e.categories, ["Print", "Studio"]);
    assert_eq!(e.sequence, 3);
    assert_eq!(ical::show_as(&e), ShowAs::Free);
    assert_eq!(ical::privacy(&e), Privacy::Private);
    assert_eq!(ical::reminder_minutes(&e), Some(30));
    let e = event(3, 3, None);
    assert_eq!((ical::show_as(&e), ical::privacy(&e), ical::reminder_minutes(&e)), (ShowAs::OutOfOffice, Privacy::Confidential, None));
    let e = event(2, 1, Some(-5));
    assert!(ical::personal(&e));
    assert_eq!((ical::show_as(&e), ical::reminder_minutes(&e)), (ShowAs::Busy, Some(15)), "a reminder that makes no sense is Outlook's own");
    let written = ical::write(&[event(1, 0, Some(10))], None, 0);
    assert!(written.contains("BEGIN:VALARM") && written.contains("CATEGORIES:Print,Studio"), "{written}");
}

/// A journal entry's text: its subject, its time in UTC (across midnight too), its own text.
#[test]
fn a_journal_entry_s_text() {
    let day = days_from_civil(2026, 10, 20) * 86_400;
    let x = Extra { log_start: Some(day + 23 * 3600), log_end: Some(day + 25 * 3600), ..Default::default() };
    assert_eq!(journal_text(&x, "Night run", "Done.\r\n"), "Night run\n2026-10-20 23:00 – 2026-10-21 01:00 UTC\nDone.");
    let x = Extra { log_start: Some(day), ..Default::default() };
    assert_eq!(journal_text(&x, "", ""), "2026-10-20 00:00 UTC");
    assert_eq!(journal_text(&Extra::default(), "", " "), "");
}

/// A changed occurrence as an appointment's pattern writes it (ExceptionInfo), and what its
/// ExtendedException repeats in UTF-16.
#[derive(Default)]
struct Info {
    start: u32,
    end: u32,
    original: u32,
    subject: Option<(&'static [u8], &'static str)>,
    location: Option<(&'static [u8], &'static str)>,
    busy: Option<u32>,
    reminder: Option<(u32, u32)>,
    all_day: Option<u32>,
    body: bool,
}

/// `base` (a pattern up to its start offset, from [`blob`]) with these changed occurrences, as a
/// writer of version `writer` writes them.
fn with_changes(base: &[u8], writer: u32, infos: &[Info]) -> Vec<u8> {
    let mut v = base.to_vec();
    let at = v.len() - 8;
    v.splice(at..at + 4, writer.to_le_bytes());
    v.extend(600u32.to_le_bytes());
    v.extend(u16::try_from(infos.len()).unwrap().to_le_bytes());
    let text = |v: &mut Vec<u8>, b: &[u8]| {
        v.extend(u16::try_from(b.len() + 1).unwrap().to_le_bytes());
        v.extend(u16::try_from(b.len()).unwrap().to_le_bytes());
        v.extend(b);
    };
    let flags = |i: &Info| {
        let mut f = 0u16;
        for (on, bit) in [
            (i.subject.is_some(), 0x1),
            (i.reminder.is_some(), 0x4 | 0x8),
            (i.location.is_some(), 0x10),
            (i.busy.is_some(), 0x20),
            (i.all_day.is_some(), 0x80),
            (i.body, 0x200),
        ] {
            if on {
                f |= bit;
            }
        }
        f
    };
    for i in infos {
        for w in [i.start, i.end, i.original] {
            v.extend(w.to_le_bytes());
        }
        v.extend(flags(i).to_le_bytes());
        if let Some((b, _)) = i.subject {
            text(&mut v, b);
        }
        if let Some((set, delta)) = i.reminder {
            v.extend(delta.to_le_bytes());
            v.extend(set.to_le_bytes());
        }
        if let Some((b, _)) = i.location {
            text(&mut v, b);
        }
        for w in [i.busy, i.all_day].into_iter().flatten() {
            v.extend(w.to_le_bytes());
        }
    }
    v.extend(0u32.to_le_bytes());
    for i in infos {
        if writer >= 0x3009 {
            v.extend(4u32.to_le_bytes());
            v.extend(0x7u32.to_le_bytes());
        }
        v.extend(0u32.to_le_bytes());
        if i.subject.is_some() || i.location.is_some() {
            for w in [i.start, i.end, i.original] {
                v.extend(w.to_le_bytes());
            }
            for (_, s) in [i.subject, i.location].into_iter().flatten() {
                let units: Vec<u16> = s.encode_utf16().collect();
                v.extend(u16::try_from(units.len()).unwrap().to_le_bytes());
                v.extend(units.iter().flat_map(|u| u.to_le_bytes()));
            }
            v.extend(0u32.to_le_bytes());
        }
    }
    v.extend(0u32.to_le_bytes());
    v
}

/// A weekly series at 09:00 in UTC+2 (07:00Z) with one occurrence deleted, one moved a day on
/// with a new subject and room, and one whose text and show-as changed and whose reminder is off.
pub(crate) fn series(writer: u32) -> (Vec<u8>, i64) {
    let first = minutes(2026, 10, 5);
    let start = days_from_civil(2026, 10, 5) * 86_400 + 7 * 3600;
    let deleted = [minutes(2026, 10, 7), minutes(2026, 10, 12), minutes(2026, 10, 19)];
    let base = blob(0x200B, 0x0001, 1, &[0b000_1010], (0x2022, 10), &deleted, &[minutes(2026, 10, 13), minutes(2026, 10, 19)], first);
    let moved = Info {
        start: minutes(2026, 10, 13) + 600,
        end: minutes(2026, 10, 13) + 660,
        original: minutes(2026, 10, 12) + 540,
        subject: Some((b"Review (moved)", "Review (moved) \u{e9}")),
        location: Some((b"Room 2", "Room 2")),
        ..Default::default()
    };
    let quiet = Info {
        start: minutes(2026, 10, 19) + 540,
        end: minutes(2026, 10, 19) + 600,
        original: minutes(2026, 10, 19) + 540,
        busy: Some(0),
        reminder: Some((0, 15)),
        body: true,
        ..Default::default()
    };
    (with_changes(&base, writer, &[moved, quiet]), start)
}

/// A pattern's changed occurrences: their times in UTC at the series' offset, what they changed
/// (the UTF-16 text over the 8-bit one), and none of them an EXDATE, though the pattern deletes
/// them too; the day a moved one is on now is no EXDATE either.
#[test]
fn a_pattern_s_changed_occurrences() {
    let at = |d: u32, h: i64| days_from_civil(2026, 10, d) * 86_400 + h * 3600;
    for writer in [0x3008, 0x3009] {
        let (b, start) = series(writer);
        let p = pattern(&b, start).unwrap();
        assert_eq!(p.rule, "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE;COUNT=10");
        assert_eq!(p.exdates, [at(7, 7)], "{writer:x}");
        assert_eq!(p.changed.len(), 2);
        let moved = &p.changed[0];
        assert_eq!((moved.original, moved.start, moved.end), (at(12, 7), at(13, 8), at(13, 9)));
        assert_eq!((moved.subject.as_deref(), moved.location.as_deref()), (Some("Review (moved) \u{e9}"), Some("Room 2")));
        assert_eq!((moved.busy, moved.reminder_set, moved.own_body), (None, None, false));
        let quiet = &p.changed[1];
        assert_eq!(
            (quiet.original, quiet.subject.as_deref(), quiet.busy, quiet.reminder_set, quiet.own_body),
            (at(19, 7), None, Some(0), Some(false), true)
        );
        assert_eq!(recurrence(&b, start).unwrap().1, [at(7, 7)]);
    }
    // Without its ExtendedException, the 8-bit text is kept.
    let (b, start) = series(0x3008);
    let p = pattern(&b[..b.len() - 70], start).unwrap();
    assert_eq!(p.changed[0].subject.as_deref(), Some("Review (moved)"));
    // Cut anywhere or damaged anywhere, never a crash, and the rule stays.
    for n in 0..b.len() {
        let _ = pattern(&b[..n], start);
        let mut bad = b.clone();
        bad[n] = 0xFF;
        let _ = pattern(&bad, start);
    }
    for start in [i64::MIN, -1, 0, i64::MAX] {
        let _ = pattern(&b, start);
    }
    assert_eq!(pattern(&b[..b.len() - 1], start).unwrap().rule, "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE;COUNT=10");
    // The fuzz target's seed is this series, as its input has it (the start, then the pattern).
    let seed = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/pst_recurrence/weekly_changed")).unwrap();
    let (begins, blob) = seed.split_first_chunk::<8>().unwrap();
    let events = crate::pst::repeating(blob, i64::from_le_bytes(*begins));
    assert_eq!(events.iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), [None, Some(at(12, 7)), Some(at(19, 7))]);
    assert_eq!(events[1].summary, "Review (moved)");
}

/// A changed occurrence becomes an event of its own that replaces its occurrence: what it
/// changed from the pattern, its exact times, text and guests from its own item (matched by the
/// occurrence it replaces), and the rest the series'.
#[test]
fn changed_occurrences_become_events() {
    let at = |d: u32, h: i64| days_from_civil(2026, 10, d) * 86_400 + h * 3600;
    let (b, start) = series(0x3009);
    let x = Extra {
        start: Some(start),
        end: Some(start + 3600),
        recurrence: b,
        location: "Room 1".into(),
        reminder: Some(10),
        uid: "0400AB".into(),
        ..Default::default()
    };
    let organizer = EmailAddress::new("Riley", "riley@studio.example");
    let guests = [EmailAddress::new("Ana", "ana@posteo.example")];
    let master = event_of(&x, "Review", "Weekly.", &organizer, &guests, "u".into()).unwrap();
    // The quiet one's item, which says the hour exactly (summer time ended: 08:00Z).
    let quiet = Extra { start: Some(at(19, 8)), end: Some(at(19, 9)), replaces: Some(at(19, 8)), ..Default::default() };
    let ben = [EmailAddress::new("Ben", "ben@studio.example")];
    let stray = Extra { start: Some(at(26, 7)), replaces: Some(at(26, 7)), ..Default::default() };
    let own = [Own { x: &stray, body: "Not this one.", people: &[] }, Own { x: &quiet, body: "Bring the proofs.", people: &ben }];
    let changed = changed_events(&x, "Review", "Weekly.", &organizer, &guests, "u", &own);
    assert_eq!(changed.len(), 2);
    let (moved, none) = &changed[0];
    assert_eq!(*none, None);
    assert_eq!((moved.uid.as_str(), moved.recurrence_id, moved.start, moved.end), ("0400AB", Some(at(12, 7)), at(13, 8), at(13, 9)));
    assert_eq!((moved.summary.as_str(), moved.location.as_str(), moved.description.as_str()), ("Review (moved) \u{e9}", "Room 2", "Weekly."));
    assert!(moved.rrule.is_none() && moved.exdates.is_empty());
    assert_eq!((ical::reminder_minutes(moved), moved.attendees.len()), (Some(10), 1));
    let (quiet, found) = &changed[1];
    assert_eq!(*found, Some(1));
    assert_eq!((quiet.recurrence_id, quiet.start, quiet.end), (Some(at(19, 7)), at(19, 8), at(19, 9)));
    assert_eq!((quiet.summary.as_str(), quiet.location.as_str(), quiet.description.as_str()), ("Review", "Room 1", "Bring the proofs."));
    assert_eq!((ical::show_as(quiet), ical::reminder_minutes(quiet)), (ShowAs::Free, None));
    assert_eq!(quiet.attendees.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@studio.example"]);
    // In the calendar, each replaces its occurrence; the deleted one is gone.
    let events = [master, moved.clone(), quiet.clone()];
    let shown: Vec<i64> = ical::occurrences(&events, at(5, 0), at(22, 0)).iter().map(|o| o.start).collect();
    assert_eq!(shown, [at(5, 7), at(13, 8), at(14, 7), at(19, 8), at(21, 7)]);
    // No pattern, or no start: none.
    assert!(changed_events(&Extra { recurrence: Vec::new(), ..x.clone() }, "", "", &organizer, &[], "u", &own).is_empty());
    assert!(changed_events(&Extra { start: None, ..x }, "", "", &organizer, &[], "u", &own).is_empty());
}

/// A request for a repeating meeting carries its changed occurrences, which an answer doesn't.
#[test]
fn a_request_carries_its_changed_occurrences() {
    let (b, start) = series(0x3008);
    let x = Extra { start: Some(start), end: Some(start + 3600), recurrence: b, ..Default::default() };
    let organizer = EmailAddress::new("Riley", "riley@studio.example");
    let guests = [EmailAddress::new("Ana", "ana@posteo.example")];
    let (_, ics) = invitation(&x, "IPM.Schedule.Meeting.Request", "Review", "", &organizer, &guests, 1, "u".into()).unwrap();
    let cal = ical::parse(&ics, 0);
    assert_eq!(cal.events.iter().filter(|e| e.recurrence_id.is_some() && e.uid == "u").count(), 2, "{ics}");
    assert_eq!(cal.events[0].exdates.len(), 1);
    let (_, ics) = invitation(&x, "IPM.Schedule.Meeting.Resp.Pos", "Review", "", &guests[0], &[organizer], 1, "u".into()).unwrap();
    assert_eq!(ical::parse(&ics, 0).events.len(), 1);
}
