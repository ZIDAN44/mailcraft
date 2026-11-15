//! The calendar's search with many results: what a frame of it costs, and what it draws.

use super::*;

/// The sample mailbox with `n` more events that "Measure" finds, a day apart from today, half
/// of them past: each with a place and a category, every third one repeating.
fn with_found_events(n: i64) -> (MailApp, std::path::PathBuf) {
    let (mut a, dir) = app(&format!("calendar-search-{n}"));
    let Some(p) = a.session.pim.get_mut("sample") else { panic!("the sample calendar") };
    let base = p.events.first().cloned().expect("a sample event");
    for k in 0..n {
        let mut e = base.clone();
        e.event.uid = format!("measure-{k}");
        e.event.summary = format!("Measure review {k}");
        e.event.location = format!("Studio {}", k % 7);
        e.event.categories = vec!["Blue category".into()];
        e.event.start = NOW + (k - n / 2) * 86_400 + 3600;
        e.event.end = e.event.start + 1800;
        e.event.all_day = false;
        e.event.rrule = (k % 3 == 0).then(|| "FREQ=YEARLY".to_string());
        e.event.recurrence_id = None;
        p.events.push(e);
    }
    a.run("view.module", json!({"name": "calendar"}));
    (a, dir)
}

/// The text drawn where it can be seen, and where.
fn seen(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2)> {
    drawn_text(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// The results scrolled down by `by` points with the mouse wheel over them, and settled.
fn scroll_by(ctx: &egui::Context, a: &mut MailApp, by: f32) {
    let over = egui::pos2(700.0, 500.0);
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -by / 10.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    };
    frame(ctx, a, vec![Event::PointerMoved(over)]);
    for _ in 0..10 {
        frame(ctx, a, vec![Event::PointerMoved(over), wheel.clone()]);
    }
    for _ in 0..40 {
        frame(ctx, a, vec![Event::PointerMoved(over)]);
    }
}

/// What a frame costs with the search's 500 results listed (the most it lists) in place of the
/// days, at the top and scrolled down (the days' own frame for scale); and a fingerprint of what
/// it draws (the same before and after a change that only makes it faster). Run in release under
/// the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_calendar_search -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_calendar_search() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = with_found_events(800);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            frame(&ctx, a, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    let days = timed(&mut a);
    println!("MEASURE the calendar's days, no search: {days:.2} ms a frame");
    a.ui.cal_search.query = "Measure".into();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.ui.cal_search.results(&a.session).len(), 500);
    for (place, scroll) in [("top", 0.0), ("scrolled", 12_000.0)] {
        if scroll > 0.0 {
            scroll_by(&ctx, &mut a, scroll);
        }
        let drawn = seen(&ctx, &mut a);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &drawn {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
        let each = timed(&mut a);
        let first = drawn.iter().find(|(t, _)| t.starts_with("Measure review")).map_or("", |(t, _)| t.as_str());
        println!(
            "MEASURE calendar search, 500 results, {place}: {each:.2} ms a frame; drawn {} texts (first {first:?}), {:016x}",
            drawn.len(),
            h.finish()
        );
    }
}

/// The search's results: the rows in view drawn (their dates, times, titles and what's under
/// them), scrolled down the ones there, and one clicked opens its event.
#[test]
fn many_results_are_listed_and_scrolled() {
    let (mut a, _) = with_found_events(40);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.cal_search.query = "Measure".into();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let hits = a.ui.cal_search.results(&a.session).to_vec();
    assert_eq!(hits.len(), 40);
    let drawn = seen(&ctx, &mut a);
    let texts: Vec<&str> = drawn.iter().map(|(t, _)| t.as_str()).collect();
    // Upcoming first, soonest first; each row its date written out and its time.
    let first = hits.iter().find(|h| h.upcoming).unwrap();
    let day = a.session.clock.local_day(first.start);
    for want in [
        "Upcoming".to_string(),
        first.summary.clone(),
        crate::calendar::long_date(day),
        format!("{} – {}", crate::calendar::hm(first.start, &a.session), crate::calendar::hm(first.end, &a.session)),
        format!("{} · Blue category · Calendar", first.location),
    ] {
        assert!(texts.contains(&want.as_str()), "{want:?} in {texts:?}");
    }
    let shown = |texts: &[&str]| texts.iter().filter(|t| t.starts_with("Measure review")).count();
    assert!((5..40).contains(&shown(&texts)), "only the rows in view: {texts:?}");
    // Scrolled to the end: the past ones, the last of them in view.
    scroll_by(&ctx, &mut a, 50_000.0);
    let drawn = seen(&ctx, &mut a);
    let last = hits.last().unwrap();
    assert!(!last.upcoming);
    let (_, at) = drawn.iter().find(|(t, _)| *t == last.summary).unwrap_or_else(|| panic!("{} in view: {drawn:?}", last.summary));
    // Clicked, it goes to its day and opens.
    click(&ctx, &mut a, *at);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.summary == last.summary), "{} opened", last.summary);
    assert_eq!(a.session.cal_day(), a.session.clock.local_day(last.start));
}
