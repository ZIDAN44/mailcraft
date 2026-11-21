//! Issue #224 against a real CalDAV server on this computer (never a real account), run by hand:
//! an event's several reminders are stored, read back each, kept when the event changes
//! otherwise, and one dismissed leaves the rest.
//!
//! ```text
//! MAILCRAFT_REAL_CALDAV="http://127.0.0.1:25232/riley/work/ riley pw" \
//!   cargo test -p mailcraft-net real_caldav_several_reminders -- --ignored --nocapture
//! ```

use mailcraft_model::ical::{CalEvent, acknowledge, acknowledged, reminders_minutes, set_reminders};
use mailcraft_model::pim::{Pim, StoredEvent};

use super::real_servers::{caldav_env, caldav_where};

#[test]
#[ignore = "needs a real CalDAV server in MAILCRAFT_REAL_CALDAV"]
fn real_caldav_several_reminders() {
    let Some((url, user, dav)) = caldav_env() else { return };
    let at = caldav_where(&url, &user);
    let zone = mailcraft_model::zone::Zone::Fixed(0);
    let now = 1_793_782_000;
    let sync = |pim: &Pim| {
        let s = crate::dav::sync(&dav, &at, pim, now, zone).unwrap();
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        s
    };
    let first = sync(&Pim::default());
    let cal = first.calendars.as_ref().and_then(|c| c.iter().find(|c| !c.read_only)).map(|c| c.id.clone()).expect("a calendar to write to");
    let uid = format!("mc224-reminders-{}@mailcraft", std::process::id());
    let mut e = CalEvent { uid: uid.clone(), summary: "Standup".into(), start: now + 7200, end: now + 9000, dtstamp: now, ..Default::default() };
    set_reminders(&mut e, &[60, 2]);
    let mut pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), caldav_home: first.caldav_home.clone(), ..Default::default() };
    pim.events.push(StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() });
    let s = sync(&pim);
    let mut stored = s.pushed_events.iter().find(|x| x.event.uid == uid).cloned().expect("stored");
    let read = || sync(&Pim::default()).events.into_iter().find(|x| x.event.uid == uid).expect("read back");
    assert_eq!(reminders_minutes(&read().event), [60, 2], "both, read back as another program reads them");

    // Renamed, its reminders untouched: both stay.
    stored.event.summary = "Standup, moved room".into();
    stored.dirty = true;
    pim.events = vec![stored.clone()];
    let s = sync(&pim);
    stored = s.pushed_events.iter().find(|x| x.event.uid == uid).cloned().expect("sent again");
    let back = read();
    assert_eq!((back.event.summary.as_str(), reminders_minutes(&back.event)), ("Standup, moved room", vec![60, 2]));

    // The first dismissed (acknowledged, as the engine does): both alarms stay, said answered.
    acknowledge(&mut stored.event, now + 3600);
    stored.dirty = true;
    pim.events = vec![stored.clone()];
    sync(&pim);
    let back = read();
    assert_eq!((acknowledged(&back.event), reminders_minutes(&back.event)), (Some(now + 3600), vec![60, 2]));
    let _ = dav.remove(&stored.href, "");
}
