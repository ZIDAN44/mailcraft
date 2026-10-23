/// Issue #1: a Google invitation as Exchange makes it text reads as it does in Outlook: each
/// `words<address>` is a link named by its words, an address alone on its line is a link of
/// its own, phone links stay text, and nothing else changes.
#[test]
fn an_invitations_notes_read_with_their_links() {
    use super::{NotePart, note_lines};
    let t = |s: &str| NotePart::Text(s.into());
    let l = |label: &str, url: &str| NotePart::Link { label: label.into(), url: url.into() };
    let notes = "Join with Google Meet – You have been invited by ana@studio.example.\n\
                     <https://meet.google.com/abc-defg-hij>\n\
                     meet.google.com/abc-defg-hij<https://meet.google.com/abc-defg-hij?hs=224>\n\
                     (US) +1 555-0100<tel:+1-555-0100%3B618%23>\n\
                     More phone numbers<https://tel.meet/abc?pin=618&hs=1>\n\
                     Ana <mailto:ana@studio.example>\n\
                     modify your RSVP. Learn more<https://support.google.com/calendar/answer/37135#forwarding>\n\
                     See https://example.org/agenda, and a < b > c.\n\
                     \n\
                     Two<https://a.example/1> and three<https://b.example/2>";
    let lines = note_lines(notes);
    assert_eq!(lines.len(), 10);
    assert_eq!(lines[0], [t("Join with Google Meet – You have been invited by ana@studio.example.")]);
    assert_eq!(lines[1], [l("https://meet.google.com/abc-defg-hij", "https://meet.google.com/abc-defg-hij")]);
    assert_eq!(lines[2], [l("meet.google.com/abc-defg-hij", "https://meet.google.com/abc-defg-hij?hs=224")]);
    assert_eq!(lines[3], [t("(US) +1 555-0100")]);
    assert_eq!(lines[4], [l("More phone numbers", "https://tel.meet/abc?pin=618&hs=1")]);
    assert_eq!(lines[5], [l("Ana", "mailto:ana@studio.example")]);
    assert_eq!(lines[6], [t("modify your RSVP. "), l("Learn more", "https://support.google.com/calendar/answer/37135#forwarding")]);
    assert_eq!(lines[7], [t("See "), l("https://example.org/agenda", "https://example.org/agenda"), t(", and a < b > c.")]);
    assert!(lines[8].is_empty());
    // Two on a line: the words after the first link are the second's (which of them are
    // its own isn't in the text).
    assert_eq!(lines[9], [l("Two", "https://a.example/1"), t(" "), l("and three", "https://b.example/2")]);
    // Words on a line, then the address alone below (Google's Yes / No / More options): one
    // link; not after a heading ending in a colon, nor after a long line.
    let answers = note_lines(
        "Reply for riley
Yes

<https://x.example/yes>
No
<https://x.example/no>
Links:
<https://x.example/a>",
    );
    assert_eq!(
        answers,
        [
            vec![t("Reply for riley")],
            vec![l("Yes", "https://x.example/yes")],
            vec![l("No", "https://x.example/no")],
            vec![t("Links:")],
            vec![l("https://x.example/a", "https://x.example/a")]
        ]
    );
    // Odd or huge text doesn't run away, and keeps its words.
    let odd = "<<<https://x.example/a>>> <mailto:> <tel:> <javascript:alert(1)> é<https://é.example>";
    let words: String = note_lines(odd)
        .concat()
        .iter()
        .map(|p| match p {
            NotePart::Text(s) | NotePart::Link { label: s, .. } => s.clone(),
        })
        .collect();
    assert!(
        words.contains("javascript:alert(1)")
            && !note_lines(odd).concat().iter().any(|p| matches!(p, NotePart::Link { url, .. } if url.starts_with("javascript")))
    );
    let _ = note_lines(&"a<https://x.example/>".repeat(100_000));
    let _ = note_lines(&"<".repeat(100_000));
}

/// Web addresses in an event's notes: written out or as footnotes, once each, without the
/// sentence's punctuation; and which one joins the meeting.
#[test]
fn an_events_links_and_the_one_that_joins_it() {
    let notes = "Join here: https://zoom.us/j/9876543210?pwd=abc. Slides (https://docs.example.org/deck) attached.\n\
                     [1]: https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0\n\
                     Again https://zoom.us/j/9876543210?pwd=abc, and ftp://files.example.org and http://localhost/x";
    assert_eq!(
        links_in(notes),
        [
            "https://zoom.us/j/9876543210?pwd=abc",
            "https://docs.example.org/deck",
            "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0",
        ]
    );
    for meeting in [
        "https://teams.microsoft.com/l/meetup-join/19%3a",
        "https://teams.live.com/meet/9312",
        "https://us02web.zoom.us/j/123",
        "https://meet.google.com/abc-defg-hij",
        "https://acme.webex.com/acme/j.php?MTID=m1",
        "https://meet.jit.si/Standup",
    ] {
        assert!(is_meeting_link(meeting), "{meeting}");
    }
    for not in [
        "https://docs.example.org/deck",
        "https://zoom.us/pricing",
        "https://teams.microsoft.com/",
        "https://evil.example/zoom.us/j/1",
        "javascript:alert(1)",
    ] {
        assert!(!is_meeting_link(not), "{not}");
    }
    // The first meeting link, from its place before its notes; the server's own first.
    let e = CalEvent { location: "Microsoft Teams Meeting".into(), description: notes.into(), ..Default::default() };
    assert_eq!(e.join_link().as_deref(), Some("https://zoom.us/j/9876543210?pwd=abc"));
    let e = CalEvent { join_url: "https://teams.microsoft.com/l/meetup-join/own".into(), description: notes.into(), ..Default::default() };
    assert_eq!(e.join_link().as_deref(), Some("https://teams.microsoft.com/l/meetup-join/own"));
    assert_eq!(e.links().first().map(String::as_str), Some("https://teams.microsoft.com/l/meetup-join/own"));
    assert_eq!(CalEvent::default().join_link(), None);
    // Long or odd text doesn't run away.
    let many = "https://a.example/x ".repeat(5000);
    assert_eq!(links_in(&many).len(), 1);
    let junk: String = (0..20_000).map(|i| char::from_u32(0x20 + (i * 7919) % 0x2000).unwrap_or('x')).collect();
    let _ = links_in(&junk);
    assert!(links_in("https://").is_empty() && links_in("http://x").is_empty());
}

use super::*;

mod zones;

fn at(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
    days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
}

const INVITE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\nMETHOD:REQUEST\r\n\
BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\n\
BEGIN:DAYLIGHT\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nDTSTART:19700329T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\n\
BEGIN:STANDARD\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nDTSTART:19701025T030000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:review-42@studio.example\r\nDTSTAMP:20261001T080000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261015T140000\r\nDTEND;TZID=Europe/Berlin:20261015T153000\r\n\
SUMMARY:Storyboard review\\, scenes 4–7\r\nLOCATION:Room 2\\; upstairs\r\n\
DESCRIPTION:Bring the revised frames.\\nAnd the colour key.\r\n\
ORGANIZER;CN=\"Zoe Park\":mailto:zoe@studio.example\r\n\
ATTENDEE;CN=Riley Moreno;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:riley@studio.example\r\n\
X-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\n\
SEQUENCE:2\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Categories (which colour an event) come in and go out, commas inside a name escaped.
#[test]
fn categories_both_ways() {
    let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:c1\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\nCATEGORIES:Clients\\, big,Mix\r\nCATEGORIES:Mix,Lab\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let e = &parse(ics, 0).events[0];
    assert_eq!(e.categories, ["Clients, big", "Mix", "Lab"]);
    assert!(e.extra.iter().all(|l| !l.starts_with("CATEGORIES")));
    let out = write(std::slice::from_ref(e), None, 0);
    assert!(out.contains("CATEGORIES:Clients\\, big,Mix,Lab"), "{out}");
    assert_eq!(parse(&out, 0).events[0].categories, e.categories);
    assert_eq!(split_text_list(" a ,, b\\,c "), ["a", "b,c"]);
}

#[test]
fn reads_an_invitation() {
    let cal = parse(INVITE, 0);
    assert_eq!(cal.method.as_deref(), Some("REQUEST"));
    let e = &cal.events[0];
    assert_eq!(e.uid, "review-42@studio.example");
    assert_eq!(e.summary, "Storyboard review, scenes 4–7");
    assert_eq!(e.location, "Room 2; upstairs");
    assert_eq!(e.description, "Bring the revised frames.\nAnd the colour key.");
    // 14:00 in Berlin in October (summer time, +2) is 12:00 UTC.
    assert_eq!(e.start, at(2026, 10, 15, 12, 0));
    assert_eq!(e.end - e.start, 5400);
    assert_eq!(e.organizer.as_ref().map(|o| (o.name.as_str(), o.email.as_str())), Some(("Zoe Park", "zoe@studio.example")));
    assert_eq!(e.attendees[0].partstat, "NEEDS-ACTION");
    assert!(e.attendees[0].rsvp);
    assert_eq!(e.sequence, 2);
    assert!(e.extra.iter().any(|x| x.starts_with("X-MICROSOFT")), "unknown properties are kept");
    assert!(e.extra.iter().any(|x| x == "TRIGGER:-PT15M"), "alarms too");
    // In winter the same zone is +1.
    let winter = INVITE.replace("20261015T140000", "20261215T140000").replace("20261015T153000", "20261215T153000");
    assert_eq!(parse(&winter, 0).events[0].start, at(2026, 12, 15, 13, 0));
}

/// A big calendar expands in one pass: the moved occurrences are gathered once, not looked
/// for again for every event.
#[test]
fn a_big_calendar_expands_quickly() {
    let day = 86_400;
    let mut events: Vec<CalEvent> = (0..30_000)
        .map(|n| CalEvent { uid: format!("e{n}"), summary: format!("{n}"), start: n * 3600, end: n * 3600 + 1800, ..Default::default() })
        .collect();
    events.push(CalEvent { uid: "s".into(), start: 0, end: 600, rrule: Some("FREQ=DAILY".into()), ..Default::default() });
    events.push(CalEvent { uid: "s".into(), recurrence_id: Some(2 * day), start: 2 * day + 60, end: 2 * day + 660, ..Default::default() });
    let took = std::time::Instant::now();
    let got = occurrences(&events, 0, 4 * day);
    assert!(took.elapsed() < std::time::Duration::from_secs(5), "{:?}", took.elapsed());
    let series: Vec<i64> = got.iter().filter(|o| events[o.event].uid == "s").map(|o| o.start).collect();
    assert_eq!(series, [0, day, 2 * day + 60, 3 * day]);
    assert_eq!(got.len(), 96 + 4);
}

#[test]
fn reminders_from_and_to_alarms() {
    // Damaged triggers: the next alarm counts; huge ones are held to the longest.
    let odd =
        |lines: &[&str]| reminder_minutes(&CalEvent { start: 1_000_000, extra: lines.iter().map(|l| l.to_string()).collect(), ..Default::default() });
    assert_eq!(odd(&["BEGIN:VALARM", "TRIGGER:-PXM", "END:VALARM", "BEGIN:VALARM", "TRIGGER:-PT5M", "END:VALARM"]), Some(5));
    assert_eq!(odd(&["BEGIN:VALARM", "TRIGGER:-P99999999999999999999W", "END:VALARM"]), None);
    assert_eq!(odd(&["BEGIN:VALARM", "TRIGGER:-P9999999999999W", "END:VALARM"]), Some(REMINDER_MAX_MINUTES));
    assert_eq!(odd(&["BEGIN:VALARM", "TRIGGER;VALUE=DATE-TIME:19700101T000000Z", "END:VALARM"]), Some(1_000_000 / 60));
    assert_eq!(odd(&["BEGIN:VALARM", "TRIGGER:PT30M", "END:VALARM"]), Some(0));
    assert_eq!(odd(&["TRIGGER:-PT5M"]), None, "outside an alarm");
    let with = |lines: &[&str]| CalEvent { start: 1_800_000_000, extra: lines.iter().map(|s| s.to_string()).collect(), ..Default::default() };
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "ACTION:DISPLAY", "TRIGGER:-PT15M", "END:VALARM"])), Some(15));
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER;RELATED=START:-P1DT2H", "END:VALARM"])), Some(26 * 60));
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER:PT0S", "END:VALARM"])), Some(0));
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER:PT10M", "END:VALARM"])), Some(0), "after the start: at it");
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER;VALUE=DATE-TIME:20270115T075000Z", "END:VALARM"])), Some(10));
    // Relative to the end, or outside an alarm, or broken: no reminder from it.
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER;RELATED=END:-PT5M", "END:VALARM"])), None);
    assert_eq!(reminder_minutes(&with(&["TRIGGER:-PT5M"])), None);
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER:soon", "END:VALARM"])), None);
    assert_eq!(reminder_minutes(&with(&["BEGIN:VALARM", "TRIGGER:-P9999W", "END:VALARM"])), Some(REMINDER_MAX_MINUTES));
    // Set: one alarm instead of the others; other lines stay; none removes it.
    let mut e = with(&[
        "X-KEEP:1",
        "BEGIN:VALARM",
        "TRIGGER:-PT5M",
        "BEGIN:X-NESTED",
        "END:X-NESTED",
        "END:VALARM",
        "BEGIN:VALARM",
        "TRIGGER:-P1D",
        "END:VALARM",
    ]);
    set_reminder(&mut e, Some(30));
    assert_eq!(reminder_minutes(&e), Some(30));
    assert_eq!(e.extra.iter().filter(|l| *l == "BEGIN:VALARM").count(), 1);
    assert!(e.extra.contains(&"X-KEEP:1".to_string()) && !e.extra.iter().any(|l| l.contains("NESTED")));
    set_reminder(&mut e, None);
    assert_eq!((reminder_minutes(&e), e.extra.clone()), (None, vec!["X-KEEP:1".to_string()]));
    // Through a write and a read.
    let mut ev = CalEvent { uid: "r1".into(), summary: "Call".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
    set_reminder(&mut ev, Some(45));
    let back = parse(&write(&[ev], None, 1_800_000_000), 0).events;
    assert_eq!(back.first().and_then(reminder_minutes), Some(45));
}

#[test]
fn writes_what_it_reads() {
    let cal = parse(INVITE, 0);
    let text = write(&cal.events, Some("REPLY"), at(2026, 10, 2, 9, 0));
    assert!(text.lines().all(|l| l.len() <= 76), "folded:\n{text}");
    assert!(text.contains("DTSTART;TZID=Europe/Berlin:20261015T140000\r\n"), "in its zone:\n{text}");
    assert!(text.contains("SUMMARY:Storyboard review\\, scenes 4–7"));
    let back = parse(&text, 0);
    assert_eq!(back.method.as_deref(), Some("REPLY"));
    let (a, b) = (&cal.events[0], &back.events[0]);
    assert_eq!((&a.uid, &a.summary, &a.location, &a.description, a.start, a.end), (&b.uid, &b.summary, &b.location, &b.description, b.start, b.end));
    assert_eq!(a.attendees, b.attendees);
    assert_eq!(a.extra, b.extra, "unknown lines survive a round trip");
    // Long text folds on character boundaries.
    let long = CalEvent { uid: "x".into(), summary: "ü".repeat(100), start: 0, end: 3600, ..Default::default() };
    assert_eq!(parse(&write(std::slice::from_ref(&long), None, 0), 0).events[0].summary, long.summary);
}

#[test]
fn all_day_durations_and_junk() {
    let text = "BEGIN:VEVENT\nUID:a\nDTSTART;VALUE=DATE:20261224\nSUMMARY:Holiday\nEND:VEVENT\n\
                    BEGIN:VEVENT\nUID:b\nDTSTART:20261015T090000Z\nDURATION:PT1H30M\nEND:VEVENT\n\
                    BEGIN:VEVENT\nUID:c\nDTSTART:20261015T090000\nEND:VEVENT\n";
    let cal = parse(text, 2 * 3600);
    assert!(cal.events[0].all_day);
    assert_eq!((cal.events[0].start, cal.events[0].end - cal.events[0].start), (days_from_civil(2026, 12, 24) * 86_400, 86_400));
    assert_eq!(cal.events[1].end - cal.events[1].start, 5400);
    assert_eq!(cal.events[2].start, at(2026, 10, 15, 7, 0), "floating time is local");
    assert_eq!(duration("P1W"), Some(604_800));
    assert_eq!(duration("-PT15M"), Some(-900));
    assert_eq!(duration("P1H"), None);
    for junk in ["", "BEGIN:VEVENT", "garbage\r\n:::\r\n;;;", "BEGIN:VEVENT\nDTSTART:2026\nEND:VEVENT", &"BEGIN:X\n".repeat(100)] {
        let _ = parse(junk, 0);
    }
}

#[test]
fn recurrences() {
    let mk = |rrule: &str, start: i64| CalEvent { uid: "r".into(), start, end: start + 3600, rrule: Some(rrule.into()), ..Default::default() };
    let mon = at(2026, 10, 5, 9, 0); // a Monday
    // Every Monday and Wednesday, 5 times.
    let e = mk("FREQ=WEEKLY;BYDAY=MO,WE;COUNT=5", mon);
    let s = starts(&e, i64::MAX);
    assert_eq!(s, vec![mon, mon + 2 * 86_400, mon + 7 * 86_400, mon + 9 * 86_400, mon + 14 * 86_400]);
    // Every other day until a date.
    let e = mk("FREQ=DAILY;INTERVAL=2;UNTIL=20261011T000000Z", mon);
    assert_eq!(starts(&e, i64::MAX).len(), 3);
    // The last Friday of each month.
    let e = mk("FREQ=MONTHLY;BYDAY=-1FR;COUNT=3", at(2026, 10, 30, 17, 0));
    assert_eq!(starts(&e, i64::MAX), vec![at(2026, 10, 30, 17, 0), at(2026, 11, 27, 17, 0), at(2026, 12, 25, 17, 0)]);
    // The 31st: months without one are skipped.
    let e = mk("FREQ=MONTHLY;BYMONTHDAY=31;COUNT=3", at(2026, 1, 31, 8, 0));
    assert_eq!(starts(&e, i64::MAX), vec![at(2026, 1, 31, 8, 0), at(2026, 3, 31, 8, 0), at(2026, 5, 31, 8, 0)]);
    // Yearly on the start's date.
    let e = mk("FREQ=YEARLY;COUNT=2", at(2024, 2, 29, 0, 0));
    assert_eq!(starts(&e, i64::MAX), vec![at(2024, 2, 29, 0, 0), at(2028, 2, 29, 0, 0)], "a leap day recurs on leap years");
    // Endless rules stop at the window and the caps.
    let e = mk("FREQ=DAILY", mon);
    assert_eq!(starts(&e, mon + 10 * 86_400).len(), 10);
    assert!(starts(&mk("FREQ=SECONDLY", mon), i64::MAX).is_empty());
    assert!(starts(&mk("FREQ=DAILY;BYMONTH=13", mon), mon + 400 * 86_400).len() <= 400);

    // Occurrences in a window, with an exception and a moved instance.
    let mut weekly = mk("FREQ=WEEKLY", mon);
    weekly.exdates.push(mon + 7 * 86_400);
    let moved = CalEvent {
        uid: "r".into(),
        start: mon + 15 * 86_400,
        end: mon + 15 * 86_400 + 3600,
        recurrence_id: Some(mon + 14 * 86_400),
        ..Default::default()
    };
    let events = vec![weekly, moved];
    let occ = occurrences(&events, mon, mon + 22 * 86_400);
    let starts_seen: Vec<i64> = occ.iter().map(|o| o.start).collect();
    assert_eq!(starts_seen, vec![mon, mon + 15 * 86_400, mon + 21 * 86_400]);
    assert_eq!(occ[1].event, 1, "the moved instance");
}

#[test]
fn nth_weekdays() {
    assert_eq!(nth_weekday(2026, 3, -1, 6), Some(days_from_civil(2026, 3, 29)), "last Sunday of March 2026");
    assert_eq!(nth_weekday(2026, 10, 2, 0), Some(days_from_civil(2026, 10, 12)), "second Monday");
    assert_eq!(nth_weekday(2026, 2, 5, 0), None, "no fifth Monday in February 2026");
    assert_eq!(month_day(2026, 2, -1), Some(days_from_civil(2026, 2, 28)));
}

/// An invitation's `DTSTAMP` is read (it tells two versions with the same `SEQUENCE` apart); a
/// stored event keeps it when written again, an invitation is stamped when it's sent.
#[test]
fn the_stamp_of_an_invitation_is_kept() {
    let text = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nPRODID:-//MailCraft tests//EN\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s1\r\nDTSTAMP:20261020T080000Z\r\nSEQUENCE:3\r\n\
                DTSTART:20261021T100000Z\r\nDTEND:20261021T110000Z\r\nX-OTHER:kept\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let e = parse(text, 0).events.remove(0);
    assert_eq!((e.sequence, e.dtstamp), (3, days_from_civil(2026, 10, 20) * 86_400 + 8 * 3600));
    assert_eq!(e.extra, ["X-OTHER:kept"], "unknown properties still round-trip; DTSTAMP is read");
    let stored = write(std::slice::from_ref(&e), None, 1_900_000_000);
    assert_eq!(parse(&stored, 0).events[0], e, "a stored event keeps its stamp");
    let sent = write(std::slice::from_ref(&e), Some("REQUEST"), 1_900_000_000);
    assert_eq!(parse(&sent, 0).events[0].dtstamp, 1_900_000_000, "an invitation is stamped when sent");
    let fresh = CalEvent { dtstamp: 0, ..e };
    assert_eq!(parse(&write(&[fresh], None, 1_900_000_000), 0).events[0].dtstamp, 1_900_000_000);
}
