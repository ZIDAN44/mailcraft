//! The part of each monitor that windows may use, without the taskbar, the Dock or the menu bar,
//! as the system says (Windows and macOS): a window is kept inside it as it opens
//! (`WindowPlace::fit_in`). In the physical pixels, from the top-left of the primary monitor, that
//! winit gives the monitors. Elsewhere none is said, and a window leaves room for a panel instead.

/// Every monitor's work area.
#[cfg(windows)]
pub(crate) fn list() -> Vec<egui::Rect> {
    use winsafe::prelude::*;
    let mut areas = Vec::new();
    // Without a list, a window keeps room for a taskbar, as before.
    let _ = winsafe::HDC::NULL.EnumDisplayMonitors(None, |monitor, _, _| {
        if let Ok(info) = monitor.GetMonitorInfo() {
            let w = info.rcWork;
            areas.push(egui::Rect::from_min_max(egui::pos2(w.left as f32, w.top as f32), egui::pos2(w.right as f32, w.bottom as f32)));
        }
        true
    });
    areas
}

/// Every screen's `visibleFrame`, turned from macOS's points up from the bottom-left of the
/// primary screen into pixels down from its top-left.
#[cfg(target_os = "macos")]
pub(crate) fn list() -> Vec<egui::Rect> {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;
    // The screens are asked for on the main thread, where the window's frames run.
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let screens = NSScreen::screens(mtm);
    // The first screen has the menu bar, and the origin of the screens' coordinates.
    let Some(primary) = screens.firstObject().map(|s| s.frame().size.height) else { return Vec::new() };
    screens
        .iter()
        .filter_map(|s| {
            let f = s.visibleFrame();
            from_bottom_left([f.origin.x, f.origin.y, f.size.width, f.size.height], primary, s.backingScaleFactor())
        })
        .collect()
}

/// None said: the window keeps room for a panel.
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn list() -> Vec<egui::Rect> {
    Vec::new()
}

/// `[x, y, width, height]` in macOS's screen points (y up from the bottom of the primary screen,
/// `primary_height` tall) as winit's monitor pixels: down from the top, times `scale`. None for
/// what isn't a screen's (not finite, or a million pixels and more).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn from_bottom_left([x, y, w, h]: [f64; 4], primary_height: f64, scale: f64) -> Option<egui::Rect> {
    let [x, y, w, h] = [x, primary_height - (y + h), w, h].map(|v| v * scale);
    let fine = [x, y, w, h].iter().all(|v| v.is_finite() && v.abs() < 1e6) && w > 0.0 && h > 0.0;
    fine.then(|| egui::Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mac's work areas as winit's monitors have them: the primary screen 900 points tall with
    /// a 25-point menu bar and an 80-point Dock, at twice the pixels; one above it, at its own.
    #[test]
    fn a_macs_visible_frame_is_turned_top_down() {
        let main = from_bottom_left([0.0, 80.0, 1440.0, 795.0], 900.0, 2.0);
        assert_eq!(main, Some(egui::Rect::from_min_size(egui::pos2(0.0, 50.0), egui::vec2(2880.0, 1590.0))));
        let above = from_bottom_left([0.0, 900.0, 1920.0, 1080.0], 900.0, 1.0);
        assert_eq!(above, Some(egui::Rect::from_min_size(egui::pos2(0.0, -1080.0), egui::vec2(1920.0, 1080.0))));
        assert_eq!(from_bottom_left([0.0, 0.0, 0.0, 795.0], 900.0, 1.0), None, "no width");
        assert_eq!(from_bottom_left([f64::NAN, 0.0, 10.0, 10.0], 900.0, 1.0), None);
        assert_eq!(from_bottom_left([1e9, 0.0, 10.0, 10.0], 900.0, 1.0), None);
    }

    /// On Windows each work area the system lists has a size (a session with no monitor, as a
    /// service's, lists none).
    #[cfg(windows)]
    #[test]
    fn windows_work_areas_have_a_size() {
        let areas = list();
        assert!(areas.iter().all(|a| a.is_positive() && a.is_finite()), "{areas:?}");
    }
}
