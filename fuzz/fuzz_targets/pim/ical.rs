//! A calendar file (an invitation, a CalDAV event, an exported calendar's to-dos) read, written
//! back and read again, its online meeting found, taken out and added; and the WebDAV answers
//! CalDAV and CardDAV servers send, a colleague's free/busy times among them (issue #242).
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::ical;
use mailcraft_net::dav;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let cal = ical::parse(&text, 3600);
    for e in cal.events.iter().take(16) {
        let _ = ical::reminder_minutes(e);
        let _ = e.links();
        // Its online meeting (issue #25): found, joined by the user's hosts, taken out, added.
        let _ = e.join_link_in(&[text.chars().take(64).collect()]);
        let mut m = e.clone();
        m.remove_online_meeting();
        if let Some(room) = ical::MeetingRoom::new(&e.summary, &e.location, &e.uid, &e.description) {
            m.add_online_meeting(&room, &ical::MeetingLabels::default());
        }
    }
    let written = ical::write(&cal.events, cal.method.as_deref(), 0);
    let again = ical::parse(&written, 3600);
    // Whom a guest handed their place to, and who handed it them (issue #254): as written once,
    // read the same ever after.
    let handed = |events: &[ical::CalEvent]| -> Vec<(String, String)> {
        events.iter().flat_map(|e| e.attendees.iter().map(|a| (a.delegated_to.clone(), a.delegated_from.clone()))).collect()
    };
    let third = ical::parse(&ical::write(&again.events, cal.method.as_deref(), 0), 3600);
    assert_eq!(handed(&third.events), handed(&again.events));
    // Its to-dos (`VTODO`), written with the events as an exported calendar is, and read back:
    // every one of them, so what a to-do keeps as written never swallows the ones after it.
    let exported = ical::write_calendar(&cal.events, &cal.tasks, 0);
    assert_eq!(ical::parse(&exported, 3600).tasks.len(), cal.tasks.len());
    let _ = ical::duration(&text);
    let _ = ical::unescape(&text);
    let _ = ical::split_text_list(&text);
    let _ = dav::multistatus(&text);
    // Free/busy times: each one ends after it starts, and there are never more than the cap.
    let times = ical::free_busy_times(&text);
    assert!(times.len() <= 5_000 && times.iter().all(|(start, end, _)| end > start));
    let answers = dav::schedule_answers(&text, &["ana@studio.example".to_string(), "ben@studio.example".into()]);
    assert_eq!(answers.len(), 2);
});
