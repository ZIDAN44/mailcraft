//! Issue #192: an event dragged in the week, its all-day strip and the month moves, all of it,
//! where it was let go; Esc while dragging leaves it; one that repeats isn't dragged.

use serde_json::Value;

use super::*;

fn texts(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<Drawn> {
    drawn_with(ctx, a, windows::layout::SIZE.with(std::cell::Cell::get), events)
}

fn at(drawn: &[Drawn], what: &str) -> egui::Rect {
    drawn.iter().find(|d| d.text.starts_with(what) && d.seen()).map(|d| d.rect).unwrap_or_else(|| {
        panic!("{what} drawn: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>());
    })
}

/// A drag with the primary button from `from` to `to`, a frame at each step; `esc`: Esc pressed
/// on the way.
fn drag(ctx: &egui::Context, a: &mut MailApp, from: egui::Pos2, to: egui::Pos2, esc: bool) {
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    texts(ctx, a, vec![Event::PointerMoved(from)]);
    texts(ctx, a, vec![press(from, true)]);
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        let mut events = vec![Event::PointerMoved(p)];
        if esc && k == 2 {
            events.push(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
        texts(ctx, a, events);
    }
    texts(ctx, a, vec![press(to, false)]);
    texts(ctx, a, vec![]);
}

/// The calendar in `mode`, with events made on the second day shown: `made` gives each's
/// parameters but the day. Their uids, and what's drawn.
fn calendar_with(name: &str, mode: &str, made: &[Value]) -> (MailApp, egui::Context, Vec<String>, i64) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    a.run("view.calendar", json!({}));
    a.run("calendar.view", json!({"mode": mode}));
    let day = a.session.cal_days().0 + if mode == "month" { 9 } else { 1 };
    let date = mailcraft_engine::model::dates::day_string(day);
    let mut uids = Vec::new();
    for p in made {
        let mut p = p.clone();
        for key in ["start", "end"] {
            if let Some(t) = p.get(key).and_then(Value::as_str).map(|t| format!("{date}{t}")) {
                p[key] = json!(t);
            }
        }
        let r = a.session.run("event.create", &p).unwrap();
        uids.push(r["uid"].as_str().unwrap().to_string());
    }
    for _ in 0..3 {
        texts(&ctx, &mut a, vec![]);
    }
    (a, ctx, uids, day)
}

/// How far apart the days' columns are: from one day's number in the headings (above `above`,
/// right of the hours' "All day", not the side pane's month) to the next.
fn column_step(drawn: &[Drawn], above: f32) -> f32 {
    let left = at(drawn, "All day").max.x;
    let mut xs: Vec<f32> = drawn
        .iter()
        .filter(|d| d.seen() && d.rect.max.y < above && d.rect.min.x > left && d.text.parse::<u32>().is_ok())
        .map(|d| d.rect.center().x)
        .collect();
    xs.sort_by(f32::total_cmp);
    xs.windows(2).map(|w| w[1] - w[0]).find(|d| *d > 20.0).expect("the days' columns")
}

fn when(a: &MailApp, uid: &str) -> (i64, i64) {
    let e = &a.session.find_event(uid).unwrap().1.event;
    (e.start, e.end)
}

#[test]
fn an_event_dragged_in_the_week_moves_to_that_day_and_time() {
    let (mut a, ctx, uids, _) = calendar_with("move-week", "week", &[json!({"summary": "Colour grade", "start": "T10:00", "end": "T11:00"})]);
    let (start, end) = when(&a, &uids[0]);
    let drawn = texts(&ctx, &mut a, vec![]);
    let r = at(&drawn, "Colour grade");
    // An hour and a quarter down (46 points an hour): 11:15.
    let next_day = a.session.local_midnight(a.session.cal_days().0 + 2) - a.session.local_midnight(a.session.cal_days().0 + 1);
    let from = r.center();
    drag(&ctx, &mut a, from, from + egui::vec2(0.0, 46.0 * 1.25), false);
    assert_eq!(when(&a, &uids[0]), (start + 4500, end + 4500), "an hour and a quarter later");
    assert!(a.ui.dialog.is_none(), "dragged, not opened");
    // To the next day: as far across as a day's column is (from one day's number to the next).
    let drawn = texts(&ctx, &mut a, vec![]);
    let r = at(&drawn, "Colour grade");
    let step = column_step(&drawn, r.min.y);
    drag(&ctx, &mut a, r.center(), r.center() + egui::vec2(step, 0.0), false);
    assert_eq!(when(&a, &uids[0]), (start + 4500 + next_day, end + 4500 + next_day), "a day later");

    // Esc on the way: where it was.
    let drawn = texts(&ctx, &mut a, vec![]);
    let r = at(&drawn, "Colour grade");
    drag(&ctx, &mut a, r.center(), r.center() + egui::vec2(0.0, 92.0), true);
    assert_eq!(when(&a, &uids[0]), (start + 4500 + next_day, end + 4500 + next_day), "called off");
    // A click still opens it.
    let drawn = texts(&ctx, &mut a, vec![]);
    click(&ctx, &mut a, at(&drawn, "Colour grade").center());
    assert!(matches!(a.ui.dialog, Some(Dialog::Event(_))), "opened");
}

#[test]
fn an_all_day_event_and_one_in_the_month_move_by_days() {
    let (mut a, ctx, uids, _) = calendar_with(
        "move-month",
        "month",
        &[json!({"summary": "Festival", "start": "", "allDay": true}), json!({"summary": "Lunch", "start": "T12:00", "end": "T13:00"})],
    );
    let festival = when(&a, &uids[0]);
    let lunch = when(&a, &uids[1]);
    let drawn = texts(&ctx, &mut a, vec![]);
    // A week down in the month: the same weekday a week on.
    let f = at(&drawn, "Festival");
    let rows: Vec<f32> = {
        let mut ys: Vec<f32> = drawn.iter().filter(|d| d.seen() && d.text.parse::<u32>().is_ok()).map(|d| d.rect.center().y).collect();
        ys.sort_by(f32::total_cmp);
        ys.dedup_by(|x, y| (*x - *y).abs() < 4.0);
        ys
    };
    let week = rows.windows(2).map(|w| w[1] - w[0]).find(|d| *d > 30.0).expect("the month's rows");
    drag(&ctx, &mut a, f.center(), f.center() + egui::vec2(0.0, week), false);
    assert_eq!(when(&a, &uids[0]), (festival.0 + 7 * 86_400, festival.1 + 7 * 86_400), "a week on, all day");
    assert!(a.session.find_event(&uids[0]).unwrap().1.event.all_day);
    // Festival gone from the day, Lunch is where it was.
    let drawn = texts(&ctx, &mut a, vec![]);
    let l = at(&drawn, "12:00 Lunch");
    drag(&ctx, &mut a, l.center(), l.center() + egui::vec2(0.0, week), false);
    let back = a.session.local_midnight(a.session.clock.local(lunch.0).div_euclid(86_400) + 7)
        - a.session.local_midnight(a.session.clock.local(lunch.0).div_euclid(86_400));
    assert_eq!(when(&a, &uids[1]), (lunch.0 + back, lunch.1 + back), "a week on, at noon");

    // In the week, the all-day strip: a day earlier.
    a.run("calendar.goto", json!({"date": mailcraft_engine::model::dates::day_string(a.session.clock.utc(when(&a, &uids[0]).0).div_euclid(86_400))}));
    a.run("calendar.view", json!({"mode": "week"}));
    for _ in 0..3 {
        texts(&ctx, &mut a, vec![]);
    }
    let drawn = texts(&ctx, &mut a, vec![]);
    let f = at(&drawn, "Festival");
    let step = column_step(&drawn, f.min.y);
    let before = when(&a, &uids[0]);
    drag(&ctx, &mut a, f.center(), f.center() - egui::vec2(step, 0.0), false);
    assert_eq!(when(&a, &uids[0]), (before.0 - 86_400, before.1 - 86_400), "a day earlier");
}

#[test]
fn a_repeating_event_isnt_dragged() {
    let (mut a, ctx, uids, _) =
        calendar_with("move-repeating", "week", &[json!({"summary": "Stand-up", "start": "T09:00", "end": "T09:30", "repeat": "daily"})]);
    let before = when(&a, &uids[0]);
    let drawn = texts(&ctx, &mut a, vec![]);
    let r = at(&drawn, "Stand-up");
    drag(&ctx, &mut a, r.center(), r.center() + egui::vec2(0.0, 92.0), false);
    assert_eq!(when(&a, &uids[0]), before, "it stays");
}

/// With calendars side by side, an event dropped on a day in another calendar's pane goes to
/// that day, still in its own calendar (it went to its own pane's last day).
#[test]
fn an_event_dropped_in_another_calendars_pane_goes_to_that_day() {
    let (mut a, ctx, uids, day) = calendar_with("move-side", "week", &[json!({"summary": "Colour grade", "start": "T10:00", "end": "T11:00"})]);
    a.run("calendar.sideBySide", json!({"on": true}));
    for _ in 0..3 {
        texts(&ctx, &mut a, vec![]);
    }
    let drawn = texts(&ctx, &mut a, vec![]);
    let r = at(&drawn, "Colour grade");
    let calendar = a.session.find_event(&uids[0]).unwrap().1.calendar.clone();
    // Two days on, in the pane farthest from its own.
    let (_, _, dd) = mailcraft_engine::model::dates::civil_from_days(day + 2);
    let left = at(&drawn, "All day").max.x;
    let target = drawn
        .iter()
        .filter(|d| d.seen() && d.text == dd.to_string() && d.rect.max.y < r.min.y && d.rect.min.x > left)
        .max_by(|x, y| (x.rect.center().x - r.center().x).abs().total_cmp(&(y.rect.center().x - r.center().x).abs()))
        .map(|d| d.rect.center().x)
        .expect("the day in the other pane");
    assert!((target - r.center().x).abs() > 100.0, "in another pane");
    let (start, end) = when(&a, &uids[0]);
    let two = a.session.local_midnight(day + 2) - a.session.local_midnight(day);
    drag(&ctx, &mut a, r.center(), egui::pos2(target, r.center().y), false);
    assert_eq!(when(&a, &uids[0]), (start + two, end + two), "two days on");
    assert_eq!(a.session.find_event(&uids[0]).unwrap().1.calendar, calendar, "in its own calendar");
}

/// Issue #212: dragged over the hours of a day, the half hours from where it was pressed to where
/// it was let go make a new event that long (a click made half an hour's); upward too.
#[test]
fn hours_dragged_over_make_an_event_that_long() {
    let (mut a, ctx, _, _) = calendar_with("drag-new", "day", &[]);
    let drawn = texts(&ctx, &mut a, vec![]);
    let (three, four) = (at(&drawn, "15:00"), at(&drawn, "16:00"));
    let hour = four.center().y - three.center().y;
    assert!(hour > 20.0, "{hour}");
    let x = three.max.x + 120.0;
    let form = |a: &MailApp| match &a.ui.dialog {
        Some(crate::Dialog::Event(f)) => Some((f.start.clone(), f.end.clone())),
        _ => None,
    };
    drag(&ctx, &mut a, egui::pos2(x, three.center().y + hour * 0.25), egui::pos2(x, four.center().y + hour * 0.75), false);
    assert_eq!(form(&a), Some(("15:00".into(), "17:00".into())), "from 15:00 to the end of the half hour let go in");
    a.ui.dialog = None;
    drag(&ctx, &mut a, egui::pos2(x, four.center().y + hour * 0.25), egui::pos2(x, three.center().y + hour * 0.75), false);
    assert_eq!(form(&a), Some(("15:30".into(), "16:30".into())), "dragged upward");
}
