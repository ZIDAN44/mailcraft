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
    let (mut a, _) = app("access");
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
}

/// The drop-downs a frame in a window of `size` tells a screen reader of: each one's name (its
/// own label, else the text of the label it's labelled by; never its value) and its value.
fn drop_downs(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<(Option<String>, Option<String>)> {
    let out = sized_frame(ctx, a, size, vec![]);
    let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
    let text = |n: &Node| n.label().or(n.value()).filter(|l| !l.trim().is_empty()).map(str::to_string);
    let named = |n: &Node| {
        n.label().filter(|l| !l.trim().is_empty()).map(str::to_string).or_else(|| {
            let by: Vec<String> = n.labelled_by().iter().filter_map(|id| nodes.iter().find(|(x, _)| x == id)).filter_map(|(_, l)| text(l)).collect();
            (!by.is_empty()).then(|| by.join(" "))
        })
    };
    nodes.iter().filter(|(_, n)| n.role() == Role::ComboBox).map(|(_, n)| (named(n), n.value().map(str::to_string))).collect()
}

/// Every drop-down is announced by what it's for, not only by what it shows (DeckCraft's
/// drop-downs taught this): the shown text was its value and it had no name, so a screen reader
/// read "combo box, Move to the folder" with nothing saying what was being chosen. The main window
/// with a message being written, a rule (both its folder and its category rows), File › Options
/// and an event.
#[test]
fn every_drop_down_says_what_it_is_for() {
    let (mut a, _) = app("access-drop-downs");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut nameless = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    let mut check = |a: &mut MailApp, what: &str, size: egui::Vec2| {
        let _size = windows::layout::Dressed::new(size, "");
        // The first frames lay out (the ribbon fits, the dialog settles); the last is read.
        for _ in 0..3 {
            frame(&ctx, a, vec![]);
        }
        let boxes = drop_downs(&ctx, a, size);
        assert!(!boxes.is_empty(), "{what}: no drop-downs drawn");
        for (name, value) in boxes {
            match name {
                Some(n) => {
                    names.insert(n);
                }
                None => nameless.push(format!("{what}: showing {value:?}")),
            }
        }
    };
    let wide = egui::vec2(1280.0, 800.0);
    a.run("mail.new", json!({}));
    check(&mut a, "new message", wide);
    a.run("compose.discard", json!({}));
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example"));
    for action in [crate::dialogs::RuleDo::MoveTo, crate::dialogs::RuleDo::Categorize] {
        if let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() {
            f.action = action;
            f.target.clear();
        }
        check(&mut a, &format!("rule {action:?}"), wide);
    }
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    check(&mut a, "event", wide);
    a.ui.dialog = None;
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    check(&mut a, "File › Options", egui::vec2(1280.0, 2400.0));
    assert!(nameless.is_empty(), "{} drop-downs a screen reader can't name:\n{}", nameless.len(), nameless.join("\n"));
    for want in ["Font size", "When mail", "Then", "Folder", "Category", "Repeat", "Reminder", "Show as", "Dates", "Language"] {
        assert!(names.contains(want), "no drop-down named {want}: {names:?}");
    }
}

/// A question that must be answered is read as an alert, by its title; a form is a dialog
/// (wordcraft's save prompt taught this).
#[test]
fn a_question_is_read_as_an_alert_by_its_title() {
    let (mut a, _) = app("access-alert");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for (d, role, title) in [
        (Dialog::ConfirmDelete { count: 2 }, Role::AlertDialog, "Delete permanently?"),
        (Dialog::SaveOnClose(1), Role::AlertDialog, "Save the message you're writing?"),
        (Dialog::MoveTo { filter: String::new(), copy: false }, Role::Dialog, "Move to folder"),
        (Dialog::MoveTo { filter: String::new(), copy: true }, Role::Dialog, "Copy to folder"),
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
    let (mut a, _) = app("access-focus");
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
}

/// Esc in a File page field leaves the field, and only the next Esc leaves the File page (as
/// GridCraft's): it used to close the whole page while typing, the edit with it.
#[test]
fn esc_leaves_a_file_page_field_before_the_page() {
    let (mut a, _) = app("esc-backstage");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tall = egui::vec2(1280.0, 2400.0);
    let _tall = windows::layout::Dressed::new(tall, "");
    let mut field = None;
    for _ in 0..4 {
        let nodes = named(&ctx, &mut a, tall);
        field = nodes.into_iter().find(|n| n.role == Role::TextInput && !n.disabled).and_then(|n| n.centre());
    }
    click(&ctx, &mut a, field.expect("the Options page has a text field"));
    assert!(ctx.text_edit_focused(), "the field has the keyboard");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!ctx.text_edit_focused(), "Esc left the field");
    assert!(a.ui.backstage.is_some(), "and the File page is still open");
    for events in [vec![], vec![key(Key::Escape, Modifiers::NONE)]] {
        frame(&ctx, &mut a, events);
    }
    assert!(a.ui.backstage.is_none(), "the next Esc leaves the File page");
}

/// Issue #52: a message whose reply was left in Drafts is read as "[Draft]" first, as the list
/// draws it, and Drafts says how many items it holds (never as unread).
#[test]
fn a_reply_left_in_drafts_is_said() {
    let (mut a, _) = app("access-drafts");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let labels = |a: &mut MailApp| named(&ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter_map(|n| n.label).collect::<Vec<_>>();
    let m = a.session.list[0].clone();
    a.run("list.open", json!({"key": m.key.to_id()}));
    a.run("mail.reply", json!({}));
    let body = format!("Soon.{}", a.session.compose.as_ref().unwrap().body);
    a.run("compose.update", json!({"body": body}));
    a.run("compose.close", json!({}));
    for _ in 0..2 {
        labels(&mut a);
    }
    let seen = labels(&mut a);
    let subject = if m.subject.is_empty() { "(no subject)".to_string() } else { m.subject.clone() };
    assert!(seen.iter().any(|l| l.starts_with("[Draft], ") && l.contains(&subject)), "{seen:?}");
    assert!(seen.iter().any(|l| l.starts_with("Drafts, Items: ")), "{seen:?}");
}

/// A ribbon button whose label carries a value (View › Arrange by: Date) shows it translated
/// once: the button took the translation as English and looked it up again (in the
/// pseudo-language, marked twice; a translation that is also an English key, swapped).
#[test]
fn a_ribbon_label_with_a_value_is_translated_once() {
    use crate::i18n::pseudo;
    let (mut a, _) = app("arrange-label");
    let ctx = egui::Context::default();
    a.ui.tab = crate::RibbonTab::View;
    frame(&ctx, &mut a, vec![]);
    let size = egui::vec2(1280.0, 800.0);
    let field = a.session.prefs.arrange.label();
    pseudo::set(true);
    let texts = drawn_strings(&ctx, &mut a, size);
    pseudo::set(false);
    let shown: Vec<&String> = texts.iter().filter(|t| t.contains("Arrange by: ")).collect();
    assert_eq!(shown, [&format!("⟦Arrange by: ⟦{field}⟧⟧")], "{texts:?}");
    // In German, as its catalog writes it (in a window wide enough for every label).
    a.ui.language = "de".into();
    let wide = egui::vec2(2600.0, 800.0);
    for _ in 0..3 {
        let _ = drawn_strings(&ctx, &mut a, wide);
    }
    let texts = drawn_strings(&ctx, &mut a, wide);
    let want = crate::i18n::tfmt("Arrange by: {field}", &[("field", crate::i18n::t(field))]);
    assert!(want.starts_with("Anordnen nach: ") && texts.contains(&want), "{want}: {texts:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// The empty reading pane says what to do translated once (it translated its caller's
/// translation again: marked twice in the pseudo-language).
#[test]
fn the_empty_reading_pane_is_translated_once() {
    use crate::i18n::pseudo;
    let (mut a, _) = app("empty-reading");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.reading.is_none());
    pseudo::set(true);
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    pseudo::set(false);
    let shown: Vec<&String> = texts.iter().filter(|t| t.contains("Select a message to read it")).collect();
    assert_eq!(shown, ["⟦Select a message to read it⟧"], "{texts:?}");
}
