//! Engine tests: calendars, people, notes and reminders.

use super::*;

mod agenda;
mod dav_merge;
mod deleting;
mod huge_times;
mod moving;
mod people;
mod server_cards;

#[test]
fn calendar_events_on_this_computer() {
    let mut s = sample_session("calendar");
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    s.run("calendar.view", &json!({"mode": "week"})).unwrap();
    let week: Vec<Value> = s.run("calendar.events", &json!({})).unwrap().as_array().unwrap().clone();
    let titles: Vec<&str> = week.iter().filter_map(|e| e["summary"].as_str()).collect();
    assert_eq!(titles.iter().filter(|t| **t == "Team stand-up").count(), 3, "Monday, Wednesday and Friday: {titles:?}");
    assert!(titles.contains(&"Storyboard review"));
    let mix = week
        .iter()
        .chain(&s.run("calendar.events", &json!({"from": "2026-10-01", "to": "2026-11-30"})).unwrap().as_array().unwrap().clone())
        .find(|e| e["summary"] == "Sound mix playback")
        .cloned()
        .unwrap();
    assert_eq!(mix["needsReply"], true, "an invitation awaiting a reply");

    // A new event at a local time; the clock is UTC here.
    let uid = s
        .run("event.create", &json!({"summary": "Colour grading", "start": "2026-10-20T10:00", "end": "2026-10-20T12:30", "location": "Suite B"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let day = s.run("calendar.events", &json!({"from": "2026-10-20", "to": "2026-10-20"})).unwrap();
    let e = day.as_array().unwrap().iter().find(|e| e["uid"] == uid.as_str()).cloned().unwrap();
    assert_eq!(e["end"].as_i64().unwrap() - e["start"].as_i64().unwrap(), 9000);
    assert!(s.run("event.create", &json!({"summary": " ", "start": "2026-10-20"})).is_err(), "a title is needed");
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-20T10:00", "end": "2026-10-20T09:00"})).is_err());
    assert!(s.run("event.create", &json!({"summary": "x", "start": "20 October"})).is_err());

    // Make it weekly, skip one week, then move the whole series.
    s.run("event.update", &json!({"uid": uid, "repeat": "weekly"})).unwrap();
    let span = json!({"from": "2026-10-20", "to": "2026-11-10"});
    let count = |s: &mut Session| s.run("calendar.events", &span).unwrap().as_array().unwrap().iter().filter(|e| e["uid"] == uid.as_str()).count();
    assert_eq!(count(&mut s), 4);
    let second =
        s.run("calendar.events", &span).unwrap().as_array().unwrap().iter().filter(|e| e["uid"] == uid.as_str()).nth(1).unwrap()["occurrence"]
            .as_i64()
            .unwrap();
    s.run("event.delete", &json!({"uid": uid, "occurrence": second})).unwrap();
    assert_eq!(count(&mut s), 3, "one occurrence skipped");
    s.run("event.update", &json!({"uid": uid, "summary": "Grading", "allDay": true})).unwrap();
    assert!(s.find_event(&uid).unwrap().1.event.all_day);

    // All of it is kept.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut again = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(again.find_event(&uid).unwrap().1.event.summary, "Grading");
    again.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(again.find_event(&uid).is_none());

    // Moving through the calendar.
    s.run("calendar.view", &json!({"mode": "month"})).unwrap();
    let r = s.run("calendar.goto", &json!({"date": "2026-01-31", "step": 1})).unwrap();
    assert_eq!(r["date"], "2026-02-28", "a month on from the 31st stays in February");
    assert_eq!(s.cal_days().1 - s.cal_days().0, 42);
    assert_eq!(s.run("calendar.goto", &json!({"date": "today"})).unwrap()["date"].as_str().map(|d| d.len()), Some(10));
}

/// In New York, a weekly 09:00 meeting made in October is still at 09:00 in December, after the
/// clocks went back: it's made in the user's zone, and shown with the offset of its own date.
#[test]
fn the_calendar_keeps_local_times_across_summer_time() {
    let mut s = sample_session("calendar-zone");
    s.set_clock(NOW, -4 * 3600);
    s.set_zone(Some("America/New_York"));
    assert!(s.clock.zone.is_some());
    let made =
        s.run("event.create", &json!({"summary": "Dailies", "start": "2026-10-19T09:00", "end": "2026-10-19T10:00", "repeat": "weekly"})).unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    assert_eq!(s.find_event(&uid).unwrap().1.event.tzid, "America/New_York");
    let at = |s: &mut Session, day: &str| {
        let list = s.run("calendar.events", &json!({"from": day, "to": day})).unwrap();
        list.as_array().unwrap().iter().find(|e| e["uid"] == uid.as_str()).unwrap()["start"].as_i64().unwrap()
    };
    let (oct, dec) = (at(&mut s, "2026-10-19"), at(&mut s, "2026-12-07"));
    let day = |year, month, date| 86_400 * mailcraft_model::dates::days_from_civil(year, month, date);
    assert_eq!((oct, dec), (day(2026, 10, 19) + 13 * 3600, day(2026, 12, 7) + 14 * 3600));
    assert_eq!((mailcraft_model::dates::civil(oct, &s.clock).hour, mailcraft_model::dates::civil(dec, &s.clock).hour), (9, 9));
    assert_eq!(s.local_midnight(day(2026, 12, 7) / 86_400), day(2026, 12, 7) + 5 * 3600, "December's days start at EST midnight");
    // A zone it doesn't know, or one that doesn't fit the clock: today's offset.
    s.set_zone(Some("Mars/Olympus"));
    assert!(s.clock.zone.is_none());
    s.set_zone(Some("Europe/Berlin"));
    assert!(s.clock.zone.is_none());
}

#[test]
fn invitations_are_answered_and_sent() {
    let mut s = sample_session("invites");
    // The invitation stays in the Inbox to be answered again (moving it: answered_invites.rs).
    s.run("options.answeredInvites", &json!({"after": "keep"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let inv = s.invite().expect("an invitation");
    assert_eq!((inv.method.as_str(), inv.answer.as_str(), inv.in_calendar), ("REQUEST", "NEEDS-ACTION", true));
    assert_eq!(inv.event.summary, "Sound mix playback");
    assert!(s.run("invite.respond", &json!({"answer": "maybe"})).is_err());

    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(s.invite().unwrap().answer, "ACCEPTED");
    let (method, raw) = sent_calendar(&s, "Sent", "Accepted: Sound mix playback");
    assert_eq!(method, "REPLY");
    assert!(raw.contains("PARTSTAT=ACCEPTED") && raw.contains("mailto:riley@studio.example"), "{raw}");
    assert!(raw.to_ascii_lowercase().contains("text/calendar"), "sent as a calendar part");
    let to = mailcraft_mime::parse(raw.as_bytes()).unwrap().to;
    assert_eq!(to, [mailcraft_model::EmailAddress::new("Ana Lima", "ana@soundlab.example")], "to the organizer");
    // As iMIP says (RFC 6047 §2.4), so Exchange takes it as an answer: inline beside the text,
    // with its method and character set, not a file.
    assert!(raw.contains("Content-Type: multipart/alternative") && raw.contains("text/calendar; charset=\"utf-8\"; method=\"REPLY\""), "{raw}");
    assert!(!raw.contains("invite.ics") && !raw.contains("Content-Disposition: attachment"), "{raw}");

    // Declining takes it out of the calendar.
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert!(s.find_event("mix@sample.mailcraft").is_none());
    assert_eq!(sent_calendar(&s, "Sent", "Declined: Sound mix playback").0, "REPLY");
    // Accepting again puts it back.
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    let e = s.find_event("mix@sample.mailcraft").unwrap().1.event.clone();
    assert!(e.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "TENTATIVE"));
}

#[test]
fn own_events_invite_and_cancel_guests() {
    let mut s = sample_session("meetings");
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>, sam@studio.example"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some("riley@studio.example"));
    assert!(e.attendees.iter().all(|a| a.partstat == "NEEDS-ACTION" && a.rsvp));
    let (method, raw) = sent_calendar(&s, "Sent", "Invitation: Trailer review");
    assert_eq!(method, "REQUEST");
    assert!(raw.contains("zoe@studio.example") && raw.contains("sam@studio.example"));
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-21", "attendees": "not an address"})).is_err());

    // A guest's answer comes back and is recorded.
    let mut reply = e.clone();
    reply.attendees = vec![mailcraft_model::ical::Attendee {
        email: "zoe@studio.example".into(),
        name: "Zoe Park".into(),
        partstat: "ACCEPTED".into(),
        rsvp: false,
    }];
    let ics = mailcraft_model::ical::write(&[reply], Some("REPLY"), NOW).into_bytes();
    let raw = calendar_mail(("Zoe Park", "zoe@studio.example"), "Accepted: Trailer review", "text/calendar; method=REPLY", ics, "<r1@x>");
    let key = mailcraft_model::MessageKey { uid: 999, ..next_key(&s, "sample", "INBOX") };
    let key = file_whole(&mut s, key, &raw);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let inv = s.invite().unwrap();
    assert_eq!((inv.method.as_str(), inv.replier.as_ref().map(|r| r.partstat.as_str())), ("REPLY", Some("ACCEPTED")));
    s.run("invite.apply", &json!({})).unwrap();
    let zoe = s.find_event(&uid).unwrap().1.event.attendees.iter().find(|a| a.email == "zoe@studio.example").cloned().unwrap();
    assert_eq!(zoe.partstat, "ACCEPTED");

    // Called off: the guests are told.
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    let (method, raw) = sent_calendar(&s, "Sent", "Cancelled: Trailer review");
    assert_eq!(method, "CANCEL");
    assert!(raw.contains("STATUS:CANCELLED"));
}

#[test]
fn dav_results_merge_without_losing_local_changes() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};
    let mut s = Session::open(Some(temp("dav-merge")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let cal = "https://dav.example.org/cal/work/".to_string();
    let ev = |uid: &str, summary: &str, href: &str| StoredEvent {
        calendar: cal.clone(),
        href: href.into(),
        etag: "\"1\"".into(),
        event: CalEvent { uid: uid.into(), summary: summary.into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() },
        dirty: false,
    };
    let first = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo {
            id: cal.clone(),
            name: "Work".into(),
            color: 2,
            read_only: false,
            ctag: "a".into(),
            ..Default::default()
        }]),
        events: vec![ev("a", "Review", &format!("{cal}a.ics")), ev("b", "Lunch", &format!("{cal}b.ics"))],
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(first) });
    let p = s.pim.get(&id).unwrap();
    assert_eq!(p.calendars.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Work"], "the empty calendar on this computer gives way");
    assert_eq!(p.events.len(), 2);
    assert!(s.agenda(NOW, NOW + 86_400).iter().any(|i| i.summary == "Review" && i.color == 2));

    // An edit here goes to the server; the server meanwhile dropped "Lunch".
    s.take_jobs();
    s.run("event.update", &json!({"uid": "a", "summary": "Review (moved)"})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "a sync is asked for");
    let mut pushed = s.find_event("a").unwrap().1.clone();
    pushed.etag = "\"2\"".into();
    pushed.dirty = false;
    let second = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo {
            id: cal.clone(),
            name: "Work".into(),
            color: 2,
            read_only: false,
            ctag: "b".into(),
            ..Default::default()
        }]),
        events: vec![pushed.clone()],
        synced_calendars: vec![cal.clone()],
        pushed_events: vec![pushed],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(second) });
    let p = s.pim.get(&id).unwrap();
    assert_eq!(p.events.len(), 1, "Lunch is gone from the server");
    assert!(!p.events[0].dirty && p.events[0].etag == "\"2\"" && p.events[0].event.summary == "Review (moved)");

    // A change made while a sync was out stays to be sent.
    s.run("event.update", &json!({"uid": "a", "summary": "Review (moved again)"})).unwrap();
    let stale = DavSync {
        calendars: Some(vec![CalendarInfo {
            id: cal.clone(),
            name: "Work".into(),
            color: 2,
            read_only: false,
            ctag: "c".into(),
            ..Default::default()
        }]),
        events: vec![ev("a", "Review (moved)", &format!("{cal}a.ics"))],
        synced_calendars: vec![cal.clone()],
        caldav_home: Some("https://dav.example.org/cal/".into()),
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(stale) });
    let e = s.find_event("a").unwrap().1.clone();
    assert!(e.dirty && e.event.summary == "Review (moved again)", "{e:?}");
    // A server without calendars: the one on this computer stays.
    let none = DavSync { checked: NOW, ..Default::default() };
    let mut s2 = Session::open(Some(temp("dav-none")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id2 = s2.run("account.add", &json!({"email": "ann@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s2.apply(Event::DavSynced { account: id2.clone(), sync: Box::new(none) });
    assert_eq!(s2.pim.get(&id2).unwrap().calendars[0].id, mailcraft_model::pim::LOCAL);
    assert_eq!(s2.pim.get(&id2).unwrap().caldav_home.as_deref(), Some(""), "remembered as none");
    // An event made on this computer, stored in the server's calendar (Exchange gives it a new
    // uid): one event after the merge, the server's.
    s2.run("event.create", &json!({"summary": "Poster review", "start": NOW + 3600, "end": NOW + 7200})).unwrap();
    let local = s2.pim.get(&id2).unwrap().events[0].clone();
    let server_cal = "CAL1".to_string();
    let pushed = StoredEvent { calendar: server_cal.clone(), href: "EV1".into(), etag: "ck".into(), event: local.event.clone(), dirty: false };
    let read = StoredEvent { event: CalEvent { uid: "ical-EV1".into(), ..local.event.clone() }, ..pushed.clone() };
    let merged = DavSync {
        caldav_home: Some("graph".into()),
        calendars: Some(vec![CalendarInfo {
            id: server_cal.clone(),
            name: "Calendar".into(),
            color: 0,
            read_only: false,
            ctag: String::new(),
            ..Default::default()
        }]),
        events: vec![read],
        synced_calendars: vec![server_cal],
        pushed_events: vec![pushed],
        checked: NOW,
        ..Default::default()
    };
    s2.apply(Event::DavSynced { account: id2.clone(), sync: Box::new(merged) });
    let p = s2.pim.get(&id2).unwrap();
    assert_eq!(p.events.iter().filter(|e| e.event.summary == "Poster review").count(), 1, "{:?}", p.events);
    assert_eq!(p.events[0].event.uid, "ical-EV1");
}

#[test]
fn notes_are_written_coloured_and_deleted() {
    let mut s = sample_session("notes");
    assert_eq!(s.notes("").len(), 3, "the sample's notes");
    let uid = s.run("note.create", &json!({"text": "Call the print shop", "color": 9})).unwrap()["uid"].as_str().unwrap().to_string();
    let n = s.notes("").into_iter().find(|n| n.note.uid == uid).unwrap();
    assert_eq!((n.note.color, n.note.text.as_str()), (4, "Call the print shop"), "a colour past the last is the last");
    s.set_clock(NOW + 60, 0);
    s.run("note.update", &json!({"uid": uid, "text": "Call the print shop\nabout paper", "color": 0})).unwrap();
    let first = &s.notes("")[0];
    assert_eq!((first.note.uid.as_str(), first.note.title(), first.note.color), (uid.as_str(), "Call the print shop", 0), "the latest first");
    assert_eq!(s.notes("paper").len(), 2);
    assert!(s.run("note.update", &json!({"uid": "nope", "text": "x"})).is_err());
    s.run("note.delete", &json!({"uid": uid})).unwrap();
    assert!(s.notes("").iter().all(|n| n.note.uid != uid));
    // Kept with the account's calendar and contacts, across starts.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    drop(s);
    let s = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s.notes("").len(), 3);
    drop(s);

    // An Exchange account: notes go to its Notes folder, and what the server says follows.
    let mut s = Session::open(Some(temp("notes-ews")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let made = s.run("note.create", &json!({"text": "Sync me"})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    assert!(s.pim[&id].notes[0].dirty);
    assert_eq!(made["uid"].as_str(), Some(s.pim[&id].notes[0].note.uid.as_str()), "it answers the new note's uid");
    let stored = mailcraft_model::pim::StoredNote { href: "note:X1".into(), note: s.pim[&id].notes[0].note.clone(), ..Default::default() };
    let server_copy =
        mailcraft_model::pim::StoredNote { note: mailcraft_model::pim::Note { uid: "note:X1".into(), ..stored.note.clone() }, ..stored.clone() };
    let other = mailcraft_model::pim::StoredNote {
        href: "note:X2".into(),
        note: mailcraft_model::pim::Note { uid: "note:X2".into(), text: "From Outlook".into(), color: 2, modified: 1, files: Vec::new() },
        ..Default::default()
    };
    let sync = mailcraft_model::pim::DavSync { pushed_notes: vec![stored], notes: Some(vec![server_copy, other]), ..Default::default() };
    s.apply_dav(&id, &sync);
    let texts: Vec<String> = s.notes("").into_iter().map(|n| n.note.text.clone()).collect();
    assert_eq!(texts.len(), 2, "{texts:?}: the note sent isn't doubled");
    assert!(texts.contains(&"From Outlook".to_string()) && texts.contains(&"Sync me".to_string()));
    s.run("note.delete", &json!({"uid": "note:X2"})).unwrap();
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "note:X2"), "deleted on the server next time");
}

/// Issue #1: notes sort A to Z (by their first line), by colour, or the last changed first.
#[test]
fn notes_can_be_arranged() {
    let mut s = sample_session("notes-order");
    for (text, color) in [("banana bread", 3), ("Apple pie\nwith cream", 1), ("cherry jam", 1)] {
        s.run("note.create", &json!({"text": text, "color": color})).unwrap();
    }
    let titles = |s: &Session| {
        s.notes("")
            .iter()
            .filter(|n| ["Apple", "banana", "cherry"].iter().any(|w| n.note.text.starts_with(w)))
            .map(|n| n.note.text.lines().next().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    s.run("note.arrange", &json!({"by": "title"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "banana bread", "cherry jam"]);
    s.run("note.arrange", &json!({"by": "color"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "cherry jam", "banana bread"]);
    assert!(s.run("note.arrange", &json!({"by": "size"})).is_err());
}

/// Issue #1: events are drawn in their category's colour (shared with mail's categories), so a
/// calendar isn't all one colour.
#[test]
fn events_take_their_categorys_colour() {
    let mut s = sample_session("event-colours");
    s.run("category.create", &json!({"name": "Clients", "color": 5})).unwrap();
    let mut add = |summary: &str, categories: Value| {
        s.run("event.create", &json!({"summary": summary, "start": "2026-10-16T10:00", "categories": categories})).unwrap();
    };
    add("Pitch", json!(["clients"]));
    add("Mix", json!("Studio, Clients"));
    add("Plain", json!([]));
    let day = crate::parse_local("2026-10-16", &s.clock).map(|(t, _)| t).unwrap();
    let item = |name: &str| s.agenda(day, day + 86_400).into_iter().find(|i| i.summary == name).unwrap();
    assert_eq!(item("Pitch").category, Some(5), "a category's own colour, whatever its case");
    let studio = item("Mix").category.unwrap();
    assert_eq!(studio, s.category_color_of("Studio"), "the first category; one from elsewhere gets one from its name");
    assert_eq!(s.category_color_of("Studio"), s.category_color_of(" studio "));
    assert_eq!(item("Plain").category, None, "no category: its calendar's colour");
    let (_, stored) = s.find_event(&item("Mix").uid).unwrap();
    assert_eq!(stored.event.categories, ["Studio", "Clients"]);
}

/// Issue #1: calendars can be hidden (Outlook's ticks) and shown side by side, and the side
/// pane can show several months; all kept with the settings.
#[test]
fn calendars_can_be_hidden_and_shown_side_by_side() {
    let mut s = sample_session("calendar-views");
    let studio = crate::sample::STUDIO_CALENDAR;
    let names = |s: &Session| s.shown_calendars().into_iter().map(|c| c.1).collect::<Vec<_>>();
    assert_eq!(names(&s), [mailcraft_model::pim::LOCAL, studio]);
    let (from, to) = (s.clock.now - 7 * 86_400, s.clock.now + 7 * 86_400);
    let in_studio = |s: &Session| s.agenda(from, to).iter().filter(|i| i.calendar == studio).count();
    assert_eq!(in_studio(&s), 2);
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio})).unwrap();
    assert_eq!(names(&s), [mailcraft_model::pim::LOCAL]);
    assert_eq!(in_studio(&s), 0, "a hidden calendar's events aren't shown");
    assert!(s.agenda(from, to).iter().any(|i| i.summary == "Lunch with Jun"));
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio, "shown": true})).unwrap();
    assert_eq!(in_studio(&s), 2);
    assert!(s.run("calendar.show", &json!({"account": "sample", "calendar": "nope"})).is_err());
    s.run("calendar.sideBySide", &json!({})).unwrap();
    assert!(s.prefs.calendars_side_by_side);
    s.run("calendar.months", &json!({"count": 9})).unwrap();
    assert_eq!(s.prefs.calendar_months, 6, "at most six");
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio, "shown": false})).unwrap();
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!((back.calendars_side_by_side, back.calendar_months, back.hidden_calendars.len()), (true, 6, 1));
    // The sample's events in categories take their colours.
    let edit = s.agenda(from, to).into_iter().find(|i| i.summary == "Lunch with Jun").unwrap();
    assert_eq!(edit.category, Some(s.category_color_of("Personal")));
}

/// Reminders: an event's reminder comes due at its time, snoozes, comes back, and goes when
/// dismissed; each occurrence of a series has its own; one without a reminder, or cancelled,
/// never comes; a hidden calendar's still does; and bad params are refused.
#[test]
fn reminders_come_snooze_and_go() {
    let mut s = sample_session("reminders");
    let tomorrow = s.clock.now - s.clock.now.rem_euclid(86_400) + 86_400;
    let (start, review_at) = (tomorrow + 9 * 3600, tomorrow + 14 * 3600);
    let day = mailcraft_model::dates::day_string(start.div_euclid(86_400));
    let make = |s: &mut Session, summary: &str, time: &str, extra: Value| -> String {
        let mut p = json!({"summary": summary, "start": format!("{day}T{time}"), "end": format!("{day}T{}", if time == "09:00" { "10:00" } else { "15:00" })});
        if let (Some(o), Some(x)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(x.clone());
        }
        s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
    };
    let standup = make(&mut s, "Stand-up", "09:00", json!({"reminder": 15, "repeat": "daily"}));
    let quiet = make(&mut s, "No reminder", "09:00", json!({}));
    let review = make(&mut s, "Review", "14:00", json!({"reminder": "60"}));
    let due = |s: &Session| s.due_reminders(s.clock.now).into_iter().map(|r| r.summary).collect::<Vec<_>>();
    // A day before: nothing yet.
    s.clock.now = start - 86_400;
    assert!(due(&s).is_empty(), "{:?}", due(&s));
    // At 08:45 the stand-up's is due; the quiet one never.
    s.clock.now = start - 15 * 60;
    assert_eq!(due(&s), ["Stand-up"]);
    let key = s.due_reminders(s.clock.now)[0].key.clone();
    assert!(key.starts_with("local|") || key.contains(&standup), "{key}");
    let listed = s.run("reminders.list", &json!({})).unwrap();
    let listed: Vec<(&str, &str)> = listed.as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap(), r["summary"].as_str().unwrap())).collect();
    assert_eq!(listed, [(key.as_str(), "Stand-up")], "the reminders listed are those due");
    // Snoozed five minutes: gone, then back.
    s.run("reminders.snooze", &json!({"key": key, "minutes": 5})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now += 5 * 60;
    assert_eq!(due(&s), ["Stand-up"]);
    // Snooze until "0 minutes before start" when that's still ahead; refused once it's passed.
    s.run("reminders.snooze", &json!({"key": key, "beforeStart": 0})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now = start;
    assert_eq!(due(&s), ["Stand-up"]);
    assert!(s.run("reminders.snooze", &json!({"key": key, "beforeStart": 5})).is_err());
    // Dismissed: gone for good; tomorrow's occurrence has its own.
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now = start + 86_400 - 10 * 60;
    assert_eq!(due(&s), ["Review", "Stand-up"], "the next day's stand-up, and yesterday's review still unanswered (overdue)");
    assert_eq!(s.run("reminders.dismiss", &json!({})).unwrap()["dismissed"], 2);
    assert!(due(&s).is_empty());
    // The 14:00 review's an hour before; hiding its calendar doesn't silence it.
    s.prefs.reminders.clear();
    s.clock.now = review_at - 3600;
    s.prefs.hidden_calendars.insert("local:local".into());
    assert_eq!(due(&s), ["Stand-up", "Review"], "the stand-up's of 09:00 is still unanswered too (soonest first)");
    s.run("event.update", &json!({"uid": standup, "reminder": "none"})).unwrap();
    assert_eq!(due(&s), ["Review"]);
    // Changing its reminder to none: gone; a cancelled event never comes.
    s.run("event.update", &json!({"uid": review, "reminder": "none"})).unwrap();
    assert!(due(&s).is_empty());
    s.run("event.update", &json!({"uid": review, "reminder": 60})).unwrap();
    assert_eq!(due(&s), ["Review"]);
    s.update_event(&review, |e| e.status = "CANCELLED".into()).unwrap();
    assert!(due(&s).is_empty());
    // Long after it ended, an unanswered reminder no longer waits.
    s.update_event(&review, |e| e.status = String::new()).unwrap();
    s.clock.now = review_at + 3 * 86_400;
    assert!(!due(&s).contains(&"Review".to_string()));
    // Junk.
    for bad in
        [json!({"key": "nope", "minutes": 5}), json!({"key": key}), json!({"key": key, "minutes": 0}), json!({"key": key, "minutes": 99_999_999})]
    {
        assert!(s.run("reminders.snooze", &bad).is_err(), "{bad}");
    }
    assert!(s.run("reminders.dismiss", &json!({"key": "nope"})).is_err());
    for bad in [json!(-1), json!(999_999), json!("soon"), json!(true)] {
        assert!(s.run("event.update", &json!({"uid": quiet, "reminder": bad})).is_err(), "{bad}");
    }
}

/// The work week: Monday to Friday of the week the day is in, moving a week at a time.
#[test]
fn the_work_week_is_monday_to_friday() {
    let mut s = sample_session("work-week");
    // Saturday 2026-10-24.
    s.run("calendar.goto", &json!({"date": "2026-10-24"})).unwrap();
    let r = s.run("calendar.view", &json!({"mode": "workWeek"})).unwrap();
    assert_eq!(r["mode"], "workweek");
    let (d0, d1) = s.cal_days();
    assert_eq!((mailcraft_model::dates::day_string(d0), d1 - d0), ("2026-10-19".to_string(), 5));
    s.run("calendar.goto", &json!({"step": 1})).unwrap();
    assert_eq!(mailcraft_model::dates::day_string(s.cal_days().0), "2026-10-26");
    // What the calendar shows is that week's: the sample's stand-up, Monday, Wednesday and Friday.
    let events = s.run("calendar.events", &json!({})).unwrap();
    let shown: Vec<(String, String)> = events
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["summary"].as_str().unwrap().to_string(), mailcraft_model::dates::day_string(e["start"].as_i64().unwrap().div_euclid(86_400))))
        .collect();
    let standup = |day: &str| ("Team stand-up".to_string(), day.to_string());
    assert_eq!(shown, [standup("2026-10-26"), standup("2026-10-28"), standup("2026-10-30")]);
    assert!(s.run("calendar.view", &json!({"mode": "fortnight"})).is_err());
}

/// Issue #7: a calendar server given by hand (Posteo's, which isn't found from the address), with
/// a password of its own. Kept with the account (the password apart, in the keychain), it
/// reconnects so the calendars are looked for again, what was found before forgotten; a new
/// password alone just syncs; plain http across the network and accounts whose calendars come
/// with them are refused.
#[test]
fn a_calendar_server_given_by_hand() {
    let mut s = sample_session("dav-given");
    let id = s
        .run(
            "account.add",
            &json!({"email": "zoe@posteo.example", "password": "mail-pw", "imapHost": "posteo.example", "smtpHost": "posteo.example"}),
        )
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.pim_mut(&id).caldav_home = Some(String::new());
    s.pim_mut(&id).dav_checked = Some(100);
    let given = json!({"account": id, "caldav": "posteo.example:8443/calendars/zoe/default", "username": "zoe@posteo.example"});
    assert!(s.run("account.dav", &given).unwrap_err().to_string().contains("password is required"));
    let mut given = given;
    given["password"] = json!("app-pw");
    s.run("account.dav", &given).unwrap();
    let dav = s.account(&id).unwrap().config.dav.clone().unwrap();
    assert_eq!(dav.caldav, "https://posteo.example:8443/calendars/zoe/default", "typed without https, read as https");
    assert_eq!((dav.carddav.as_str(), dav.username.as_deref()), ("", Some("zoe@posteo.example")));
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::DavPassword { password, .. }, Job::Start { account, .. }] if password.0 == "app-pw" && account.dav.is_some()),
        "{jobs:?}"
    );
    assert!(s.pim[&id].caldav_home.is_none() && s.pim[&id].dav_checked.is_none(), "looked for again");
    let saved = std::fs::read_to_string(s.store.as_ref().unwrap().root().join("accounts.json")).unwrap();
    assert!(saved.contains("calendars/zoe/default") && !saved.contains("app-pw"), "the password stays out of MailCraft's files");
    // A new password alone: kept, and the calendars synced with it, no reconnecting.
    s.run("account.dav", &json!({"account": id, "password": "app-pw-2"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::DavPassword { password, .. }, Job::SyncDav { .. }] if password.0 == "app-pw-2"));
    // Nothing changed: nothing to do.
    s.run("account.dav", &json!({"account": id})).unwrap();
    assert!(s.take_jobs().is_empty());
    // Another user name needs its own password: the one kept was the other user's.
    assert!(s.run("account.dav", &json!({"account": id, "username": "other@posteo.example"})).unwrap_err().to_string().contains("password"));
    // Another server: what came from the old one goes, except what was changed here and not yet
    // sent, which stays on this computer instead of going to the old server with this password.
    {
        let p = s.pim_mut(&id);
        let old = "https://caldav.posteo.example/cal/work/".to_string();
        p.calendars.push(mailcraft_model::pim::CalendarInfo { id: old.clone(), name: "Work".into(), ..Default::default() });
        for (uid, dirty) in [("synced", false), ("edited", true)] {
            p.events.push(mailcraft_model::pim::StoredEvent {
                calendar: old.clone(),
                href: format!("{old}{uid}.ics"),
                etag: "\"1\"".into(),
                event: mailcraft_model::ical::CalEvent {
                    uid: uid.into(),
                    summary: uid.into(),
                    start: 1_800_000_000,
                    end: 1_800_003_600,
                    ..Default::default()
                },
                dirty,
            });
        }
        p.deleted.push(mailcraft_model::pim::Tombstone { href: format!("{old}gone.ics"), ..Default::default() });
    }
    s.run("account.dav", &json!({"account": id, "caldav": "https://dav.other.example/"})).unwrap();
    let p = &s.pim[&id];
    assert!(p.deleted.is_empty() && p.calendars.iter().all(|c| c.id == mailcraft_model::pim::LOCAL));
    assert_eq!(p.events.iter().map(|e| (e.event.uid.as_str(), e.calendar.as_str(), e.href.as_str())).collect::<Vec<_>>(), [("edited", "local", "")]);
    s.take_jobs();
    // Cleared: found from the address again, with the mail's sign-in; its own password goes.
    s.run("account.dav", &json!({"account": id, "caldav": "", "username": null})).unwrap();
    assert!(s.account(&id).unwrap().config.dav.is_none());
    assert!(matches!(s.take_jobs().as_slice(), [Job::DavPassword { password, .. }, Job::Start { .. }] if password.0.is_empty()));
    // Refused: http across the network, junk, and the sample (its calendar is its own).
    for bad in ["http://posteo.example/", "ftp://posteo.example/", "https://", "a b"] {
        assert!(s.run("account.dav", &json!({"account": id, "caldav": bad})).is_err(), "{bad}");
    }
    s.run("account.dav", &json!({"account": id, "carddav": "http://localhost:5232/zoe/"})).unwrap();
    assert!(s.run("account.dav", &json!({"account": "sample", "caldav": "https://dav.example/"})).is_err());
}

/// An event at the very end of time in the cache (a damaged file) doesn't stop the calendar.
#[test]
fn an_event_at_the_end_of_time_is_drawn_without_overflow() {
    let mut s = sample_session("calendar-end-of-time");
    let e = mailcraft_model::ical::CalEvent { uid: "end".into(), summary: "End".into(), start: i64::MAX, end: i64::MAX, ..Default::default() };
    s.pim_mut("sample").events.push(mailcraft_model::pim::StoredEvent {
        calendar: mailcraft_model::pim::LOCAL.into(),
        event: e,
        ..Default::default()
    });
    for mode in ["day", "week", "month"] {
        s.run("calendar.view", &json!({"mode": mode})).unwrap();
        assert!(s.run("calendar.events", &json!({})).is_ok());
    }
    let _ = s.agenda(i64::MAX - 100, i64::MAX);
    let _ = s.agenda(i64::MIN, i64::MIN + 100);
}

/// Issue #142: Update Folder (Shift+F9) in Calendar, People, Tasks or Notes reads the accounts'
/// calendars, contacts, tasks and notes again, as Outlook's does there; in Mail, the folder shown.
/// It read the mail folder chosen behind them, and the calendar wasn't read again.
#[test]
fn update_folder_outside_mail_reads_the_calendars_again() {
    let mut s = Session::open(Some(temp("update-folder-pim")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@studio.example", "password": "pw", "imapHost": "imap.example", "smtpHost": "smtp.example"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).ok();
    for module in ["calendar", "people", "tasks", "notes"] {
        s.run("view.module", &json!({"name": module})).unwrap();
        s.take_jobs();
        s.run("folder.sync", &json!({})).unwrap();
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "{module}: {jobs:?}");
        assert!(!jobs.iter().any(|j| matches!(j, Job::SyncFolder { .. })), "{module}: {jobs:?}");
    }
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.take_jobs();
    s.run("folder.sync", &json!({})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "in Mail, the folder");
}

/// Issue #235: one occurrence of a meeting called off says which, in its subject and words: they
/// read as if the whole series were cancelled. The whole one says as before.
#[test]
fn an_occurrence_called_off_says_which() {
    let mut s = sample_session("meetings-occurrence-off");
    let uid = s
        .run("event.create", &json!({"summary": "Stand-up", "start": "2026-10-21T09:00", "repeat": "weekly", "attendees": "zoe@studio.example"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let second = s.find_event(&uid).unwrap().1.event.start + 7 * 86_400;
    s.run("event.delete", &json!({"uid": uid, "occurrence": second})).unwrap();
    let when = mailcraft_model::dates::full_time(second, &s.clock);
    let (method, raw) = sent_calendar(&s, "Sent", &format!("Cancelled: Stand-up ({when})"));
    assert_eq!(method, "CANCEL");
    assert!(raw.contains("RECURRENCE-ID") && raw.contains(&format!("Stand-up on {when} has been cancelled.")), "{raw}");
}
