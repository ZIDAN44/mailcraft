//! Engine tests: calendars, people, notes and reminders.

use super::*;

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
    done(s);
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
fn people_in_the_address_book() {
    let mut s = sample_session("people");
    let all = s.run("contact.list", &json!({})).unwrap();
    assert!(all.as_array().unwrap().len() >= 5);
    let hits = s.run("contact.list", &json!({"query": "lantern"})).unwrap();
    assert!(hits.as_array().unwrap().iter().all(|p| p["org"] == "Studio Lantern"));
    let uid = s
        .run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example, mira.chen@home.example", "org": "Colour House"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.run("contact.create", &json!({"name": "Bad", "emails": ["not an address"]})).is_err());
    assert!(s.run("contact.create", &json!({})).is_err(), "a name or address is needed");
    // Saved people are suggested before people only seen in mail.
    let top: Vec<String> = s.suggest_contacts("mi", 2).into_iter().map(|c| c.address.email).collect();
    assert!(top.contains(&"mira@colour.example".to_string()), "{top:?}");
    s.run("contact.update", &json!({"uid": uid, "phones": ["+1 555 0101"], "title": "Colourist"})).unwrap();
    let p = s.find_person(&uid).unwrap();
    assert_eq!((p.card.phones[0].value.as_str(), p.card.title.as_str(), p.card.emails.len()), ("+1 555 0101", "Colourist", 2));
    s.run("contact.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_person(&uid).is_none());
    // The sender of a message, once.
    let k = s.list.iter().find(|m| m.from.email == "win@prizes.example").map(|m| m.key.clone());
    if let Some(k) = k {
        s.run("list.open", &json!({"key": k.to_id()})).unwrap();
        let first = s.run("contact.fromMessage", &json!({})).unwrap();
        let second = s.run("contact.fromMessage", &json!({})).unwrap();
        assert_eq!(first["uid"], second["uid"]);
        assert_eq!(second["existing"], true);
    }
    done(s);
}

#[test]
fn invitations_are_answered_and_sent() {
    let mut s = sample_session("invites");
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
    assert!(raw.contains("To: \"Ana Lima\" <ana@soundlab.example>") || raw.contains("ana@soundlab.example"));

    // Declining takes it out of the calendar.
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert!(s.find_event("mix@sample.mailcraft").is_none());
    assert_eq!(sent_calendar(&s, "Sent", "Declined: Sound mix playback").0, "REPLY");
    // Accepting again puts it back.
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    let e = s.find_event("mix@sample.mailcraft").unwrap().1.event.clone();
    assert!(e.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "TENTATIVE"));
    done(s);
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
    let ics = mailcraft_model::ical::write(&[reply], Some("REPLY"), NOW);
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: "Accepted: Trailer review".into(),
        body: "Zoe accepted.".into(),
        attachments: vec![mailcraft_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: "text/calendar; method=REPLY".into(),
            data: ics.into_bytes(),
        }],
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example"), NOW, "<r1@x>").unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 999 };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
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
    done(s);
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
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "a".into() }]),
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
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "b".into() }]),
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
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "c".into() }]),
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
        calendars: Some(vec![CalendarInfo { id: server_cal.clone(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() }]),
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
    done(s);
    done(s2);
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
    done(s);

    // An Exchange account: notes go to its Notes folder, and what the server says follows.
    let mut s = Session::open(Some(temp("notes-ews")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let uid = s.run("note.create", &json!({"text": "Sync me"})).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    assert!(s.pim[&id].notes[0].dirty);
    let stored = mailcraft_model::pim::StoredNote { href: "note:X1".into(), note: s.pim[&id].notes[0].note.clone(), ..Default::default() };
    let server_copy =
        mailcraft_model::pim::StoredNote { note: mailcraft_model::pim::Note { uid: "note:X1".into(), ..stored.note.clone() }, ..stored.clone() };
    let other = mailcraft_model::pim::StoredNote {
        href: "note:X2".into(),
        note: mailcraft_model::pim::Note { uid: "note:X2".into(), text: "From Outlook".into(), color: 2, modified: 1 },
        ..Default::default()
    };
    let sync = mailcraft_model::pim::DavSync { pushed_notes: vec![stored], notes: Some(vec![server_copy, other]), ..Default::default() };
    s.apply_dav(&id, &sync);
    let texts: Vec<String> = s.notes("").into_iter().map(|n| n.note.text).collect();
    assert_eq!(texts.len(), 2, "{texts:?}: the note sent isn't doubled");
    assert!(texts.contains(&"From Outlook".to_string()) && texts.contains(&"Sync me".to_string()));
    let _ = uid;
    s.run("note.delete", &json!({"uid": "note:X2"})).unwrap();
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "note:X2"), "deleted on the server next time");
    done(s);
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

/// Issue #1: a signature goes into the message being written from the message itself.
#[test]
fn a_signature_goes_in_from_the_message() {
    let mut s = sample_session("insert-signature");
    s.run("account.setSignature", &json!({"account": "sample", "signature": ""})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.compose.as_mut().unwrap().body = "Hello there".into();
    assert!(s.run("compose.insertSignature", &json!({})).is_err(), "no signature yet");
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley\nStudio"})).unwrap();
    let r = s.run("compose.insertSignature", &json!({"at": 5})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hello\n-- \nRiley\nStudio\n there");
    assert_eq!(r["end"], 23);
    // One already there is replaced, wherever the cursor is.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R."})).unwrap();
    s.run("compose.insertSignature", &json!({"at": 9999})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hello\n-- \nR.\n there");
    // A reply made with the old signature: that one is replaced.
    let c = s.compose.as_mut().unwrap();
    c.body = "Hi\n\n-- \nOld\n\n> quoted".into();
    c.signature = "-- \nOld".into();
    s.run("compose.insertSignature", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hi\n\n-- \nR.\n\n> quoted");
}

/// Issue #1: a signature pasted from Outlook keeps its look: kept as HTML, and put into new
/// messages, replies and Insert signature with it; its text is the plain-text twin.
#[test]
fn a_formatted_signature_keeps_its_look() {
    let mut s = sample_session("formatted-signature");
    let html = "<p class=MsoNormal><b><span style='color:#BFBFBF'>Company bv</span></b></p>\
                <p class=MsoNormal>An street 1<br><a href=\"http://www.company.com/\">www.company.com</a></p>\
                <p class=MsoNormal><i>Confidential.</i></p>";
    s.run("account.setSignature", &json!({"account": "sample", "signature": "ignored", "html": html})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert_eq!(config.signature, "Company bv\nAn street 1\nwww.company.com\nConfidential.");
    assert!(!config.signature_html.is_empty());
    let check = |s: &Session| {
        let c = s.compose.as_ref().unwrap();
        let at = c.body.find("Company bv").map(|b| c.body[..b].chars().count()).unwrap();
        let st = c.styles.style(at);
        assert!(st.bold && st.color == Some([0xBF, 0xBF, 0xBF]), "{st:?}");
        let link = c.body.find("www.company").map(|b| c.body[..b].chars().count()).unwrap();
        assert_eq!(c.styles.style(link).link.as_deref(), Some("http://www.company.com/"));
        assert!(!c.styles.style(0).bold, "the text before it stays as it was");
        assert_eq!(c.styles.chars.len(), c.body.chars().count());
        let html = mailcraft_mime::rich::to_html(&c.body, &c.styles);
        assert!(html.contains("http://www.company.com/"), "{html}");
    };
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().body.ends_with("-- \nCompany bv\nAn street 1\nwww.company.com\nConfidential."));
    check(&s);
    assert!(!s.compose_written(), "a message with only its signature has nothing written");
    // Insert signature replaces it, keeping the look.
    s.run("compose.insertSignature", &json!({})).unwrap();
    check(&s);
    s.run("compose.discard", &json!({})).ok();
    s.compose = None;
    let k = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    check(&s);
    // Saved plain again, the look goes.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R."})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_html.is_empty());
    assert!(s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": "<p><b>\u{0}"})).is_ok(), "junk HTML");
}

/// Issue #1: a contact's numbers keep what they are (business, home, mobile…), and its
/// addresses and birthday are fields of their own, as in Outlook.
#[test]
fn contacts_keep_typed_numbers_addresses_and_birthdays() {
    let mut s = sample_session("contact-fields");
    let uid = s
        .run(
            "contact.create",
            &json!({
                "name": "Jos Peeters",
                "emails": ["jos@company.example", "jos@home.example"],
                "phones": [{"value": "+32 2 123 45 67", "kind": "work"}, {"value": "+32 490 12 34 56", "kind": "cell"}, {"value": "+32 9 765 43 21", "kind": "home"}],
                "addresses": [{"kind": "work", "street": "An street 1", "postcode": "1000", "city": "Brussel", "country": "Belgium"}, {"kind": "home", "city": "Gent"}, {"street": " "}],
                "birthday": "1980-02-29",
            }),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.find_person(&uid).unwrap().card;
    let kinds: Vec<(&str, &str)> = c.phones.iter().map(|p| (p.kind.as_str(), p.value.as_str())).collect();
    assert_eq!(kinds, [("work", "+32 2 123 45 67"), ("cell", "+32 490 12 34 56"), ("home", "+32 9 765 43 21")]);
    assert_eq!(c.addresses.len(), 2, "an empty one isn't kept");
    assert_eq!(c.addresses[0].one_line(), "An street 1, 1000 Brussel, Belgium");
    assert_eq!(c.birthday, "1980-02-29");
    assert_eq!(c.emails.len(), 2);
    // Changing one thing leaves the rest; a birthday that isn't a date is refused.
    s.run("contact.update", &json!({"uid": uid, "title": "Owner"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 3);
    assert!(s.run("contact.update", &json!({"uid": uid, "birthday": "next tuesday"})).is_err());
    s.run("contact.update", &json!({"uid": uid, "birthday": ""})).unwrap();
    assert!(s.find_person(&uid).unwrap().card.birthday.is_empty());
    // Strings still work for phones (kind unknown).
    s.run("contact.update", &json!({"uid": uid, "phones": "+1 555 0100, +1 555 0101"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 2);
}

/// Issue #1: Select Names opens with the address book chosen (Outlook's "Show this address list
/// first"), and one address book can be shown alone.
#[test]
fn the_address_book_opens_with_the_one_chosen() {
    let mut s = sample_session("default-book");
    let all = s.run("contact.addressBook", &json!({})).unwrap();
    let people = s.run("contact.addressBook", &json!({"source": "people"})).unwrap();
    assert!(all.as_array().unwrap().len() >= people.as_array().unwrap().len());
    let books = s.address_books();
    let (book, _, _) = books.first().cloned().expect("the sample has an address book");
    assert!(s.run("contact.defaultBook", &json!({"source": "book:nobody:x"})).is_err());
    s.run("contact.defaultBook", &json!({"source": book})).unwrap();
    assert_eq!(s.prefs.address_book, book);
    let mine = s.run("contact.addressBook", &json!({})).unwrap();
    assert_eq!(mine, s.run("contact.addressBook", &json!({"source": book})).unwrap(), "it opens with the one chosen");
    assert!(!mine.as_array().unwrap().is_empty());
    let (account, id) = book.strip_prefix("book:").and_then(|b| b.split_once(':')).unwrap();
    let in_book = s.people("").into_iter().filter(|p| p.account == account && p.book == id).flat_map(|p| p.card.emails).count();
    assert_eq!(mine.as_array().unwrap().len(), in_book);
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.address_book, book, "kept with the settings");
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
    let day = mailcraft_engine_day(&s);
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

/// Issue #1: an organisation's shared contacts (Exchange public folders) can be read, not
/// changed or deleted.
#[test]
fn shared_contacts_are_read_only() {
    use mailcraft_model::pim::{BookInfo, DavSync, StoredCard};
    let mut s = sample_session("shared-contacts");
    let card = mailcraft_model::vcard::Card { uid: "desk".into(), name: "Front desk".into(), ..Default::default() };
    let mut books = s.pim.get("sample").unwrap().books.clone();
    books.push(BookInfo { id: "public:pc".into(), name: "Company contacts".into(), read_only: true, ctag: String::new() });
    let sync = DavSync {
        books: Some(books),
        cards: vec![StoredCard { book: "public:pc".into(), href: "public:desk".into(), card, ..Default::default() }],
        synced_books: vec!["public:pc".into()],
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    assert!(s.find_person("desk").is_some(), "shown with the others");
    assert!(s.person_read_only("desk"));
    assert!(s.run("contact.update", &json!({"uid": "desk", "name": "Reception"})).is_err());
    assert!(s.run("contact.delete", &json!({"uid": "desk"})).is_err());
    assert!(s.find_person("desk").is_some());
    // New contacts go to a book that can take them.
    let made = s.run("contact.create", &json!({"name": "New one"})).unwrap();
    assert!(!s.person_read_only(made["uid"].as_str().unwrap()));
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
    assert!(s.run("reminders.list", &json!({})).unwrap().is_array());
    done(s);
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
    let events = s.run("calendar.events", &json!({})).unwrap();
    assert!(events.as_array().is_some());
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
    // Cleared: found from the address again, with the mail's sign-in.
    s.run("account.dav", &json!({"account": id, "caldav": "", "username": null})).unwrap();
    assert!(s.account(&id).unwrap().config.dav.is_none());
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { .. }]));
    // Refused: http across the network, junk, and the sample (its calendar is its own).
    for bad in ["http://posteo.example/", "ftp://posteo.example/", "https://", "a b"] {
        assert!(s.run("account.dav", &json!({"account": id, "caldav": bad})).is_err(), "{bad}");
    }
    s.run("account.dav", &json!({"account": id, "carddav": "http://localhost:5232/zoe/"})).unwrap();
    assert!(s.run("account.dav", &json!({"account": "sample", "caldav": "https://dav.example/"})).is_err());
    done(s);
}
