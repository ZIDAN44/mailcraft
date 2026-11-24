//! Where a real CalDAV and CardDAV server keeps an account's calendars and contacts, looked for
//! and kept as a sync goes on, against a real server on this computer (never a real account),
//! run by hand on Radicale, Nextcloud, Stalwart and SOGo:
//!
//! ```text
//! MAILCRAFT_REAL_CALDAV="http://127.0.0.1:25232/ann@rad.test/personal/ ann@rad.test pw" \
//!   cargo test -p mailcraft-net real_dav_ -- --ignored --nocapture --test-threads 1
//! ```

use mailcraft_model::pim::Pim;
use mailcraft_model::zone::Zone;

use super::real_servers::{caldav_env, caldav_where};

/// A server not found is looked for again a day after it was looked for, however often the
/// account syncs meanwhile (each sync used to count as a look, so it never was).
#[test]
#[ignore = "needs a real CalDAV server in MAILCRAFT_REAL_CALDAV"]
fn real_dav_a_server_not_found_is_looked_for_a_day_after_the_look() {
    let Some((url, user, dav)) = caldav_env() else { return };
    let at = crate::dav::Where { carddav: Some(&url), ..caldav_where(&url, &user) };
    let now = crate::worker::now();
    let sync = |pim: &Pim, at_time: i64| crate::dav::sync(&dav, &at, pim, at_time, Zone::UTC).unwrap();

    let first = sync(&Pim::default(), now);
    assert!(!first.homes_kept, "looked for, the first time");
    let contacts = first.carddav_home.clone().expect("the server's contacts home");

    // Saved as the contacts not found, looked for at `now`: a sync a quarter of an hour on, and
    // one most of a day on, take that as it is.
    let pim = Pim { caldav_home: first.caldav_home.clone(), carddav_home: Some(String::new()), dav_checked: Some(now), ..Default::default() };
    for later in [900, 86_000] {
        let s = sync(&pim, now + later);
        assert!(s.homes_kept && s.carddav_home.is_none(), "{later} s on: {s:?}");
    }
    // A day and a minute after the look: looked for again, and found.
    let s = sync(&pim, now + 86_460);
    assert!(!s.homes_kept, "{s:?}");
    assert_eq!(s.carddav_home, Some(contacts));
}
