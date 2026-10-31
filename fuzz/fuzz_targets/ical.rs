//! A calendar file (an invitation, a CalDAV event) read, written back and read again, its online
//! meeting found, taken out and added; and the WebDAV answers CalDAV and CardDAV servers send.
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
    let _ = ical::parse(&written, 3600);
    let _ = ical::duration(&text);
    let _ = ical::unescape(&text);
    let _ = ical::split_text_list(&text);
    let _ = dav::multistatus(&text);
});
