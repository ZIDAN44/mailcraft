//! Calendar tests: the calendar, people, tasks and notes, and online meetings in the event form.

use super::*;

mod dates;
mod day_view;
mod free_busy;
mod lists;
mod meetings;
mod moving;
mod pim;
mod reminders;
mod repeat;
mod ribbon;
mod schedule;
mod search;
mod subscribed;
mod tasks;
mod todo_bar;

/// A link reads as one, in the app colour: the colour the theme forced on all text took links'
/// too, and "Subscribe to a calendar…" looked like a line of plain text.
#[test]
fn links_are_drawn_in_the_app_colour() {
    let (mut a, _) = app("links-coloured");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "calendar"}));
    let mut out = sized_frame(&ctx, &mut a, egui::vec2(1280.0, 800.0), vec![]);
    for _ in 0..2 {
        out = sized_frame(&ctx, &mut a, egui::vec2(1280.0, 800.0), vec![]);
    }
    let color = out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text == "Subscribe to a calendar…" => {
            let own = t.galley.job.sections.first().map(|x| x.format.color).filter(|c| *c != egui::Color32::PLACEHOLDER);
            Some(t.override_text_color.or(own).unwrap_or(t.fallback_color))
        }
        _ => None,
    });
    assert_eq!(color, Some(a.ui.tokens.accent));
}
