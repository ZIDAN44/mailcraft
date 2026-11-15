//! UI tests: Select Names with a long address book (what it draws, scrolled, and a row clicked).

use super::*;

/// Select Names open over `n` people, `Person 0000` to `Person {n-1}`.
fn names_of(ctx: &egui::Context, a: &mut MailApp, n: usize) {
    a.session.prefs.new_mail_window = false;
    frame(ctx, a, vec![]);
    a.run("mail.new", json!({}));
    frame(ctx, a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, crate::dialogs::Recipients::To));
    frame(ctx, a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    f.list = (0..n).map(|i| mailcraft_engine::model::EmailAddress::new(format!("Person {i:04}"), format!("p{i:04}@x.example"))).collect();
    frame(ctx, a, vec![]);
}

/// Each text the frame draws that can be seen (not wholly outside its clip), where.
fn drawn(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(String, egui::Pos2)> {
    drawn_with(ctx, a, egui::vec2(1280.0, 800.0), events).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// The people drawn, top to bottom.
fn people(texts: &[(String, egui::Pos2)]) -> Vec<String> {
    texts.iter().filter(|(t, _)| t.starts_with("Person ")).map(|(t, _)| t.clone()).collect()
}

/// The list scrolled down by `by` points with the mouse wheel over it, and settled.
fn scroll(ctx: &egui::Context, a: &mut MailApp, by: f32) {
    let texts = drawn(ctx, a, vec![]);
    let over = texts.iter().find(|(t, _)| t.starts_with("Person ")).map(|(_, p)| *p + egui::vec2(20.0, 8.0)).expect("a person drawn");
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -by / 10.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    };
    let _ = drawn(ctx, a, vec![Event::PointerMoved(over)]);
    for _ in 0..10 {
        let _ = drawn(ctx, a, vec![Event::PointerMoved(over), wheel.clone()]);
    }
    for _ in 0..40 {
        let _ = drawn(ctx, a, vec![Event::PointerMoved(over)]);
    }
}

/// With 2,000 people the list shows the first in order; scrolled, later ones; a row clicked is
/// the person selected, and To adds them.
#[test]
fn a_long_address_book_scrolls_and_selects() {
    let (mut a, _) = app("names-long");
    let ctx = egui::Context::default();
    names_of(&ctx, &mut a, 2000);
    let texts = drawn(&ctx, &mut a, vec![]);
    let shown = people(&texts);
    assert!((6..12).contains(&shown.len()), "{shown:?}");
    assert_eq!(shown, (0..shown.len()).map(|i| format!("Person {i:04}")).collect::<Vec<_>>());
    let ys: Vec<f32> = texts.iter().filter(|(t, _)| t.starts_with("Person ")).map(|(_, p)| p.y).collect();
    let pitch = ys[1] - ys[0];
    // Scrolled 25,000 points: the row there is the one at that height in the whole list.
    scroll(&ctx, &mut a, 25_000.0);
    let texts = drawn(&ctx, &mut a, vec![]);
    let shown = people(&texts);
    let first: usize = shown.first().and_then(|p| p.strip_prefix("Person ")).and_then(|n| n.parse().ok()).expect("people drawn");
    assert_eq!(first, (25_000.0 / pitch) as usize, "scrolled to {first}, rows {pitch} apart");
    assert_eq!(shown, (first..first + shown.len()).map(|i| format!("Person {i:04}")).collect::<Vec<_>>());
    // The third row drawn, clicked.
    let (name, at) = texts.iter().filter(|(t, _)| t.starts_with("Person ")).nth(2).cloned().expect("three people drawn");
    let at = at + egui::vec2(10.0, 6.0);
    click(&ctx, &mut a, at);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert_eq!(f.selected.and_then(|i| f.list.get(i)).map(|p| p.name.clone()), Some(name));
    // Scrolled to the end: the last person is the last row, all of it in view.
    scroll(&ctx, &mut a, 100_000.0);
    let whole: Vec<String> = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0))
        .into_iter()
        .filter(|d| d.text.starts_with("Person ") && d.clip.contains_rect(d.rect))
        .map(|d| d.text)
        .collect();
    assert_eq!(whole.last().map(String::as_str), Some("Person 1999"), "{whole:?}");
    assert!(whole.len() >= 5, "{whole:?}");
}

/// A frame of Select Names with 2,000 people (address_book's most), at the top and scrolled,
/// and a fingerprint of what it draws (the same before and after a change that only makes it
/// faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_select_names -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_select_names() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-names");
    let ctx = egui::Context::default();
    names_of(&ctx, &mut a, 2000);
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            let _ = drawn(&ctx, a, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    // The window (the message being written) without the dialog, to take away.
    let dialog = a.ui.dialog.take();
    let _ = drawn(&ctx, &mut a, vec![]);
    let window = timed(&mut a);
    println!("MEASURE the window alone: {window:.2} ms a frame");
    a.ui.dialog = dialog;
    for (place, by) in [("at the top", 0.0), ("scrolled", 25_000.0)] {
        if by > 0.0 {
            scroll(&ctx, &mut a, by);
        }
        let texts = drawn(&ctx, &mut a, vec![]);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &texts {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
        let each = timed(&mut a);
        println!(
            "MEASURE select names, 2000 people, {place}: {each:.2} ms a frame, the dialog {:.2} ms; drawn {} texts, {:016x}",
            each - window,
            texts.len(),
            h.finish()
        );
    }
}

/// Opened from Bcc, a double-click adds the person to Bcc; a field's button adds the one selected
/// to that field.
#[test]
fn select_names_adds_to_the_field_asked() {
    use crate::dialogs::Recipients;
    let (mut a, _) = app("names-fields");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, Recipients::Bcc));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    f.list = (0..3).map(|i| mailcraft_engine::model::EmailAddress::new(format!("Person {i:04}"), format!("p{i:04}@x.example"))).collect();
    frame(&ctx, &mut a, vec![]);
    let fields = |a: &MailApp| match a.ui.dialog.as_ref() {
        Some(Dialog::SelectNames(f)) => (f.to.clone(), f.cc.clone(), f.bcc.clone()),
        _ => panic!("no dialog"),
    };
    let texts = drawn(&ctx, &mut a, vec![]);
    let at = texts.iter().find(|(t, _)| t == "Person 0001").map(|(_, p)| *p + egui::vec2(10.0, 6.0)).expect("Person 0001 drawn");
    click(&ctx, &mut a, at);
    click(&ctx, &mut a, at);
    assert_eq!(fields(&a), (String::new(), String::new(), "Person 0001 <p0001@x.example>; ".to_string()));
    click_named(&ctx, &mut a, "Cc ->");
    assert_eq!(fields(&a).1, "Person 0001 <p0001@x.example>; ");
    click_named(&ctx, &mut a, "To ->");
    assert_eq!(fields(&a).0, "Person 0001 <p0001@x.example>; ");
}
