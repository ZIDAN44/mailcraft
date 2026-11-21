//! Issue #224: an event's several reminders (Google's "an hour before and 2 minutes before")
//! each come in turn: the first dismissed, here or in another program, the next still comes; and
//! they're set and kept through `event.update`.

use super::*;

/// An event an hour after `NOW` that reminds an hour before and 2 minutes before.
fn twice(uid: &str) -> StoredEvent {
    let mut e = with_reminder(uid, &format!("/cal/{uid}.ics"), false);
    ical::set_reminders(&mut e.event, &[60, 2]);
    e
}

#[test]
fn each_reminder_comes_in_turn() {
    let (mut s, id) = account_with("reminders-several", vec![twice("g"), twice("h")], false);
    let start = NOW + 3600;
    s.clock.now = NOW - 60;
    assert!(due(&s).is_empty(), "before the first");
    s.clock.now = NOW + 1;
    assert_eq!(due(&s), ["g", "h"], "an hour before");
    let g = reminder_key(&s, "g");
    s.run("reminders.dismiss", &json!({"key": g})).unwrap();
    s.clock.now = NOW + 30 * 60;
    assert_eq!(due(&s), ["h"], "dismissed until its next");
    s.clock.now = start - 2 * 60;
    assert_eq!(due(&s), ["g", "h"], "2 minutes before: the next comes");
    s.run("reminders.dismiss", &json!({})).unwrap();
    s.clock.now = start - 60;
    assert!(due(&s).is_empty(), "both dismissed after their last");

    // Dismissed in another program after its first (written into the event): its next comes.
    let mut elsewhere = twice("k");
    ical::acknowledge(&mut elsewhere.event, NOW + 10);
    let sync = DavSync {
        calendars: s.pim.get(&id).map(|p| p.calendars.clone()),
        events: vec![elsewhere],
        synced_calendars: vec![CAL.into()],
        kept: vec!["/cal/g.ics".into(), "/cal/h.ics".into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id, sync: Box::new(sync) });
    s.clock.now = NOW + 20 * 60;
    assert!(due(&s).is_empty(), "the first answered elsewhere");
    s.clock.now = start - 2 * 60;
    assert_eq!(due(&s), ["k"]);
    // An answer kept by a build before (0): dismissed for good.
    let k = reminder_key(&s, "k");
    s.prefs.reminders.insert(k, 0);
    assert!(due(&s).is_empty());
}

#[test]
fn reminders_are_set_and_kept_through_an_update() {
    let (mut s, _) = account_with("reminders-set", vec![twice("g")], false);
    // The same ones, in another order: nothing changes.
    s.run("event.update", &json!({"uid": "g", "reminders": [2, 60]})).unwrap();
    let (_, e) = s.find_event("g").unwrap();
    assert!(!e.dirty, "saved unchanged: no change");
    assert_eq!(ical::reminders_minutes(&e.event), [60, 2]);
    // Another reminder added, then none.
    s.run("event.update", &json!({"uid": "g", "reminders": [1440, 60, 2]})).unwrap();
    assert_eq!(ical::reminders_minutes(&s.find_event("g").unwrap().1.event), [1440, 60, 2]);
    s.run("event.update", &json!({"uid": "g", "reminders": []})).unwrap();
    assert!(ical::reminders_minutes(&s.find_event("g").unwrap().1.event).is_empty());
    // What isn't a list of times is refused, and changes nothing.
    for bad in [json!("15"), json!([-1]), json!([60, "x"]), json!(vec![5; 11])] {
        assert!(s.run("event.update", &json!({"uid": "g", "reminders": bad})).is_err(), "{bad}");
    }
}
