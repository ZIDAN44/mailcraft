//! People, Tasks, Notes and the event, task and contact forms as they read (audit C12 to C20):
//! rows with room, one place for a filter, a note picked before it's written on, grouped fields,
//! categories from a drop-down, the event form's details where they belong.

use egui::accesskit::Role;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

fn settled(name: &str, module: &str) -> (MailApp, egui::Context) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.animations = false;
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": module}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, ctx)
}

/// C12: a person's row in the side list has room for two lines (it was 40 points).
#[test]
fn peoples_rows_have_room() {
    let (mut a, ctx) = settled("people-rows", "people");
    let texts = drawn_text(&ctx, &mut a, SIZE);
    let y = |name: &str| texts.iter().find(|d| d.text == name).map(|d| d.rect.top()).unwrap_or_else(|| panic!("{name}"));
    assert!(y("Jun Watanabe") - y("Ana Lima") >= 52.0 + 20.0, "a row and a letter's heading apart");
}

/// C13: hiding what's done has one place, the side pane's Active tasks (a row across it); the
/// ribbon's Hide completed is gone; a priority is drawn, not a "!".
#[test]
fn tasks_have_one_filter() {
    let (mut a, ctx) = settled("tasks-filter", "tasks");
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(!texts.iter().any(|t| t == "Hide completed" || t == "!"), "{texts:?}");
    let rows = named(&ctx, &mut a, SIZE);
    let active = rows.iter().find(|n| n.label.as_deref() == Some("Active tasks")).and_then(|n| n.rect).expect("Active tasks");
    assert!(active.width() > 150.0, "across the side pane: {active:?}");
    click_named(&ctx, &mut a, "Active tasks");
    assert!(a.session.prefs.tasks_hide_done);
}

/// C14: a click picks a note (Delete and Color take it), a double click writes on it; the side
/// pane is the strip alone.
#[test]
fn a_note_is_picked_then_written_on() {
    let (mut a, ctx) = settled("notes-pick", "notes");
    let texts = drawn_text(&ctx, &mut a, SIZE);
    let note = texts.iter().find(|d| d.text.starts_with("Festival poster")).map(|d| d.rect.center()).expect("a note");
    let off =
        |ctx: &egui::Context, a: &mut MailApp| named(ctx, a, SIZE).iter().find(|n| n.label.as_deref() == Some("Delete")).is_none_or(|n| n.disabled);
    assert!(off(&ctx, &mut a), "nothing picked");
    click(&ctx, &mut a, note);
    assert!(a.ui.note_picked.is_some() && a.ui.note_edit.is_none(), "picked, not written on");
    assert!(!off(&ctx, &mut a), "Delete takes it");
    click(&ctx, &mut a, note);
    assert!(a.ui.note_edit.is_some(), "a double click writes on it");
    assert!(!named(&ctx, &mut a, SIZE).iter().any(|n| n.label.as_deref() == Some("Show the folder pane")), "no pane to open");
}

/// C15 and C16: the contact form shows one email and "+ Add email" for another; a task's
/// categories are picked from Categorize, not a row of pills.
#[test]
fn forms_ask_for_what_they_need() {
    let (mut a, ctx) = settled("forms-ask", "people");
    a.ui.dialog = Some(Dialog::new_contact());
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(texts.iter().any(|t| t == "Email") && !texts.iter().any(|t| t == "Email 2"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Name and work"), "in groups");
    click_named(&ctx, &mut a, "+ Add email");
    frame(&ctx, &mut a, vec![]);
    assert!(drawn_strings(&ctx, &mut a, SIZE).iter().any(|t| t == "Email 2"), "another, asked for");
    a.run("view.module", json!({"name": "tasks"}));
    a.ui.dialog = Some(Dialog::new_task(&a.session, None));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(texts.iter().any(|t| t == "Categorize") && !texts.iter().any(|t| t == "● Important"), "{texts:?}");
}

/// C20: All day sits on the Starts row, Private is a checkbox.
#[test]
fn the_event_forms_details_are_where_they_belong() {
    let (mut a, ctx) = settled("event-details", "calendar");
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_text(&ctx, &mut a, SIZE);
    let y = |w: &str| texts.iter().find(|d| d.text == w).map(|d| d.rect.center().y).unwrap_or_else(|| panic!("{w}"));
    // (The day view behind it has an "All day" of its own.)
    let starts = y("Starts");
    assert!(texts.iter().any(|d| d.text == "All day" && (d.rect.center().y - starts).abs() < 6.0), "on the Starts row");
    let private = named(&ctx, &mut a, SIZE).into_iter().find(|n| n.label.as_deref() == Some("Private"));
    assert!(private.is_some_and(|n| n.role == Role::CheckBox), "a checkbox");
}
