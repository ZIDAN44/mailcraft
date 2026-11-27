//! The message list and the reading pane as they read: a conversation's count, the follow-up
//! bar, the two headings' line, what a search found, the single-line list's columns, and To's
//! hint (audit M16 to M23).

use super::*;

/// The sample's Inbox, a frame drawn, `cmds` run first.
fn shown(name: &str, cmds: &[(&str, serde_json::Value)]) -> (MailApp, egui::Context) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    for (id, p) in cmds {
        a.run(id, p.clone());
    }
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, ctx)
}

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// M16: a conversation's count is a badge after the sender, not "(2)" written into the name; the
/// reading pane lists its messages without a box of their own round them.
#[test]
fn a_conversations_count_is_a_badge() {
    let (mut a, ctx) = shown("count-badge", &[("view.conversations", json!({}))]);
    let texts = drawn_text(&ctx, &mut a, SIZE);
    assert!(texts.iter().any(|d| d.text == "Sam Okafor"), "the name alone");
    assert!(!texts.iter().any(|d| d.text.contains("(2)")), "no count in brackets: {:?}", texts.iter().map(|d| &d.text).collect::<Vec<_>>());
    let name = texts.iter().find(|d| d.text == "Sam Okafor").map(|d| d.rect).unwrap();
    assert!(
        texts.iter().any(|d| d.text == "2" && (d.rect.center().y - name.center().y).abs() < 4.0 && d.rect.left() > name.right()),
        "a badge after it"
    );
}

/// M19 and M20: the follow-up bar has no full stops; the reading pane's subject is on the list's
/// title's line.
#[test]
fn the_follow_up_bar_and_the_headings_line() {
    let (mut a, ctx) = shown("follow-up-bar", &[("list.open", json!({"key": "sample:1:1:INBOX"}))]);
    let texts = drawn_text(&ctx, &mut a, SIZE);
    assert!(texts.iter().any(|d| d.text.starts_with("Follow up")), "{:?}", texts.iter().map(|d| &d.text).collect::<Vec<_>>());
    assert!(!texts.iter().any(|d| d.text.starts_with("Follow up") && d.text.ends_with('.')), "no full stop");
    // The list's title: the biggest "Inbox" (the folder pane has one too).
    let title =
        texts.iter().filter(|d| d.text == "Inbox").map(|d| d.rect).max_by(|x, y| x.height().total_cmp(&y.height())).expect("the list's title");
    let subject = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default();
    // The reading pane's: the biggest (the list's row has it too).
    let heading = texts.iter().filter(|d| d.text == subject).map(|d| d.rect).max_by(|x, y| x.height().total_cmp(&y.height())).expect("the subject");
    assert!((heading.top() - title.top()).abs() <= 2.0, "one line: the list's title at {}, the subject at {}", title.top(), heading.top());
}

/// M21: what a search found in a subject is tinted, its words in their own colour (black on solid
/// yellow, it was).
#[test]
fn a_search_hit_keeps_the_texts_colour() {
    let job =
        crate::reading::marked_job("Storyboard review", &[std::ops::Range { start: 0, end: 10 }], crate::theme::regular(14.0), egui::Color32::WHITE);
    // The second section: the text before the hit is empty, then the hit.
    let hit = job.sections.iter().find(|s| s.format.background != egui::Color32::TRANSPARENT).expect("the hit");
    assert_eq!(hit.format.color, egui::Color32::WHITE, "its own colour");
    assert!(hit.format.background.a() < 255, "a tint, not solid: {:?}", hit.format.background);
}

/// M18: on one line, the time is a column of its own: a file's clip before it lines up down the
/// list, whatever the time's width ("02:12", "Fri 21:37").
#[test]
fn the_single_line_lists_markers_line_up() {
    let (mut a, ctx) = shown("single-line-columns", &[("view.singleLine", json!({}))]);
    let out = sized_frame(&ctx, &mut a, SIZE, vec![]);
    let texts = texts_in(&out.shapes);
    // The times of the rows with a file, as the list writes them.
    let with_files: Vec<String> = a
        .session
        .list
        .iter()
        .filter(|m| m.has_attachments)
        .map(|m| crate::i18n::date(&mailcraft_engine::model::dates::list_time(m.when(), &a.session.clock)))
        .collect();
    // Each row with a file: the rightmost small drawing left of its time.
    let clips: Vec<f32> = texts
        .iter()
        .filter(|d| with_files.contains(&d.text))
        .filter_map(|time| {
            let row = time.rect.center().y;
            out.shapes
                .iter()
                .filter(|s| !matches!(s.shape, egui::Shape::Text(_)))
                .map(|s| s.shape.visual_bounding_rect())
                .filter(|r| {
                    r.is_finite() && r.width() < 16.0 && r.height() < 16.0 && (r.center().y - row).abs() < 6.0 && r.right() < time.rect.left()
                })
                .map(|r| r.right())
                .reduce(f32::max)
        })
        .collect();
    assert!(clips.len() >= 2, "rows with a file: {clips:?}");
    let (lo, hi) = clips.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), x| (l.min(*x), h.max(*x)));
    // Within a clip's and a flag's own drawing (the times' widths put them 30 apart).
    assert!(hi - lo <= 2.5, "clips at {clips:?}");
}

/// M23: To says what to do ("Add recipients"), not how addresses are written.
#[test]
fn to_says_add_recipients() {
    let (mut a, ctx) = shown("to-hint", &[("mail.new", json!({}))]);
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(texts.iter().any(|t| t == "Add recipients"), "{texts:?}");
}
