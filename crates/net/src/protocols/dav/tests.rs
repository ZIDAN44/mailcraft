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
