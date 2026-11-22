//! Issue #140: a colleague's calendar shared for its free/busy time alone, in the Calendar: its
//! times drawn among the events (opening nothing), and its state beside its name; colleagues'
//! calendars picked from the address book, and calendar groups.

use mailcraft_engine::model::Event as NetEvent;
use mailcraft_engine::model::ical::ShowAs;
use mailcraft_engine::model::pim::free_busy::{Busy, FreeBusyAnswer};
use mailcraft_engine::model::pim::{CalendarInfo, shared_calendar_id};

use super::*;

const ANA: &str = "ana@contoso.example";

fn texts(ctx: &egui::Context, a: &mut MailApp) -> Vec<Drawn> {
    drawn_with(ctx, a, windows::layout::SIZE.with(std::cell::Cell::get), vec![])
}

/// The Calendar on its week, with Ana's calendar shared for its times alone.
fn with_ana(name: &str) -> (MailApp, egui::Context, String) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    let account = a.session.accounts[0].config.id.clone();
    a.session.pim_mut(&account).calendars.push(CalendarInfo {
        id: shared_calendar_id(ANA),
        name: ANA.into(),
        read_only: true,
        free_busy: true,
        ..Default::default()
    });
    a.run("view.calendar", json!({}));
    a.run("calendar.view", json!({"mode": "week"}));
    for _ in 0..2 {
        texts(&ctx, &mut a);
    }
    (a, ctx, account)
}

/// Ana's answer for the week shown: a meeting on its Tuesday morning, or why there's none.
fn answer(a: &mut MailApp, account: &str, error: Option<&str>) {
    let (d0, d1) = a.session.cal_days();
    let (from, to) = (a.session.local_midnight(d0 - 7), a.session.local_midnight(d1 + 7));
    let tuesday = a.session.local_midnight(d0 + 1);
    let busy = vec![Busy {
        start: tuesday + 9 * 3600,
        end: tuesday + 11 * 3600,
        show_as: ShowAs::Busy,
        subject: "Budget review".into(),
        ..Default::default()
    }];
    let answers = vec![FreeBusyAnswer {
        email: ANA.into(),
        busy: if error.is_some() { Vec::new() } else { busy },
        error: error.map(str::to_string),
        work: None,
    }];
    a.session.apply(NetEvent::FreeBusy { account: account.into(), from, to, answers });
}

#[test]
fn a_colleagues_times_are_drawn_and_open_nothing() {
    let (mut a, ctx, account) = with_ana("free-busy-drawn");
    let drawn = texts(&ctx, &mut a);
    let strings: Vec<String> = drawn.iter().map(|d| d.text.clone()).collect();
    assert!(has(&strings, ANA) && has(&strings, "free/busy"), "{strings:?}");
    assert!(!strings.iter().any(|t| t.starts_with("Budget review")));
    let (d0, d1) = a.session.cal_days();
    let bold = crate::panes::calendar::busy_days(&a.session, d0 - 7, d1 + 7);

    answer(&mut a, &account, None);
    texts(&ctx, &mut a);
    let drawn = texts(&ctx, &mut a);
    let review = drawn.iter().find(|d| d.text.starts_with("Budget review\n") && d.seen()).map(|d| d.rect.center());
    let review = review.unwrap_or_else(|| panic!("drawn: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>()));
    // A click on it opens nothing (there's no event of hers here to open), nor starts a new one.
    click(&ctx, &mut a, review);
    texts(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "{:?}", a.ui.dialog.as_ref().map(std::mem::discriminant));
    // Her times make no day of the user's bold in the small months.
    assert_eq!(crate::panes::calendar::busy_days(&a.session, d0 - 7, d1 + 7), bold);
}

#[test]
fn times_that_cant_be_had_are_said_beside_the_calendar() {
    let (mut a, ctx, account) = with_ana("free-busy-failed");
    answer(&mut a, &account, Some("doesn't share their free/busy time"));
    texts(&ctx, &mut a);
    let strings: Vec<String> = texts(&ctx, &mut a).into_iter().map(|d| d.text).collect();
    assert!(has(&strings, "couldn't be read") && !has(&strings, "free/busy"), "{strings:?}");
    assert!(!strings.iter().any(|t| t.starts_with("Budget review")));
}

/// Calendar groups (issue #140): the group's people under its name in the side pane (not with
/// the account's own calendars too), shown and hidden together by its box; a new one is made
/// from the address book.
#[test]
fn a_calendar_group_lists_its_people_and_hides_them_together() {
    let (mut a, _) = app("calendar-group");
    let ctx = egui::Context::default();
    a.run("account.add", json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}));
    let id = a.session.accounts.iter().find(|x| x.config.ews.is_some()).map(|x| x.config.id.clone()).expect("the Exchange account");
    a.run("calendar.groupCreate", json!({"account": id, "name": "Design team", "members": "ana@contoso.example; ben@contoso.example"}));
    // Their calendars, as the server lists them once read.
    for who in ["ana@contoso.example", "ben@contoso.example"] {
        a.session.pim_mut(&id).calendars.push(CalendarInfo { id: shared_calendar_id(who), name: who.into(), read_only: true, ..Default::default() });
    }
    a.run("view.calendar", json!({}));
    for _ in 0..2 {
        texts(&ctx, &mut a);
    }
    let drawn = texts(&ctx, &mut a);
    let heading = drawn.iter().find(|d| d.text == "Design team" && d.seen()).map(|d| d.rect).expect("the group's name");
    for who in ["ana@contoso.example", "ben@contoso.example"] {
        let rows: Vec<egui::Rect> = drawn.iter().filter(|d| d.text == who && d.seen()).map(|d| d.rect).collect();
        assert_eq!(rows.len(), 1, "{who} listed once");
        assert!(rows[0].min.y > heading.max.y, "{who} under the group");
    }
    // Its box, before its name: both hidden at a click, shown at the next.
    click(&ctx, &mut a, egui::pos2(heading.min.x - 12.0, heading.center().y));
    for who in ["ana@contoso.example", "ben@contoso.example"] {
        assert!(!a.session.calendar_shown(&id, &shared_calendar_id(who)), "{who} hidden");
    }
    texts(&ctx, &mut a);
    click(&ctx, &mut a, egui::pos2(heading.min.x - 12.0, heading.center().y));
    assert!(a.session.calendar_shown(&id, &shared_calendar_id("ana@contoso.example")));

    // New calendar group… (at the pane's end): the address book, in which to name the group and
    // pick its people.
    let tall = egui::vec2(1280.0, 1800.0);
    drawn_with(&ctx, &mut a, tall, vec![]);
    assert!(drawn_with(&ctx, &mut a, tall, vec![]).iter().any(|d| d.text == "New calendar group…" && d.seen()));
    a.ui.dialog = Some(crate::Dialog::names_for(crate::dialogs::NamesFor::Group, ""));
    texts(&ctx, &mut a);
    let shown: Vec<String> = texts(&ctx, &mut a).into_iter().map(|d| d.text).collect();
    for what in ["New calendar group", "Group name", "Add ->"] {
        assert!(shown.iter().any(|s| s == what), "{what}: {shown:?}");
    }
    assert!(!shown.iter().any(|s| s == "Cc ->"), "a message's fields aren't there");
}

/// Open shared calendar: picked from the address book too (Outlook's From Address Book), what was
/// typed carried over (issue #140).
#[test]
fn a_shared_calendar_is_picked_from_the_address_book() {
    let (mut a, _) = app("calendar-from-book");
    let ctx = egui::Context::default();
    a.run("view.calendar", json!({}));
    a.ui.dialog = Some(crate::Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: None });
    for _ in 0..3 {
        texts(&ctx, &mut a);
    }
    let drawn = texts(&ctx, &mut a);
    let link = drawn.iter().find(|d| d.text == "Pick from the address book…" && d.seen()).map(|d| d.rect);
    let link = link.unwrap_or_else(|| panic!("the link: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>()));
    click(&ctx, &mut a, link.center());
    let Some(crate::Dialog::SelectNames(f)) = &a.ui.dialog else {
        panic!("the address book: {:?}", a.ui.dialog.as_ref().map(std::mem::discriminant))
    };
    assert_eq!((f.purpose, f.to.as_str()), (crate::dialogs::NamesFor::Calendars, "ana@contoso.example"));
}

/// Issues #209 and #210: a group's menu has Add people…, the address book for that group; the
/// people picked show as chips by name (rows that scroll past four, the address on hover), one's
/// × takes them out, and OK puts the rest in the group.
#[test]
fn people_are_added_to_a_group_shown_by_name() {
    let (mut a, _) = app("calendar-group-add");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("account.add", json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}));
    let id = a.session.accounts.iter().find(|x| x.config.ews.is_some()).map(|x| x.config.id.clone()).expect("the Exchange account");
    a.run("calendar.groupCreate", json!({"account": id, "name": "Design team", "members": "ana@contoso.example"}));
    a.run("view.calendar", json!({}));
    for _ in 0..2 {
        texts(&ctx, &mut a);
    }
    let heading = texts(&ctx, &mut a).into_iter().find(|d| d.text == "Design team" && d.seen()).map(|d| d.rect).expect("the group");
    click_at(&ctx, &mut a, heading.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    let item = texts(&ctx, &mut a).into_iter().rev().find(|d| d.text == "Add people…").map(|d| d.rect).expect("on its menu");
    click(&ctx, &mut a, item.center());
    assert!(matches!(&a.ui.dialog, Some(crate::Dialog::SelectNames(f)) if f.group == "Design team" && f.adding), "the address book, for it");
    let people = ["Cleo Diaz", "Dev Rao", "Eli Moss", "Fay Kim", "Gus Lund", "Hana Oki"];
    if let Some(crate::Dialog::SelectNames(f)) = &mut a.ui.dialog {
        f.to = people.iter().map(|n| format!("{n} <{}@contoso.example>; ", n.split(' ').next().unwrap_or("x").to_lowercase())).collect();
    }
    texts(&ctx, &mut a);
    let shown: Vec<String> = texts(&ctx, &mut a).into_iter().filter(|d| d.seen()).map(|d| d.text).collect();
    assert!(shown.iter().any(|s| s == "Add people to the group"), "{shown:?}");
    assert!(shown.iter().any(|s| s == "Cleo Diaz") && shown.iter().any(|s| s == "Dev Rao"), "by name: {shown:?}");
    assert!(!shown.iter().any(|s| s.contains("cleo@contoso.example")), "not their addresses: {shown:?}");
    // Cleo taken out by her ×.
    let x =
        named(&ctx, &mut a, egui::vec2(1280.0, 800.0)).into_iter().find(|n| n.label.as_deref() == Some("Remove: Cleo Diaz")).and_then(|n| n.centre());
    click(&ctx, &mut a, x.expect("her ×"));
    let Some(crate::Dialog::SelectNames(f)) = &a.ui.dialog else { panic!("still open") };
    assert!(!f.to.contains("cleo@") && f.to.contains("dev@contoso.example"), "{}", f.to);
    let ok = texts(&ctx, &mut a).into_iter().rev().find(|d| d.text == "OK" && d.seen()).map(|d| d.rect).expect("OK");
    click(&ctx, &mut a, ok.center());
    let members = a.session.pim[&id].groups.first().map(|g| g.members.clone()).unwrap_or_default();
    assert_eq!(
        members,
        ["ana@contoso.example", "dev@contoso.example", "eli@contoso.example", "fay@contoso.example", "gus@contoso.example", "hana@contoso.example"]
    );
}

/// Issue #242: Open shared calendar… and New calendar group… are there for an account whose
/// CalDAV server schedules (as for an Exchange one), not before that's known; a colleague opened
/// from it is listed with their times' state.
#[test]
fn colleagues_open_from_a_calendar_server_that_schedules() {
    let (mut a, _) = app("calendar-caldav-others");
    let ctx = egui::Context::default();
    a.run("account.add", json!({"email": "zoe@example.org", "username": "zoe", "password": "pw", "dav": "https://dav.example.org/"}));
    let id = a.session.accounts.iter().find(|x| x.config.is_calendar_only()).map(|x| x.config.id.clone()).expect("the calendar account");
    a.run("view.calendar", json!({}));
    let tall = egui::vec2(1280.0, 1800.0);
    let links = |a: &mut MailApp| -> Vec<String> {
        drawn_with(&ctx, a, tall, vec![]);
        let drawn = drawn_with(&ctx, a, tall, vec![]);
        drawn
            .into_iter()
            .filter(|d| d.seen() && d.text.ends_with('…') && (d.text.starts_with("Open shared") || d.text.starts_with("New calendar group")))
            .map(|d| d.text)
            .collect()
    };
    assert!(links(&mut a).is_empty(), "not known to schedule yet");
    let sync = mailcraft_engine::model::pim::DavSync {
        caldav_home: Some("https://dav.example.org/calendars/zoe/".into()),
        calendars: Some(vec![CalendarInfo {
            id: "https://dav.example.org/calendars/zoe/personal/".into(),
            name: "Personal".into(),
            ..Default::default()
        }]),
        auto_schedule: Some(true),
        checked: a.session.clock.now,
        ..Default::default()
    };
    a.session.apply(NetEvent::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert_eq!(links(&mut a), ["Open shared calendar…", "New calendar group…"]);
    a.run("calendar.openShared", json!({"email": ANA}));
    assert!(a.session.pim[&id].calendars.iter().any(|c| c.id == shared_calendar_id(ANA) && c.free_busy));
    let drawn = drawn_with(&ctx, &mut a, tall, vec![]);
    assert!(drawn.iter().any(|d| d.text == ANA && d.seen()), "listed: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>());
}
