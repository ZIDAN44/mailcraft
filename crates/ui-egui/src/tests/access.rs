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
        let dialog = nodes.iter().find(|(_, n)| n.role() == role).map(|(_, n)| n.clone()).unwrap_or_else(|| {
            panic!("no {role:?} for {title}: {:?}", nodes.iter().map(|(_, n)| n.role()).collect::<std::collections::BTreeSet<_>>())
        });
        assert!(dialog.is_modal(), "{title}");
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
