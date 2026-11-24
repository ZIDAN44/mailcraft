//! Plain text out of the way: not a link over the message's text, where it was clicked by
//! mistake, but in the message's More actions (for that message), View › Plain text and File ›
//! Options (for all mail, where Outlook keeps reading all mail as plain text), with the way back
//! to one message's formatting above its text.

use super::*;

/// A screen tall enough for File › Options' Reading group without scrolling.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 2000.0);

fn shows(ctx: &egui::Context, a: &mut MailApp, label: &str) -> bool {
    named(ctx, a, SCREEN).iter().any(|n| n.name() == Some(label))
}

/// Click what a screen reader names `label` (the last of that name: the reading pane's, below
/// the ribbon's).
fn click_last(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let all = named(ctx, a, SCREEN);
    let at = all.iter().filter(|n| n.name() == Some(label)).filter_map(Named::centre).next_back();
    let at = at.unwrap_or_else(|| panic!("no {label} to click: {:?}", all.iter().filter_map(Named::name).collect::<Vec<_>>()));
    click(ctx, a, at);
}

#[test]
fn plain_text_is_in_more_actions_the_view_ribbon_and_options() {
    let _tall = windows::layout::Dressed::new(SCREEN, "");
    let (mut a, _) = app("plain-text");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    // Formatted, with nothing over its text to turn it plain.
    assert!(a.session.prefs.show_html);
    assert!(shows(&ctx, &mut a, "Tracking pictures, never loaded: 1 · Links that report clicks: 1"), "the HTML bar is drawn");
    assert!(!shows(&ctx, &mut a, "Show as plain text"), "no plain-text link over the message");
    assert!(!shows(&ctx, &mut a, "Show formatted"));

    // The message's More actions (⋯) has it, for that message: the setting stays.
    click_last(&ctx, &mut a, "More actions");
    assert!(shows(&ctx, &mut a, "Show as plain text"), "in More actions");
    click_last(&ctx, &mut a, "Show as plain text");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.show_html, "one message's More actions left the setting for all mail");
    // Shown as text, said, with the way back above the text.
    assert!(shows(&ctx, &mut a, "Shown as text."), "More actions turned it plain");
    click_last(&ctx, &mut a, "Show formatted");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.show_html);
    assert!(!shows(&ctx, &mut a, "Shown as text."), "Show formatted went back");

    // View › Plain text, both ways.
    a.ui.tab = crate::RibbonTab::View;
    frame(&ctx, &mut a, vec![]);
    click_last(&ctx, &mut a, "Plain text");
    assert!(!a.session.prefs.show_html, "View › Plain text");
    click_last(&ctx, &mut a, "Plain text");
    assert!(a.session.prefs.show_html, "View › Plain text, again");

    // File › Options: read all mail as plain text.
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    // Its switch: right of its label.
    let at = named(&ctx, &mut a, SCREEN)
        .iter()
        .filter(|n| n.name() == Some("Read all mail as plain text"))
        .filter_map(Named::centre)
        .max_by(|p, q| p.x.total_cmp(&q.x));
    click(&ctx, &mut a, at.expect("the Options switch is shown"));
    assert!(!a.session.prefs.show_html, "the Options switch");

    // All mail as plain text: Show formatted above one message shows that one formatted, and
    // the setting stays on (as Outlook's Display as HTML does).
    a.ui.backstage = None;
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(shows(&ctx, &mut a, "Shown as text."), "read as plain text");
    click_last(&ctx, &mut a, "Show formatted");
    frame(&ctx, &mut a, vec![]);
    assert!(!shows(&ctx, &mut a, "Shown as text."), "this message formatted");
    assert!(!a.session.prefs.show_html, "Show formatted left the setting for all mail on");
    // Another message is still read as plain text.
    let other = a.session.list.iter().find(|m| m.key != key && m.subject != "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    assert!(a.ui.shown_as.formatted(&key, false) && !a.ui.shown_as.formatted(&other, false), "only that one");

    // The command keeps its id, for the CLI and --cmd.
    a.run("view.html", json!({"on": true}));
    assert!(a.session.prefs.show_html);
}

/// A message in a window of its own can be shown as plain text too: its toolbar's More actions
/// (⋯) has it (the reading pane's buttons aren't drawn there).
#[test]
fn a_message_window_can_be_shown_as_plain_text() {
    let _tall = windows::layout::Dressed::new(SCREEN, "");
    let (a, _) = app("plain-text-window");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened)));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let key = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    a.show_message_page(&key.to_id());
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(!shows(&ctx, &mut a, "Shown as text."));
    click_last(&ctx, &mut a, "More actions");
    assert!(shows(&ctx, &mut a, "View message source"), "the window's More actions");
    click_last(&ctx, &mut a, "Show as plain text");
    frame(&ctx, &mut a, vec![]);
    assert!(shows(&ctx, &mut a, "Shown as text."), "the window's message as plain text");
    assert!(a.session.prefs.show_html, "for that message alone");
    click_last(&ctx, &mut a, "Show formatted");
    frame(&ctx, &mut a, vec![]);
    assert!(!shows(&ctx, &mut a, "Shown as text."), "and back");
}
