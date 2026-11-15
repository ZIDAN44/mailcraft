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
            assert_eq!(params(&a)["repeat"], word, "{title}: sent");
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
            ("FREQ=WEEKLY;BYDAY=MO,WE", "Custom", "FREQ=WEEKLY;BYDAY=MO,WE", "FREQ=WEEKLY;BYDAY=MO,WE"),
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
            assert_eq!(params(&a)["repeat"], word, "{title}: sent");
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
