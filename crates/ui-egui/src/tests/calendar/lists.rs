//! UI tests: People, Tasks and Notes with long lists (what they draw at the top and scrolled, a
//! change seen on the next frame, the note being written on kept), and what a frame costs.

use super::*;

const FIRST: [&str; 12] = ["Alex", "Blair", "Casey", "Drew", "Emery", "Finley", "Gray", "Harper", "Indigo", "Jules", "Kai", "Logan"];

/// `people` contacts (names under twelve letters, companies in another order), `tasks` tasks
/// and `notes` notes put in the sample account directly.
fn big_lists(a: &mut MailApp, people: usize, tasks: usize, notes: usize) {
    a.run("contact.create", json!({"name": "Seed", "emails": "seed@x.example"}));
    a.run("task.create", json!({"subject": "Seed task"}));
    a.run("note.create", json!({"text": "Seed note"}));
    let p = a.session.pim_mut("sample");
    let (Some(card), Some(task), Some(note)) =
        (p.cards.iter().find(|c| c.card.name == "Seed").cloned(), p.tasks.last().cloned(), p.notes.last().cloned())
    else {
        panic!("the seeds made");
    };
    for n in 0..people {
        let mut c = card.clone();
        c.card.uid = format!("big-{n}");
        c.card.name = format!("{} {n:05}", FIRST[n % FIRST.len()]);
        c.card.org = format!("Company {:04}", (n * 7919) % 10_000);
        if let Some(e) = c.card.emails.first_mut() {
            e.value = format!("p{n}@x.example");
        }
        p.cards.push(c);
    }
    for n in 0..tasks {
        let mut t = task.clone();
        t.task.uid = format!("task-{n}");
        t.task.title = format!("Task {n:05}");
        t.task.due = Some(NOW + 86_400 * (n as i64 % 40));
        p.tasks.push(t);
    }
    for n in 0..notes {
        let mut x = note.clone();
        x.note.uid = format!("note-{n}");
        x.note.text = format!("Note {n:05}\nwhat it says");
        x.note.modified = NOW - n as i64;
        p.notes.push(x);
    }
    a.session.pim_revision = a.session.pim_revision.wrapping_add(1);
}

/// Each text the frame draws that can be seen (not wholly outside its clip), where.
fn drawn(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(String, egui::Pos2)> {
    drawn_with(ctx, a, egui::vec2(1280.0, 800.0), events).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// The texts drawn that `is` picks (their first lines), left of x = 300 or right of it, top to
/// bottom.
fn picked(texts: &[(String, egui::Pos2)], is: impl Fn(&str) -> bool, left: bool) -> Vec<String> {
    texts.iter().filter(|(t, p)| is(t) && (p.x < 300.0) == left).map(|(t, _)| t.lines().next().unwrap_or_default().to_string()).collect()
}

fn a_name(t: &str) -> bool {
    FIRST.iter().any(|f| t.strip_prefix(f).is_some_and(|r| r.starts_with(' ') && r.len() == 6))
}

/// Scrolled down by `by` points with the mouse wheel over the first text `is` picks (left of
/// x = 300 or right of it), and settled.
fn scroll(ctx: &egui::Context, a: &mut MailApp, is: impl Fn(&str) -> bool, left: bool, by: f32) {
    let texts = drawn(ctx, a, vec![]);
    let Some(over) = texts.iter().find(|(t, p)| is(t) && (p.x < 300.0) == left).map(|(_, p)| *p + egui::vec2(20.0, 8.0)) else {
        panic!("something to scroll over: {texts:?}");
    };
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

/// `shown` is a run of `all`, in order, starting past `from` (at the top: at it).
fn a_run_of(shown: &[String], all: &[String], from: usize, top: bool) {
    assert!(shown.len() >= 5, "{shown:?}");
    let Some(at) = all.iter().position(|x| *x == shown[0]) else { panic!("{} listed", shown[0]) };
    assert!(if top { at == from } else { at > from + 50 }, "starts at {at}: {shown:?}");
    assert_eq!(shown, &all[at..at + shown.len()]);
}

/// People, Tasks and Notes with long lists: each draws its first items in order, and later ones
/// in order once scrolled; a contact added or a search typed is seen on the next frame; the
/// List view sorts by the column picked; a note scrolled out of view while written on still
/// takes what's typed.
#[test]
fn long_lists_scroll_in_order_and_follow_changes() {
    let (mut a, _) = app("long-lists");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    big_lists(&mut a, 1200, 400, 120);
    a.run("view.module", json!({"name": "people"}));
    let names = |a: &MailApp| a.session.people("").iter().map(|p| p.card.display()).filter(|n| a_name(n)).collect::<Vec<_>>();
    let all = names(&a);
    let texts = drawn(&ctx, &mut a, vec![]);
    a_run_of(&picked(&texts, a_name, true), &all, 0, true);
    // The first letter's heading above them (the index beside them is in lower case).
    assert_eq!(picked(&texts, |t| t == "A", true), ["A"]);
    scroll(&ctx, &mut a, a_name, true, 25_000.0);
    let texts = drawn(&ctx, &mut a, vec![]);
    let shown = picked(&texts, a_name, true);
    a_run_of(&shown, &all, 0, false);
    // A letter of the index clicked: the list goes to its heading, the first name under it next.
    let Some(g) = texts.iter().find(|(t, p)| t == "g" && p.x < 300.0).map(|(_, p)| *p + egui::vec2(2.0, 4.0)) else { panic!("{texts:?}") };
    click(&ctx, &mut a, g);
    for _ in 0..60 {
        let _ = drawn(&ctx, &mut a, vec![]);
    }
    let texts = drawn(&ctx, &mut a, vec![]);
    let side: Vec<&String> = texts.iter().filter(|(t, p)| p.x < 300.0 && (a_name(t) || t.as_str() == "G")).map(|(t, _)| t).collect();
    assert_eq!(side.get(..2), Some(&[&"G".to_string(), &"Gray 00006".to_string()][..]), "{side:?}");
    let shown = picked(&texts, a_name, true);
    // A contact added (by name just after the first in view): there on the next frame, and
    // counted. A search typed: what it finds, on the next frame.
    let added = format!("{}x", shown[0]);
    a.run("contact.create", json!({ "name": added }));
    let texts = drawn(&ctx, &mut a, vec![]);
    let items = format!("Items: {}", a.session.people("").len());
    assert!(texts.iter().any(|(t, _)| *t == items), "{items}: {texts:?}");
    let side: Vec<&String> = texts.iter().filter(|(_, p)| p.x < 300.0).map(|(t, _)| t).collect();
    let at = side.iter().position(|t| **t == shown[0]);
    // Its name, its address, the next one's initials, then the next one's name.
    assert!(at.is_some_and(|i| side.get(i + 3) == Some(&&added)), "{added} after {}: {side:?}", shown[0]);
    a.ui.people_query = "Blair 0001".into();
    let found: Vec<String> = a.session.people("Blair 0001").iter().map(|p| p.card.display()).filter(|n| a_name(n)).collect();
    assert!(!found.is_empty() && found.len() < 20, "{found:?}");
    // (The list scrolled far down takes a frame to come back to its shorter end.)
    let _ = drawn(&ctx, &mut a, vec![]);
    assert_eq!(picked(&drawn(&ctx, &mut a, vec![]), a_name, true), found);
    a.ui.people_query.clear();
    // The List view, by company, Z to A.
    a.ui.people_view = crate::people::PeopleView::Phone;
    a.ui.people_list.sort = Some(crate::people::Column::Company);
    a.ui.people_list.descending = true;
    let texts = drawn(&ctx, &mut a, vec![]);
    let companies: Vec<String> = picked(&texts, |t| t.starts_with("Company 9"), false);
    assert!(companies.len() >= 5 && companies.windows(2).all(|w| w[0] >= w[1]), "{companies:?}");
    a.ui.people_view = crate::people::PeopleView::Cards;
    let texts = drawn(&ctx, &mut a, vec![]);
    let cards = picked(&texts, a_name, false);
    a_run_of(&cards, &names(&a), 0, true);
    // Tasks.
    a.run("view.module", json!({"name": "tasks"}));
    let is_task = |t: &str| t.starts_with("Task 0");
    let all: Vec<String> = a.session.tasks("").iter().map(|t| t.task.title.clone()).filter(|t| is_task(t)).collect();
    a_run_of(&picked(&drawn(&ctx, &mut a, vec![]), is_task, false), &all, 0, true);
    scroll(&ctx, &mut a, is_task, false, 25_000.0);
    a_run_of(&picked(&drawn(&ctx, &mut a, vec![]), is_task, false), &all, 0, false);
    // Notes: the first written on, scrolled away, still takes what's typed.
    a.run("view.module", json!({"name": "notes"}));
    let is_note = |t: &str| t.starts_with("Note 0");
    let all: Vec<String> = a.session.notes("").iter().map(|n| n.note.title().to_string()).filter(|t| is_note(t)).collect();
    let texts = drawn(&ctx, &mut a, vec![]);
    let shown = picked(&texts, is_note, false);
    assert_eq!(shown.first(), all.first(), "{shown:?}");
    let Some(first) = texts.iter().find(|(t, _)| t.lines().next() == Some(all[0].as_str())).map(|(_, p)| *p + egui::vec2(10.0, 30.0)) else {
        panic!("{texts:?}")
    };
    click(&ctx, &mut a, first);
    assert!(a.ui.note_edit.as_ref().is_some_and(|(u, _)| u == "note-0"), "{:?}", a.ui.note_edit);
    scroll(&ctx, &mut a, is_note, false, 8_000.0);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(!texts.iter().any(|(t, _)| t.starts_with("Note 00000")), "scrolled away");
    let _ = drawn(&ctx, &mut a, vec![Event::Text("!".into())]);
    assert!(a.ui.note_edit.as_ref().is_some_and(|(_, text)| text.ends_with('!')), "{:?}", a.ui.note_edit);
}

/// What People, Tasks and Notes cost a frame with 5,000 contacts, 3,000 tasks and 1,000 notes, at
/// the top and scrolled, and a fingerprint of what each draws (the same before and after a change
/// that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_long_lists -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_long_lists() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-lists");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    big_lists(&mut a, 5000, 3000, 1000);
    let is_task = |t: &str| t.starts_with("Task 0");
    let is_note = |t: &str| t.starts_with("Note 0");
    // (The List view's names start left of x = 300: scrolled over its companies, not the
    // column's heading.)
    let is_company = |t: &str| t.strip_prefix("Company ").is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()));
    type Pick = fn(&str) -> bool;
    let cases: [(&str, &str, Pick, bool); 5] = [
        ("people", "people", a_name, true),
        ("cards", "people", a_name, false),
        ("list by company", "people", is_company, false),
        ("tasks", "tasks", is_task, false),
        ("notes", "notes", is_note, false),
    ];
    for (case, module, is, left) in cases {
        a.run("view.module", json!({"name": module}));
        a.ui.people_view = match case {
            "cards" => crate::people::PeopleView::Cards,
            "list by company" => crate::people::PeopleView::Phone,
            _ => crate::people::PeopleView::People,
        };
        a.ui.people_list.sort = Some(crate::people::Column::Company);
        for place in ["at the top", "scrolled"] {
            if place == "scrolled" {
                scroll(&ctx, &mut a, is, left, 18_000.0);
            } else {
                let _ = drawn(&ctx, &mut a, vec![]);
            }
            let texts = drawn(&ctx, &mut a, vec![]);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (t, p) in &texts {
                (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
            }
            let t = std::time::Instant::now();
            for _ in 0..20 {
                let _ = drawn(&ctx, &mut a, vec![]);
            }
            let each = t.elapsed().as_secs_f64() * 1000.0 / 20.0;
            println!("MEASURE {case}, {place}: {each:.2} ms a frame; drawn {} texts, {:016x}", texts.len(), h.finish());
        }
    }
}
