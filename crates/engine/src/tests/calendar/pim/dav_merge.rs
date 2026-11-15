//! A calendar server's sync of a lot of events merged here.

use mailcraft_model::ical::CalEvent;
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;

const CAL: &str = "https://dav.example.org/cal/work/";

/// A sync of the calendar `CAL` that read `events` (and found `kept` unchanged).
fn sync(events: Vec<StoredEvent>, kept: Vec<String>) -> DavSync {
    DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: CAL.into(), name: "Work".into(), ctag: "1".into(), ..Default::default() }]),
        events,
        kept,
        synced_calendars: vec![CAL.into()],
        checked: NOW,
        ..Default::default()
    }
}

/// `n` events of the calendar as the server has them.
fn served(n: usize) -> Vec<StoredEvent> {
    (0..n)
        .map(|i| {
            let start = NOW + i64::try_from(i).unwrap_or(0) * 3600;
            let event =
                CalEvent { uid: format!("e{i}@dav.example.org"), summary: format!("Meeting {i}"), start, end: start + 1800, ..Default::default() };
            StoredEvent { calendar: CAL.into(), href: format!("{CAL}e{i}.ics"), etag: "1".into(), event, dirty: false }
        })
        .collect()
}

/// A first sync of a calendar of 5,000 events, then one that finds them unchanged, then one that
/// reads them all again, timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_dav_merge -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_dav_merge() {
    let mut s = Session::open(Some(temp("measure-dav-merge")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "ana@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let events = served(20_000);
    let hrefs: Vec<String> = events.iter().map(|e| e.href.clone()).collect();
    for (what, sync) in
        [("first", sync(events.clone(), Vec::new())), ("unchanged", sync(Vec::new(), hrefs.clone())), ("all again", sync(events, Vec::new()))]
    {
        let t = std::time::Instant::now();
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
        println!("MEASURE dav merge of 20000, {what}: {} ms", t.elapsed().as_millis());
        assert_eq!(s.pim[&id].events.len(), 20_000, "{what}");
    }
}

/// One of each part after a merge: an event the server sent twice is kept once, as it last sent
/// it; one changed here stays as it was changed; read again in full, nothing is doubled.
#[test]
fn a_merge_keeps_one_of_each_part() {
    let mut s = Session::open(Some(temp("dav-merge-parts")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "ana@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let mut events = served(3);
    let mut newer = events[1].clone();
    newer.event.summary = "Meeting 1, moved".into();
    events.push(newer);
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync(events, Vec::new())) });
    let summaries = |s: &Session| {
        let mut v: Vec<String> = s.pim[&id].events.iter().map(|e| e.event.summary.clone()).collect();
        v.sort();
        v
    };
    assert_eq!(summaries(&s), ["Meeting 0", "Meeting 1, moved", "Meeting 2"]);
    s.run("event.update", &json!({"uid": "e0@dav.example.org", "summary": "Meeting 0, here"})).unwrap();
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync(served(3), Vec::new())) });
    assert_eq!(summaries(&s), ["Meeting 0, here", "Meeting 1", "Meeting 2"]);
}
