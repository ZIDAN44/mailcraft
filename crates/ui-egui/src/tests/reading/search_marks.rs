//! Issue #173: what a search found, marked: in the open message's text (formatted or plain) and
//! subject, and in the list's subjects and lines of text; nothing with no search on.

use egui::Color32;

use super::*;

/// The texts a frame draws on a highlight: the sections of its texts with a background.
fn on_highlight(out: &egui::FullOutput) -> Vec<String> {
    let mut found = Vec::new();
    for s in &out.shapes {
        if let egui::Shape::Text(t) = &s.shape {
            let job = &t.galley.job;
            for sec in job.sections.iter().filter(|sec| sec.format.background != Color32::TRANSPARENT) {
                found.extend(job.text.get(sec.byte_range.start.0..sec.byte_range.end.0).map(str::to_string));
            }
        }
    }
    found
}

fn marked(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let size = egui::vec2(1280.0, 900.0);
    for _ in 0..3 {
        sized_frame(ctx, a, size, vec![]);
    }
    on_highlight(&sized_frame(ctx, a, size, vec![]))
}

/// A search run to its end, as the window runs it a frame at a time.
fn search(ctx: &egui::Context, a: &mut MailApp, query: &str) {
    a.run("search.run", json!({"query": query}));
    for _ in 0..200 {
        if !a.session.list.is_empty() && a.session.search_progress().is_none_or(|p| p.done) {
            return;
        }
        frame(ctx, a, vec![]);
    }
    panic!("the search for {query} never ended");
}

fn open(a: &mut MailApp, subject: &str) {
    let k = a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap_or_else(|| panic!("{subject} listed"));
    a.run("list.open", json!({"key": k.to_id()}));
}

#[test]
fn what_a_search_found_is_marked() {
    let (mut a, _) = app("search-marks");
    let ctx = egui::Context::default();
    open(&mut a, "Storyboard review on Thursday");
    assert!(marked(&ctx, &mut a).is_empty(), "no search on: nothing marked");

    // A word of its text: there, and in the list's line of it.
    search(&ctx, &mut a, "animatic");
    open(&mut a, "Storyboard review on Thursday");
    let found = marked(&ctx, &mut a);
    assert!(found.iter().filter(|f| f.eq_ignore_ascii_case("animatic")).count() >= 2, "the text and the list's line: {found:?}");
    assert!(found.iter().all(|f| f.eq_ignore_ascii_case("animatic")), "only what was found: {found:?}");

    // A word's start, in the subject (the reading pane's and the list's), as far as it was typed.
    search(&ctx, &mut a, "storyb");
    open(&mut a, "Storyboard review on Thursday");
    let found = marked(&ctx, &mut a);
    assert!(found.iter().filter(|f| *f == "Storyb").count() >= 2, "the subject, in the pane and the list: {found:?}");

    // Read as plain text, the same.
    a.session.prefs.show_html = false;
    search(&ctx, &mut a, "animatic");
    open(&mut a, "Storyboard review on Thursday");
    let found = marked(&ctx, &mut a);
    assert!(found.iter().any(|f| f == "animatic"), "plain text: {found:?}");
    a.ui.dark = Some(true);
    assert!(marked(&ctx, &mut a).iter().any(|f| f == "animatic"), "in the dark theme too");
    a.ui.dark = Some(false);

    // The search over: nothing marked.
    a.run("search.clear", json!({}));
    open(&mut a, "Storyboard review on Thursday");
    assert!(marked(&ctx, &mut a).is_empty());
}
