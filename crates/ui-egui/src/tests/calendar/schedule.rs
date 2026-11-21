//! Issue #141: the Schedule view, a day's hours across with a row for each calendar shown, and
//! a time chosen there making a meeting with the people whose row it is.

use mailcraft_engine::model::Event as NetEvent;
use mailcraft_engine::model::ical::ShowAs;
use mailcraft_engine::model::pim::free_busy::{Busy, FreeBusyAnswer};
use mailcraft_engine::model::pim::{CalendarInfo, shared_calendar_id};

use super::*;

const ANA: &str = "ana@contoso.example";

fn texts(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<Drawn> {
    drawn_with(ctx, a, windows::layout::SIZE.with(std::cell::Cell::get), events)
}

fn at(drawn: &[Drawn], what: &str) -> egui::Rect {
    drawn.iter().find(|d| d.text == what && d.seen()).map(|d| d.rect).unwrap_or_else(|| {
        panic!("{what} drawn: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>());
    })
}

/// Where a row's name is drawn: in the Schedule, right of the side pane (which names the
/// calendars too).
fn row(drawn: &[Drawn], name: &str) -> egui::Rect {
    let found = drawn.iter().filter(|d| d.text == name && d.seen()).max_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x)).map(|d| d.rect);
    found.unwrap_or_else(|| panic!("{name}'s row"))
}

/// The Schedule view on the sample's day, with Ana's calendar (shared for its times alone)
/// beside the sample's own, her morning taken.
fn with_ana(name: &str) -> (MailApp, egui::Context, Vec<Drawn>) {
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
    a.run("calendar.view", json!({"mode": "schedule"}));
    let day = a.session.local_midnight(a.session.cal_days().0);
    let busy =
        vec![Busy { start: day + 8 * 3600, end: day + 9 * 3600, show_as: ShowAs::Busy, subject: "Budget review".into(), ..Default::default() }];
    let answers = vec![FreeBusyAnswer { email: ANA.into(), busy, error: None, work: None }];
    a.session.apply(NetEvent::FreeBusy { account, from: day - 86_400, to: day + 2 * 86_400, answers });
    for _ in 0..2 {
        texts(&ctx, &mut a, vec![]);
    }
    let drawn = texts(&ctx, &mut a, vec![]);
    (a, ctx, drawn)
}

/// A drag with the primary button from `from` to `to`, a frame at each step.
fn drag(ctx: &egui::Context, a: &mut MailApp, from: egui::Pos2, to: egui::Pos2) {
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    texts(ctx, a, vec![Event::PointerMoved(from)]);
    texts(ctx, a, vec![press(from, true)]);
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        texts(ctx, a, vec![Event::PointerMoved(p)]);
    }
    texts(ctx, a, vec![press(to, false)]);
    texts(ctx, a, vec![]);
}

#[test]
fn each_calendar_shown_has_a_row_on_one_days_hours() {
    let (a, _, drawn) = with_ana("schedule-rows");
    let strings: Vec<&str> = drawn.iter().map(|d| d.text.as_str()).collect();
    // A row for all together, then each calendar's, Ana's among them, on the hours.
    for row in ["All together", ANA, "Budget review", "09:00", "10:00"] {
        at(&drawn, row);
    }
    assert!(strings.contains(&"Schedule"), "the view's button: {strings:?}");
    let (d0, d1) = a.session.cal_days();
    assert_eq!(d1 - d0, 1, "one day");
    // The rows one under another, all together on top.
    assert!(at(&drawn, "All together").center().y < row(&drawn, ANA).center().y);
    // Her time taken on her row, at its hour.
    let (review, nine) = (at(&drawn, "Budget review"), at(&drawn, "09:00"));
    assert!((review.center().y - row(&drawn, ANA).center().y).abs() < 20.0, "{review:?} {:?}", row(&drawn, ANA));
    assert!(review.max.x <= nine.min.x + 2.0, "08:00 – 09:00: {review:?} {nine:?}");
}

/// Dragged across free time on Ana's row: a meeting then, with her invited, in the account's
/// own calendar. On the row of all together: with everyone shown (Ana, the user's own calendars
/// inviting no one).
#[test]
fn a_time_dragged_across_makes_a_meeting_with_those_people() {
    let (mut a, ctx, drawn) = with_ana("schedule-meeting");
    // The hours' names stand just right of their line.
    let x = |h: &str| at(&drawn, h).min.x - 4.0;
    let y = row(&drawn, ANA).center().y;
    drag(&ctx, &mut a, egui::pos2(x("10:00") + 5.0, y), egui::pos2(x("11:00") - 5.0, y));
    let Some(Dialog::Event(f)) = &a.ui.dialog else { panic!("a meeting: {:?}", a.ui.dialog.as_ref().map(std::mem::discriminant)) };
    assert_eq!((f.start.as_str(), f.end.as_str()), ("10:00", "11:00"));
    assert_eq!(f.guests, ANA);
    let (account, calendar, _) = &f.calendars[f.calendar];
    assert_eq!(account, &a.session.accounts[0].config.id);
    assert_ne!(calendar, &shared_calendar_id(ANA), "her calendar is hers");

    a.ui.dialog = None;
    texts(&ctx, &mut a, vec![]);
    let drawn = texts(&ctx, &mut a, vec![]);
    let together = at(&drawn, "All together").center().y;
    click(&ctx, &mut a, egui::pos2(x("13:00") + 40.0, together));
    let Some(Dialog::Event(f)) = &a.ui.dialog else { panic!("a meeting") };
    assert_eq!((f.start.as_str(), f.end.as_str(), f.guests.as_str()), ("13:30", "14:00", ANA), "a click: its half hour");
}

/// The Schedule view moves a day at a time.
#[test]
fn the_schedule_moves_a_day_at_a_time() {
    let (mut a, _, _) = with_ana("schedule-days");
    let (d0, _) = a.session.cal_days();
    a.run("calendar.goto", json!({"step": 1}));
    assert_eq!(a.session.cal_days(), (d0 + 1, d0 + 2));
}

/// Issue #185: each row shades the hours its owner doesn't work: the user's own, 08:00 to 17:00;
/// Ana's as her server gave them (09:00 to 17:00 an hour east, 08:00 to 16:00 here); all
/// together's where not everyone works.
#[test]
fn each_row_shades_the_hours_not_worked() {
    use mailcraft_engine::model::pim::work_hours::{WorkHours, WorkPeriod, WorkZone};
    let (mut a, ctx, _) = with_ana("schedule-work-hours");
    // A Wednesday.
    a.run("calendar.goto", json!({"date": "2026-11-04"}));
    let account = a.session.accounts[0].config.id.clone();
    let day = a.session.local_midnight(a.session.cal_days().0);
    let work = WorkHours { periods: vec![WorkPeriod { start: 9 * 60, ..Default::default() }], zone: WorkZone { bias: -60, ..Default::default() } };
    let answers = vec![FreeBusyAnswer { email: ANA.into(), busy: Vec::new(), error: None, work: Some(work) }];
    a.session.apply(NetEvent::FreeBusy { account, from: day - 86_400, to: day + 2 * 86_400, answers });
    texts(&ctx, &mut a, vec![]);
    let out = sized_frame(&ctx, &mut a, windows::layout::SIZE.with(std::cell::Cell::get), vec![]);
    let drawn = texts_in(&out.shapes);
    // The hours' places, from their names (just right of their line).
    let name = |h: &str| drawn.iter().find(|d| d.text == h).map(|d| d.rect.min.x - 4.0).unwrap();
    let w = name("10:00") - name("09:00");
    let hour = |x: f32| ((x - name("09:00")) / w + 9.0).round() as i64;
    let shade = if a.ui.tokens.dark { egui::Color32::from_black_alpha(70) } else { egui::Color32::from_black_alpha(13) };
    let shaded = |y: f32| {
        let mut hours: Vec<(i64, i64)> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == shade && r.rect.min.y <= y && r.rect.max.y >= y => Some((hour(r.rect.min.x), hour(r.rect.max.x))),
                _ => None,
            })
            .collect();
        hours.sort_unstable();
        hours
    };
    let own = a.session.pim[&a.session.accounts[0].config.id].calendars.iter().find(|c| !c.free_busy).map(|c| c.name.clone()).unwrap();
    let own = if own.is_empty() || own == mailcraft_engine::model::pim::LOCAL { "Calendar".to_string() } else { own };
    assert_eq!(shaded(row(&drawn, &own).center().y), [(0, 8), (17, 24)], "the user's own");
    assert_eq!(shaded(row(&drawn, ANA).center().y), [(0, 8), (16, 24)], "Ana's");
    assert_eq!(shaded(at(&drawn, "All together").center().y), [(0, 8), (16, 24)], "when not everyone works");
}

/// Issue #186: Next free time finds when everyone shown is free and marks it; clicked, it's a
/// meeting then with them.
#[test]
fn the_next_free_time_is_marked_and_makes_the_meeting() {
    let (mut a, ctx, _) = with_ana("schedule-free-time");
    ctx.enable_accesskit();
    texts(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Next free time");
    let (from, to) = a.session.free_slot().unwrap_or_else(|| panic!("found: {:?}", a.session.notices));
    assert_eq!(to - from, 1800, "half an hour, as it starts");
    assert!(from >= a.session.clock.now);
    // In the day view, tall enough for every hour: marked, and clicked, the meeting.
    a.run("calendar.view", json!({"mode": "day"}));
    let size = egui::vec2(1200.0, 1600.0);
    for _ in 0..3 {
        sized_frame(&ctx, &mut a, size, vec![]);
    }
    let all = named(&ctx, &mut a, size);
    let at = all.iter().filter(|n| n.label.as_deref() == Some("Free for everyone")).find_map(Named::centre);
    let at = at.unwrap_or_else(|| panic!("marked: {:?}", all.iter().filter_map(Named::name).collect::<Vec<_>>()));
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
        sized_frame(&ctx, &mut a, size, events);
    }
    let Some(Dialog::Event(f)) = &a.ui.dialog else { panic!("a meeting: {:?}", a.ui.dialog.as_ref().map(std::mem::discriminant)) };
    assert_eq!(
        (f.start.as_str(), f.end.as_str(), f.guests.as_str()),
        (crate::calendar::hm(from, &a.session).as_str(), crate::calendar::hm(to, &a.session).as_str(), ANA)
    );
    // Moved by hand, it's gone.
    a.ui.dialog = None;
    a.run("calendar.goto", json!({"step": 1}));
    assert_eq!(a.session.free_slot(), None);
}

/// Issue #206: Only work hours narrows the time line to the hours anyone shown works (here the
/// default, eight to five), whole hours; off, it's the whole day as before. Their meetings are
/// where they were.
#[test]
fn only_work_hours_narrows_the_day() {
    let (mut a, ctx, drawn) = with_ana("schedule-only-work-hours");
    assert!(drawn.iter().any(|d| d.text == "06:00"), "the whole day, at first");
    let switch = at(&drawn, "Only work hours");
    click(&ctx, &mut a, switch.center());
    assert!(a.session.prefs.schedule_work_hours, "switched on");
    texts(&ctx, &mut a, vec![]);
    let drawn = texts(&ctx, &mut a, vec![]);
    assert!(!drawn.iter().any(|d| d.text == "06:00" || d.text == "07:00"), "before work: not drawn");
    let hours: Vec<&Drawn> = drawn.iter().filter(|d| d.seen() && d.text.len() == 5 && d.text.as_bytes().get(2) == Some(&b':')).collect();
    let first = hours.iter().min_by(|x, y| x.rect.min.x.total_cmp(&y.rect.min.x)).map(|d| d.text.as_str());
    assert_eq!(first, Some("08:00"), "{:?}", hours.iter().map(|d| &d.text).collect::<Vec<_>>());
    let (review, nine) = (at(&drawn, "Budget review"), at(&drawn, "09:00"));
    assert!(review.max.x <= nine.min.x + 2.0, "her 08:00 – 09:00 still there: {review:?} {nine:?}");
}
