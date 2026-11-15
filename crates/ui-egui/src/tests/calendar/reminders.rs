//! UI tests: the Reminders window over a large calendar: a change shows on the next frame, and
//! what a frame costs.

use super::*;

/// Three meetings whose reminders are due at [`NOW`], then `events` past events (no reminder) and
/// `tasks` tasks (none reminding) put in the sample account's calendar directly.
fn big_calendar(a: &mut MailApp, events: usize, tasks: usize) -> Vec<String> {
    let day = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400));
    let hm = |s: i64| format!("{day}T{:02}:{:02}", s.div_euclid(3600).rem_euclid(24), s.div_euclid(60).rem_euclid(60));
    let mut due = Vec::new();
    for n in 0..3 {
        let p = json!({"summary": format!("Review {n}"), "start": hm(NOW + 600 + n * 60), "reminder": 15});
        a.run("event.create", p);
        due.push(format!("Review {n}"));
    }
    a.run("event.create", json!({"summary": "Old", "start": hm(NOW - 7200), "reminder": "none"}));
    a.run("task.create", json!({"subject": "Chore"}));
    let p = a.session.pim_mut("sample");
    let (Some(event), Some(task)) = (p.events.iter().find(|e| e.event.summary == "Old").cloned(), p.tasks.last().cloned()) else {
        panic!("the event and task made");
    };
    for n in 0..events {
        let mut e = event.clone();
        e.event.uid = format!("old-{n}");
        e.event.start -= 3600 * n as i64;
        e.event.end -= 3600 * n as i64;
        p.events.push(e);
    }
    for n in 0..tasks {
        let mut t = task.clone();
        t.task.uid = format!("chore-{n}");
        p.tasks.push(t);
    }
    a.session.pim_revision = a.session.pim_revision.wrapping_add(1);
    due
}

/// An event's reminder due, renamed (its time, status and everything else the same): the window
/// says the new name on the next frame, not at the next 15-second look.
#[test]
fn a_due_reminder_changed_shows_on_the_next_frame() {
    let (mut a, _) = app("reminder-renamed");
    let ctx = egui::Context::default();
    // Looked at once already: what follows is seen by what changed, not by the time.
    frame(&ctx, &mut a, vec![]);
    let due = big_calendar(&mut a, 10, 10);
    frame(&ctx, &mut a, vec![]);
    let names = |a: &MailApp| a.ui.reminders.due.iter().map(|r| r.summary.clone()).collect::<Vec<_>>();
    assert_eq!(names(&a), due);
    let uid = a.ui.reminders.due[1].uid.clone();
    a.run("event.update", json!({"uid": uid, "summary": "Review X"}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(names(&a), ["Review 0", "Review X", "Review 2"]);
    // Dismissed: gone on the next frame, the others kept.
    let key = a.ui.reminders.due[0].key.clone();
    a.act(crate::Action::Run("reminders.dismiss", json!({ "key": key })));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(names(&a), ["Review X", "Review 2"]);
    // Snoozed: the same.
    let key = a.ui.reminders.due[0].key.clone();
    a.act(crate::Action::Run("reminders.snooze", json!({ "key": key, "minutes": 5 })));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(names(&a), ["Review 2"]);
    // The account removed: its reminders go with it.
    a.run("account.remove", json!({"account": "sample"}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.reminders.due.is_empty(), "{:?}", names(&a));
}

/// The window's Snooze: 5 minutes until another is picked, and "0 minutes before start" until
/// the event starts.
#[test]
fn snooze_in_the_window_takes_the_choice_picked() {
    let (mut a, _) = app("reminder-snooze");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    big_calendar(&mut a, 0, 0);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.reminders.snooze, Some(crate::reminders::Snooze::Minutes(5)), "offered first");
    let size = egui::vec2(1280.0, 800.0);
    let snooze = |a: &mut MailApp, ctx: &egui::Context| {
        let r = a.ui.reminders.due[0].clone();
        let texts = drawn_text(ctx, a, size);
        let row = texts.iter().find(|d| d.text.starts_with(&format!("{}  ·", r.summary))).map(|d| d.rect.center()).unwrap();
        click(ctx, a, row);
        let button = texts.iter().filter(|d| d.text == "Snooze").map(|d| d.rect.center()).max_by(|x, y| x.y.total_cmp(&y.y)).unwrap();
        click(ctx, a, button);
        frame(ctx, a, vec![]);
        assert!(a.ui.reminders.due.iter().all(|d| d.key != r.key), "{} snoozed", r.summary);
        (a.session.prefs.reminders.get(&r.key).copied(), r.start)
    };
    let (until, _) = snooze(&mut a, &ctx);
    assert_eq!(until, Some(NOW + 5 * 60));
    a.ui.reminders.snooze = Some(crate::reminders::Snooze::AtStart);
    let (until, start) = snooze(&mut a, &ctx);
    assert_eq!(until, Some(start));
}

/// What the reminders cost a frame with 20,000 events and 5,000 tasks, three of them due (the
/// window up), and a fingerprint of what the window draws (the same before and after a change
/// that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_reminders -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_reminders() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-reminders");
    let ctx = egui::Context::default();
    big_calendar(&mut a, 20_000, 5_000);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    // The work a frame does when nothing changed.
    let mut st = a.ui.reminders.clone();
    let t = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = crate::reminders::refresh(&mut st, &a.session, false);
    }
    let each = t.elapsed().as_secs_f64() * 1_000_000.0 / 1000.0;
    let t = std::time::Instant::now();
    for _ in 0..50 {
        frame(&ctx, &mut a, vec![]);
    }
    let whole = t.elapsed().as_secs_f64() * 1000.0 / 50.0;
    let drawn = drawn_text(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in &drawn {
        (&d.text, d.rect.min.x.to_bits(), d.rect.min.y.to_bits()).hash(&mut h);
    }
    let texts = drawn.len();
    for r in &a.ui.reminders.due {
        (&r.summary, r.start, r.due).hash(&mut h);
    }
    println!(
        "MEASURE reminders, 20000 events, 5000 tasks: refresh {each:.2} us a frame; whole frame {whole:.2} ms; {} due, drawn {texts} texts, {:016x}",
        a.ui.reminders.due.len(),
        h.finish()
    );
}
