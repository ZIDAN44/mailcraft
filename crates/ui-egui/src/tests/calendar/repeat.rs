//! The Repeat list of the event and task forms: each choice picked from the list saves its rule,
//! and a rule the list has no word for is shown as Custom and kept as it was.

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The text drawn in a frame given `events`: each piece with where it is.
fn drawn(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(String, egui::Rect)> {
    drawn_with(ctx, a, SIZE, events).into_iter().map(|d| (d.text, d.rect)).collect()
}

/// What the Repeat list shows, beside its label.
fn shown(ctx: &egui::Context, a: &mut MailApp) -> (String, egui::Rect) {
    let all = drawn(ctx, a, vec![]);
    let row = all.iter().find(|(s, _)| s == "Repeat").map(|(_, r)| *r).unwrap_or_else(|| panic!("no Repeat row in {all:?}"));
    all.into_iter()
        .filter(|(s, r)| s != "Repeat" && r.left() > row.right() && (r.center().y - row.center().y).abs() < 12.0)
        .min_by(|x, y| x.1.left().total_cmp(&y.1.left()))
        .unwrap_or_else(|| panic!("nothing beside Repeat"))
}

/// The choices the list offers once opened (what's drawn below the box that wasn't before), top
/// to bottom.
fn offered(ctx: &egui::Context, a: &mut MailApp, list: egui::Rect) -> Vec<(String, egui::Rect)> {
    let before = drawn(ctx, a, vec![]);
    click(ctx, a, list.center());
    let mut items: Vec<(String, egui::Rect)> = drawn(ctx, a, vec![])
        .into_iter()
        .filter(|x| !before.contains(x) && x.1.top() > list.bottom() && (x.1.left() - list.left()).abs() < 20.0)
        .collect();
    items.sort_by(|x, y| x.1.top().total_cmp(&y.1.top()));
    items
}

/// Opens the list, checks what it offers, and picks `choice` from it.
fn pick(ctx: &egui::Context, a: &mut MailApp, offers: &[&str], choice: &str) {
    let (_, list) = shown(ctx, a);
    let items = offered(ctx, a, list);
    let names: Vec<&str> = items.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(names, offers, "the list's choices");
    let at = items.iter().find(|(s, _)| s == choice).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {choice:?} offered"));
    click(ctx, a, at);
}

fn params(a: &MailApp) -> serde_json::Value {
    match &a.ui.dialog {
        Some(Dialog::Event(f)) => f.params(),
        Some(Dialog::Task(f)) => f.params(),
        _ => panic!("no form open"),
    }
}

/// The rule saved for what the form's Save sends.
fn saved(a: &mut MailApp, title: &str) -> Option<String> {
    let event = matches!(a.ui.dialog, Some(Dialog::Event(_)));
    let p = params(a);
    match (event, p["uid"].as_str().map(str::to_string)) {
        (true, uid) => {
            a.act(crate::Action::Submit(if uid.is_some() { "event.update" } else { "event.create" }, p));
            assert!(a.ui.dialog.is_none(), "{title}: saved");
            let item = a.session.agenda(NOW, NOW + 30 * 86_400).into_iter().find(|i| i.summary == title).unwrap();
            a.session.find_event(&item.uid).unwrap().1.event.rrule.clone()
        }
        (false, uid) => {
            a.act(crate::Action::Submit(if uid.is_some() { "task.update" } else { "task.create" }, p));
            assert!(a.ui.dialog.is_none(), "{title}: saved");
            let uid = uid.or_else(|| a.ui.task.clone()).unwrap();
            a.session.find_task(&uid).unwrap().task.rrule
        }
    }
}

const PLAIN: [&str; 5] = ["Never", "Every day", "Every week", "Every month", "Every year"];

/// Every choice of a new event's and a new task's list: the list shows it, Save sends its word,
/// and what's saved is its rule.
#[test]
fn every_repeat_choice_saves_its_rule() {
    let (mut a, _) = app("repeat-choices");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let choices = [
        ("Never", "none", None),
        ("Every day", "daily", Some("FREQ=DAILY")),
        ("Every week", "weekly", Some("FREQ=WEEKLY")),
        ("Every month", "monthly", Some("FREQ=MONTHLY")),
        ("Every year", "yearly", Some("FREQ=YEARLY")),
    ];
    for kind in ["event", "task"] {
        for (label, word, rule) in choices {
            let title = format!("{kind} {label}");
            let mut d = if kind == "event" { Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)) } else { Dialog::new_task(&a.session, None) };
            match &mut d {
                Dialog::Event(f) => f.summary = title.clone(),
                Dialog::Task(f) => f.title = title.clone(),
                _ => {}
            }
            a.ui.dialog = Some(d);
            drawn(&ctx, &mut a, vec![]);
            assert_eq!(shown(&ctx, &mut a).0, "Never", "{title}: a new one's");
            pick(&ctx, &mut a, &PLAIN, label);
            assert_eq!(shown(&ctx, &mut a).0, label, "{title}: shown");
            // The task form says a repeating task comes back.
            let back = drawn(&ctx, &mut a, vec![]).iter().any(|(s, _)| s == "Marked complete, it comes back on its next due date.");
            assert_eq!(back, kind == "task" && rule.is_some(), "{title}: the note");
            // An event's form sends the rule it makes (issue #103), a task's the list's word.
            let sent = if kind == "event" { rule.unwrap_or("none") } else { word };
            assert_eq!(params(&a)["repeat"], sent, "{title}: sent");
            assert_eq!(saved(&mut a, &title).as_deref(), rule, "{title}: saved");
        }
    }
}

/// A rule the list has no word for is shown as Custom, offered after the five, and saved as it
/// was; a plain one written in small letters is shown as its word. Picking a word replaces it.
#[test]
fn a_rule_of_its_own_is_kept_as_custom() {
    let (mut a, _) = app("repeat-custom");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let start = NOW + 3 * 86_400;
    for kind in ["event", "task"] {
        for (rule, label, word, kept) in [
            ("FREQ=DAILY;BYHOUR=9,17", "Custom", "FREQ=DAILY;BYHOUR=9,17", "FREQ=DAILY;BYHOUR=9,17"),
            ("freq=weekly", "Every week", "weekly", "FREQ=WEEKLY"),
        ] {
            let title = format!("{kind} {rule}");
            let open = |a: &MailApp| {
                if kind == "event" {
                    let item = a.session.agenda(NOW, NOW + 30 * 86_400).into_iter().find(|i| i.summary == title).unwrap();
                    Dialog::open_event(&a.session, &item.uid, item.occurrence).unwrap()
                } else {
                    let uid = a.session.tasks("").into_iter().find(|t| t.task.title == title).unwrap().task.uid.clone();
                    Dialog::open_task(&a.session, &uid).unwrap()
                }
            };
            if kind == "event" {
                a.session.run("event.create", &json!({"summary": title, "start": start, "end": start + 3600, "repeat": rule})).unwrap();
            } else {
                a.session.run("task.create", &json!({"title": title, "due": "2026-10-20", "repeat": rule})).unwrap();
            }
            a.ui.dialog = Some(open(&a));
            drawn(&ctx, &mut a, vec![]);
            assert_eq!(shown(&ctx, &mut a).0, label, "{title}: shown");
            let sent = if kind == "event" && label != "Custom" { kept } else { word };
            assert_eq!(params(&a)["repeat"], sent, "{title}: sent");
            assert_eq!(saved(&mut a, &title).as_deref(), Some(kept), "{title}: saved");
            if label != "Custom" {
                continue;
            }
            // Custom is offered while the rule is the form's, and picking it keeps it.
            a.ui.dialog = Some(open(&a));
            drawn(&ctx, &mut a, vec![]);
            let all: Vec<&str> = PLAIN.iter().copied().chain(["Custom"]).collect();
            pick(&ctx, &mut a, &all, "Custom");
            assert_eq!(params(&a)["repeat"], word, "{title}: Custom picked");
            // A word picked replaces it, and Custom is no longer offered.
            pick(&ctx, &mut a, &all, "Every day");
            assert_eq!(shown(&ctx, &mut a).0, "Every day", "{title}: replaced");
            pick(&ctx, &mut a, &PLAIN, "Every day");
            assert_eq!(saved(&mut a, &title).as_deref(), Some("FREQ=DAILY"), "{title}: replaced");
        }
    }
}

fn event_form(a: &mut MailApp) -> &mut crate::pim_dialogs::EventForm {
    match &mut a.ui.dialog {
        Some(Dialog::Event(f)) => f,
        _ => panic!("no event form"),
    }
}

/// Issue #103: every second week on Tuesdays and Thursdays, six times; the form shows how it
/// repeats under Repeat, saves its rule, and opens with it again to change.
#[test]
fn a_pattern_is_saved_and_opens_again() {
    let (mut a, _) = app("repeat-pattern");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)));
    event_form(&mut a).summary = "Mix review".into();
    drawn(&ctx, &mut a, vec![]);
    pick(&ctx, &mut a, &PLAIN, "Every week");
    let texts: Vec<String> = drawn(&ctx, &mut a, vec![]).into_iter().map(|(s, _)| s).collect();
    for row in ["Repeats every", "weeks", "On these days", "Mon", "Series ends", "Never"] {
        assert!(texts.iter().any(|s| s == row), "{row} in {texts:?}");
    }
    let f = event_form(&mut a);
    let p = f.pattern.as_mut().expect("a pattern for Every week");
    (p.interval, p.days, p.end) = (2, vec![1, 3], mailcraft_engine::model::ical::PatternEnd::Count(6));
    let rule = "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,TH;COUNT=6";
    assert_eq!(params(&a)["repeat"], rule);
    assert_eq!(saved(&mut a, "Mix review").as_deref(), Some(rule));

    let item = a.session.agenda(NOW, NOW + 30 * 86_400).into_iter().find(|i| i.summary == "Mix review").unwrap();
    a.ui.dialog = Some(Dialog::open_event(&a.session, &item.uid, item.occurrence).unwrap());
    assert_eq!(shown(&ctx, &mut a).0, "Every week", "shown as a week's, not Custom");
    let f = event_form(&mut a);
    assert_eq!(f.pattern.as_ref().map(|p| (p.interval, p.days.clone())), Some((2, vec![1, 3])));
    assert_eq!(params(&a)["repeat"], rule, "unchanged, it's sent as it was");
}

/// A series ending on a day ends at the end of that day where it is, in UTC (RFC 5545 for a
/// timed event).
#[test]
fn a_series_until_a_day_ends_that_day() {
    let (mut a, _) = app("repeat-until");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)));
    event_form(&mut a).summary = "Daily until".into();
    drawn(&ctx, &mut a, vec![]);
    pick(&ctx, &mut a, &PLAIN, "Every day");
    drawn(&ctx, &mut a, vec![]);
    let f = event_form(&mut a);
    let last = crate::dialogs::pim_dialogs::pattern_day(&f.date).unwrap() + 4;
    f.pattern.as_mut().unwrap().end = mailcraft_engine::model::ical::PatternEnd::Until(last);
    let rule = saved(&mut a, "Daily until").unwrap();
    assert!(rule.starts_with("FREQ=DAILY;UNTIL=") && rule.ends_with('Z'), "{rule}");
    let item = a.session.agenda(NOW, NOW + 30 * 86_400).into_iter().filter(|i| i.summary == "Daily until").count();
    assert_eq!(item, 5, "five days, the last one's included");
}

/// The end keeps the event's length when the start is changed (issue #103); an end changed by
/// hand gives it a new one.
#[test]
fn the_end_follows_the_start() {
    use crate::pim_dialogs::{end_follows_start, typed_moment};
    let (a, _) = app("repeat-end-follows");
    let Dialog::Event(mut f) = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400)) else { panic!("no event form") };
    (f.date, f.start, f.end_date, f.end) = ("2026-11-19".into(), "10:00".into(), "2026-11-19".into(), "10:30".into());
    let typed = |f: &crate::pim_dialogs::EventForm| typed_moment(&f.date, &f.start, f.all_day);
    let now = typed(&f);
    end_follows_start(&mut f, now);
    let before = typed(&f);
    f.start = "23:45".into();
    end_follows_start(&mut f, before);
    assert_eq!((f.end_date.as_str(), f.end.as_str()), ("2026-11-20", "00:15"), "half an hour later, past midnight");
    let before = typed(&f);
    f.date = "2026-11-21".into();
    end_follows_start(&mut f, before);
    assert_eq!((f.end_date.as_str(), f.end.as_str()), ("2026-11-22", "00:15"), "a new day too");
    // The end changed by hand: the new length from then on.
    f.end = "01:45".into();
    let now = typed(&f);
    end_follows_start(&mut f, now);
    let before = typed(&f);
    f.start = "09:00".into();
    end_follows_start(&mut f, before);
    assert_eq!((f.end_date.as_str(), f.end.as_str()), ("2026-11-21", "11:00"));
    // Half typed, the end waits.
    let before = typed(&f);
    f.start = "9:".into();
    end_follows_start(&mut f, before);
    assert_eq!(f.end, "11:00");
}

/// Issue #241: an occurrence of a series opened from the calendar asks whether that occurrence
/// or the series is opened. This occurrence: a form at its own date, with no Repeat, which saves
/// that occurrence alone; the series opens as before.
#[test]
fn an_occurrence_opens_and_is_changed_on_its_own() {
    let (mut a, _) = app("occurrence-open");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.run("event.create", json!({"summary": "Stand-up", "start": "2026-10-21T09:00", "end": "2026-10-21T09:15", "repeat": "weekly"}));
    let ours = |a: &MailApp| -> Vec<mailcraft_engine::AgendaItem> {
        a.session.agenda(NOW - 86_400, NOW + 30 * 86_400).into_iter().filter(|i| i.summary.starts_with("Stand-up")).collect()
    };
    let second = ours(&a)[1].clone();
    a.ui.dialog = Dialog::open_item(&a.session, &second.uid, second.occurrence);
    assert!(matches!(&a.ui.dialog, Some(Dialog::OpenRecurring { .. })), "{:?}", a.ui.dialog);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Open this occurrence");
    frame(&ctx, &mut a, vec![]);
    let f = event_form(&mut a);
    assert!(f.this_occurrence && !f.recurring, "that occurrence alone");
    assert_eq!(
        f.date,
        mailcraft_engine::model::dates::day_string(mailcraft_engine::model::dates::today(&mailcraft_engine::model::dates::Clock::new(
            second.start,
            0
        )))
    );
    f.summary = "Stand-up (Room 2)".into();
    let p = params(&a);
    assert_eq!(p["occurrence"], second.occurrence);
    assert!(p.get("repeat").is_none(), "{p}");
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    assert!(!texts.iter().any(|s| s == "Repeat"), "no Repeat for one occurrence");
    assert!(texts.iter().any(|s| s == "Changes apply to this occurrence alone."));
    a.act(crate::Action::Submit("event.update", p));
    let now: Vec<String> = ours(&a).into_iter().map(|i| i.summary).collect();
    assert_eq!(now.get(..3), Some(&["Stand-up".to_string(), "Stand-up (Room 2)".to_string(), "Stand-up".to_string()][..]), "{now:?}");
    // The series: as before, its form for all of it.
    a.ui.dialog = Dialog::open_item(&a.session, &second.uid, second.occurrence);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Open the series");
    frame(&ctx, &mut a, vec![]);
    let f = event_form(&mut a);
    assert!(f.recurring && !f.this_occurrence && f.summary == "Stand-up");
}
