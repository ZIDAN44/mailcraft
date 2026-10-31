//! Issue #7: calendars subscribed to by their links (iCal, `webcal://`), read-only, read again
//! now and then.

use super::*;

const HOLIDAYS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Holidays//EN\r\nX-WR-CALNAME:Public holidays\r\n\
X-PUBLISHED-TTL:PT12H\r\nBEGIN:VEVENT\r\nUID:new-year@holidays.example\r\nDTSTART;VALUE=DATE:20270101\r\n\
SUMMARY:New Year's Day\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:20261225\r\nSUMMARY:Christmas Day\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// The fetch the session asked for: (account, calendar, url, etag).
fn fetches(s: &mut Session) -> Vec<(String, String, String, String)> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::FetchCalendar { account, calendar, url, etag, .. } => Some((account, calendar, url, etag)),
            _ => None,
        })
        .collect()
}

fn read(s: &mut Session, account: &str, calendar: &str, ics: &str, etag: &str) {
    s.apply(Event::CalendarFetched {
        account: account.into(),
        calendar: calendar.into(),
        ics: Some(ics.into()),
        etag: etag.into(),
        modified: String::new(),
        unchanged: false,
        error: None,
    });
}

fn summaries(s: &Session, account: &str, calendar: &str) -> Vec<String> {
    let mut v: Vec<String> = s.pim[account].events.iter().filter(|e| e.calendar == calendar).map(|e| e.event.summary.clone()).collect();
    v.sort();
    v
}

#[test]
fn a_calendar_is_subscribed_to_by_its_link() {
    let mut s = sample_session("web-calendars");
    s.take_jobs();
    // Not a calendar link: refused before anything is asked.
    for bad in ["", "ftp://x.example/a.ics", "webcal://", "https://user:pw@x.example/a.ics", "https://x.example/a b.ics", "javascript:alert(1)"] {
        assert!(s.run("calendar.subscribe", &json!({"url": bad})).is_err(), "{bad:?}");
    }
    assert!(s.run("calendar.subscribe", &json!({"url": format!("https://x.example/{}", "a".repeat(3000))})).is_err());
    assert!(fetches(&mut s).is_empty());

    // webcal:// is read over HTTPS, at once.
    let r = s.run("calendar.subscribe", &json!({"url": " WEBCAL://Holidays.example/de.ics "})).unwrap();
    let (account, id) = (r["account"].as_str().unwrap().to_string(), r["calendar"].as_str().unwrap().to_string());
    assert_eq!(fetches(&mut s), [(account.clone(), id.clone(), "https://Holidays.example/de.ics".to_string(), String::new())]);
    let cal = s.pim[&account].calendars.iter().find(|c| c.id == id).unwrap().clone();
    assert!(cal.read_only);
    assert_eq!(cal.name, "Holidays.example", "named for its host until it's read");
    assert!(s.run("calendar.subscribe", &json!({"url": "webcals://Holidays.example/de.ics"})).is_err(), "subscribed already");

    // Read: its events, its own name, read-only; no reminders from it.
    read(&mut s, &account, &id, HOLIDAYS, "\"v1\"");
    assert_eq!(summaries(&s, &account, &id), ["Christmas Day", "New Year's Day"]);
    assert_eq!(s.pim[&account].calendars.iter().find(|c| c.id == id).unwrap().name, "Public holidays");
    let uid = s.pim[&account].events.iter().find(|e| e.event.summary == "Christmas Day").unwrap().event.uid.clone();
    assert!(!uid.is_empty(), "an event without a uid gets one");
    assert!(s.run("event.update", &json!({"uid": "new-year@holidays.example", "summary": "Mine"})).is_err(), "read-only");
    assert!(s.run("event.delete", &json!({"uid": "new-year@holidays.example"})).is_err(), "nor deleted");
    assert!(s.run("event.delete", &json!({"uid": "new-year@holidays.example", "occurrence": 1_798_761_600})).is_err());
    assert_eq!(summaries(&s, &account, &id).len(), 2);
    assert!(s.run("event.create", &json!({"calendar": id, "summary": "x", "start": 1_800_000_000, "end": 1_800_003_600})).is_err());
    let new_year = 1_798_761_600; // 2027-01-01 00:00 UTC
    assert!(s.due_reminders(new_year).iter().all(|r| r.uid != "new-year@holidays.example"), "a subscribed calendar's alarms aren't ours");

    // Read again when the calendar asks (12 hours), asking only for what changed.
    s.clock.now += 11 * 3600;
    s.tick();
    assert!(fetches(&mut s).is_empty());
    s.clock.now += 3600 + 1;
    s.tick();
    assert_eq!(fetches(&mut s).first().map(|f| f.3.clone()), Some("\"v1\"".to_string()));
    s.apply(Event::CalendarFetched {
        account: account.clone(),
        calendar: id.clone(),
        ics: None,
        etag: "\"v1\"".into(),
        modified: String::new(),
        unchanged: true,
        error: None,
    });
    assert_eq!(summaries(&s, &account, &id).len(), 2, "unchanged: kept");
    s.tick();
    assert!(fetches(&mut s).is_empty(), "asked once");
    // The clock set back: read again rather than wait for it.
    let now = s.clock.now;
    s.clock.now -= 86_400;
    s.tick();
    assert_eq!(fetches(&mut s).len(), 1, "clock set back");
    s.clock.now = now;
    read(&mut s, &account, &id, HOLIDAYS, "\"v1\"");

    // A failure keeps what was read, says why, and is tried again in half an hour.
    s.clock.now += 12 * 3600 + 1;
    s.tick();
    assert_eq!(fetches(&mut s).len(), 1);
    s.apply(Event::CalendarFetched {
        account: account.clone(),
        calendar: id.clone(),
        ics: None,
        etag: String::new(),
        modified: String::new(),
        unchanged: false,
        error: Some("HTTP 503".into()),
    });
    assert_eq!(summaries(&s, &account, &id).len(), 2);
    assert_eq!(s.pim[&account].subscribed[0].error.as_deref(), Some("HTTP 503"));
    s.clock.now += 29 * 60;
    s.tick();
    assert!(fetches(&mut s).is_empty());
    s.clock.now += 61;
    s.tick();
    assert_eq!(fetches(&mut s).len(), 1);
    // Something that isn't a calendar (a login page): an error, the events kept.
    read(&mut s, &account, &id, "<html><body>Sign in</body></html>", "");
    assert!(s.pim[&account].subscribed[0].error.as_deref().is_some_and(|e| e.contains("calendar")));
    assert_eq!(summaries(&s, &account, &id).len(), 2);
    // Changed: the new events replace the old.
    read(&mut s, &account, &id, &HOLIDAYS.replace("Christmas Day", "Christmas"), "\"v2\"");
    assert_eq!(summaries(&s, &account, &id), ["Christmas", "New Year's Day"]);
    assert_eq!(s.pim[&account].subscribed[0].error, None);

    // Refresh reads it now; kept across a restart, with its events.
    s.run("calendar.refreshSubscribed", &json!({})).unwrap();
    assert_eq!(fetches(&mut s).len(), 1);
    let saved: mailcraft_model::pim::Pim = s.store.as_ref().unwrap().load_account_doc(&account, mailcraft_model::pim::PIM_DOC).unwrap();
    assert_eq!(saved.subscribed.first().map(|w| w.url.as_str()), Some("https://Holidays.example/de.ics"));
    assert!(saved.events.iter().any(|e| e.calendar == id && e.event.summary == "New Year's Day"));

    // Removed: it and its events go.
    s.run("calendar.unsubscribe", &json!({"calendar": id})).unwrap();
    assert!(s.pim[&account].calendars.iter().all(|c| c.id != id));
    assert!(summaries(&s, &account, &id).is_empty());
    assert!(s.run("calendar.unsubscribe", &json!({"calendar": id})).is_err());
    assert!(s.run("calendar.refreshSubscribed", &json!({})).is_err(), "none left");
    done(s);
}

/// A calendar named by the user keeps that name; a sync of the account's own calendars (CalDAV)
/// leaves subscribed ones alone, even when an event's uid is the same; an answer for a calendar
/// no longer subscribed to is dropped.
#[test]
fn a_subscribed_calendar_survives_the_accounts_sync() {
    let mut s = sample_session("web-calendars-sync");
    let r = s.run("calendar.subscribe", &json!({"url": "https://holidays.example/de.ics", "name": "Feiertage"})).unwrap();
    let (account, id) = (r["account"].as_str().unwrap().to_string(), r["calendar"].as_str().unwrap().to_string());
    s.take_jobs();
    read(&mut s, &account, &id, HOLIDAYS, "");
    assert_eq!(s.pim[&account].calendars.iter().find(|c| c.id == id).unwrap().name, "Feiertage");

    let server = mailcraft_model::pim::CalendarInfo { id: "/cal/work/".into(), name: "Work".into(), ..Default::default() };
    let mut clash = mailcraft_model::ical::parse(HOLIDAYS, 0).events.remove(0);
    clash.summary = "Office closed".into();
    let sync = mailcraft_model::pim::DavSync {
        calendars: Some(vec![server]),
        synced_calendars: vec!["/cal/work/".into()],
        events: vec![mailcraft_model::pim::StoredEvent {
            calendar: "/cal/work/".into(),
            href: "/cal/work/1.ics".into(),
            event: clash,
            ..Default::default()
        }],
        ..Default::default()
    };
    s.apply_dav(&account, &sync);
    assert!(s.pim[&account].calendars.iter().any(|c| c.id == id), "still there");
    assert_eq!(summaries(&s, &account, &id), ["Christmas Day", "New Year's Day"]);
    assert_eq!(summaries(&s, &account, "/cal/work/"), ["Office closed"]);

    s.run("calendar.unsubscribe", &json!({"calendar": "https://holidays.example/de.ics"})).unwrap();
    read(&mut s, &account, &id, HOLIDAYS, "");
    assert!(summaries(&s, &account, &id).is_empty(), "a late answer brings nothing back");
    done(s);
}
