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

/// A Google account signed in before MailCraft asked for calendars: Google refuses its token
/// the permission (403, `insufficient_scope`). That's said, the rest syncs and none is saved;
/// it isn't asked again at every sync, only a day on or once signed in again (the engine then
/// saves the last look as long ago).
#[test]
fn a_token_without_the_permission_waits_for_a_new_sign_in() {
    const T: i64 = 1_800_000_000;
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    let token = "test-token";
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some(token.to_string());
        d.bearer_forbidden = true;
    }
    let dav = Dav::bearer(token, &["127.0.0.1"]);
    let caldav_url = format!("{}/dav/", server.base);
    let at =
        Where { email: USER, imap_host: "imap.example", caldav: Some(&caldav_url), carddav: Some(&caldav_url), mails: true, ..Default::default() };
    let mut pim = Pim::default();
    pim.ensure_defaults();

    let out = sync(&dav, &at, &pim, T, Zone::UTC).unwrap();
    assert_eq!(out.caldav_home, None);
    // Said for the calendars and the contacts, both looked for at the address given.
    assert_eq!(out.errors, ["sign in again to allow MailCraft this permission: https://www.googleapis.com/auth/calendar"; 2], "{out:?}");

    // As the engine saves it: none, looked for at T. The next sync doesn't ask.
    (pim.caldav_home, pim.carddav_home, pim.dav_checked) = (Some(String::new()), Some(String::new()), Some(T));
    server.data.lock().unwrap().bearer_forbidden = false;
    let asked = server.data.lock().unwrap().log.len();
    let out = sync(&dav, &at, &pim, T + 600, Zone::UTC).unwrap();
    assert_eq!((out.caldav_home, out.errors.len()), (None, 0));
    assert_eq!(server.data.lock().unwrap().log.len(), asked, "not asked again at every sync");

    // Signed in again: found.
    pim.dav_checked = Some(0);
    let out = sync(&dav, &at, &pim, T + 1200, Zone::UTC).unwrap();
    assert_eq!(out.caldav_home, Some(format!("{}/dav/cal/", server.base)), "{out:?}");
    assert!(out.errors.is_empty(), "{out:?}");
}

/// What Google said in refusing a token: the permission it lacks, or the API not turned on for
/// the client (its own words, capped); any other 403 isn't the token's.
#[test]
fn a_refused_token_says_what_google_said() {
    let header = |v: &str| vec![("www-authenticate".to_string(), v.to_string())];
    assert_eq!(
        bearer::refusal(&header("Bearer realm=\"https://accounts.google.com/\", error=\"insufficient_scope\""), b""),
        Some("sign in again to allow MailCraft this permission".into())
    );
    assert_eq!(
        bearer::refusal(&header("Bearer error=\"insufficient_scope\", scope=\"https://www.googleapis.com/auth/carddav\""), b""),
        Some("sign in again to allow MailCraft this permission: https://www.googleapis.com/auth/carddav".into())
    );
    let body = b"<errors xmlns='http://schemas.google.com/g/2005'><error><domain>GData</domain><code>accessNotConfigured</code>\
        <internalReason>CalDAV API has not been used in project 123 before or it is disabled. Enable it by visiting \
        https://console.developers.google.com/apis/api/caldav.googleapis.com/overview?project=123 then retry.</internalReason></error></errors>";
    let said = bearer::refusal(&[], body).unwrap_or_default();
    assert!(said.starts_with("CalDAV API has not been used") && said.ends_with("then retry."), "{said}");
    assert_eq!(bearer::refusal(&[], b"<code>accessNotConfigured</code>"), Some("the server's API isn't turned on for this Google client".into()));
    // Capped on a character's boundary (hostile input).
    let long = format!("<code>accessNotConfigured</code><internalReason>{}</internalReason>", "€".repeat(400));
    let said = bearer::refusal(&[], long.as_bytes()).unwrap_or_default();
    assert!(said.len() <= 500 && said.len() > 490 && said.chars().all(|c| c == '€'), "{}", said.len());
    // Not the token's: a calendar or item the user may not change, or no reason given.
    assert_eq!(bearer::refusal(&[], b"<internalReason>You don't have permission</internalReason>"), None);
    assert_eq!(bearer::refusal(&header("Bearer realm=\"x\""), b"not xml at all"), None);
}

/// A 403 for one item is that item's on a token session too: refused, the rest of the sync goes
/// on. Google refusing the token itself, there, is the sign-in's.
#[test]
fn a_403_on_a_token_session_is_the_items_unless_the_token_is_refused() {
    use crate::fakes::fake_dav::{CAL, serve};
    let token = "test-token";
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some(token.to_string());
        d.forbidden.push(format!("{CAL}forbidden.ics"));
        d.unreadable.push(CAL.to_string());
    }
    let dav = Dav::bearer(token, &["127.0.0.1"]);
    let url = format!("{}{CAL}forbidden.ics", server.base);
    let ics = "BEGIN:VCALENDAR\r\nUID:test\r\nEND:VCALENDAR";
    assert!(matches!(dav.store(&url, "text/calendar; charset=utf-8", ics, None), Ok(Err(Refused::Forbidden))));
    assert!(matches!(dav.remove(&url, ""), Ok(Err(Refused::Forbidden))));
    assert!(!matches!(dav.collections(Kind::Calendars, &format!("{}{CAL}", server.base)), Err(NetError::Auth(_))));

    server.data.lock().unwrap().bearer_forbidden = true;
    let url = format!("{}{CAL}new.ics", server.base);
    assert!(matches!(dav.store(&url, "text/calendar; charset=utf-8", ics, None), Err(NetError::Auth(_))));
    assert!(matches!(dav.remove(&url, "\"e1\""), Err(NetError::Auth(_))));
}

/// The access token opens the whole Google account: it goes over https to the hosts the session
/// was made for, and nowhere else (another host, a redirect's, plain http but to this computer).
#[test]
fn the_token_goes_to_its_hosts_alone() {
    use crate::fakes::fake_dav::serve;
    let b = bearer::Bearer::new("t", &["apidata.googleusercontent.com", "127.0.0.1"]);
    assert_eq!(b.for_url("https://apidata.googleusercontent.com/caldav/v2/a/events"), Some("Bearer t"));
    assert_eq!(b.for_url("https://APIDATA.googleusercontent.com./x"), Some("Bearer t"));
    assert_eq!(b.for_url("http://apidata.googleusercontent.com/x"), None, "in the clear");
    assert_eq!(b.for_url("https://apidata.googleusercontent.com.attacker.example/x"), None);
    assert_eq!(b.for_url("https://attacker.example/x"), None);
    assert_eq!(b.for_url("http://127.0.0.1:8080/dav/"), Some("Bearer t"), "a test server on this computer");
    assert_eq!(b.for_url("not a url"), None);

    // A server of another name is asked without it.
    let server = serve();
    server.data.lock().unwrap().bearer = Some("t".into());
    let dav = Dav::bearer("t", &["localhost"]);
    assert!(matches!(dav.home_from(Kind::Calendars, &format!("{}/dav/", server.base)), Err(NetError::Auth(_))));
    assert_eq!(server.data.lock().unwrap().offered, 0, "no request carried the token");
}

/// An event and a contact on the fake server, to see each read.
fn seed(server: &crate::fakes::fake_dav::Server) {
    use crate::fakes::fake_dav::{BOOK, CAL};
    let mut d = server.data.lock().unwrap();
    let event = CalEvent { uid: "g-event".into(), summary: "Planning".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
    d.put(&format!("{CAL}g-event.ics"), &ical::write(&[event], None, 0));
    d.put(&format!("{BOOK}g-card.vcf"), &vcard::write(&vcard::Card { uid: "g-card".into(), name: "Zoe Park".into(), ..Default::default() }));
}

/// The uids of the events and the contacts a sync read.
fn read(out: &DavSync) -> (Vec<&str>, Vec<&str>) {
    (out.events.iter().map(|e| e.event.uid.as_str()).collect(), out.cards.iter().map(|c| c.card.uid.as_str()).collect())
}

/// A Google token with both permissions syncs calendars and contacts.
#[test]
fn a_token_syncs_calendars_and_contacts() {
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    server.data.lock().unwrap().bearer = Some("test-token".into());
    seed(&server);
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    let url = format!("{}/dav/", server.base);
    let at = Where { email: USER, imap_host: "imap.example", caldav: Some(&url), carddav: Some(&url), mails: true, ..Default::default() };
    let mut pim = Pim::default();
    pim.ensure_defaults();

    let out = sync(&dav, &at, &pim, 1_800_000_000, Zone::UTC).unwrap();
    assert_eq!(out.caldav_home, Some(format!("{}/dav/cal/", server.base)), "{out:?}");
    assert_eq!(out.carddav_home, Some(format!("{}/dav/card/", server.base)), "{out:?}");
    assert!(out.errors.is_empty(), "{out:?}");
    assert_eq!(read(&out), (vec!["g-event"], vec!["g-card"]));
}

/// A sign-in that granted the calendars and not the contacts (Google asks for each, and an
/// account signed in before MailCraft asked for contacts has the calendars' alone): the calendars
/// sync, the contacts' refusal is said once and they're saved as none, looked for again a day on
/// or once signed in again, not at every sync.
#[test]
fn a_token_without_contacts_syncs_the_calendars() {
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    const T: i64 = 1_800_000_000;
    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some("test-token".into());
        d.bearer_lacks_contacts = true;
    }
    seed(&server);
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    let url = format!("{}/dav/", server.base);
    let at = Where { email: USER, imap_host: "imap.example", caldav: Some(&url), carddav: Some(&url), mails: true, ..Default::default() };
    let mut pim = Pim::default();
    pim.ensure_defaults();

    let out = sync(&dav, &at, &pim, T, Zone::UTC).unwrap();
    assert_eq!(out.caldav_home, Some(format!("{}/dav/cal/", server.base)));
    assert_eq!(read(&out), (vec!["g-event"], vec![]), "the calendars synced");
    assert_eq!((out.carddav_home.as_deref(), out.books.as_ref()), (None, None));
    assert_eq!(out.errors, ["sign in again to allow MailCraft this permission: https://www.googleapis.com/auth/carddav"]);

    // As the engine saves it. The next sync doesn't ask for the contacts.
    (pim.caldav_home, pim.carddav_home, pim.dav_checked) = (out.caldav_home.clone(), Some(String::new()), Some(T));
    server.data.lock().unwrap().bearer_lacks_contacts = false;
    server.data.lock().unwrap().log.clear();
    let out = sync(&dav, &at, &pim, T + 600, Zone::UTC).unwrap();
    assert_eq!((out.carddav_home.as_deref(), out.errors.len()), (None, 0));
    assert!(!server.data.lock().unwrap().log.iter().any(|l| l.contains("/dav/card/")), "not asked again at every sync");

    // Signed in again: found and read.
    pim.dav_checked = Some(0);
    let out = sync(&dav, &at, &pim, T + 1200, Zone::UTC).unwrap();
    assert_eq!(out.carddav_home, Some(format!("{}/dav/card/", server.base)), "{out:?}");
    assert!(out.errors.is_empty(), "{out:?}");
    assert_eq!(read(&out).1, ["g-card"]);
}

/// Contacts found before, and a later sign-in that didn't grant them: the calendars still sync,
/// and the contacts are saved as none (looked for again as above), what was read of them dropped.
#[test]
fn contacts_refused_after_a_new_sign_in_leave_the_calendars_syncing() {
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some("test-token".into());
        d.bearer_lacks_contacts = true;
    }
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    seed(&server);
    let at = Where { email: USER, imap_host: "imap.example", mails: true, ..Default::default() };
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(format!("{}/dav/card/", server.base)),
        dav_checked: Some(1_800_000_000),
        ..Default::default()
    };
    pim.ensure_defaults();

    let out = sync(&dav, &at, &pim, 1_800_000_600, Zone::UTC).unwrap();
    assert_eq!(read(&out), (vec!["g-event"], vec![]), "the calendars synced");
    assert_eq!((out.carddav_home.as_deref(), out.books.as_ref()), (None, None));
    assert_eq!(out.errors.len(), 1, "{out:?}");
}

/// A contact deleted here, and a later sign-in that didn't grant the contacts: the deletion is
/// kept for later (said once), and doesn't stop the calendars, nor lose them what they did.
#[test]
fn a_contact_deletion_refused_its_token_waits_and_the_calendars_sync() {
    use crate::fakes::fake_dav::{BOOK, USER, serve};
    use mailcraft_model::pim::Tombstone;
    use mailcraft_model::zone::Zone;

    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some("test-token".into());
        d.bearer_lacks_contacts = true;
    }
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    let at = Where { email: USER, imap_host: "imap.example", mails: true, ..Default::default() };
    let gone = format!("{}{BOOK}gone.vcf", server.base);
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(format!("{}/dav/card/", server.base)),
        dav_checked: Some(1_800_000_000),
        deleted: vec![Tombstone { href: gone.clone(), ..Default::default() }],
        ..Default::default()
    };
    pim.ensure_defaults();

    seed(&server);
    let out = sync(&dav, &at, &pim, 1_800_000_600, Zone::UTC).unwrap();
    assert_eq!(read(&out).0, ["g-event"], "the calendars synced");
    assert!(!out.deleted.contains(&gone), "kept to delete later");
    assert_eq!(out.errors, ["sign in again to allow MailCraft this permission: https://www.googleapis.com/auth/carddav"]);
    assert_eq!(out.carddav_home, None);
}

/// A later sign-in that granted the contacts and not the calendars: the contacts sync, the
/// calendars are said once and looked for again as when none were found, nothing of them sent,
/// though the server is the contacts' too (an event changed here waits).
#[test]
fn a_token_refused_its_calendars_leaves_the_contacts_syncing() {
    use crate::fakes::fake_dav::{CAL, USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some("test-token".into());
        d.bearer_lacks_calendars = true;
    }
    seed(&server);
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    let at = Where { email: USER, imap_host: "imap.example", mails: true, ..Default::default() };
    let changed = CalEvent { uid: "here".into(), summary: "Moved here".into(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(format!("{}/dav/card/", server.base)),
        dav_checked: Some(1_800_000_000),
        events: vec![StoredEvent { calendar: format!("{}{CAL}", server.base), event: changed, dirty: true, ..Default::default() }],
        ..Default::default()
    };
    pim.ensure_defaults();

    let out = sync(&dav, &at, &pim, 1_800_000_600, Zone::UTC).unwrap();
    assert_eq!(read(&out), (vec![], vec!["g-card"]), "the contacts synced");
    assert_eq!((out.caldav_home.as_deref(), out.calendars.as_ref()), (None, None));
    assert_eq!(out.errors, ["sign in again to allow MailCraft this permission: https://www.googleapis.com/auth/calendar"]);
    let log = server.data.lock().unwrap().log.clone();
    assert_eq!(log.iter().filter(|l| !l.contains("/dav/card/")).count(), 1, "one request to the calendars: {log:?}");

    // The syncs after, as the engine saved it (no calendars, looked for just now): the event
    // still waits, and the contacts sync.
    (pim.caldav_home, pim.dav_checked) = (Some(String::new()), Some(1_800_000_600));
    server.data.lock().unwrap().log.clear();
    let out = sync(&dav, &at, &pim, 1_800_001_200, Zone::UTC).unwrap();
    assert_eq!((read(&out), out.errors.len()), ((vec![], vec!["g-card"]), 0));
    let log = server.data.lock().unwrap().log.clone();
    assert!(log.iter().all(|l| l.contains("/dav/card/")), "nothing asked of the calendars: {log:?}");
}

/// The token refused outright (401) for the contacts isn't a permission it lacks: the sync stops
/// as for a refused sign-in, and the contacts' saved server is kept, read again as it was once
/// the token is taken.
#[test]
fn a_token_refused_outright_keeps_the_contacts_server() {
    use crate::fakes::fake_dav::{USER, serve};
    use mailcraft_model::zone::Zone;

    let server = serve();
    {
        let mut d = server.data.lock().unwrap();
        d.bearer = Some("test-token".into());
        d.contacts_reject_token = true;
    }
    seed(&server);
    let dav = Dav::bearer("test-token", &["127.0.0.1"]);
    let at = Where { email: USER, imap_host: "imap.example", mails: true, ..Default::default() };
    let mut pim = Pim {
        caldav_home: Some(format!("{}/dav/cal/", server.base)),
        carddav_home: Some(format!("{}/dav/card/", server.base)),
        dav_checked: Some(1_800_000_000),
        ..Default::default()
    };
    pim.ensure_defaults();

    assert!(matches!(sync(&dav, &at, &pim, 1_800_000_600, Zone::UTC), Err(NetError::Auth(_))));

    // Nothing was saved: the next sync, the token taken, reads the contacts where they were.
    server.data.lock().unwrap().contacts_reject_token = false;
    let out = sync(&dav, &at, &pim, 1_800_001_200, Zone::UTC).unwrap();
    assert_eq!(out.carddav_home, pim.carddav_home);
    assert_eq!((read(&out), out.errors.len()), ((vec!["g-event"], vec!["g-card"]), 0));
}

/// A server not found is looked for again a day after it was looked for, however often the
/// account syncs meanwhile: a sync that only takes the homes as saved says so, and the engine
/// keeps the time of the last look.
#[test]
fn a_server_not_found_is_looked_for_again_a_day_after_the_look() {
    use crate::fakes::fake_dav::{PASSWORD, USER, serve};
    use mailcraft_model::zone::Zone;

    const T: i64 = 1_800_000_000;
    let server = serve();
    let dav = Dav::new(USER, PASSWORD);
    let url = format!("{}/dav/", server.base);
    let at = Where { email: USER, imap_host: "imap.example", caldav: Some(&url), carddav: Some(&url), mails: true, ..Default::default() };
    // The calendars found; the contacts looked for at T and not found.
    let mut pim =
        Pim { caldav_home: Some(format!("{}/dav/cal/", server.base)), carddav_home: Some(String::new()), dav_checked: Some(T), ..Default::default() };
    pim.ensure_defaults();

    // Syncs every quarter of an hour for a day: none looks, each says so.
    for i in 1..=96 {
        let out = sync(&dav, &at, &pim, T + i * 900, Zone::UTC).unwrap();
        assert!(out.homes_kept && out.carddav_home.is_none(), "sync {i}: {out:?}");
    }
    // A day and a moment after the look: looked for again, and found.
    let out = sync(&dav, &at, &pim, T + 86_400 + 60, Zone::UTC).unwrap();
    assert!(!out.homes_kept, "{out:?}");
    assert_eq!(out.carddav_home, Some(format!("{}/dav/card/", server.base)));
}
