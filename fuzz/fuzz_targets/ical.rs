//! A calendar file (an invitation, a CalDAV event) read, written back and read again; and the
//! WebDAV answers CalDAV and CardDAV servers send.
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
    }
    let written = ical::write(&cal.events, cal.method.as_deref(), 0);
    let _ = ical::parse(&written, 3600);
    let _ = ical::duration(&text);
    let _ = ical::unescape(&text);
    let _ = ical::split_text_list(&text);
    let _ = dav::multistatus(&text);
});
