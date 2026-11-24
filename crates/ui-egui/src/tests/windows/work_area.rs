//! A window opening inside the part of its monitor the system leaves to windows (its work area):
//! below a taskbar along the top, and no bigger than the room beside it (VectorCraft's fix).

use crate::{Monitors, WindowPlace};

fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

/// A window whose outer rect is `outer` and inner `inner`, on a 1920 x 1080 monitor.
fn opened(outer: egui::Rect, inner: egui::Rect) -> egui::ViewportInfo {
    egui::ViewportInfo {
        outer_rect: Some(outer),
        inner_rect: Some(inner),
        monitor_size: Some(egui::vec2(1920.0, 1080.0)),
        native_pixels_per_point: Some(1.0),
        ..Default::default()
    }
}

/// A taskbar 48 points tall along the top of a 1920 x 1080 monitor.
const TOP_BAR: egui::Rect = egui::Rect { min: egui::pos2(0.0, 48.0), max: egui::pos2(1920.0, 1080.0) };

/// A window saved at the top edge, under a taskbar along the top, comes down below it, staying
/// where it was across; one below the taskbar stays put. Its title bar can be grabbed again.
#[test]
fn a_window_under_a_taskbar_along_the_top_comes_down_below_it() {
    let under = opened(rect(100.0, 0.0, 1000.0, 700.0), rect(100.0, 0.0, 1000.0, 700.0));
    assert_eq!(WindowPlace::fit_in(&under, true, TOP_BAR), Some((egui::pos2(100.0, 48.0), egui::vec2(1000.0, 700.0))));
    let below = opened(rect(100.0, 200.0, 1000.0, 700.0), rect(100.0, 200.0, 1000.0, 700.0));
    assert_eq!(WindowPlace::fit_in(&below, true, TOP_BAR), None);
    // Its title bar past the top of a screen with no taskbar there: brought down onto it.
    let whole = rect(0.0, 0.0, 1920.0, 1040.0);
    let past_top = opened(rect(300.0, -20.0, 1000.0, 700.0), rect(300.0, -20.0, 1000.0, 700.0));
    assert_eq!(WindowPlace::fit_in(&past_top, true, whole), Some((egui::pos2(300.0, 0.0), egui::vec2(1000.0, 700.0))));
}

/// A window taller than the room below the taskbar, its system frame included, is made to fit
/// it and centred there; one that fits but for the borders Windows leaves unseen round a framed
/// window (a window snapped to half the screen) is left as it is.
#[test]
fn a_window_too_tall_for_the_work_area_is_made_to_fit_it() {
    // 1900 x 1060 inside a 16 x 39 frame (title bar and borders).
    let tall = opened(rect(0.0, 0.0, 1916.0, 1099.0), rect(8.0, 31.0, 1900.0, 1060.0));
    let (pos, size) = WindowPlace::fit_in(&tall, true, TOP_BAR).unwrap();
    assert_eq!(size, egui::vec2(1900.0, 1032.0 - 39.0));
    assert_eq!(pos, egui::pos2(2.0, 48.0));
    // Snapped to the left half of a screen with its taskbar at the bottom: 7 unseen pixels at
    // the left, right and bottom.
    let work = rect(0.0, 0.0, 1920.0, 1032.0);
    let snapped = opened(rect(-7.0, 0.0, 974.0, 1039.0), rect(1.0, 31.0, 958.0, 1000.0));
    assert_eq!(WindowPlace::fit_in(&snapped, true, work), None);
    // Never smaller than the smallest window, where the work area has room for it.
    let huge = opened(rect(0.0, 0.0, 4000.0, 3000.0), rect(0.0, 0.0, 4000.0, 3000.0));
    assert_eq!(WindowPlace::fit_in(&huge, true, rect(0.0, 0.0, 1366.0, 728.0)).map(|f| f.1), Some(egui::vec2(1366.0, 728.0)));
    assert_eq!(WindowPlace::fit_in(&huge, true, rect(0.0, 0.0, 640.0, 400.0)).map(|f| f.1), Some(egui::vec2(640.0, 400.0)));
    // Maximized: the system keeps it to the work area itself.
    let maximized = egui::ViewportInfo { maximized: Some(true), ..tall };
    assert_eq!(WindowPlace::fit_in(&maximized, true, TOP_BAR), None);
}

/// A window the system placed past the work area's edge is centred in it; restored there (the
/// user put it so), it stays.
#[test]
fn a_window_placed_past_the_work_area_is_centred_in_it() {
    let past = opened(rect(1200.0, 400.0, 1000.0, 700.0), rect(1200.0, 400.0, 1000.0, 700.0));
    assert_eq!(WindowPlace::fit_in(&past, false, TOP_BAR), Some((egui::pos2(460.0, 214.0), egui::vec2(1000.0, 700.0))));
    assert_eq!(WindowPlace::fit_in(&past, true, TOP_BAR), None);
}

/// The window opens inside the work area of the monitor most of it is on, in its points; with
/// none listed it keeps room for a panel on each side of its monitor, as before.
#[test]
fn the_work_area_is_the_one_of_the_monitor_its_on() {
    let left = rect(0.0, 0.0, 3840.0, 2160.0);
    let right = rect(3840.0, 0.0, 1920.0, 1080.0);
    let monitors = Monitors {
        all: vec![left, right],
        primary: Some(left),
        // A taskbar along the top of the right-hand monitor, 48 pixels tall.
        work: vec![rect(0.0, 0.0, 3840.0, 2064.0), rect(3840.0, 48.0, 1920.0, 1032.0)],
    };
    // On the right-hand monitor at its title bar's height, at 100 %.
    let on_right = opened(rect(4000.0, 0.0, 1000.0, 700.0), rect(4000.0, 0.0, 1000.0, 700.0));
    assert_eq!(crate::app::work_area(&on_right, Some(&monitors), 1.0), Some(rect(3840.0, 48.0, 1920.0, 1032.0)));
    // On the left-hand one, at 200 %: its work area in points.
    let on_left =
        egui::ViewportInfo { native_pixels_per_point: Some(2.0), ..opened(rect(100.0, 100.0, 800.0, 600.0), rect(100.0, 100.0, 800.0, 600.0)) };
    assert_eq!(crate::app::work_area(&on_left, Some(&monitors), 1.0), Some(rect(0.0, 0.0, 1920.0, 1032.0)));
    assert_eq!(crate::app::work_area(&on_left, Some(&Monitors { work: vec![], ..monitors }), 1.0), None);
    assert_eq!(crate::app::work_area(&on_left, None, 1.0), None);
}
