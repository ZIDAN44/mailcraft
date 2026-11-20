//! UI tests: the folder pane over a big tree of folders (what it draws collapsed and open, a
//! folder's menu, and what a frame costs).

use super::*;

/// `parents` folders `Projects/p{n}` under `Projects`, each with `children` folders `c{m}` in it,
/// put in the sample account.
fn big_tree(a: &mut MailApp, parents: usize, children: usize) {
    let Some(base) = a.session.accounts[0].folders.iter().find(|f| f.path == "Projects").cloned() else {
        panic!("the sample's Projects");
    };
    for n in 0..parents {
        let parent = mailcraft_engine::model::Folder { path: format!("Projects/p{n}"), delimiter: Some('/'), ..base.clone() };
        a.session.accounts[0].folders.push(parent);
        for m in 0..children {
            let child = mailcraft_engine::model::Folder { path: format!("Projects/p{n}/c{m}"), delimiter: Some('/'), ..base.clone() };
            a.session.accounts[0].folders.push(child);
        }
    }
}

/// The folder pane alone, 220 by `height` points: each text drawn that can be seen (not wholly
/// outside its clip), and where.
fn pane(ctx: &egui::Context, a: &mut MailApp, height: f32, events: Vec<Event>) -> Vec<(String, egui::Pos2)> {
    let t = crate::theme::Tokens::light();
    let mut acts = Vec::new();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(220.0, height))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        ui.allocate_ui(egui::vec2(220.0, height), |ui| crate::nav::show(&a.session, &t, ui, false, &mut acts));
    });
    out.textures_delta.clear();
    for act in acts {
        a.act(act);
    }
    texts_in(&out.shapes).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// A collapsed folder hides what's inside it, and the account collapsed hides every folder; a
/// folder's menu says whether it's in Favorites, and Show in Favorites puts it there.
#[test]
fn the_folder_tree_collapses_and_its_menu_knows_favorites() {
    let (mut a, _) = app("folder-tree");
    let ctx = egui::Context::default();
    big_tree(&mut a, 3, 2);
    let names = |texts: &[(String, egui::Pos2)]| {
        texts.iter().map(|(t, _)| t.clone()).filter(|t| t.starts_with('p') || t.starts_with('c')).collect::<Vec<_>>()
    };
    let open = pane(&ctx, &mut a, 2000.0, vec![]);
    assert_eq!(names(&open), ["p0", "c0", "c1", "p1", "c0", "c1", "p2", "c0", "c1"]);
    // p1 collapsed: its folders go, the others stay.
    a.run("view.collapse", json!({"key": "folder:sample/Projects/p1"}));
    assert_eq!(names(&pane(&ctx, &mut a, 2000.0, vec![])), ["p0", "c0", "c1", "p1", "p2", "c0", "c1"]);
    // Projects collapsed: everything in it goes.
    a.run("view.collapse", json!({"key": "folder:sample/Projects"}));
    let texts = pane(&ctx, &mut a, 2000.0, vec![]);
    assert!(names(&texts).is_empty(), "{texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "Projects"));
    a.run("view.collapse", json!({"key": "folder:sample/Projects"}));
    // The account collapsed: no folder under it (Favorites keeps its own).
    a.run("view.collapse", json!({"key": "account:sample"}));
    let texts = pane(&ctx, &mut a, 2000.0, vec![]);
    let under: Vec<&String> =
        texts.iter().map(|(t, _)| t).skip_while(|t| !t.starts_with("riley@")).skip(1).take_while(|t| *t != "Search Folders").collect();
    assert!(under.is_empty(), "{texts:?}");
    a.run("view.collapse", json!({"key": "account:sample"}));
    // A folder's menu: Show in Favorites for one that isn't, which puts it there; Remove from
    // Favorites for one that is.
    let at = |texts: &[(String, egui::Pos2)], name: &str| texts.iter().filter(|(t, _)| t == name).map(|(_, p)| *p + egui::vec2(4.0, 6.0)).next_back();
    for (folder, said) in [("p0", "Show in Favorites"), ("p0", "Remove from Favorites")] {
        let texts = pane(&ctx, &mut a, 2000.0, vec![]);
        let Some(p) = at(&texts, folder) else { panic!("{folder} drawn: {texts:?}") };
        let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
        pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(p)]);
        pane(&ctx, &mut a, 2000.0, vec![button(true)]);
        pane(&ctx, &mut a, 2000.0, vec![button(false)]);
        let menu = pane(&ctx, &mut a, 2000.0, vec![]);
        let Some(item) = at(&menu, said) else { panic!("{said} in the menu: {menu:?}") };
        let click = |pressed| Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(item)]);
        pane(&ctx, &mut a, 2000.0, vec![click(true)]);
        pane(&ctx, &mut a, 2000.0, vec![click(false)]);
        let favorite = a.session.favorites().iter().any(|r| r.folder == "Projects/p0");
        assert_eq!(favorite, said == "Show in Favorites", "after {said}");
    }
}

/// What the folder pane costs a frame with 3,300 folders (300 with ten in each) and ten of them
/// in Favorites: every folder open, every one of the 300 collapsed, and the account collapsed;
/// and a fingerprint of what it draws (the same before and after a change that only makes it
/// faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_folder_pane -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_folder_pane() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-folder-pane");
    let ctx = egui::Context::default();
    big_tree(&mut a, 300, 10);
    for n in 0..10 {
        a.run("folder.favorite", json!({"account": "sample", "folder": format!("Projects/p{}/c0", n * 30), "on": true}));
    }
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            let _ = pane(&ctx, a, 900.0, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    for case in ["every folder open", "the 300 collapsed", "the account collapsed"] {
        match case {
            "the 300 collapsed" => {
                for n in 0..300 {
                    a.run("view.collapse", json!({"key": format!("folder:sample/Projects/p{n}")}));
                }
            }
            "the account collapsed" => a.run("view.collapse", json!({"key": "account:sample"})),
            _ => {}
        }
        let drawn = pane(&ctx, &mut a, 900.0, vec![]);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &drawn {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
        let each = timed(&mut a);
        println!("MEASURE folder pane, 3300 folders, {case}: {each:.3} ms a frame; drawn {} texts, {:016x}", drawn.len(), h.finish());
    }
}

/// Scrolled up past the top of the folder pane with a Mac's trackpad, or a mouse that scrolls
/// smoothly as one does, the pointer left resting on the pane: the folders stay at its top. They
/// were pulled a third of the pane down, and stayed there with blank space above them (#150).
#[test]
fn scrolling_past_the_top_of_a_pane_leaves_no_room_above() {
    let (mut a, _) = app("overscroll");
    // More folders than the pane holds, so that it scrolls.
    big_tree(&mut a, 60, 0);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let inbox = |texts: Vec<Drawn>| texts.into_iter().find(|d| d.text == "Inbox" && d.rect.min.x < 300.0).map(|d| d.rect.min);
    let _ = drawn_with(&ctx, &mut a, size, vec![]);
    let top = inbox(drawn_with(&ctx, &mut a, size, vec![])).expect("the Inbox in the folder pane");
    let over = top + egui::vec2(10.0, 5.0);
    // Where it is with the pointer on it, before any scrolling.
    for _ in 0..3 {
        let _ = drawn_with(&ctx, &mut a, size, vec![Event::PointerMoved(over)]);
    }
    let top = inbox(drawn_with(&ctx, &mut a, size, vec![Event::PointerMoved(over)])).expect("the Inbox, pointed at");
    let wheel = |phase| Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 60.0),
        modifiers: Modifiers::NONE,
        phase,
        source: egui::MouseWheelSource::Trackpad,
    };
    let _ = drawn_with(&ctx, &mut a, size, vec![Event::PointerMoved(over), wheel(egui::TouchPhase::Start)]);
    for _ in 0..40 {
        let _ = drawn_with(&ctx, &mut a, size, vec![Event::PointerMoved(over), wheel(egui::TouchPhase::Move)]);
    }
    for _ in 0..30 {
        let _ = drawn_with(&ctx, &mut a, size, vec![Event::PointerMoved(over)]);
    }
    let after = inbox(drawn_with(&ctx, &mut a, size, vec![])).expect("the Inbox still drawn");
    assert!((after.y - top.y).abs() < 1.0, "pulled from {top:?} to {after:?}");
}

/// Issue #181: a folder dragged onto another goes inside it. The drop asked first for dragged
/// mail, which took the dragged folder away as the wrong kind, and nothing happened.
#[test]
fn a_folder_dropped_on_a_folder_goes_inside_it() {
    let (mut a, _) = app("folder-drop");
    let ctx = egui::Context::default();
    a.run("folder.create", json!({"account": "sample", "name": "Clients"}));
    let at = |texts: &[(String, egui::Pos2)], name: &str| texts.iter().filter(|(t, _)| t == name).map(|(_, p)| *p + egui::vec2(4.0, 6.0)).next_back();
    let texts = pane(&ctx, &mut a, 2000.0, vec![]);
    let (Some(from), Some(to)) = (at(&texts, "Clients"), at(&texts, "Projects")) else { panic!("Clients and Projects drawn: {texts:?}") };
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(from)]);
    pane(&ctx, &mut a, 2000.0, vec![press(from, true)]);
    for k in 1..=4 {
        pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    pane(&ctx, &mut a, 2000.0, vec![press(to, false)]);
    pane(&ctx, &mut a, 2000.0, vec![]);
    let paths: Vec<String> = a.session.accounts[0].folders.iter().map(|f| f.path.clone()).collect();
    assert!(paths.iter().any(|p| p == "Projects/Clients"), "moved into Projects: {paths:?}");
    assert!(!paths.iter().any(|p| p == "Clients"));
}

/// Issue #188: Minimize sits at the end of the folder pane's first row (the Favorites heading,
/// or with Favorites hidden the first account), so the folders start right under the ribbon. It
/// had a row of its own, an empty strip above the first account; and the account's name keeps
/// clear of it.
#[test]
fn minimize_sits_on_the_first_row() {
    let (mut a, _) = app("pane-corner");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    let find = |all: &[Named], label: &str| all.iter().find(|n| n.label.as_deref() == Some(label)).and_then(|n| n.rect);
    for favorites in [true, false] {
        a.session.prefs.show_favorites = favorites;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
        let all = named(&ctx, &mut a, size);
        let minimize = find(&all, "Minimize the folder pane").expect("Minimize drawn");
        let first = if favorites { "Favorites" } else { "riley@studio.example (sample)" };
        let row = find(&all, first).unwrap_or_else(|| panic!("{first} drawn"));
        assert!(row.y_range().contains(minimize.center().y), "{first} {row:?}, Minimize {minimize:?}");
        assert!(minimize.right() <= row.right() + 1.0 && minimize.left() > row.center().x, "at its end: {row:?} {minimize:?}");
        // Nothing above it but the ribbon's bar under the window's top.
        assert!(row.top() < 135.0, "{first} starts at {}", row.top());
    }
    click_named(&ctx, &mut a, "Minimize the folder pane");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.folder_pane_minimized);
}

/// Issue #184: the folder pane's density (View › Folder density) sets its rows' height, text
/// size and indent: Compact fits about one and a half times the folders Spacious does in the
/// same height, a level indented less. There was no setting.
#[test]
fn the_folder_density_fits_more_folders() {
    let (mut a, _) = app("folder-density");
    let ctx = egui::Context::default();
    big_tree(&mut a, 10, 4);
    let mut seen = Vec::new();
    for value in ["spacious", "medium", "compact"] {
        a.run("view.folderDensity", json!({"value": value}));
        let texts = pane(&ctx, &mut a, 600.0, vec![]);
        let folders = texts.iter().filter(|(t, _)| (t.starts_with('p') || t.starts_with('c')) && t.len() <= 3).count();
        let x = |name: &str| texts.iter().find(|(t, _)| t == name).map(|(_, p)| p.x).unwrap_or_else(|| panic!("{name} drawn: {texts:?}"));
        seen.push((folders, x("c0") - x("p0")));
    }
    let [(spacious, indent_s), (medium, _), (compact, indent_c)] = seen[..] else { panic!("{seen:?}") };
    assert!(spacious < medium && medium < compact, "{seen:?}");
    assert!(compact as f32 >= spacious as f32 * 1.35, "{seen:?}");
    assert!(indent_c < indent_s, "less indent a level: {seen:?}");
    assert!(a.session.run("view.folderDensity", &json!({"value": "tiny"})).is_err());
    assert_eq!(a.session.prefs.folder_density, mailcraft_engine::FolderDensity::Compact, "left as it was");
}

/// Issue #183: a folder dragged to another's top edge goes before it, in the folder pane's own
/// order; the menu's Move down moves it back after; the account's Sort folders A to Z undoes it.
/// The pane kept folders A to Z, whatever was done.
#[test]
fn a_folder_dropped_at_an_edge_goes_beside_it() {
    let (mut a, _) = app("folder-order-drag");
    let ctx = egui::Context::default();
    a.run("folder.create", json!({"account": "sample", "name": "Clients"}));
    a.run("folder.create", json!({"account": "sample", "name": "Zeta"}));
    let order = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        pane(ctx, a, 2000.0, vec![]).into_iter().map(|(t, _)| t).filter(|t| ["Clients", "Projects", "Zeta"].contains(&t.as_str())).collect()
    };
    assert_eq!(order(&ctx, &mut a), ["Clients", "Projects", "Zeta"]);
    let texts = pane(&ctx, &mut a, 2000.0, vec![]);
    let at = |name: &str| texts.iter().filter(|(t, _)| t == name).map(|(_, p)| *p).next_back().unwrap_or_else(|| panic!("{name} drawn"));
    // From Zeta's middle to the top edge of Clients' row (its text's top is 8 above the middle).
    let (from, to) = (at("Zeta") + egui::vec2(4.0, 6.0), at("Clients") + egui::vec2(4.0, -4.0));
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(from)]);
    pane(&ctx, &mut a, 2000.0, vec![press(from, true)]);
    for k in 1..=4 {
        pane(&ctx, &mut a, 2000.0, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    pane(&ctx, &mut a, 2000.0, vec![press(to, false)]);
    assert_eq!(order(&ctx, &mut a), ["Zeta", "Clients", "Projects"], "before Clients, not inside it");
    assert!(a.session.folder("sample", "Zeta").is_some());
    a.run("folder.moveDown", json!({"account": "sample", "folder": "Zeta"}));
    assert_eq!(order(&ctx, &mut a), ["Clients", "Zeta", "Projects"]);
    a.run("folder.sortAZ", json!({"account": "sample"}));
    assert_eq!(order(&ctx, &mut a), ["Clients", "Projects", "Zeta"]);
}
