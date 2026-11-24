//! CalDAV and CardDAV syncs end to end, against the in-memory server.

use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{LOCAL, Pim, StoredEvent};
use mailcraft_model::vcard::{self, Card, Typed};
use mailcraft_model::zone::Zone;

use super::*;
use crate::dav::{Dav, Kind, Where, sync};

/// Found from the address, as an account with nothing given by hand. The tests' syncs never
/// look for anything there, out on the network: their calendars and contacts are given, or
/// were looked for already (`dav_checked`), the contacts none.
const AT: Where<'static> = Where { email: USER, username: "", others: &[], imap_host: "imap.example", caldav: None, carddav: None, mails: true };

fn event(uid: &str, summary: &str, start: i64) -> CalEvent {
    CalEvent { uid: uid.into(), summary: summary.into(), start, end: start + 3600, ..Default::default() }
}

/// Found in review: the fake unfolded only lines folded with a bare LF, so a long uid folded
/// as `ical::write` folds it (CR LF) kept a CR inside, and a calendar-query found nothing.
#[test]
fn a_folded_uid_is_read_whole() {
    let long = "x".repeat(125);
    let data = ical::write(&[event(&long, "Long", 1_800_000_000)], None, 0);
    assert!(data.contains("\r\n "), "folded: {data}");
    assert_eq!(uid_of(&data), Some(long));
    assert_eq!(uid_of("BEGIN:VEVENT\r\nUID:ab\r\n\tc\r\nEND:VEVENT\r\n"), Some("abc".into()));
    assert_eq!(uid_of("BEGIN:VEVENT\nUID:ab\n c\nEND:VEVENT\n"), Some("abc".into()));
}

/// Found in the audit: a uid with a control character went into the calendar-query raw, XML
/// 1.0 doesn't allow it, and Radicale answered 400, so the item was never found. Radicale
/// keeps the uid without the control, and so finds it by the query with the control left out.
#[test]
fn a_uid_with_a_control_character_is_looked_for() {
    let server = serve();
    let href = format!("{CAL}c.ics");
    server.data.lock().unwrap().put(&href, &ical::write(&[event("probe\"two&<x>", "Kept", 1_800_000_000)], None, 0));
    let cal = format!("{}{CAL}", server.base);
    let found = Dav::new(USER, PASSWORD).find_uid(&cal, "probe\"\u{1}two&<x>").unwrap();
    assert_eq!(found, vec![format!("{}{href}", server.base)]);
}

#[test]
fn finds_reads_writes_and_deletes() {
    let server = serve();
    let data = server.data.clone();
    {
        let mut d = data.lock().unwrap();
        d.put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Review", 1_800_000_000)], None, 0));
        d.put(
            &format!("{BOOK}z.vcf"),
            &vcard::write(&Card {
                uid: "z".into(),
                name: "Zoe Park".into(),
                emails: vec![Typed { value: "zoe@studio.example".into(), kind: String::new() }],
                ..Default::default()
            }),
        );
    }
    let dav = Dav::new(USER, PASSWORD);
    assert!(matches!(
        Dav::new(USER, "nope").home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)),
        Err(crate::NetError::Auth(_))
    ));
    // Discovery: the well-known address redirects to the principal, which names the homes.
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
    assert_eq!(home, format!("{}/dav/cal/", server.base));
    let book_home = dav.home_from(Kind::Contacts, &format!("{}/.well-known/carddav", server.base)).unwrap().unwrap();

    // A first sync reads everything.
    let mut pim = Pim { caldav_home: Some(home), carddav_home: Some(book_home), ..Default::default() };
    pim.ensure_defaults();
    let first = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    let cal_id = format!("{}{CAL}", server.base);
    assert_eq!(first.calendars.as_ref().map(|c| (c.len(), c[0].name.as_str(), c[0].id.as_str())), Some((1, "Work", cal_id.as_str())));
    assert_eq!(first.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
    assert_eq!(first.cards.iter().map(|c| c.card.display()).collect::<Vec<_>>(), ["Zoe Park"]);

    // The engine would merge this; mirror it, then change things here.
    pim.calendars = first.calendars.clone().unwrap();
    pim.books = first.books.clone().unwrap();
    pim.events = first.events.clone();
    pim.cards = first.cards.clone();
    let review = pim.events[0].clone();
    pim.events[0].event.summary = "Review (moved)".into();
    pim.events[0].dirty = true;
    pim.events.push(StoredEvent { calendar: cal_id.clone(), event: event("b@x", "New / one", 1_800_100_000), dirty: true, ..Default::default() });
    pim.events.push(StoredEvent { calendar: LOCAL.into(), event: event("l", "Only here", 1_800_000_000), dirty: false, ..Default::default() });
    pim.deleted.push(mailcraft_model::pim::Tombstone { href: pim.cards[0].href.clone(), etag: pim.cards[0].etag.clone(), ..Default::default() });
    let second = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(second.deleted.len(), 1);
    assert_eq!(second.pushed_events.len(), 2);
    let new = second.pushed_events.iter().find(|e| e.event.uid == "b@x").unwrap();
    let name = new.href.rsplit('/').next().unwrap_or_default().to_string();
    assert!(name.starts_with("b-x~") && name.ends_with(".ics") && !new.etag.is_empty(), "{new:?}");
    {
        let d = data.lock().unwrap();
        assert!(d.items.get(&format!("{CAL}a.ics")).is_some_and(|(_, ics)| ics.contains("Review (moved)")));
        assert!(d.items.contains_key(&format!("{CAL}{name}")));
        assert!(!d.items.keys().any(|k| k.starts_with(BOOK)), "the card was deleted");
        assert!(!d.items.values().any(|(_, v)| v.contains("Only here")), "a local event stays local");
    }
    assert!(second.events.iter().any(|e| e.event.summary == "Review (moved)"), "read back after the change");

    // A stale copy isn't written over a newer one: the server's comes in its place (it was
    // sent again, and refused again, at every sync).
    let mut stale = pim.clone();
    stale.deleted.clear();
    stale.events = vec![StoredEvent { event: CalEvent { summary: "Old edit".into(), ..review.event.clone() }, dirty: true, ..review.clone() }];
    let third = sync(&dav, &AT, &stale, 300, Zone::UTC).unwrap();
    assert_eq!(third.errors.len(), 1, "{:?}", third.errors);
    assert!(!data.lock().unwrap().items.values().any(|(_, v)| v.contains("Old edit")));
    assert_eq!(third.refused, ["a"]);
    assert!(third.events.iter().any(|e| e.event.uid == "a" && e.event.summary == "Review (moved)"), "the server's copy read");
    // The server has what was changed here already (SOGo put a guest's answer in the
    // organizer's copy itself): its copy is kept without a word.
    let mut same = pim.clone();
    same.deleted.clear();
    same.events = vec![StoredEvent { event: CalEvent { summary: "Review (moved)".into(), ..review.event.clone() }, dirty: true, ..review.clone() }];
    let quiet = sync(&dav, &AT, &same, 300, Zone::UTC).unwrap();
    assert_eq!((quiet.errors.len(), quiet.refused.len()), (0, 1), "{:?}", quiet.errors);
    // Nothing changed since: the calendar isn't read again.
    let mut settled = pim.clone();
    settled.deleted.clear();
    settled.calendars = third.calendars.clone().unwrap();
    settled.events.iter_mut().for_each(|e| e.dirty = false);
    settled.cards.clear();
    let fourth = sync(&dav, &AT, &settled, 400, Zone::UTC).unwrap();
    assert_eq!(fourth.unchanged_calendars, vec![cal_id]);
    assert!(fourth.events.is_empty());
}

/// When the servers were last looked for, read from a damaged cache (far off either way), is a
/// time like any other: the sync goes ahead, nothing overflows.
#[test]
fn a_damaged_last_look_is_no_overflow() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
    let book_home = dav.home_from(Kind::Contacts, &format!("{}/.well-known/carddav", server.base)).unwrap().unwrap();
    for checked in [i64::MIN, i64::MAX, -1] {
        let pim = Pim { caldav_home: Some(home.clone()), carddav_home: Some(book_home.clone()), dav_checked: Some(checked), ..Default::default() };
        let out = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
        assert!(out.errors.is_empty(), "{:?}", out.errors);
    }
}

/// Found in review: the fake heeded `If-Match` only spelled so, though every other header was
/// read in any case: a change to an item since changed was stored over it when the header came
/// in lower case.
#[test]
fn a_condition_is_heeded_in_any_case() {
    use std::io::{Read, Write};
    let server = serve();
    let href = format!("{CAL}a.ics");
    server.data.lock().unwrap().put(&href, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
    let auth = mailcraft_model::base64::encode(format!("{USER}:{PASSWORD}").as_bytes());
    let mut s = std::net::TcpStream::connect(server.base.trim_start_matches("http://")).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    write!(s, "PUT {href} HTTP/1.1\r\nHost: x\r\nAuthorization: Basic {auth}\r\nif-match: \"stale\"\r\nContent-Length: 2\r\n\r\nhi").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 412 "), "{out}");
    assert!(server.data.lock().unwrap().items[&href].1.starts_with("BEGIN:"), "the item as it was");
}

/// Two new events whose long uids differ only after the part a file name keeps each get an
/// item of their own; an event already on the server keeps its address.
#[test]
fn long_uids_get_items_of_their_own() {
    let server = serve();
    let data = server.data.clone();
    let dav = Dav::new(USER, PASSWORD);
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();
    let cal_id = format!("{}{CAL}", server.base);
    let mut pim = Pim { caldav_home: Some(home), carddav_home: Some(String::new()), dav_checked: Some(100), ..Default::default() };
    pim.ensure_defaults();
    let long = "x".repeat(125);
    let old_href = format!("{cal_id}{}.ics", "y".repeat(120));
    let etag = data
        .lock()
        .unwrap()
        .put(&format!("{CAL}{}.ics", "y".repeat(120)), &ical::write(&[event(&format!("{long}-old"), "Old", 1_800_000_000)], None, 0));
    let new = |uid: &str| StoredEvent { calendar: cal_id.clone(), event: event(uid, uid, 1_800_000_000), dirty: true, ..Default::default() };
    pim.events =
        vec![new(&format!("{long}-one")), new(&format!("{long}-two")), StoredEvent { href: old_href.clone(), etag, ..new(&format!("{long}-old")) }];
    let out = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let hrefs: Vec<&str> = out.pushed_events.iter().map(|e| e.href.as_str()).collect();
    assert_eq!(hrefs.len(), 3);
    assert_ne!(hrefs[0], hrefs[1], "{hrefs:?}");
    assert!(hrefs.contains(&old_href.as_str()), "an item already stored keeps its address: {hrefs:?}");
    let d = data.lock().unwrap();
    for uid in ["one", "two"] {
        let on_server = d.items.values().any(|(_, v)| ical::parse(v, 0).events.iter().any(|e| e.uid == format!("{long}-{uid}")));
        assert!(on_server, "{uid} is on the server");
    }
}

/// A calendar or address book the user may not read (403: a calendar shared for free / busy
/// only) is neither the whole sync's failure nor a password refused, as it was (found through
/// a proxy in front of Baïkal, issue #61): the rest syncs, and it's kept as it is, and said.
#[test]
fn a_collection_the_user_may_not_read_is_kept_and_the_rest_synced() {
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Review", 1_800_000_000)], None, 0));
        d.unreadable.push(BOOK.into());
    }
    let home = |k: &str| Some(format!("{}/dav/{k}/", server.base));
    let mut pim = Pim { caldav_home: home("cal"), carddav_home: home("card"), dav_checked: Some(100), ..Default::default() };
    pim.ensure_defaults();
    let out = sync(&Dav::new(USER, PASSWORD), &AT, &pim, 100, Zone::UTC).unwrap();
    assert_eq!(out.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
    let book = format!("{}{BOOK}", server.base);
    assert_eq!((out.unchanged_books.clone(), out.synced_books.len()), (vec![book], 0));
    assert!(out.errors.iter().any(|e| e.contains("couldn't be read") && e.contains("403")), "{:?}", out.errors);
}

/// Found in review: while an account's calendar server is looked for, the mail password
/// went with the first request to every place tried (the address's domain, a host DNS
/// named, wherever a redirect led). It goes only to a server of the account's that asks.
#[test]
fn the_password_goes_only_to_the_accounts_own_server_while_looking() {
    let server = serve();
    let start = format!("{}/.well-known/caldav", server.base);
    // Not the account's (its mail is elsewhere): asked without a password, refused, and
    // that's "not here".
    let dav = Dav::new(USER, PASSWORD);
    dav.looking_for("riley@studio.example", "imap.studio.example");
    assert!(matches!(dav.home_from(Kind::Calendars, &start), Err(crate::NetError::Auth(_))));
    assert_eq!(server.data.lock().unwrap().offered, 0, "no password left for it");
    // The account's own (its mail server): asked without one first, then signed in to.
    let dav = Dav::new(USER, PASSWORD);
    dav.looking_for("riley@studio.example", "127.0.0.1");
    assert_eq!(dav.home_from(Kind::Calendars, &start).unwrap().unwrap(), format!("{}/dav/cal/", server.base));
    let (offered, refused) = {
        let d = server.data.lock().unwrap();
        (d.offered, d.log.len() - d.offered)
    };
    assert!(offered >= 2 && refused >= 2, "asked first, once: {offered} with it, {refused} without");
    // Found: as before, with the password from the first request.
    dav.found();
    let before = server.data.lock().unwrap().log.len();
    dav.collections(Kind::Calendars, &format!("{}/dav/cal/", server.base)).unwrap();
    let d = server.data.lock().unwrap();
    assert_eq!(d.log.len() - before, 1);

    use crate::dav::guard::Guard;
    let g = Guard::new("Riley@Own-Domain.example", "IMAP.provider.example");
    for (host, own) in [
        ("own-domain.example", true),
        ("caldav.OWN-domain.example", true),
        ("imap.provider.example", true),
        ("caldav.provider.example", true),
        ("d1.dav.provider.example.", true),
        ("provider.example", true),
        ("notown-domain.example", false),
        ("evil.example", false),
        ("provider.example.evil.example", false),
        ("example", false),
        ("other.example", false),
        ("", false),
    ] {
        assert_eq!(g.own(host), own, "{host}");
    }
    // A mail server at a domain's top has no neighbours; nor has an address.
    assert!(!Guard::new("a@b.example", "mailhost.example").own("other.example"));
    assert!(!Guard::new("a@b.example", "192.168.1.5").own("168.1.5"));
    assert!(!Guard::new("nothing", "").own("anything.example"));
}

/// Issue #7: an address given by hand, as a user copies it from the provider's help: the
/// server's own address, the home, or one calendar in it (whose parent is the home); one with
/// no calendars, or not https across the network, is said, with what the server answered there;
/// a wrong password is refused.
#[test]
fn an_address_given_by_hand_finds_the_home() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let cal_home = format!("{}/dav/cal/", server.base);
    let book_home = format!("{}/dav/card/", server.base);
    for given in
        [server.base.clone(), format!("{}/", server.base), cal_home.clone(), format!("{}{CAL}", server.base), format!("{}/dav/cal/work", server.base)]
    {
        assert_eq!(dav.home_given(Kind::Calendars, &given).unwrap(), cal_home, "{given}");
    }
    assert_eq!(dav.home_given(Kind::Contacts, &format!("{}{BOOK}", server.base)).unwrap(), book_home);
    let none = dav.home_given(Kind::Calendars, &format!("{}/nothing/here", server.base)).unwrap_err();
    assert!(none.to_string().contains("no calendars were found") && none.to_string().contains("HTTP 404"), "{none}");
    // Not to be read with these credentials (another user's, #147): what the server answered
    // is said, not "no calendars" alone.
    server.data.lock().unwrap().unreadable.push("/dav/someone/".into());
    let refused = dav.home_given(Kind::Calendars, &format!("{}/dav/someone/", server.base)).unwrap_err().to_string();
    assert!(refused.contains("no calendars were found") && refused.contains("HTTP 403"), "{refused}");
    assert!(matches!(Dav::new(USER, "nope").home_given(Kind::Calendars, &cal_home), Err(crate::NetError::Auth(_))));
    assert!(dav.home_given(Kind::Calendars, "http://calendar.example/dav/").unwrap_err().to_string().contains("https"));
    assert!(dav.home_given(Kind::Calendars, "").is_err());

    // In a sync: the given calendar's home is found, its events read; contacts are found at
    // the address given for them too (from the account's address, they'd be looked for on
    // the network). A given address that holds nothing is said once and remembered as none,
    // not an error that stops the sync.
    let at = Where { caldav: Some(&format!("{}{CAL}", server.base)), carddav: Some(&format!("{}{BOOK}", server.base)), ..AT };
    let mut pim = Pim::default();
    pim.ensure_defaults();
    server.data.lock().unwrap().put(&format!("{CAL}a.ics"), &ical::write(&[event("a", "Planning", 1_800_000_000)], None, 0));
    let out = sync(&dav, &at, &pim, 100, Zone::UTC).unwrap();
    assert_eq!(out.caldav_home.as_deref(), Some(cal_home.as_str()));
    assert_eq!(out.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Planning"]);
    let wrong = format!("{}/nothing/", server.base);
    let out = sync(&dav, &Where { caldav: Some(&wrong), ..at }, &pim, 100, Zone::UTC).unwrap();
    assert!(out.caldav_home.is_none() && out.errors.iter().any(|e| e.contains("no calendars")), "{:?}", out.errors);
    assert!(matches!(sync(&Dav::new(USER, "nope"), &at, &pim, 100, Zone::UTC), Err(crate::NetError::Auth(_))));
}

/// From the review: nothing goes to a server other than the one the homes are on (items
/// left from an old calendar server aren't sent there with this one's password); a server
/// that can't be reached isn't taken for one with nothing there; plain http goes nowhere but
/// this computer, whatever points there.
#[test]
fn only_the_found_server_is_written_to() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let home = format!("{}/dav/cal/", server.base);
    let mut pim = Pim { caldav_home: Some(home.clone()), carddav_home: Some(String::new()), dav_checked: Some(100), ..Default::default() };
    pim.ensure_defaults();
    let elsewhere = "http://127.0.0.2:9/cal/work/";
    pim.events.push(StoredEvent {
        calendar: elsewhere.into(),
        href: format!("{elsewhere}x.ics"),
        etag: "\"1\"".into(),
        event: event("x", "Old server", 1_800_000_000),
        dirty: true,
    });
    pim.deleted.push(mailcraft_model::pim::Tombstone { href: format!("{elsewhere}gone.ics"), ..Default::default() });
    let out = sync(&dav, &AT, &pim, 100, Zone::UTC).unwrap();
    assert!(out.errors.is_empty() && out.pushed_events.is_empty() && out.deleted.is_empty(), "{:?}", out.errors);
    assert!(!server.data.lock().unwrap().log.iter().any(|l| l.contains("x.ics") || l.contains("gone")));

    // Nothing listening: said as it is (and tried again), not "no calendars".
    let closed = "http://127.0.0.1:9/cal/";
    let e = dav.home_given(Kind::Calendars, closed).unwrap_err();
    assert!(!matches!(e, crate::NetError::No(_)), "{e:?}");
    assert!(sync(&dav, &Where { caldav: Some(closed), ..AT }, &Pim::default(), 100, Zone::UTC).is_err());

    // Plain http to another computer: refused before anything is sent.
    assert!(matches!(dav.home_from(Kind::Calendars, "http://calendar.example/dav/"), Err(crate::NetError::Tls(_))));
}

/// A calendar on the server, read, as the engine keeps it.
fn read(server: &Server, dav: &Dav) -> Pim {
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(String::new()),
        dav_checked: Some(100),
        ..Default::default()
    };
    pim.ensure_defaults();
    let first = sync(dav, &AT, &pim, 100, Zone::UTC).unwrap();
    pim.calendars = first.calendars.unwrap();
    pim.events = first.events;
    pim
}

/// As a real Mailcow's SOGo does: an event the user may see and not change (a role for its
/// classification, in a calendar where they may change others) is refused with 403. It was
/// taken for a wrong password, and the whole sync of calendars and contacts stopped, at
/// every sync. Now the change and the deletion are given up and the server's copy read
/// again; the sync goes on.
#[test]
fn a_change_the_server_forbids_is_given_up() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    {
        let mut d = server.data.lock().unwrap();
        d.put(&format!("{CAL}conf.ics"), &ical::write(&[event("conf", "Interview", 1_800_000_000)], None, 0));
        d.put(&format!("{CAL}mine.ics"), &ical::write(&[event("mine", "Lunch", 1_800_100_000)], None, 0));
        d.forbidden = vec![format!("{CAL}conf.ics"), format!("{CAL}mine.ics")];
    }
    let mut pim = read(&server, &dav);
    let conf = pim.events.iter().position(|e| e.event.uid == "conf").unwrap();
    pim.events[conf].event.summary = "Bob was here".into();
    pim.events[conf].dirty = true;
    let mine = pim.events.iter().position(|e| e.event.uid == "mine").unwrap();
    let gone = pim.events.remove(mine);
    pim.deleted.push(mailcraft_model::pim::Tombstone { href: gone.href.clone(), etag: gone.etag.clone(), ..Default::default() });
    pim.events.push(StoredEvent {
        calendar: pim.calendars[0].id.clone(),
        event: event("new", "Standup", 1_800_200_000),
        dirty: true,
        ..Default::default()
    });
    let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert_eq!(out.refused, ["conf"]);
    assert_eq!(out.deleted, std::slice::from_ref(&gone.href), "the deletion is given up");
    assert_eq!(out.errors.len(), 2, "both said: {:?}", out.errors);
    assert!(out.errors.iter().any(|e| e.contains("Interview") || e.contains("Bob was here")), "{:?}", out.errors);
    let back: Vec<&str> = out.events.iter().map(|e| e.event.summary.as_str()).collect();
    assert!(back.contains(&"Interview") && back.contains(&"Lunch"), "the server's copies read again: {back:?}");
    assert_eq!(out.pushed_events.iter().map(|e| e.event.uid.as_str()).collect::<Vec<_>>(), ["new"], "the rest is stored");
    let d = server.data.lock().unwrap();
    assert!(!d.items.values().any(|(_, v)| v.contains("Bob was here")));
}

/// SOGo puts a meeting in the calendar of each of its users invited, under the organizer's
/// item name, before the invitation's mail comes. The invitation taken in here (a new item,
/// named by its uid) was refused (409, "Event UID already in use") at every sync, and stayed
/// apart from the server's copy. It goes over the server's copy now, so there's one, when it
/// has the user's answer; otherwise (and when the server's is a newer version) the server's
/// comes in its place. Stored over, an answer given on another device was undone (a real
/// Mailcow: accepted on the phone, back to "needs action" when the laptop woke up).
#[test]
fn a_meeting_the_server_put_in_the_calendar_already_is_stored_over() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let meeting = |uid: &str, sequence: u32, summary: &str, answer: &str| CalEvent {
        sequence,
        attendees: vec![ical::Attendee { email: USER.into(), partstat: answer.into(), ..Default::default() }],
        ..event(uid, summary, 1_800_000_000)
    };
    {
        let mut d = server.data.lock().unwrap();
        d.put(&format!("{CAL}organizers-name.ics"), &ical::write(&[meeting("m1", 0, "Planning", "NEEDS-ACTION")], None, 0));
        d.put(&format!("{CAL}updated.ics"), &ical::write(&[meeting("m2", 2, "Review (moved)", "NEEDS-ACTION")], None, 0));
        d.put(&format!("{CAL}phone.ics"), &ical::write(&[meeting("m3", 0, "Standup", "ACCEPTED")], None, 0));
        // Another whose uid holds this one's: not taken for it.
        d.put(&format!("{CAL}longer.ics"), &ical::write(&[meeting("m1-and-more", 0, "Other", "NEEDS-ACTION")], None, 0));
    }
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(String::new()),
        dav_checked: Some(100),
        ..Default::default()
    };
    pim.ensure_defaults();
    let cal = format!("{}{CAL}", server.base);
    // Answered here: m1. Taken in from the mail only: m2 (older) and m3 (answered elsewhere).
    let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
    pim.events.push(new(meeting("m1", 0, "Planning", "TENTATIVE")));
    pim.events.push(new(meeting("m2", 1, "Review", "NEEDS-ACTION")));
    pim.events.push(new(meeting("m3", 0, "Standup", "NEEDS-ACTION")));
    let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let m1 = out.pushed_events.iter().find(|e| e.event.uid == "m1").expect("stored");
    assert_eq!(m1.href, format!("{cal}organizers-name.ics"), "over the server's copy");
    assert_eq!(out.refused, ["m2", "m3"], "the server's is newer, or has all this one has");
    assert!(out.events.iter().any(|e| e.event.uid == "m2" && e.event.summary == "Review (moved)"), "read in its place");
    assert!(out.events.iter().any(|e| e.event.uid == "m3" && e.event.attendees[0].partstat == "ACCEPTED"), "read in its place");
    let d = server.data.lock().unwrap();
    let with = |uid: &str| d.items.values().filter(|(_, v)| uid_of(v).as_deref() == Some(uid)).count();
    assert_eq!((with("m1"), with("m2"), with("m3"), with("m1-and-more")), (1, 1, 1, 1));
    assert!(d.items[&format!("{CAL}organizers-name.ics")].1.contains("PARTSTAT=TENTATIVE"));
    assert!(d.items[&format!("{CAL}updated.ics")].1.contains("Review (moved)"));
    assert!(d.items[&format!("{CAL}phone.ics")].1.contains("PARTSTAT=ACCEPTED"), "the answer given elsewhere stays");
}

/// SOGo stores a new item over one of the same name, though asked not to (If-None-Match),
/// and puts a meeting in each invited user's calendar under the organizer's item name, which
/// MailCraft's (and so the invitation taken in here) names by its uid. On a real Mailcow, bob
/// accepted on his phone, his laptop woke up, took the invitation in from the mail and stored
/// it over the calendar's copy: back to "needs action". The calendar is asked for the meeting
/// first now; its copy comes in, unless this one has an answer the server's lacks.
#[test]
fn a_meeting_stored_under_its_own_name_already_isnt_written_over() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let ann = Some(ical::Attendee { email: "ann@x".into(), ..Default::default() });
    let meeting = |uid: &str, answer: &str| CalEvent {
        organizer: ann.clone(),
        attendees: vec![ical::Attendee { email: USER.into(), partstat: answer.into(), ..Default::default() }],
        ..event(uid, "Planning", 1_800_000_000)
    };
    {
        let mut d = server.data.lock().unwrap();
        d.overwrites = true;
        d.put(&format!("{CAL}p1.ics"), &ical::write(&[meeting("p1", "ACCEPTED")], None, 0));
        d.put(&format!("{CAL}p2.ics"), &ical::write(&[meeting("p2", "NEEDS-ACTION")], None, 0));
    }
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(String::new()),
        dav_checked: Some(100),
        ..Default::default()
    };
    pim.ensure_defaults();
    let cal = format!("{}{CAL}", server.base);
    let new = |e: CalEvent| StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() };
    // p1: taken in from the mail, answered elsewhere since. p2: answered here.
    pim.events.push(new(meeting("p1", "NEEDS-ACTION")));
    pim.events.push(new(meeting("p2", "DECLINED")));
    let out = sync(&dav, &AT, &pim, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.refused, ["p1"]);
    assert!(out.events.iter().any(|e| e.event.uid == "p1" && e.event.attendees[0].partstat == "ACCEPTED"), "the server's copy read");
    assert_eq!(out.pushed_events.iter().map(|e| e.event.uid.as_str()).collect::<Vec<_>>(), ["p2"]);
    let d = server.data.lock().unwrap();
    assert!(d.items[&format!("{CAL}p1.ics")].1.contains("PARTSTAT=ACCEPTED"), "the answer given elsewhere stays");
    assert!(d.items[&format!("{CAL}p2.ics")].1.contains("PARTSTAT=DECLINED"));
}

/// SOGo writes a guest's answer into the organizer's copy (and the other guests'), without a
/// new SEQUENCE. A change made here meanwhile (the meeting's notes, a version on) was refused
/// with 412 and the server's copy kept: on a real Mailcow, the agenda ann wrote right after
/// taking bob's answer in was gone, without a word. It's stored now, with the answers the
/// server has. A change to the same version as the server's is still given way to, said
/// unless the server's copy has all of it.
#[test]
fn a_change_crossing_a_guests_answer_keeps_both() {
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let guests = |bob: &str, carol: &str| {
        vec![
            ical::Attendee { email: "bob@x".into(), partstat: bob.into(), ..Default::default() },
            ical::Attendee { email: "carol@x".into(), partstat: carol.into(), ..Default::default() },
        ]
    };
    let base = CalEvent { attendees: guests("NEEDS-ACTION", "NEEDS-ACTION"), ..event("m", "Budget", 1_800_000_000) };
    server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(std::slice::from_ref(&base), None, 0));
    let pim = read(&server, &dav);
    // bob answers: the server writes it in, the same version.
    let answered = CalEvent { attendees: guests("ACCEPTED", "NEEDS-ACTION"), ..base.clone() };
    server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(std::slice::from_ref(&answered), None, 0));
    // The organizer writes the agenda (a new version) and takes carol's answer in.
    let mut mine = pim.clone();
    mine.events[0].event.description = "Agenda: numbers".into();
    mine.events[0].event.sequence = 1;
    mine.events[0].event.attendees = guests("NEEDS-ACTION", "DECLINED");
    mine.events[0].dirty = true;
    let out = sync(&dav, &AT, &mine, 200, Zone::UTC).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.refused, ["m"], "the stored copy is read back");
    let back = out.events.iter().find(|e| e.event.uid == "m").expect("read back");
    assert_eq!(back.event.description, "Agenda: numbers");
    let answers: Vec<&str> = back.event.attendees.iter().map(|a| a.partstat.as_str()).collect();
    assert_eq!(answers, ["ACCEPTED", "DECLINED"], "both answers");
    // The same version changed here and there: the server's is kept, and the change that
    // isn't in it is said (it used to go without a word when the title and times matched).
    let mut same = read(&server, &dav);
    server.data.lock().unwrap().put(&format!("{CAL}m.ics"), &ical::write(&[CalEvent { sequence: 1, ..back.event.clone() }], None, 0));
    same.events[0].event.description = "Agenda: other numbers".into();
    same.events[0].dirty = true;
    let out = sync(&dav, &AT, &same, 300, Zone::UTC).unwrap();
    assert_eq!((out.errors.len(), out.refused.len()), (1, 1), "{:?}", out.errors);
    assert!(server.data.lock().unwrap().items[&format!("{CAL}m.ics")].1.contains("Agenda: numbers"));
    // An event without guests changed elsewhere (another program needn't give it a new
    // SEQUENCE): not written over, the server's kept and the change here said.
    server.data.lock().unwrap().put(&format!("{CAL}solo.ics"), &ical::write(&[event("solo", "Dentist", 1_800_100_000)], None, 0));
    let mut solo = read(&server, &dav);
    server.data.lock().unwrap().put(&format!("{CAL}solo.ics"), &ical::write(&[event("solo", "Dentist (moved)", 1_800_100_000)], None, 0));
    solo.events.retain(|e| e.event.uid == "solo");
    solo.events[0].event.description = "Bring the card".into();
    solo.events[0].event.sequence = 1;
    solo.events[0].dirty = true;
    let out = sync(&dav, &AT, &solo, 400, Zone::UTC).unwrap();
    assert_eq!((out.errors.len(), out.refused.as_slice()), (1, ["solo".to_string()].as_slice()), "{:?}", out.errors);
    assert!(server.data.lock().unwrap().items[&format!("{CAL}solo.ics")].1.contains("Dentist (moved)"));
}

/// Google OAuth account: CalDAV sync with a Bearer token.
#[test]
fn google_oauth_bearer_auth() {
    let server = serve();
    let token = "test-access-token-12345";
    server.data.lock().unwrap().bearer = Some(token.into());
    server.data.lock().unwrap().put(&format!("{CAL}g.ics"), &ical::write(&[event("g", "Meeting", 1_800_000_000)], None, 0));

    let dav = Dav::bearer(token, &["127.0.0.1"]);
    // Discover the home with Bearer auth.
    let home = dav.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)).unwrap().unwrap();

    // The contacts at the fake's address too: nothing is looked for out on the network.
    let contacts = format!("{}/dav/", server.base);
    let at_google = Where {
        email: "user@gmail.com",
        username: "",
        others: &[],
        imap_host: "imap.gmail.com",
        caldav: None,
        carddav: Some(&contacts),
        mails: true,
    };

    // First sync: discover and read.
    let mut pim = Pim { caldav_home: Some(home), ..Default::default() };
    pim.ensure_defaults();
    let first = sync(&dav, &at_google, &pim, 100, Zone::UTC).unwrap();
    assert!(first.errors.is_empty(), "{:?}", first.errors);

    // Events synced with Bearer auth.
    let cal_id = format!("{}{CAL}", server.base);
    assert_eq!(first.calendars.as_ref().map(|c| (c.len(), c[0].name.as_str(), c[0].id.as_str())), Some((1, "Work", cal_id.as_str())));
    assert_eq!(first.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Meeting"]);

    // Wrong token refused.
    let dav_wrong = Dav::bearer("wrong-token", &["127.0.0.1"]);
    assert!(matches!(dav_wrong.home_from(Kind::Calendars, &format!("{}/.well-known/caldav", server.base)), Err(crate::NetError::Auth(_))));
}
