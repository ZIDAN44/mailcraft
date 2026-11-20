//! Issue #155: the day view's events side by side where they overlap, each sharing the column
//! only with the events it overlaps (directly or through others). Issue #185: the work week's
//! days and the hours not worked, as Options' work time sets them.

use super::*;

/// Where an event's box is: the room its text is cut to.
fn box_of(drawn: &[Drawn], title: &str) -> egui::Rect {
    let found = drawn.iter().find(|d| d.text.starts_with(&format!("{title}\n")) && d.seen()).map(|d| d.clip);
    found.unwrap_or_else(|| panic!("{title} drawn: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>()))
}

#[test]
fn only_events_that_overlap_share_the_column() {
    let (mut a, _) = app("day-lanes");
    let ctx = egui::Context::default();
    // A day with nothing else on it (the sample's meetings repeat).
    a.run("view.calendar", json!({}));
    a.run("calendar.view", json!({"mode": "day"}));
    a.run("calendar.goto", json!({"date": "2027-03-03"}));
    while !a.session.agenda(a.session.local_midnight(a.session.cal_days().0), a.session.local_midnight(a.session.cal_days().1)).is_empty() {
        a.run("calendar.goto", json!({"step": 1}));
    }
    let day = a.session.local_midnight(a.session.cal_days().0);
    let at = |h: i64, m: i64| day + h * 3600 + m * 60;
    for (summary, start, end) in [
        ("Alone in the morning", at(9, 0), at(10, 0)),
        ("First at two", at(14, 0), at(15, 0)),
        ("Long at two", at(14, 0), at(16, 0)),
        ("Half past two", at(14, 30), at(15, 30)),
        ("Alone at five", at(17, 0), at(18, 0)),
    ] {
        a.act(crate::Action::Submit("event.create", json!({"summary": summary, "start": start, "end": end})));
    }
    // A tall window: every hour of the day on screen, nothing scrolled.
    let size = egui::vec2(1200.0, 1600.0);
    for _ in 0..3 {
        drawn_with(&ctx, &mut a, size, vec![]);
    }
    let drawn = drawn_with(&ctx, &mut a, size, vec![]);
    let (morning, five) = (box_of(&drawn, "Alone in the morning"), box_of(&drawn, "Alone at five"));
    let (first, long, half) = (box_of(&drawn, "First at two"), box_of(&drawn, "Long at two"), box_of(&drawn, "Half past two"));
    // Alone: the whole column, as wide as the three overlapping side by side.
    let whole =
        [first, long, half].iter().map(|r| r.max.x).fold(0.0, f32::max) - [first, long, half].iter().map(|r| r.min.x).fold(f32::MAX, f32::min);
    assert!((morning.width() - five.width()).abs() < 1.0 && morning.width() > whole - 8.0, "{morning:?} {five:?} {first:?} {half:?}");
    // The three: a third each, side by side.
    for r in [first, long, half] {
        assert!((r.width() * 3.0 - morning.width()).abs() < 12.0, "{r:?} a third of {morning:?}");
    }
    let mut three = [first, long, half];
    three.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    assert!(three[0].max.x <= three[1].min.x + 1.0 && three[1].max.x <= three[2].min.x + 1.0, "side by side: {three:?}");
}

/// Issue #185: the work week shows the days worked (Sunday to Thursday here), each shading the
/// hours outside the work hours (09:30 to 18:00).
#[test]
fn the_work_week_shows_the_days_worked_and_shades_the_hours_off() {
    let (mut a, _) = app("work-week-days");
    let ctx = egui::Context::default();
    a.run("view.calendar", json!({}));
    a.run("calendar.view", json!({"mode": "workWeek"}));
    a.run("calendar.workTime", json!({"days": [6, 0, 1, 2, 3], "start": "09:30", "end": "18:00"}));
    let size = egui::vec2(1200.0, 1600.0);
    drawn_with(&ctx, &mut a, size, vec![]);
    let out = sized_frame(&ctx, &mut a, size, vec![]);
    let strings: Vec<String> = texts_in(&out.shapes).into_iter().map(|d| d.text).collect();
    for day in ["Sun", "Mon", "Thu"] {
        assert!(strings.iter().any(|s| s == day), "{day}: {strings:?}");
    }
    for day in ["Fri", "Sat"] {
        assert!(!strings.iter().any(|s| s == day), "{day} isn't worked: {strings:?}");
    }
    // Each day's column: 9½ hours shaded before, 6 after.
    let shade = if a.ui.tokens.dark { egui::Color32::from_black_alpha(70) } else { egui::Color32::from_black_alpha(13) };
    let mut columns: std::collections::BTreeMap<i64, Vec<f32>> = std::collections::BTreeMap::new();
    for s in &out.shapes {
        if let egui::Shape::Rect(r) = &s.shape
            && r.fill == shade
        {
            columns.entry(r.rect.min.x.round() as i64).or_default().push(r.rect.height());
        }
    }
    let days: Vec<&Vec<f32>> = columns.values().filter(|h| h.len() == 2 && (h[0] / h[1] - 9.5 / 6.0).abs() < 0.01).collect();
    assert_eq!(days.len(), 5, "{columns:?}");
}

/// Issue #185: File › Options › Calendar sets the work time: a day clicked is worked, or not.
#[test]
fn options_sets_the_work_days() {
    let (mut a, _) = app("work-time-options");
    let ctx = egui::Context::default();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tall = egui::vec2(1280.0, 3000.0);
    drawn_text(&ctx, &mut a, tall);
    let drawn = drawn_text(&ctx, &mut a, tall);
    for what in ["Work days", "Work hours", "08:00", "17:00"] {
        assert!(drawn.iter().any(|d| d.text == what), "{what}: {:?}", drawn.iter().map(|d| d.text.as_str()).collect::<Vec<_>>());
    }
    // Saturday, once the page has settled.
    for _ in 0..3 {
        drawn_text(&ctx, &mut a, tall);
    }
    ctx.enable_accesskit();
    let all = named(&ctx, &mut a, tall);
    let sat = all.iter().filter(|n| n.label.as_deref() == Some("Sat")).find_map(Named::centre).unwrap();
    let press = |pressed| Event::PointerButton { pos: sat, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(sat)], vec![press(true)], vec![press(false)], vec![]] {
        sized_frame(&ctx, &mut a, tall, events);
    }
    assert!(a.session.work_time().works_on(5), "Saturday worked: {:?}", a.session.work_time());
    assert!(a.session.work_time().works_on(0), "Monday still");
}
