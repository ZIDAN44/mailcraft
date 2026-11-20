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
