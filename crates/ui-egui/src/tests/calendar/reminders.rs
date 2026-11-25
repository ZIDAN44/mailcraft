//! UI tests: the Reminders window over a large calendar: a change shows on the next frame, and
//! what a frame costs; flagged mail reminding in it.

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

/// Gap 9: a flagged message given a reminder by Follow Up › Add reminder… reminds in the window
/// when its time comes; Open Item opens it in a window of its own; Dismiss takes the reminder
/// off, the message still flagged.
#[test]
fn flagged_mail_reminds_in_the_window() {
    let (mut a, _) = app("reminder-flagged");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Character turnarounds").map(|m| m.key.clone()).unwrap();
    a.session.selection = vec![key.clone()];
    let f = crate::pim_dialogs::FollowUpForm::for_selection(&a.session, true).unwrap();
    assert!(f.remind && f.target == crate::pim_dialogs::follow_up::FollowUpTarget::Selection(1), "{f:?}");
    a.ui.dialog = Some(Dialog::FollowUp(Box::new(f)));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let last = |a: &mut MailApp, what: &str| {
        drawn_text(&ctx, a, size).into_iter().rev().find(|d| d.text == what).map(|d| d.rect.center()).unwrap_or_else(|| panic!("no {what:?}"))
    };
    let ok = last(&mut a, "OK");
    click(&ctx, &mut a, ok);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "the form closed");
    let m = a.session.summary(&key).cloned().unwrap();
    assert!(m.flags.flagged, "flagged with it");
    let at = a.session.message_reminder(&m).expect("a reminder set");
    assert!(a.ui.reminders.due.iter().all(|r| r.message.is_none()), "not yet");

    // Its time comes: in the window, once every folder's flagged mail is read.
    a.services = Box::new(crate::NullServices(at + 60));
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    let r = a.ui.reminders.due.iter().find(|r| r.message.as_ref() == Some(&key)).cloned().expect("listed");
    let row = drawn_text(&ctx, &mut a, size).into_iter().find(|d| d.text.starts_with(&format!("{}  ·", r.summary))).map(|d| d.rect.center());
    click(&ctx, &mut a, row.expect("drawn in the window"));
    let open = last(&mut a, "Open Item");
    click(&ctx, &mut a, open);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.windows.iter().any(|w| w.key == key), "opened in a window of its own");
    let dismiss = last(&mut a, "Dismiss");
    click(&ctx, &mut a, dismiss);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.reminders.due.iter().all(|r| r.message.is_none()), "dismissed");
    let m = a.session.summary(&key).cloned().unwrap();
    assert!(m.flags.flagged && a.session.message_reminder(&m).is_none());
}

/// The ribbon's Follow Up menu offers a day and a reminder of the selection's own (Custom…, Add
/// reminder…), each in the form, its reminder on for the second.
#[test]
fn the_ribbons_follow_up_menu_gives_a_reminder() {
    let (mut a, _) = app("reminder-ribbon");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Character turnarounds").map(|m| m.key.clone()).unwrap();
    for (item, remind) in [("Custom…", false), ("Add reminder…", true)] {
        a.session.selection = vec![key.clone()];
        frame(&ctx, &mut a, vec![]);
        click_named(&ctx, &mut a, "Follow up");
        frame(&ctx, &mut a, vec![]);
        let at = drawn_text(&ctx, &mut a, size).into_iter().rev().find(|d| d.text == item).map(|d| d.rect.center());
        click(&ctx, &mut a, at.unwrap_or_else(|| panic!("no {item:?} in the menu")));
        frame(&ctx, &mut a, vec![]);
        assert!(matches!(&a.ui.dialog, Some(Dialog::FollowUp(f)) if f.remind == remind && f.subject == "Character turnarounds"), "{item}: the form");
        a.ui.dialog = None;
    }
}

/// Issue #220: Follow Up › Custom… is as tall as its fields, OK and Cancel under them at its
/// bottom; it was as tall as the screen, the buttons halfway down. The new contact form's buttons
/// too, on a tall window.
#[test]
fn small_forms_are_as_tall_as_their_fields() {
    let (mut a, _) = app("follow-up-height");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let size = egui::vec2(1280.0, 2000.0);
    let gap = |a: &mut MailApp, dialog: Dialog, last: &str| {
        a.ui.dialog = Some(dialog);
        let mut shown = Vec::new();
        for _ in 0..4 {
            shown = drawn_text(&ctx, a, size);
        }
        let at = |what: &str| shown.iter().find(|d| d.text == what).map(|d| d.rect).unwrap_or_else(|| panic!("no {what:?}"));
        let (field, ok) = (at(last), at("Cancel"));
        a.ui.dialog = None;
        ok.top() - field.bottom()
    };
    let form = crate::pim_dialogs::FollowUpForm::for_selection(&a.session, false).expect("the form");
    let custom = gap(&mut a, Dialog::FollowUp(Box::new(form)), "Remind me");
    assert!(custom < 120.0, "Cancel {custom} below the reminder");
    let contact = gap(&mut a, Dialog::Contact(Box::default()), "Notes");
    assert!(contact < 400.0, "Cancel {contact} below the notes");
}

/// Issue #239: an online meeting's reminder has Join meeting beside Open Item, which opens its
/// link at once; a meeting with no link, a task or flagged mail has none.
#[test]
fn a_meetings_reminder_joins_it() {
    let (mut a, _) = app("reminder-join");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let day = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400));
    let at = |s: i64| format!("{day}T{:02}:{:02}", s.div_euclid(3600).rem_euclid(24), s.div_euclid(60).rem_euclid(60));
    let link = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0";
    a.run("event.create", json!({"summary": "Stand-up", "start": at(NOW + 600), "reminder": 15, "location": link}));
    a.run("event.create", json!({"summary": "Lunch", "start": at(NOW + 660), "reminder": 15}));
    let size = egui::vec2(1280.0, 800.0);
    let joins = |a: &mut MailApp, summary: &str| {
        frame(&ctx, a, vec![]);
        let key = a.ui.reminders.due.iter().find(|r| r.summary == summary).map(|r| r.key.clone()).expect("due");
        a.ui.reminders.selected = Some(key);
        let mut shown = Vec::new();
        for _ in 0..3 {
            shown = drawn_text(&ctx, a, size);
        }
        shown.iter().find(|d| d.text == "Join meeting").map(|d| d.rect.center())
    };
    assert!(joins(&mut a, "Lunch").is_none(), "no link, no Join");
    let at = joins(&mut a, "Stand-up").expect("Join meeting");
    a.session.notices.clear();
    click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    // The test's services open nothing: that they were asked to is the notice they give.
    assert!(a.session.notices.iter().any(|n| n.text.contains("The link couldn't be opened")), "{:?}", a.session.notices);
}

/// Issue #239: in German, Dismiss All was drawn over Join meeting (the buttons wider than the
/// window has room for on one line); and one minute off read "In 1 Minuten". Each button stands
/// clear of the others, in every language, and one of each says so in the singular.
#[test]
fn a_reminders_buttons_stand_clear_and_say_one_minute() {
    let (mut a, _) = app("reminder-buttons");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let day = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400));
    let at = |s: i64| format!("{day}T{:02}:{:02}", s.div_euclid(3600).rem_euclid(24), s.div_euclid(60).rem_euclid(60));
    let link = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc%40thread.v2/0";
    a.run("event.create", json!({"summary": "Stand-up", "start": at(NOW + 90), "reminder": 15, "location": link}));
    let size = egui::vec2(1280.0, 800.0);
    for lang in crate::i18n::Lang::for_picker() {
        crate::i18n::set_current(lang);
        let mut shown = Vec::new();
        for _ in 0..3 {
            shown = drawn_text(&ctx, &mut a, size);
        }
        let rect = |what: &str| shown.iter().find(|d| d.text == crate::i18n::t(what)).map(|d| d.rect);
        let buttons: Vec<egui::Rect> = ["Join meeting", "Dismiss All", "Open Item", "Dismiss"].iter().filter_map(|b| rect(b)).collect();
        assert_eq!(buttons.len(), 4, "{}: every button drawn", lang.code());
        for (i, x) in buttons.iter().enumerate() {
            for y in buttons.iter().skip(i + 1) {
                assert!(!x.intersects(*y), "{}: {x:?} over {y:?}", lang.code());
            }
        }
        assert!(shown.iter().any(|d| d.text.contains(crate::i18n::t("In 1 minute"))), "{}: one minute, in the singular", lang.code());
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// Issue #224: an event's several reminders (Google's "an hour before and 2 minutes before") each
/// have a list in its form; Add reminder gives one more and its button takes one away, and the
/// form saves them all. A calendar that keeps one an event (Exchange's) isn't offered more.
#[test]
fn an_events_reminders_each_have_a_list_in_its_form() {
    let (mut a, _) = app("several-reminders");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("event.create", json!({"summary": "Standup", "start": "2026-11-23T10:00", "reminders": [60, 2]}));
    let e = a.session.pim_mut("sample").events.iter().find(|e| e.event.summary == "Standup").map(|e| e.event.clone()).expect("made");
    let f = crate::pim_dialogs::EventForm::open(&a.session, &e.uid, e.start).expect("its form");
    assert_eq!((f.reminder, f.more_reminders.clone()), (Some(60), vec![2]));
    a.ui.dialog = Some(Dialog::Event(Box::new(f)));
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(has(&texts, "1 hour") && has(&texts, "2 minutes") && has(&texts, "Add reminder"), "{texts:?}");
    let form = |a: &MailApp| match &a.ui.dialog {
        Some(Dialog::Event(f)) => (f.reminder, f.more_reminders.clone()),
        other => panic!("{other:?}"),
    };
    let add = drawn_text(&ctx, &mut a, size).into_iter().find(|d| d.text == "Add reminder").map(|d| d.rect.center()).expect("Add reminder");
    click(&ctx, &mut a, add);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&a), (Some(60), vec![2, 15]), "one more, of Outlook's default");
    // The topmost one's button (a screen reader's list of them isn't in the order they're drawn).
    let first = named(&ctx, &mut a, size)
        .iter()
        .filter(|n| n.label.as_deref() == Some("Remove reminder"))
        .filter_map(crate::tests::Named::centre)
        .min_by(|x, y| x.y.total_cmp(&y.y))
        .expect("Remove reminder");
    click(&ctx, &mut a, first);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&a), (Some(60), vec![15]), "the first after the first gone");
    let Some(Dialog::Event(f)) = &a.ui.dialog else { panic!("no form") };
    let p = f.params();
    assert_eq!(p["reminders"], json!([60, 15]));
    a.act(crate::Action::Submit("event.update", p));
    let back = a.session.find_event(&e.uid).map(|(_, e)| mailcraft_engine::model::ical::reminders_minutes(&e.event));
    assert_eq!(back, Some(vec![60, 15]));
    // On a calendar that keeps one an event: none offered past it.
    let Some(Dialog::Event(f)) = &mut a.ui.dialog else { return };
    f.one_reminder = vec!["sample".into()];
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(!has(&texts, "Add reminder"), "{texts:?}");
}
