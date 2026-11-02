//! Plain text out of the way: not a link over the message's text, where it was clicked by
//! mistake, but in the message's More actions, View › Plain text and File › Options (where
//! Outlook keeps reading all mail as plain text), with the way back above the text.

use super::*;

/// A screen tall enough for File › Options' Reading group without scrolling.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 1400.0);

fn tall(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// What a screen reader is told is on screen: each node's name and where it is.
fn nodes(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, Option<egui::Pos2>)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let name = n.label().or(n.value())?.to_string();
            let at = n.bounds().map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
            Some((name, at))
        })
        .collect()
}

fn shows(ctx: &egui::Context, a: &mut MailApp, label: &str) -> bool {
    nodes(ctx, a).iter().any(|(n, _)| n == label)
}

/// Click what a screen reader names `label` (the last of that name: the reading pane's, below
/// the ribbon's).
fn click_named(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let all = nodes(ctx, a);
    let at = all.iter().filter(|(n, _)| n == label).filter_map(|(_, at)| *at).next_back();
    let at = at.unwrap_or_else(|| panic!("no {label} to click: {:?}", all.iter().map(|(n, _)| n).collect::<Vec<_>>()));
    press(ctx, a, at);
}

fn press(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    tall(ctx, a, vec![Event::PointerMoved(at)]);
    tall(ctx, a, vec![button(true)]);
    tall(ctx, a, vec![button(false)]);
}

#[test]
fn plain_text_is_in_more_actions_the_view_ribbon_and_options() {
    let (mut a, dir) = app("plain-text");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    tall(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        tall(&ctx, &mut a, vec![]);
    }
    // Formatted, with nothing over its text to turn it plain.
    assert!(a.session.prefs.show_html);
    assert!(shows(&ctx, &mut a, "Tracking pictures, never loaded: 1 · Links that report clicks: 1"), "the HTML bar is drawn");
    assert!(!shows(&ctx, &mut a, "Show as plain text"), "no plain-text link over the message");
    assert!(!shows(&ctx, &mut a, "Show formatted"));

    // The message's More actions (⋯) has it.
    click_named(&ctx, &mut a, "More actions");
    assert!(shows(&ctx, &mut a, "Show as plain text"), "in More actions");
    click_named(&ctx, &mut a, "Show as plain text");
    tall(&ctx, &mut a, vec![]);
    assert!(!a.session.prefs.show_html, "More actions turned it plain");
    // Shown as text, said, with the way back above the text.
    assert!(shows(&ctx, &mut a, "Shown as text."));
    click_named(&ctx, &mut a, "Show formatted");
    tall(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.show_html, "Show formatted went back");

    // View › Plain text, both ways.
    a.ui.tab = crate::RibbonTab::View;
    tall(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Plain text");
    assert!(!a.session.prefs.show_html, "View › Plain text");
    click_named(&ctx, &mut a, "Plain text");
    assert!(a.session.prefs.show_html, "View › Plain text, again");

    // File › Options: read all mail as plain text.
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    for _ in 0..2 {
        tall(&ctx, &mut a, vec![]);
    }
    // Its switch: right of its label.
    let at = nodes(&ctx, &mut a)
        .into_iter()
        .filter(|(n, _)| n == "Read all mail as plain text")
        .filter_map(|(_, at)| at)
        .max_by(|p, q| p.x.total_cmp(&q.x));
    press(&ctx, &mut a, at.expect("the Options switch is shown"));
    assert!(!a.session.prefs.show_html, "the Options switch");

    // The command keeps its id, for the CLI and --cmd.
    a.run("view.html", json!({"on": true}));
    assert!(a.session.prefs.show_html);
    let _ = std::fs::remove_dir_all(dir);
}
