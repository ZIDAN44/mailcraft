use super::*;

#[test]
fn reads_multistatus() {
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/">
  <d:response>
<d:href>/dav/calendars/me/work/</d:href>
<d:propstat>
  <d:prop>
    <d:displayname>Work &amp; studio</d:displayname>
    <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
    <cs:getctag>"42"</cs:getctag>
    <c:supported-calendar-component-set><c:comp name="VEVENT"/><c:comp name="VTODO"/></c:supported-calendar-component-set>
    <d:current-user-privilege-set><d:privilege><d:read/></d:privilege><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
  </d:prop>
  <d:status>HTTP/1.1 200 OK</d:status>
</d:propstat>
<d:propstat>
  <d:prop><d:getetag/></d:prop>
  <d:status>HTTP/1.1 404 Not Found</d:status>
</d:propstat>
  </d:response>
  <d:response>
<d:href>/dav/principals/me/</d:href>
<d:propstat><d:prop><d:current-user-principal><d:href>/dav/principals/me/</d:href></d:current-user-principal>
<c:calendar-data><![CDATA[BEGIN:VCALENDAR
END:VCALENDAR]]></c:calendar-data></d:prop>
<d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  </d:response>
</d:multistatus>"#;
    let items = multistatus(xml);
    assert_eq!(items.len(), 2);
    let w = &items[0];
    assert_eq!(w.href, "/dav/calendars/me/work/");
    assert_eq!(w.props.get("displayname").map(String::as_str), Some("Work & studio"));
    assert_eq!(w.props.get("getctag").map(String::as_str), Some("\"42\""));
    assert!(w.kinds.contains(&"calendar".to_string()) && w.kinds.contains(&"collection".to_string()));
    assert_eq!(w.comps, vec!["VEVENT", "VTODO"]);
    assert_eq!(w.writable, Some(true));
    assert!(!w.props.contains_key("getetag"), "404 properties are left out");
    assert_eq!(items[1].hrefs.get("current-user-principal"), Some(&vec!["/dav/principals/me/".to_string()]));
    assert!(items[1].props.get("calendar-data").is_some_and(|d| d.starts_with("BEGIN:VCALENDAR")));
    for junk in
        ["", "<", "<d:multistatus>", "<a><b><c>", &"<x>".repeat(1000), "<?xml?><!DOCTYPE x [<!ENTITY e \"boom\">]><d:multistatus>&e;</d:multistatus>"]
    {
        let _ = multistatus(junk);
    }
    assert_eq!(absolute("https://h.example/dav/cal/", "/dav/cal/1.ics").unwrap(), "https://h.example/dav/cal/1.ics");
}

/// A response past the depth cap is left out with all it holds: its end doesn't end the response
/// it's in, whose properties after it are still read.
#[test]
fn past_the_depth_cap_a_response_keeps_its_properties() {
    // `multistatus`, `response` and 62 `x`s fill the 64 levels.
    let (opens, closes) = ("<x>".repeat(62), "</x>".repeat(62));
    let xml = format!(
        "<d:multistatus xmlns:d=\"DAV:\"><d:response><d:href>/a/</d:href>{opens}<d:response><d:href>/evil/</d:href></d:response>{closes}\
         <d:propstat><d:prop><d:displayname>Kept</d:displayname></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>\
         <d:response><d:href>/b/</d:href></d:response></d:multistatus>"
    );
    let items = multistatus(&xml);
    let hrefs: Vec<&str> = items.iter().map(|i| i.href.as_str()).collect();
    assert_eq!(hrefs, ["/a/", "/b/"]);
    assert_eq!(items[0].props.get("displayname").map(String::as_str), Some("Kept"));
    assert!(items[0].hrefs.is_empty(), "nothing past the cap is read: {:?}", items[0].hrefs);
}

/// Item names are one per uid, short, and safe in an address.
#[test]
fn item_names_dont_collide() {
    assert_eq!(resource_name("plain-uid_1.2", "ics"), "plain-uid_1.2.ics", "a plain uid is its own name");
    let odd = resource_name("a/b c@x", "ics");
    assert!(odd.starts_with("a-b-c-x~") && odd.ends_with(".ics"), "{odd}");
    assert_ne!(resource_name("a/b", "ics"), resource_name("a b", "ics"), "changed characters don't collide");
    assert_ne!(resource_name("a-b", "ics"), resource_name("a b", "ics"));
    let long = "z".repeat(200);
    let (one, two) = (resource_name(&format!("{long}1"), "ics"), resource_name(&format!("{long}2"), "ics"));
    assert_ne!(one, two, "long uids alike at the start don't collide");
    assert_ne!(resource_name(&"y".repeat(101), "ics"), resource_name(&"y".repeat(102), "ics"));
    assert_eq!(one, resource_name(&format!("{long}1"), "ics"), "stable");
    for uid in ["", "é", "../..", "a\r\nb", &"€".repeat(500), &long] {
        let n = resource_name(uid, "vcf");
        assert!(n.len() <= NAME_KEEP + 21 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~".contains(c)), "{uid:?}: {n}");
    }
}

/// Where an SRV record sends the search (Posteo's: posteo.de port 8443, `path=/`): its TXT path,
/// its well-known address, its root, each once; port 443 isn't written.
#[test]
fn an_srv_record_and_its_path_give_the_starts() {
    let s = srv_starts(Kind::Calendars, Some(("posteo.de".into(), 8443, Some("/".into()))));
    assert_eq!(s, ["https://posteo.de:8443/", "https://posteo.de:8443/.well-known/caldav"]);
    let s = srv_starts(Kind::Contacts, Some(("dav.example".into(), 443, Some("/card/".into()))));
    assert_eq!(s, ["https://dav.example/card/", "https://dav.example/.well-known/carddav", "https://dav.example/"]);
    let s = srv_starts(Kind::Calendars, Some(("dav.example".into(), 8443, None)));
    assert_eq!(s, ["https://dav.example:8443/.well-known/caldav", "https://dav.example:8443/"]);
    assert!(srv_starts(Kind::Calendars, None).is_empty());
}

#[test]
fn addresses_typed_by_hand_read_as_https() {
    assert_eq!(given_url("posteo.de:8443").as_deref(), Some("https://posteo.de:8443/"));
    assert_eq!(given_url(" https://posteo.de:8443/calendars/me/default ").as_deref(), Some("https://posteo.de:8443/calendars/me/default"));
    assert_eq!(given_url("HTTPS://Dav.Example/x").as_deref().map(|u| u.ends_with("/x")), Some(true));
    assert_eq!(given_url("http://localhost:5232/").as_deref(), Some("http://localhost:5232/"));
    assert_eq!(given_url("http://127.0.0.1:5232/u/").as_deref(), Some("http://127.0.0.1:5232/u/"));
    for bad in ["", "   ", "http://dav.example/", "https://a b/", "ftp://dav.example/", &format!("https://{}", "a".repeat(3000))] {
        assert_eq!(given_url(bad), None, "{bad}");
    }
    assert_eq!(parent_of("https://p.example:8443/calendars/me/default").as_deref(), Some("https://p.example:8443/calendars/me/"));
    assert_eq!(parent_of("https://p.example/calendars/me/").as_deref(), Some("https://p.example/calendars/"));
    assert_eq!(parent_of("https://p.example/").as_deref(), None);
}

/// Issue #30: SOGo lists the calendars the user subscribed to in their own home; one whose
/// owner isn't the user (nor the home's owner) is someone else's. A server that names no owner
/// marks nothing.
#[test]
fn calendars_owned_by_someone_else_are_told_apart() {
    let listing = |owner_of_home: &str, cup: &str| {
        format!(
            r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
<d:response><d:href>/SOGo/dav/riley/Calendar/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>{owner_of_home}{cup}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/SOGo/dav/riley/Calendar/sam_team/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:owner><d:href>https://mail.example/SOGo/dav/sam/</d:href></d:owner></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/SOGo/dav/riley/Calendar/personal/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:owner><d:href>/SOGo/dav/riley</d:href></d:owner></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
<d:response><d:href>/SOGo/dav/riley/Calendar/nobody/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
</d:multistatus>"#
        )
    };
    let at = "https://mail.example/SOGo/dav/riley/Calendar/";
    let foreign = |xml: &str| {
        let mut items = multistatus(xml);
        mark_foreign(&mut items, at, at);
        items.iter().map(|i| i.foreign).collect::<Vec<_>>()
    };
    let owner = "<d:owner><d:href>/SOGo/dav/riley/</d:href></d:owner>";
    let cup = "<d:current-user-principal><d:href>/SOGo/dav/riley/</d:href></d:current-user-principal>";
    assert_eq!(foreign(&listing(owner, "")), [false, true, false, false]);
    assert_eq!(foreign(&listing("", cup)), [false, true, false, false]);
    assert_eq!(foreign(&listing("", "")), [false, false, false, false], "no owner of the user's named: nothing marked");
    let mut junk = multistatus(r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>%%%</d:href></d:response></d:multistatus>"#);
    mark_foreign(&mut junk, "not a url", "");
}

/// A first sync of a calendar and an address book of 20,000 items each (every item read), and
/// one with 20,000 events changed here in a calendar of another server (none sent: what's timed
/// is finding each one's parts). Run by hand, in release:
/// `cargo test --release -p mailcraft-net --lib measure_big_collections -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_big_collections() {
    use crate::fakes::fake_dav::{BOOK, CAL, PASSWORD, USER, serve};
    use mailcraft_model::zone::Zone;
    const N: usize = 20_000;
    let uid = |i: usize| format!("{i:08x}-5f2c-4b1e-9a7d-3c6e8f0a1b2c");
    let event = |i: usize| {
        let start = 1_800_000_000 + i64::try_from(i).unwrap_or_default() * 3600;
        CalEvent { uid: uid(i), summary: format!("Event {i}"), start, end: start + 1800, ..Default::default() }
    };
    let at = Where { email: USER, imap_host: "imap.example", mails: true, ..Default::default() };
    let pim_at = |base: &str| {
        let mut pim = Pim {
            caldav_home: Some(format!("{base}/dav/cal/")),
            carddav_home: Some(format!("{base}/dav/card/")),
            dav_checked: Some(100),
            ..Default::default()
        };
        pim.ensure_defaults();
        pim
    };
    let dav = Dav::new(USER, PASSWORD);

    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        for i in 0..N {
            d.put(&format!("{CAL}{}.ics", uid(i)), &ical::write(&[event(i)], None, 0));
            d.put(&format!("{BOOK}{}.vcf", uid(i)), &vcard::write(&vcard::Card { uid: uid(i), name: format!("Person {i}"), ..Default::default() }));
        }
    }
    let t = std::time::Instant::now();
    let out = sync(&dav, &at, &pim_at(&server.base), 100, Zone::UTC).unwrap();
    let read = t.elapsed();
    assert_eq!((out.events.len(), out.cards.len(), out.kept.len(), out.errors.len()), (N, N, 0, 0));
    let digest = mailcraft_store::fnv64(&format!("{out:?}").replace(&server.base, ""));
    println!("MEASURE first read of {N} events and {N} cards: {:.0} ms (result {digest:016x})", read.as_secs_f64() * 1000.0);

    let empty = serve();
    let mut pim = pim_at(&empty.base);
    pim.events = (0..N)
        .map(|i| StoredEvent {
            calendar: "https://old.example/cal/".into(),
            href: format!("https://old.example/cal/{}.ics", uid(i)),
            etag: "\"1\"".into(),
            event: event(i),
            dirty: true,
        })
        .collect();
    let t = std::time::Instant::now();
    let out = sync(&dav, &at, &pim, 100, Zone::UTC).unwrap();
    let pushed = t.elapsed();
    assert_eq!((out.pushed_events.len(), out.errors.len()), (0, 0));
    let digest = mailcraft_store::fnv64(&format!("{out:?}").replace(&empty.base, ""));
    println!("MEASURE sync with {N} events changed for another server: {:.0} ms (result {digest:016x})", pushed.as_secs_f64() * 1000.0);
}

/// An invitation whose uid holds control characters, kept, stored in the calendar at `calendar`
/// as MailCraft stores it, then looked for by the uid kept: found, and read back with that uid.
/// The item is deleted after.
fn an_odd_uid_is_found(dav: &Dav, calendar: &str, tag: &str) {
    let invitation = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:odd\u{1}uid\u{1b}-{tag}\r\nDTSTAMP:20261101T090000Z\r\n\
         DTSTART:20261102T090000Z\r\nDTEND:20261102T100000Z\r\nSUMMARY:Odd\u{1} one\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let kept = ical::parse(&invitation, 0).events.remove(0);
    assert_eq!(kept.uid, format!("odd\u{1}uid\u{1b}-{tag}"));
    let href = format!("{}/odd-{tag}.ics", calendar.trim_end_matches('/'));
    dav.put(&href, "text/calendar; charset=utf-8", &ical::write_for_server(std::slice::from_ref(&kept), 0), None).unwrap();
    let found = dav.find_uid(calendar, &kept.uid).unwrap();
    assert_eq!(found, std::slice::from_ref(&href));
    let read = dav.multiget(Kind::Calendars, calendar, &found).unwrap();
    let back: Vec<CalEvent> = read.iter().flat_map(|(_, _, data)| ical::parse(data, 0).events).collect();
    assert!(back.iter().any(|e| e.uid == ical::stored_uid(&kept.uid) && e.summary == "Odd one"), "{read:?}");
    let _ = dav.remove(&href, "");
}

/// Found in the audit: an invitation's uid with a control character was kept so and written so;
/// the server kept it without, and the meeting wasn't found by the uid kept here.
#[test]
fn an_invitations_odd_uid_is_found_where_it_was_stored() {
    use crate::fakes::fake_dav::{CAL, PASSWORD, USER, serve};
    let server = serve();
    an_odd_uid_is_found(&Dav::new(USER, PASSWORD), &format!("{}{CAL}", server.base), "fake");
}

/// The same on a real CalDAV server on this computer (Radicale; a made-up account):
/// `MAILCRAFT_REAL_CALDAV="http://127.0.0.1:21232/odd/work/ odd pw"` (the calendar, made first).
#[test]
#[ignore = "needs a real CalDAV server in MAILCRAFT_REAL_CALDAV"]
fn real_an_invitations_odd_uid_is_found_where_it_was_stored() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALDAV") else {
        eprintln!("MAILCRAFT_REAL_CALDAV isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALDAV: url user password") };
    an_odd_uid_is_found(&Dav::new(user, password), url, &std::process::id().to_string());
}

/// A Bearer session refused with 403 (insufficient OAuth scope) returns Auth error and no home
/// is remembered; the same account, once the token is accepted, finds its calendar on the next
/// sync even though dav_checked is recent.
#[test]
fn bearer_403_is_auth_error_and_retries_while_no_home() {
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    let token = "test-token";
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some(token.to_string());
        d.bearer_forbidden = true;
    }

    let dav = Dav::bearer(token);
    let caldav_url = format!("{}/dav/", server.base);
    let at = Where { email: USER, imap_host: "imap.example", caldav: Some(&caldav_url), mails: true, ..Default::default() };
    let mut pim = Pim { dav_checked: Some(100), ..Default::default() };
    pim.ensure_defaults();

    // First sync: 403 becomes Auth error, no home is saved.
    let err = sync(&dav, &at, &pim, 200, Zone::UTC).unwrap_err();
    assert!(matches!(err, NetError::Auth(_)), "403 on Bearer should be Auth: {err:?}");
    assert!(err.to_string().contains("sign in again"), "error should mention signing in again: {err}");

    // No home was remembered.
    pim.dav_checked = Some(200);
    pim.caldav_home = None;

    // Second sync: bearer_forbidden is still true, but dav_checked is recent (200, now 300).
    // For Bearer with no home, it should still try (not wait a day).
    let err = sync(&dav, &at, &pim, 300, Zone::UTC).unwrap_err();
    assert!(matches!(err, NetError::Auth(_)), "should retry on Bearer with no home: {err:?}");

    // Now the token is accepted.
    {
        let mut d = server.data.lock().unwrap();
        d.bearer_forbidden = false;
    }

    // Third sync: dav_checked is still recent (200, now 400), but Bearer with no home retries.
    let out = sync(&dav, &at, &pim, 400, Zone::UTC).unwrap();
    assert!(out.caldav_home.is_some(), "calendar home should be found: {out:?}");
    assert!(out.caldav_home.as_ref().unwrap().contains("/dav/cal/"), "home: {:?}", out.caldav_home);
    assert_eq!(out.errors.len(), 0, "no errors: {out:?}");
}
