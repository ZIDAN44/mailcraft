//! MailCraft's calendar against SOGo in a real Mailcow on this computer: private events and Show
//! as (issue #32) as SOGo stores them and as it serves them to colleagues by its roles, the
//! calendars a colleague subscribed to in SOGo (issue #30), online meetings (issue #25) and the
//! calendar's search over them all (issue #33). Never a real account; run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//!   cargo test -p mailcraft-cli --test real_mailcow_calendar -- --ignored --nocapture --test-threads 1
//! ```
//!
//! See `real_mailcow_mail.rs` for the Mailcow it needs. ann's personal calendar is shared in
//! SOGo with bob ("see date and time only" for private and confidential events, everything of
//! public ones) and carol (nothing of private ones, everything else), both subscribed to it.

#[path = "support/mailcow.rs"]
mod mailcow;

use mailcow::{App, Mailcow};
use mailcraft_engine::model::ical::{Privacy, ShowAs, privacy, show_as};
use serde_json::{Value, json};

/// Events the tests made, by their titles' start.
const MARK: &str = "Mailcow test";

/// Take out what an earlier run left in ann's calendar.
fn clean(m: &Mailcow) {
    let cal = format!("{}personal/", m.dav_home(0));
    for (href, data) in m.calendar(0, &cal) {
        if data.contains(&format!("SUMMARY:{MARK}")) {
            let (st, _) = m.dav(0, "DELETE", &format!("{}{href}", m.web), None, "");
            eprintln!("cleaned {href}: {st}");
        }
    }
}

/// The calendar data stored for a title (as ann sees it).
fn stored(m: &Mailcow, summary: &str) -> String {
    let cal = format!("{}personal/", m.dav_home(0));
    m.calendar(0, &cal)
        .into_iter()
        .map(|(_, d)| d)
        .find(|d| d.contains(&format!("SUMMARY:{summary}\r")) || d.contains(&format!("SUMMARY:{summary}\n")))
        .unwrap_or_else(|| panic!("{summary} not in SOGo"))
}

/// The one event of an item.
fn vevent(ics: &str) -> String {
    ics.split("BEGIN:VEVENT").nth(1).and_then(|v| v.split("END:VEVENT").next()).unwrap_or_default().to_string()
}

/// ann's calendar shared in SOGo with bob and carol, by its roles per classification.
fn share(m: &Mailcow) {
    let sogo = m.sogo(0);
    let rights = |private: &str, confidential: &str| json!({"Public": "Viewer", "Confidential": confidential, "Private": private, "canCreateObjects": 0, "canEraseObjects": 0});
    sogo.share("Calendar/personal", m.user(1).0, rights("DAndTViewer", "DAndTViewer"), true);
    sogo.share("Calendar/personal", m.user(2).0, rights("None", "Viewer"), true);
    eprintln!("ann's calendar ACL: {}", sogo.get("Calendar/personal/acls").text());
}

fn events(app: &mut App) -> Vec<Value> {
    app.s.run("calendar.events", &json!({"from": "2026-11-15", "to": "2026-11-22"})).unwrap().as_array().cloned().unwrap_or_default()
}

/// Issues #32, #30, #25 and #33 against SOGo.
#[test]
#[ignore]
fn real_mailcow_private_events_and_shared_calendars() {
    let Some(m) = Mailcow::from_env() else { return };
    clean(&m);
    share(&m);
    let web = format!("{}/", m.web);

    // ann's MailCraft makes four events: private and out of office, show time only and busy,
    // public and free, and one with an online meeting.
    let (mut ann, aid) = App::new(&m, 0, "calendar");
    ann.s.run("account.dav", &json!({"account": aid, "caldav": web})).unwrap();
    let sync = ann.sync_dav(&aid);
    let mine = ann.s.pim[&aid].default_calendar().map(|c| c.id.clone()).expect("ann's default calendar");
    eprintln!("ann's calendars: {:?}, default {mine}", sync.calendars.as_ref().map(|c| c.iter().map(|c| c.id.clone()).collect::<Vec<_>>()));
    assert!(mine.ends_with(&format!("/SOGo/dav/{}/Calendar/personal/", m.user(0).0)));
    ann.s.run("meetings.addRoom", &json!({"name": "BBB", "url": "https://meet.example.edu/rooms/mc3-1sg-og0", "code": "4711"})).unwrap();
    let make = |ann: &mut App, title: &str, start: &str, extra: Value| -> String {
        let mut v = json!({"account": aid, "summary": format!("{MARK} {title}"), "start": start, "location": format!("{title} room"), "description": format!("{title} notes")});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        ann.s.run("event.create", &v).unwrap()["uid"].as_str().unwrap().to_string()
    };
    let private = make(&mut ann, "Doctor", "2026-11-16T09:00", json!({"privacy": "private", "showAs": "oof"}));
    let confidential = make(&mut ann, "Interview", "2026-11-16T11:00", json!({"privacy": "confidential", "showAs": "busy"}));
    let public = make(&mut ann, "Lunch", "2026-11-16T13:00", json!({"showAs": "free"}));
    let online = make(&mut ann, "Standup", "2026-11-16T15:00", json!({"onlineMeeting": "BBB"}));
    ann.pump(120, "stored in SOGo", |s, _| {
        s.pim[&aid].events.iter().filter(|e| e.event.summary.starts_with(MARK)).all(|e| !e.dirty && !e.href.is_empty())
    });

    // As SOGo stores them.
    let doctor = vevent(&stored(&m, &format!("{MARK} Doctor")));
    eprintln!("--- private, as SOGo stores it:\n{doctor}");
    for want in ["CLASS:PRIVATE", "TRANSP:OPAQUE", "X-MICROSOFT-CDO-BUSYSTATUS:OOF", "LOCATION:Doctor room"] {
        assert!(doctor.contains(want), "{want}");
    }
    let interview = vevent(&stored(&m, &format!("{MARK} Interview")));
    // Busy is what an event without TRANSP is: nothing to add.
    assert!(interview.contains("CLASS:CONFIDENTIAL") && !interview.contains("TRANSP:TRANSPARENT"), "{interview}");
    let lunch = vevent(&stored(&m, &format!("{MARK} Lunch")));
    assert!(lunch.contains("TRANSP:TRANSPARENT") && lunch.contains("X-MICROSOFT-CDO-BUSYSTATUS:FREE"), "{lunch}");
    assert!(!lunch.contains("CLASS:PRIVATE") && !lunch.contains("CLASS:CONFIDENTIAL"), "{lunch}");
    let standup = vevent(&stored(&m, &format!("{MARK} Standup")));
    eprintln!("--- online meeting, as SOGo stores it:\n{standup}");
    assert!(standup.contains("CONFERENCE") && standup.contains("meet.example.edu/rooms/mc3-1sg-og0"), "{standup}");
    assert!(standup.contains("LOCATION:https://meet.example.edu/rooms/mc3-1sg-og0") || standup.contains("LOCATION:Standup room"), "{standup}");
    assert!(standup.contains("4711"), "the access code in the notes: {standup}");
    // Read back by MailCraft from SOGo (a fresh read, nothing kept): each the same.
    let (mut again, aid2) = App::new(&m, 0, "calendar-again");
    again.s.run("account.dav", &json!({"account": aid2, "caldav": web})).unwrap();
    again.sync_dav(&aid2);
    for (uid, p, sa) in [
        (&private, Privacy::Private, ShowAs::OutOfOffice),
        (&confidential, Privacy::Confidential, ShowAs::Busy),
        (&public, Privacy::Public, ShowAs::Free),
    ] {
        let (_, e) = again.s.find_event(uid).unwrap_or_else(|| panic!("{uid} read back"));
        assert_eq!((privacy(&e.event), show_as(&e.event)), (p, sa), "{}", e.event.summary);
    }
    let conf = again.s.find_event(&online).map(|(_, e)| e.event.extra.iter().any(|l| l.starts_with("CONFERENCE"))).unwrap_or_default();
    assert!(conf, "CONFERENCE read back");
    let joined = again.s.run("calendar.search", &json!({"query": "meet.example.edu/rooms/mc3-1sg-og0"})).unwrap();
    eprintln!("found by the room's link: {joined}");
    assert!(joined.as_array().unwrap().iter().any(|h| h["uid"] == online.as_str()), "{joined}");
    drop(again);

    // bob subscribed to ann's calendar in SOGo: SOGo lists it in his home, someone else's.
    let (mut bob, bid) = App::new(&m, 1, "calendar");
    bob.s.run("account.dav", &json!({"account": bid, "caldav": web})).unwrap();
    let sync = bob.sync_dav(&bid);
    let cals = sync.calendars.clone().unwrap_or_default();
    eprintln!("bob's calendars: {:?}", cals.iter().map(|c| (&c.name, &c.id, c.read_only, c.foreign)).collect::<Vec<_>>());
    let theirs = cals
        .iter()
        .find(|c| c.id.contains(&format!("/{}/Calendar/", m.user(1).0)) && c.id.contains("_personal/"))
        .expect("ann's calendar in bob's home");
    assert!(theirs.foreign, "SOGo names ann its owner");
    assert!(theirs.read_only, "bob may only look");
    let bobs_default = bob.s.pim[&bid].default_calendar().map(|c| c.id.clone()).unwrap_or_default();
    assert!(bobs_default.ends_with(&format!("/SOGo/dav/{}/Calendar/personal/", m.user(1).0)), "{bobs_default}");
    let theirs = theirs.id.clone();

    // What SOGo serves bob: the private and confidential ones without their details. MailCraft
    // shows them as private appointments, busy, at their times.
    let seen = events(&mut bob);
    let in_theirs: Vec<&Value> = seen.iter().filter(|e| e["calendar"] == theirs.as_str()).collect();
    eprintln!("bob sees in ann's calendar: {in_theirs:#?}");
    // The test's day (2026-11-16, its clock UTC): ann may keep other events.
    let day = 1_794_787_200..1_794_873_600;
    let at = |h: i64| -> Vec<&Value> {
        in_theirs.iter().copied().filter(|e| e["start"].as_i64().is_some_and(|s| day.contains(&s) && (s / 3600) % 24 == h)).collect()
    };
    for (h, what) in [(9, "Doctor"), (11, "Interview")] {
        let got = at(h);
        assert_eq!(got.len(), 1, "{what} at {h}:00: {got:?}");
        let e = got[0];
        assert_eq!(e["privacy"], "private", "{what} is someone's private appointment: {e}");
        assert!(e["summary"].as_str().unwrap_or_default().trim().is_empty(), "{what}'s title SOGo hid stays hidden: {e}");
        assert_eq!(e["showAs"], "busy", "{e}");
        assert_eq!(e["end"].as_i64().unwrap() - e["start"].as_i64().unwrap(), 3600, "{e}");
    }
    let lunch = at(13);
    assert_eq!(lunch.first().map(|e| e["summary"].clone()), Some(json!(format!("{MARK} Lunch"))), "public: all of it");
    assert_eq!(lunch[0]["showAs"], "free");
    // Not his to change: refused here, nothing sent to SOGo.
    let hidden_uid = at(9)[0]["uid"].as_str().unwrap().to_string();
    let r = bob.s.run("event.update", &json!({"uid": hidden_uid, "summary": "Bob was here"}));
    eprintln!("bob's edit: {r:?}");
    assert!(r.is_err(), "a read-only calendar");
    let r = bob.s.run("event.delete", &json!({"uid": hidden_uid}));
    assert!(r.is_err(), "a read-only calendar: {r:?}");
    bob.settle(3);
    assert!(stored(&m, &format!("{MARK} Doctor")).contains("CLASS:PRIVATE"));

    // Search (#33) over bob's own calendar and ann's: the private ones by is:private, the
    // public one by its title.
    let hits = bob.s.run("calendar.search", &json!({"query": "is:private"})).unwrap();
    let starts: Vec<i64> = hits
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["calendar"] == theirs.as_str())
        .filter_map(|h| h["start"].as_i64())
        .filter(|t| day.contains(t))
        .collect();
    eprintln!("bob's is:private: {hits}");
    assert_eq!(starts.len(), 2, "{hits}");
    let hits = bob.s.run("calendar.search", &json!({"query": format!("\"{MARK} Lunch\"")})).unwrap();
    assert_eq!(hits.as_array().map(Vec::len), Some(1), "{hits}");
    let hits = bob.s.run("calendar.search", &json!({"query": "Doctor"})).unwrap();
    assert_eq!(hits.as_array().map(Vec::len), Some(0), "SOGo hid the title: {hits}");

    // carol: nothing of private ones, everything of confidential ones.
    let (mut carol, cid) = App::new(&m, 2, "calendar");
    carol.s.run("account.dav", &json!({"account": cid, "caldav": web})).unwrap();
    carol.sync_dav(&cid);
    let seen = events(&mut carol);
    let titles: Vec<String> = seen
        .iter()
        .filter(|e| e["calendar"].as_str().is_some_and(|c| c.contains("_personal/")))
        .map(|e| e["summary"].as_str().unwrap_or_default().to_string())
        .collect();
    eprintln!("carol sees in ann's calendar: {titles:?}");
    assert!(
        !titles.iter().any(|t| t.contains("Doctor"))
            && seen.iter().filter(|e| e["start"].as_i64().is_some_and(|s| day.contains(&s) && (s / 3600) % 24 == 9)).count() == 0
    );
    let interview = seen.iter().find(|e| e["summary"] == format!("{MARK} Interview").as_str()).expect("carol sees the confidential one whole");
    assert_eq!(interview["privacy"], "confidential");

    // Made private in SOGo's own web pages, renamed in MailCraft: still private, SOGo's lines kept.
    let sogo = m.sogo(0);
    let id = sogo.json("Calendar/personal/newguid")["id"].as_str().unwrap().to_string();
    sogo.post(
        &format!("Calendar/personal/{id}/saveAsAppointment"),
        &json!({"summary": format!("{MARK} Therapy"), "location": "Room 9", "comment": "secret", "classification": "private",
                "startDate": "2026-11-17", "startTime": "10:00", "endDate": "2026-11-17", "endTime": "11:00", "isAllDay": 0, "isTransparent": 0}),
    );
    ann.sync_dav(&aid);
    let uid =
        ann.s.pim[&aid].events.iter().find(|e| e.event.summary == format!("{MARK} Therapy")).map(|e| e.event.uid.clone()).expect("SOGo's event");
    assert_eq!(privacy(&ann.s.find_event(&uid).unwrap().1.event), Privacy::Private, "the lock");
    ann.s.run("event.update", &json!({"uid": uid, "summary": format!("{MARK} Therapy (moved)")})).unwrap();
    ann.pump(120, "the change stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    let therapy = vevent(&stored(&m, &format!("{MARK} Therapy (moved)")));
    eprintln!("--- SOGo's private event renamed in MailCraft:\n{therapy}");
    assert_eq!(therapy.matches("CLASS:").count(), 1, "{therapy}");
    assert!(therapy.contains("CLASS:PRIVATE") && therapy.contains("TRANSP:OPAQUE"), "{therapy}");
    clean(&m);
}
