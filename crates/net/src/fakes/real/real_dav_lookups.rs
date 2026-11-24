//! Where a real CalDAV and CardDAV server keeps an account's calendars and contacts, looked for
//! and kept as a sync goes on, against a real server on this computer (never a real account),
//! run by hand on Radicale, Nextcloud, Stalwart and SOGo:
//!
//! ```text
//! MAILCRAFT_REAL_CALDAV="http://127.0.0.1:25232/ann@rad.test/personal/ ann@rad.test pw" \
//!   cargo test -p mailcraft-net real_dav_ -- --ignored --nocapture --test-threads 1
//! ```

use mailcraft_model::pim::{Pim, Tombstone};
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

/// A contact deleted here goes to the server only while the contacts are found there: on a
/// server holding the calendars and the contacts, a contact's deletion waits while its contacts
/// aren't found (or are refused), and goes once they are.
#[test]
#[ignore = "needs a real CalDAV server in MAILCRAFT_REAL_CALDAV"]
fn real_dav_a_contacts_deletion_waits_for_its_server() {
    let Some((url, user, dav)) = caldav_env() else { return };
    let at = crate::dav::Where { carddav: Some(&url), ..caldav_where(&url, &user) };
    let now = crate::worker::now();
    let sync = |pim: &Pim| crate::dav::sync(&dav, &at, pim, now, Zone::UTC).unwrap();
    let read = |uid: &str| sync(&Pim::default()).cards.iter().any(|c| c.card.uid == uid);

    let first = sync(&Pim::default());
    let book = first.books.iter().flatten().find(|b| !b.read_only).map(|b| b.id.clone()).expect("an address book to write to");
    let uid = format!("mc-waits-{}", std::process::id());
    let href = format!("{}/{uid}.vcf", book.trim_end_matches('/'));
    let card = mailcraft_model::vcard::Card { uid: uid.clone(), name: "Waits Here".into(), ..Default::default() };
    let etag = dav.put(&href, "text/vcard; charset=utf-8", &mailcraft_model::vcard::write(&card), None).unwrap();
    assert!(read(&uid), "stored");
    // Where the server's own href for it may differ from the one written to (SOGo's).
    let href = sync(&Pim::default()).cards.iter().find(|c| c.card.uid == uid).map(|c| c.href.clone()).unwrap_or(href);

    let pim = |carddav: Option<String>| Pim {
        caldav_home: first.caldav_home.clone(),
        carddav_home: carddav,
        dav_checked: Some(now),
        calendars: first.calendars.clone().unwrap_or_default(),
        books: first.books.clone().unwrap_or_default(),
        deleted: vec![Tombstone { href: href.clone(), etag: etag.clone(), ..Default::default() }],
        ..Default::default()
    };
    // The contacts saved as not found: the deletion waits, the contact stays on the server.
    let s = sync(&pim(Some(String::new())));
    assert!(!s.deleted.contains(&href), "{s:?}");
    assert!(read(&uid), "still on the server");
    // Found: it goes.
    let s = sync(&pim(first.carddav_home.clone()));
    assert!(s.deleted.contains(&href), "{s:?}");
    assert!(!read(&uid), "deleted on the server");
}
