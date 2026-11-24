//! The interface's text size (the comment on issue #267): egui's zoom over every window, kept
//! with the preferences, its keys MailCraft's (and said); windows keep their size and place on
//! the screen at any text size.

use egui::accesskit::Role;

use super::*;
use crate::{Monitors, WindowPlace};

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

/// What the frame's live regions say.
fn spoken(out: &egui::FullOutput) -> Vec<String> {
    let u = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
    u.nodes.iter().filter(|(_, n)| n.role() == Role::Status).filter_map(|(_, n)| n.label().map(str::to_string)).collect()
}

/// The text size zooms every window, as chosen; Ctrl and +, − and 0 change it, kept, each saying
/// where it's left; egui's own keys, which kept nothing, are off.
#[test]
fn the_text_size_zooms_the_window_and_its_keys_keep_it() {
    let (mut a, _) = app("text-size-keys");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // The first frame loads the fonts alone.
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.0);
    assert!(!ctx.options(|o| o.zoom_with_keyboard), "egui's own keys are off");
    a.run("view.textSize", json!({"percent": 150}));
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.5);

    let out = sized_frame(&ctx, &mut a, SIZE, vec![key(Key::Plus, Modifiers::COMMAND)]);
    assert_eq!(a.session.prefs.text_size, 160);
    assert_eq!(spoken(&out), vec!["Text size: 160%"]);
    // egui zooms from the pass after it's set.
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.6);
    sized_frame(&ctx, &mut a, SIZE, vec![key(Key::Equals, Modifiers::COMMAND)]);
    assert_eq!(a.session.prefs.text_size, 170, "Ctrl and = (the plus key unshifted)");
    sized_frame(&ctx, &mut a, SIZE, vec![key(Key::Minus, Modifiers::COMMAND)]);
    assert_eq!(a.session.prefs.text_size, 160);
    let out = sized_frame(&ctx, &mut a, SIZE, vec![key(Key::Num0, Modifiers::COMMAND)]);
    assert_eq!((a.session.prefs.text_size, spoken(&out)), (100, vec!["Text size: 100%".to_string()]));
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(ctx.zoom_factor(), 1.0);
}

/// File › Options › General's Text size shows it, and its arrows set it.
#[test]
fn the_text_size_is_set_in_options() {
    let (mut a, _) = app("text-size-options");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let row = |ctx: &egui::Context, a: &mut MailApp| {
        let out = sized_frame(ctx, a, SIZE, vec![]);
        let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
        nodes
            .into_iter()
            .find(|(_, n)| n.role() == Role::ComboBox && n.label() == Some("Text size"))
            .map(|(id, n)| (id, n.value().unwrap_or_default().to_string()))
    };
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    let (id, shown) = row(&ctx, &mut a).expect("the row");
    assert_eq!(shown, "100%");
    let focus = Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Focus,
        target_node: id,
        target_tree: egui::accesskit::TreeId::ROOT,
        data: None,
    });
    sized_frame(&ctx, &mut a, SIZE, vec![focus]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(a.session.prefs.text_size, 110);
    assert_eq!(row(&ctx, &mut a).map(|r| r.1).as_deref(), Some("110%"));
}

/// A window's place is kept in its monitor's own points: at 200 % it opens where and as large as
/// it was on the screen; a monitor and its work area are measured at the text size, so a window
/// past its monitor at 200 % (which at 100 % would seem on it) comes back, and a taskbar 48
/// pixels tall is 24 points; the size a window opens at and its least stay as on the screen.
#[test]
fn windows_keep_their_size_and_place_on_the_screen_at_any_text_size() {
    let kept = WindowPlace { x: 200.0, y: 100.0, w: 1600.0, h: 1200.0, maximized: false, monitor: Some([1920.0, 1080.0]) };
    let at_200 = kept.at_zoom(2.0);
    assert_eq!(at_200, WindowPlace { x: 100.0, y: 50.0, w: 800.0, h: 600.0, maximized: false, monitor: Some([960.0, 540.0]) });
    assert_eq!(at_200.at_zoom(0.5), kept, "and back");
    assert_eq!(kept.at_zoom(f32::NAN), kept, "a zoom that makes no sense is the interface's own");
    let b = kept.apply(crate::os_window(), 2.0);
    assert_eq!((b.position, b.inner_size), (Some(egui::pos2(100.0, 50.0)), Some(egui::vec2(800.0, 600.0))));

    let screen = rect(0.0, 0.0, 1920.0, 1080.0);
    let one = Monitors { all: vec![screen], primary: Some(screen), work: vec![rect(0.0, 48.0, 1920.0, 1032.0)] };
    let at = |x: f32| egui::ViewportInfo {
        monitor_size: Some(egui::vec2(960.0, 540.0)),
        outer_rect: Some(rect(x, 50.0, 800.0, 630.0)),
        native_pixels_per_point: Some(1.0),
        ..Default::default()
    };
    assert_eq!(at_200.rescue(&at(100.0), Some(&one), 2.0), None, "on its monitor");
    assert_eq!(at_200.rescue(&at(1000.0), Some(&one), 2.0), Some(egui::pos2(80.0, 0.0)), "2000 pixels in: past it");
    assert_eq!(crate::app::work_area(&at(100.0), Some(&one), 2.0), Some(rect(0.0, 24.0, 960.0, 516.0)));
    assert_eq!(crate::app::at_zoom([520.0, 420.0], 2.0), [260.0, 210.0]);
    assert_eq!(crate::app::at_zoom([520.0, 420.0], 0.0), [520.0, 420.0]);
}
