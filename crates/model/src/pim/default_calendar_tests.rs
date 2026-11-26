use super::*;

fn cal(id: &str, read_only: bool, foreign: bool) -> CalendarInfo {
    CalendarInfo { id: id.into(), name: id.into(), read_only, foreign, ..Default::default() }
}

/// Issue #30: the user's choice, else the account's own (SOGo's `personal` first) before
/// someone else's listed with it, else whatever can be added to; never a read-only one or
/// one subscribed to by its link.
#[test]
fn the_accounts_own_calendar_is_the_default() {
    let list = vec![
        cal("https://d.example/web/", true, false),
        cal("https://d.example/u/sam_team/", false, true),
        cal("https://d.example/u/work/", false, false),
        cal("https://d.example/u/personal/", false, false),
        cal(LOCAL, false, false),
    ];
    let pick = |chosen: Option<&str>| pick_calendar(list.iter(), chosen, None).map(|c| c.id.clone());
    assert_eq!(pick(None).as_deref(), Some("https://d.example/u/personal/"));
    assert_eq!(pick(Some("https://d.example/u/sam_team/")).as_deref(), Some("https://d.example/u/sam_team/"), "chosen");
    assert_eq!(pick(Some("https://d.example/web/")).as_deref(), Some("https://d.example/u/personal/"), "read-only: not taken");
    assert_eq!(pick(Some("gone")).as_deref(), Some("https://d.example/u/personal/"));
    let theirs = [cal("https://d.example/u/sam_team/", false, true), cal(&web_calendar_id("https://x/a.ics"), false, false)];
    assert_eq!(pick_calendar(theirs.iter(), None, None).map(|c| c.id.as_str()), Some("https://d.example/u/sam_team/"), "only someone else's");
    assert!(pick_calendar([cal("https://d.example/ro/", true, false)].iter(), None, None).is_none());
    let p = Pim { calendars: list.clone(), ..Default::default() };
    assert_eq!(p.server_calendar().map(|c| c.id.as_str()), Some("https://d.example/u/personal/"));
    let only_here = Pim { calendars: vec![cal(LOCAL, false, false), cal(&shared_calendar_id("a@b.example"), false, false)], ..Default::default() };
    assert!(only_here.server_calendar().is_none());
    assert_eq!(only_here.default_calendar().map(|c| c.id.as_str()), Some(LOCAL));
}

/// Issue #278: Stalwart lists a second calendar before the account's `default`; the
/// default is still `default`, for new events and for events moved to the server.
#[test]
fn a_calendar_named_default_beats_the_listing_order() {
    let list = vec![
        cal("https://d.example/dav/cal/zoe/social-posts/", false, false),
        cal("https://d.example/dav/cal/zoe/Default/", false, false),
        cal(LOCAL, false, false),
    ];
    let p = Pim { calendars: list, ..Default::default() };
    assert_eq!(p.default_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/Default/"));
    assert_eq!(p.server_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/Default/"));
}

/// Issue #278: the calendar the server names its default comes before any name, the one the
/// user chose before it; one the server names that isn't the user's own, or can't be added
/// to, is passed over.
#[test]
fn the_calendar_the_server_names_its_default_comes_first() {
    let list = vec![
        cal("https://d.example/dav/cal/zoe/default/", false, false),
        cal("https://d.example/dav/cal/zoe/posts/", false, false),
        cal("https://d.example/dav/cal/zoe/archive/", true, false),
        cal("https://d.example/dav/cal/zoe/sam/", false, true),
    ];
    let mut p = Pim { calendars: list, server_default: Some("https://d.example/dav/cal/zoe/posts".into()), ..Default::default() };
    assert_eq!(p.default_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/posts/"), "a trailing slash aside");
    assert_eq!(p.server_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/posts/"));
    p.chosen_calendar = Some("https://d.example/dav/cal/zoe/default/".into());
    assert_eq!(p.default_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/default/"), "the user's choice first");
    p.chosen_calendar = None;
    for passed_over in ["https://d.example/dav/cal/zoe/archive/", "https://d.example/dav/cal/zoe/sam/", ""] {
        p.server_default = Some(passed_over.into());
        assert_eq!(p.default_calendar().map(|c| c.id.as_str()), Some("https://d.example/dav/cal/zoe/default/"), "{passed_over}");
    }
    p.leave_server();
    assert_eq!(p.server_default, None, "the old server's default forgotten");
}
