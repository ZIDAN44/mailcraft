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
        ui.allocate_ui(egui::vec2(220.0, height), |ui| crate::nav::show(&a.session, &t, ui, &mut acts));
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
