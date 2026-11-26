use super::*;

/// Issue #7: the links calendars are published with, and what isn't one.
#[test]
fn calendar_links_are_read_as_web_addresses() {
    assert_eq!(calendar_link(" webcal://p01-calendars.example.com/x.ics ").as_deref(), Some("https://p01-calendars.example.com/x.ics"));
    assert_eq!(calendar_link("WEBCALS://cal.example/a?b=1").as_deref(), Some("https://cal.example/a?b=1"));
    assert_eq!(calendar_link("http://192.168.1.10:5232/riley/work/").as_deref(), Some("http://192.168.1.10:5232/riley/work/"));
    assert_eq!(calendar_link("https://[::1]:8443/cal.ics").as_deref(), Some("https://[::1]:8443/cal.ics"));
    assert_eq!(calendar_link("https://cal.example/a.ics#top").as_deref(), Some("https://cal.example/a.ics"));
    for bad in [
        "",
        "webcal://",
        "https:///x",
        "ftp://x.example/a.ics",
        "file:///etc/passwd",
        "https://a@b.example/x",
        "https://x.example/a b",
        "https://:80/x",
        "x.example/a.ics",
    ] {
        assert_eq!(calendar_link(bad), None, "{bad:?}");
    }
    assert_eq!(calendar_link(&format!("https://x.example/{}", "a".repeat(MAX_CALENDAR_LINK))), None);
    assert!(is_web_calendar(&web_calendar_id("https://x.example/a.ics")));
    assert!(!is_web_calendar(LOCAL) && !is_web_calendar(&shared_calendar_id("a@b.example")));
}
