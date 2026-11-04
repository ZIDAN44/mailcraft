//! What a screen reader is told: every control that can be clicked has a role and a name.

use egui::accesskit::{Action, Node, Role};

use super::*;

/// The controls a screen reader would read as nameless or as "unknown", in a frame drawn with
/// AccessKit on: `role: name` for each, nameless ones as `role: ?`.
fn unnamed(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    let name = |n: &Node| n.label().filter(|l| !l.trim().is_empty()).or(n.value()).map(str::to_string);
    tree.nodes
        .iter()
        // egui's own scroll bars are named by what they scroll.
        .filter(|(_, n)| n.supports_action(Action::Click) && n.role() != Role::ScrollBar)
        .filter(|(_, n)| n.role() == Role::Unknown || (name(n).is_none() && n.labelled_by().is_empty()))
        .map(|(_, n)| format!("{:?}: {} at {:?}", n.role(), name(n).unwrap_or_else(|| "?".into()), n.bounds()))
        .collect()
}

/// The main window (ribbon, folders, list, reading pane), a message being written and the File
/// page, light and dark, simplified and classic ribbon: no control a screen reader can't name
/// (wordcraft's ribbon fields taught this: drawn buttons say nothing unless told).
#[test]
fn every_control_says_what_it_is() {
    let (mut a, dir) = app("access");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut seen = Vec::new();
    let mut check = |a: &mut MailApp, what: &str| {
        // The first frames lay out (the ribbon fits, panes settle); the last is read.
        for _ in 0..3 {
            frame(&ctx, a, vec![]);
        }
        for u in unnamed(&ctx, a) {
            seen.push(format!("{what}: {u}"));
        }
    };
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    for classic in [false, true] {
        a.run("view.classicRibbon", json!({"on": classic}));
        for tab in [crate::RibbonTab::Home, crate::RibbonTab::SendReceive, crate::RibbonTab::View] {
            a.ui.tab = tab;
            check(&mut a, &format!("{tab:?} classic={classic}"));
        }
    }
    a.run("view.classicRibbon", json!({"on": false}));
    a.ui.tab = crate::RibbonTab::Home;
    a.ui.dark = Some(true);
    a.run("mail.reply", json!({}));
    check(&mut a, "reply");
    a.run("mail.new", json!({}));
    check(&mut a, "new message");
    a.run("ui.file", json!({}));
    check(&mut a, "File page");
    seen.sort();
    seen.dedup();
    assert!(seen.is_empty(), "{} controls a screen reader can't name:\n{}", seen.len(), seen.join("\n"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A question that must be answered is read as an alert, by its title; a form is a dialog
/// (wordcraft's save prompt taught this).
#[test]
fn a_question_is_read_as_an_alert_by_its_title() {
    let (mut a, dir) = app("access-alert");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for (d, role, title) in [
        (Dialog::ConfirmDelete { count: 2 }, Role::AlertDialog, "Delete permanently?"),
        (Dialog::SaveOnClose(1), Role::AlertDialog, "Save the message you're writing?"),
        (Dialog::MoveTo { filter: String::new() }, Role::Dialog, "Move to folder"),
    ] {
        a.ui.dialog = Some(d);
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
        // The dialog's own node, the one modal node with its role: egui gives the area of every
        // `Modal` the Dialog role too (`UiKind::Modal`), not modal, and the map's order puts
        // either first.
        let modal: Vec<_> = nodes.iter().filter(|(_, n)| n.role() == role && n.is_modal()).map(|(_, n)| n.clone()).collect();
        assert_eq!(
            modal.len(),
            1,
            "one modal {role:?} for {title}: {:?}",
            nodes.iter().map(|(_, n)| n.role()).collect::<std::collections::BTreeSet<_>>()
        );
        let dialog = &modal[0];
        let named: Vec<String> = dialog
            .labelled_by()
            .iter()
            .filter_map(|id| nodes.iter().find(|(n, _)| n == id))
            .filter_map(|(_, n)| n.value().or(n.label()).map(str::to_string))
            .collect();
        assert_eq!(named, vec![title.to_string()]);
        a.ui.dialog = None;
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Every update the window sends is one the system's screen-reader adapter takes: its focus is
/// among its nodes. The search box holds the keyboard, then the File page covers it (the box isn't
/// drawn), then a message is written and the page goes: each frame's update is fed to the
/// consumer the adapters run, which panics, closing the window, on one that breaks the rule
/// (wordcraft #2726).
#[test]
fn every_update_keeps_its_focus_among_its_nodes() {
    struct Quiet;
    impl accesskit_consumer::TreeChangeHandler for Quiet {
        fn node_added(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
        fn node_updated(&mut self, _: &accesskit_consumer::NodeRef<'_>, _: &accesskit_consumer::NodeRef<'_>) {}
        fn focus_moved(&mut self, _: Option<&accesskit_consumer::NodeRef<'_>>, _: Option<&accesskit_consumer::NodeRef<'_>>) {}
        fn node_removed(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
    }
    let (mut a, dir) = app("access-focus");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut tree: Option<accesskit_consumer::Tree> = None;
    let mut step = |a: &mut MailApp, events: Vec<Event>| {
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let Some(update) = out.platform_output.accesskit_update else { return };
        assert!(update.nodes.iter().any(|(id, _)| *id == update.focus), "the focus is among the update's nodes");
        tree = Some(match tree.take() {
            None => accesskit_consumer::Tree::new(update, true),
            Some(mut t) => {
                t.update_and_process_changes(update, &mut Quiet);
                t
            }
        });
    };
    for _ in 0..3 {
        step(&mut a, vec![]);
    }
    a.act(crate::Action::FocusSearch);
    step(&mut a, vec![]);
    step(&mut a, vec![Event::Text("budget".into())]);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    step(&mut a, vec![]);
    step(&mut a, vec![]);
    a.ui.backstage = None;
    a.run("mail.new", json!({}));
    step(&mut a, vec![Event::Text("Hello".into())]);
    a.run("compose.discard", json!({}));
    step(&mut a, vec![]);
    let tree = tree.expect("the window sends AccessKit updates");
    assert!(tree.state().has_node(tree.state().focus_id_in_tree()));
    let _ = std::fs::remove_dir_all(dir);
}

/// Esc in a File page field leaves the field, and only the next Esc leaves the File page (as
/// GridCraft's): it used to close the whole page while typing, the edit with it.
#[test]
fn esc_leaves_a_file_page_field_before_the_page() {
    let (mut a, dir) = app("esc-backstage");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let mut field = None;
    for _ in 0..4 {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 2400.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
        field = nodes.iter().find(|(_, n)| n.role() == Role::TextInput && !n.is_disabled()).and_then(|(_, n)| n.bounds());
    }
    let r = field.expect("the Options page has a text field");
    let at = egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32);
    let tall = |events: Vec<Event>| RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 2400.0))),
        events,
        ..Default::default()
    };
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
        let mut out = ctx.run_ui(tall(events), |ui| a.ui(ui));
        out.textures_delta.clear();
    }
    assert!(ctx.text_edit_focused(), "the field has the keyboard");
    let mut out = ctx.run_ui(tall(vec![key(Key::Escape, Modifiers::NONE)]), |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(!ctx.text_edit_focused(), "Esc left the field");
    assert!(a.ui.backstage.is_some(), "and the File page is still open");
    for events in [vec![], vec![key(Key::Escape, Modifiers::NONE)]] {
        let mut out = ctx.run_ui(tall(events), |ui| a.ui(ui));
        out.textures_delta.clear();
    }
    assert!(a.ui.backstage.is_none(), "the next Esc leaves the File page");
    let _ = std::fs::remove_dir_all(dir);
}
