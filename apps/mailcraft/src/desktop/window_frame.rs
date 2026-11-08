//! Keeps the window borderless while MailCraft's top bar is its title bar (issue #22), as
//! PhotoCraft's own title bar does: ported from PhotoCraft's `window_frame.rs`, with the issues it
//! fixed named by their PhotoCraft numbers (#2246, #3144, #4845).
//!
//! winit 0.30 gives an undecorated window the `WS_CAPTION | WS_BORDER` styles and hides the
//! non-client area by spreading the client rect over it (`WM_NCCALCSIZE`). On some systems
//! (reported: Windows 10 LTSB, Intel HD Graphics 4600, 1366 × 768) the desktop compositor still
//! reserves that frame for the window surface: the content is drawn offset by the caption and
//! border, leaving black bars at the window's top-left, while the pointer keeps the un-offset
//! client coordinates, so it no longer points at what is drawn under it.
//!
//! winit 0.27 — before rust-windowing/winit#2419 — simply omitted those styles for undecorated
//! windows, and there was no offset. [`service`] restores that: it strips `WS_CAPTION` and
//! `WS_BORDER` and forces Windows to recalculate the frame. winit puts them back whenever it
//! changes the window flags (maximize, restore, …), so the returned guard checks every frame.
//!
//! Windows sizes a maximize by the frame its styles describe, so a window maximized while its
//! caption is gone is sized without one: its rect covers the whole monitor, taskbar included.
//! The shell reads a caption-less window covering its monitor as a full-screen app and drops the
//! taskbar's backdrop behind it — a black band on stock Windows 11, a glitched unresponsive one
//! under ExplorerPatcher (#3144). The guard therefore also trims a parked maximize's rect back to
//! the work area plus its own frame, clear of the taskbar. winit pins the client area to the work
//! area either way (`WM_NCCALCSIZE`), so the app's pixels do not move.
//!
//! An auto-hidden taskbar reserves no work area, so that trim leaves the maximize covering the
//! whole monitor, and the shell, taking it for a full-screen app, no longer reveals the taskbar
//! when the pointer reaches its edge (#4845). On each edge with an auto-hidden taskbar the guard
//! instead stops the window one pixel short of the monitor, as other custom-frame apps do.
//!
//! winit gives an undecorated window its drop shadow by moving the client rect one pixel down
//! (`WM_NCCALCSIZE`), past the window's bottom edge. DirectX then fits the 820 rows MailCraft draws
//! into the 819 that are left, so every row but the first is resampled and all text is soft from
//! top to bottom, with a black line along the top. So winit's shadow is turned off when the window
//! is made (`window_created` in `main.rs`) and the desktop compositor is asked for one instead, by
//! extending the frame a pixel into the client area, where MailCraft's own pixels cover it.
//!
//! Windows plays its zoom animation (maximize, restore) only for a window with a caption. On
//! Windows 11 the caption is left alone (the systems reported above are Windows 10): Windows
//! animates every way of maximizing and restoring and sizes a maximize clear of the taskbar
//! itself, leaving only the auto-hide trim to do. On Windows 10 the caption still comes off, and
//! [`FrameGuard::before_zoom`] hands it back for MailCraft's own Maximize, Restore and double-click:
//! those animate, and a maximize isn't sized over the taskbar, which the shell took for a full-screen
//! app and dropped the taskbar for a frame, what was behind it showing through. The caption comes
//! off again once the animation is over, without a frame change on a maximized window (one there
//! sizes it over the taskbar again). Win+Up and dragging don't pass through MailCraft first, so on
//! Windows 10 they stay as they were: not animated, and trimmed after the fact.
//!
//! The Windows calls go through safe `winsafe` wrappers; the workspace forbids `unsafe`, and
//! winsafe can't wrap a raw handle without it, so the window is found by its handle instead.

pub use platform::service;

/// What keeps the window borderless while it draws its own title bar: shared by the start-up
/// screen, then MailCraft.
#[derive(Clone)]
pub(crate) struct FrameGuard {
    keep: std::rc::Rc<dyn Fn()>,
    before_zoom: std::rc::Rc<dyn Fn()>,
}

impl FrameGuard {
    /// Run once a frame.
    pub(crate) fn keep(&self) {
        (self.keep)();
    }

    /// Run in the frame that asks to maximize or restore the window, before winit does it (see
    /// above).
    pub(crate) fn before_zoom(&self) {
        (self.before_zoom)();
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::FrameGuard;
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};
    use winsafe::prelude::Handle;
    use winsafe::{AtomStr, HWND, HwndPlace, POINT, SIZE, co};

    /// A rect in desktop coordinates: `(left, top, right, bottom)`, right and bottom exclusive.
    type Rect = (i32, i32, i32, i32);

    /// How long Windows' zoom animation runs: the caption it needs stays on for this long.
    const ZOOM: Duration = Duration::from_millis(400);

    /// The window the guard looks after.
    struct Frame {
        hwnd: HWND,
        /// Windows 11: the caption stays on, and only the auto-hide trim is left to do.
        keeps_caption: bool,
        /// Until when the caption is left on, for Windows' zoom animation.
        zooming_until: Cell<Option<Instant>>,
    }

    impl Frame {
        /// One pass of the per-frame guard.
        fn keep(&self) {
            if strip_due(self.keeps_caption, self.zooming_until.get(), Instant::now()) {
                strip(&self.hwnd);
            }
            keep_clear_of_the_taskbar(&self.hwnd);
        }

        /// The window is about to be maximized or restored by MailCraft: the caption goes back
        /// on, not as a frame change (nothing moves until the zoom does), for the animation.
        fn before_zoom(&self) {
            if !self.keeps_caption {
                self.hwnd.set_style(captioned(self.hwnd.style()));
                self.zooming_until.set(Some(Instant::now() + ZOOM));
            }
        }
    }

    /// A monitor edge.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Edge {
        Left,
        Top,
        Right,
        Bottom,
    }

    /// A guard that keeps the window borderless each frame, or `None` when the app uses the
    /// system title bar (the window is decorated) or its handle can't be found.
    pub fn service(cc: &eframe::CreationContext<'_>, custom_titlebar: bool) -> Option<FrameGuard> {
        if !custom_titlebar {
            return None;
        }
        let target = match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => h.hwnd.get(),
            _ => return None,
        };
        let Some(hwnd) = find_window(target) else {
            log::warn!("window frame: MailCraft's window wasn't found among the desktop's, so the system's frame may come back");
            return None;
        };
        // The window's shadow, which winit's own is turned off for (see above).
        let shadow = winsafe::MARGINS { cyTopHeight: 1, ..Default::default() };
        if let Err(e) = hwnd.DwmExtendFrameIntoClientArea(&shadow) {
            log::warn!("window frame: the window has no shadow: {e}");
        }
        let frame = Rc::new(Frame { hwnd, keeps_caption: windows_11(), zooming_until: Cell::new(None) });
        frame.keep();
        let keeps = Rc::clone(&frame);
        Some(FrameGuard { keep: Rc::new(move || keeps.keep()), before_zoom: Rc::new(move || frame.before_zoom()) })
    }

    /// The top-level window with the given raw handle. `EnumWindows` hands out real `HWND`s, so
    /// this matches winit's own window without wrapping a raw handle (which needs `unsafe`).
    fn find_window(target: isize) -> Option<HWND> {
        let mut found = None;
        let _ = winsafe::EnumWindows(|hwnd| {
            if hwnd.ptr() as isize == target {
                found = Some(hwnd);
                false
            } else {
                true
            }
        });
        found
    }

    /// Whether this is Windows 11 or later. From the registry: Windows tells a program with no
    /// version manifest that it runs on Windows 8.
    fn windows_11() -> bool {
        let build =
            winsafe::HKEY::LOCAL_MACHINE.RegGetValue(Some(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), Some("CurrentBuild"), co::RRF::RT_ANY);
        matches!(build, Ok(winsafe::RegistryValue::Sz(text)) if build_is_windows_11(&text))
    }

    /// Whether a Windows build number is Windows 11's (22000) or later.
    fn build_is_windows_11(build: &str) -> bool {
        build.trim().parse::<u32>().is_ok_and(|b| b >= 22000)
    }

    /// Whether the guard takes the caption off this frame: not on Windows 11, and not while
    /// Windows' zoom animation needs it (`zooming_until`).
    fn strip_due(keeps_caption: bool, zooming_until: Option<Instant>, now: Instant) -> bool {
        !keeps_caption && zooming_until.is_none_or(|until| now >= until)
    }

    /// Removes `WS_CAPTION`/`WS_BORDER` and has Windows recalculate the frame. A no-op once they
    /// are gone, so it is cheap to call every frame. A maximized window keeps its rect: a frame
    /// change there would make Windows size it over the taskbar again (see above).
    fn strip(hwnd: &HWND) {
        let style = hwnd.style();
        let without = borderless(style);
        if without.raw() == style.raw() {
            return;
        }
        hwnd.set_style(without);
        if !needs_a_frame_change(hwnd.IsZoomed()) {
            return;
        }
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT::new(),
            SIZE::new(),
            co::SWP::FRAMECHANGED | co::SWP::NOCOPYBITS | co::SWP::NOMOVE | co::SWP::NOSIZE | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
    }

    /// How far a parked maximize reaches past a monitor/work-area edge: the resize
    /// border (about 8 pt at 100 %, more with DPI). Past that, a maximized window's rect is not
    /// parked but being dragged or snapped, and is left alone.
    const FRAME: i32 = 32;

    /// Keep a parked maximize inside all four work-area edges, including a top taskbar.
    fn keep_clear_of_the_taskbar(hwnd: &HWND) {
        if !hwnd.IsZoomed() {
            return;
        }
        let (Ok(r), Ok(mi)) = (hwnd.GetWindowRect(), hwnd.MonitorFromWindow(co::MONITOR::DEFAULTTONEAREST).GetMonitorInfo()) else {
            return;
        };
        let (m, w) = (mi.rcMonitor, mi.rcWork);
        let (rect, monitor, work) = ((r.left, r.top, r.right, r.bottom), (m.left, m.top, m.right, m.bottom), (w.left, w.top, w.right, w.bottom));
        // The taskbars (a couple of `FindWindowEx` calls) are looked up only while the window
        // covers the whole monitor: a maximize trimmed short of an auto-hidden one no longer
        // does, but one on a monitor with no taskbar to keep clear of still does, and looks them
        // up each frame it draws.
        let Some((left, top, right, bottom)) = clamped_rect(rect, monitor, work)
            .or_else(|| covers(rect, monitor).then(|| short_of_autohide_taskbars(rect, monitor, work, &taskbar_edges(monitor))).flatten())
        else {
            return;
        };
        let (Some(width), Some(height)) = (right.checked_sub(left), bottom.checked_sub(top)) else {
            return;
        };
        // Moving the outer frame while winit pins the client rect to the work area must
        // not copy the old frame's pixels into the custom title bar.
        let _ = hwnd.SetWindowPos(
            HwndPlace::None,
            POINT { x: left, y: top },
            SIZE { cx: width, cy: height },
            co::SWP::FRAMECHANGED | co::SWP::NOCOPYBITS | co::SWP::NOZORDER | co::SWP::NOACTIVATE,
        );
        let _ = hwnd.InvalidateRect(None, false);
    }

    /// Return the corrected outer rect in desktop coordinates, preserving the resize border.
    /// Each edge must be parked just outside either the monitor or work-area edge. Accepting
    /// both makes the guard idempotent and handles Windows already correcting some edges.
    /// Dragged/snapped windows are left alone; no monitor-origin or DPI assumptions are needed.
    fn clamped_rect(rect: (i32, i32, i32, i32), monitor: (i32, i32, i32, i32), work: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
        let (l, t, r, b) = rect;
        let (ml, mt, mr, mb) = monitor;
        let (wl, wt, wr, wb) = work;
        if ml >= mr || mt >= mb || wl < ml || wt < mt || wr > mr || wb > mb || wl >= wr || wt >= wb {
            return None;
        }
        // Infer the frame from an edge without a taskbar. On an obstructed edge, the
        // monitor and work-area offsets can both look like valid borders (e.g. auto-hide).
        let border = [
            (ml == wl).then(|| ml.checked_sub(l)).flatten(),
            (mt == wt).then(|| mt.checked_sub(t)).flatten(),
            (mr == wr).then(|| r.checked_sub(mr)).flatten(),
            (mb == wb).then(|| b.checked_sub(mb)).flatten(),
        ]
        .into_iter()
        .flatten()
        .filter(|n| (0..=FRAME).contains(n))
        .min()?;
        let parked = |monitor: Option<i32>, work: Option<i32>| monitor == Some(border) || work == Some(border);
        if !parked(ml.checked_sub(l), wl.checked_sub(l))
            || !parked(mt.checked_sub(t), wt.checked_sub(t))
            || !parked(r.checked_sub(mr), r.checked_sub(wr))
            || !parked(b.checked_sub(mb), b.checked_sub(wb))
        {
            return None;
        }
        let corrected = (wl.checked_sub(border)?, wt.checked_sub(border)?, wr.checked_add(border)?, wb.checked_add(border)?);
        (corrected != rect).then_some(corrected)
    }

    /// Whether `rect` reaches every edge of `monitor`: what the shell takes for a full-screen app.
    fn covers(rect: Rect, monitor: Rect) -> bool {
        rect.0 <= monitor.0 && rect.1 <= monitor.1 && rect.2 >= monitor.2 && rect.3 >= monitor.3
    }

    /// The edges of `monitor` that have a taskbar (the primary `Shell_TrayWnd` or a secondary
    /// monitor's `Shell_SecondaryTrayWnd`), shown or hidden.
    fn taskbar_edges(monitor: Rect) -> Vec<Edge> {
        let mut edges = Vec::new();
        for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
            let mut after: Option<HWND> = None;
            // One taskbar per monitor; the bound only guards against a misbehaving shell.
            for _ in 0..64 {
                let Ok(Some(tray)) = HWND::NULL.FindWindowEx(after.as_ref(), AtomStr::from_str(class), None) else {
                    break;
                };
                if let Ok(r) = tray.GetWindowRect() {
                    edges.extend(taskbar_edge((r.left, r.top, r.right, r.bottom), monitor));
                }
                after = Some(tray);
            }
        }
        edges
    }

    /// The edge of `monitor` a taskbar with rect `taskbar` sits on: the one nearest its centre,
    /// or `None` when it is on another monitor. An auto-hidden taskbar is parked mostly past its
    /// edge with a sliver left on screen, so its centre may lie outside the monitor.
    fn taskbar_edge(taskbar: Rect, monitor: Rect) -> Option<Edge> {
        let (tl, tt, tr, tb) = (i64::from(taskbar.0), i64::from(taskbar.1), i64::from(taskbar.2), i64::from(taskbar.3));
        let (ml, mt, mr, mb) = (i64::from(monitor.0), i64::from(monitor.1), i64::from(monitor.2), i64::from(monitor.3));
        if tl >= tr || tt >= tb || ml >= mr || mt >= mb || tr <= ml || tl >= mr || tb <= mt || tt >= mb {
            return None;
        }
        // Doubled centre and distances, so nothing rounds.
        let (cx, cy) = (tl + tr, tt + tb);
        [(Edge::Left, cx - 2 * ml), (Edge::Top, cy - 2 * mt), (Edge::Right, 2 * mr - cx), (Edge::Bottom, 2 * mb - cy)]
            .into_iter()
            .min_by_key(|&(_, distance)| distance)
            .map(|(edge, _)| edge)
    }

    /// A maximize covering its monitor, trimmed to stop one pixel short of each edge where a
    /// taskbar reserves no work area (it auto-hides), so the shell still reveals it there.
    /// `None` when there is nothing to trim.
    fn short_of_autohide_taskbars(rect: Rect, monitor: Rect, work: Rect, taskbars: &[Edge]) -> Option<Rect> {
        if !covers(rect, monitor) {
            return None;
        }
        let (mut l, mut t, mut r, mut b) = rect;
        for edge in taskbars {
            match edge {
                Edge::Left if work.0 == monitor.0 => l = monitor.0.checked_add(1)?,
                Edge::Top if work.1 == monitor.1 => t = monitor.1.checked_add(1)?,
                Edge::Right if work.2 == monitor.2 => r = monitor.2.checked_sub(1)?,
                Edge::Bottom if work.3 == monitor.3 => b = monitor.3.checked_sub(1)?,
                _ => {}
            }
        }
        let trimmed = (l, t, r, b);
        (l < r && t < b && trimmed != rect).then_some(trimmed)
    }

    /// `style` without the OS title bar and frame (`WS_CAPTION`, which already includes
    /// `WS_BORDER`). The sizing border (`WS_THICKFRAME`) stays, so edge resizing and Aero snap
    /// keep working, as they did with winit 0.27's undecorated windows.
    fn borderless(style: co::WS) -> co::WS {
        style & !(co::WS::CAPTION | co::WS::BORDER)
    }

    /// `style` with the caption Windows sizes a maximize's frame by.
    fn captioned(style: co::WS) -> co::WS {
        style | co::WS::CAPTION
    }

    /// Whether taking the styles off has to recalculate the frame: not on a maximized window,
    /// where that would move it (see above).
    fn needs_a_frame_change(zoomed: bool) -> bool {
        !zoomed
    }

    #[cfg(test)]
    mod tests {
        use super::{
            Edge, ZOOM, borderless, build_is_windows_11, captioned, clamped_rect, needs_a_frame_change, short_of_autohide_taskbars, strip_due,
            taskbar_edge,
        };
        use std::time::Instant;
        use winsafe::co;

        #[test]
        fn the_borderless_style_drops_the_caption_and_border_only() {
            // `WS_CAPTION` is 0x00c0_0000 and already contains `WS_BORDER` (0x0080_0000).
            let decorated = co::WS::CAPTION | co::WS::BORDER | co::WS::SYSMENU | co::WS::THICKFRAME | co::WS::MAXIMIZEBOX | co::WS::MINIMIZEBOX;
            let bare = borderless(decorated);
            assert!(!bare.has(co::WS::CAPTION), "caption gone");
            assert!(!bare.has(co::WS::BORDER), "border gone");
            assert!(bare.has(co::WS::THICKFRAME), "edge resizing kept");
            assert!(bare.has(co::WS::MAXIMIZEBOX) && bare.has(co::WS::SYSMENU), "window buttons kept");
            // Already borderless: unchanged, so the per-frame call does nothing.
            assert_eq!(bare.raw(), borderless(bare).raw());
        }

        #[test]
        fn the_window_keeps_its_caption_from_windows_11_on() {
            assert!(build_is_windows_11("22000") && build_is_windows_11("26300"));
            assert!(!build_is_windows_11("19045"), "Windows 10");
            assert!(!build_is_windows_11("junk") && !build_is_windows_11(""), "not known: as Windows 10");
        }

        #[test]
        fn the_caption_waits_for_the_zoom_animation() {
            let now = Instant::now();
            assert!(strip_due(false, None, now), "Windows 10 with no zoom going on: off it comes");
            assert!(!strip_due(false, Some(now + ZOOM), now), "not during the animation");
            assert!(strip_due(false, Some(now), now + ZOOM), "and off it comes when that's over");
            assert!(!strip_due(true, None, now), "Windows 11 keeps its caption");
        }

        #[test]
        fn a_maximize_gets_its_caption_back_first_and_keeps_its_rect() {
            let bare = borderless(co::WS::CAPTION | co::WS::BORDER | co::WS::SYSMENU | co::WS::THICKFRAME);
            let with = captioned(bare);
            assert!(with.has(co::WS::CAPTION), "Windows sizes the maximize with a frame, clear of the taskbar");
            assert!(with.has(co::WS::THICKFRAME) && with.has(co::WS::SYSMENU), "the rest of the style is kept");
            assert_eq!(with.raw(), captioned(with).raw(), "already has it: unchanged");
            assert_eq!(borderless(with).raw(), bare.raw(), "and the guard takes it off again");
            assert!(needs_a_frame_change(false), "a restored window's frame is recalculated");
            assert!(!needs_a_frame_change(true), "a maximized window's would move it over the taskbar again");
        }

        #[test]
        fn a_maximize_respects_taskbars_on_every_edge() {
            let monitor = (0, 0, 1920, 1080);
            let rect = (-8, -8, 1928, 1088);
            for (work, expected) in [
                ((0, 0, 1920, 1046), (-8, -8, 1928, 1054)),
                ((0, 48, 1920, 1080), (-8, 40, 1928, 1088)),
                ((0, 2, 1920, 1080), (-8, -6, 1928, 1088)),
                ((0, 0, 1920, 1078), (-8, -8, 1928, 1086)),
                ((48, 0, 1920, 1080), (40, -8, 1928, 1088)),
                ((0, 0, 1872, 1080), (-8, -8, 1880, 1088)),
            ] {
                assert_eq!(clamped_rect(rect, monitor, work), Some(expected));
                assert_eq!(clamped_rect(expected, monitor, work), None, "already corrected");
            }
        }

        #[test]
        fn a_top_taskbar_keeps_its_offset_at_other_origins_and_frame_sizes() {
            let monitor = (-1920, -1080, 0, 0);
            let work = (-1920, -1032, 0, 0);
            for border in [0, 8, 16, 32] {
                let rect = (-1920 - border, -1080 - border, border, border);
                let expected = (-1920 - border, -1032 - border, border, border);
                assert_eq!(clamped_rect(rect, monitor, work), Some(expected));
                assert_eq!(clamped_rect(expected, monitor, work), None);
            }
        }

        #[test]
        fn a_maximize_without_a_reserved_taskbar_is_left_alone() {
            let monitor = (0, 0, 1920, 1080);
            assert_eq!(clamped_rect((-8, -8, 1928, 1088), monitor, monitor), None);
        }

        #[test]
        fn a_maximize_being_dragged_or_snapped_is_left_alone() {
            let monitor = (0, 0, 1920, 1080);
            let work = (0, 48, 1920, 1080);
            assert_eq!(clamped_rect((100, 200, 1100, 900), monitor, work), None, "moved");
            assert_eq!(clamped_rect((-500, -8, 1428, 1088), monitor, work), None, "dragged");
            assert_eq!(clamped_rect((0, 48, 960, 1080), monitor, work), None, "half-screen");
        }

        #[test]
        fn a_maximize_leaves_a_pixel_strip_on_an_auto_hidden_taskbar_edge() {
            // #4845: an auto-hidden taskbar reserves no work area.
            let monitor = (0, 0, 1920, 1080);
            let rect = (-8, -8, 1928, 1088);
            for (edge, expected) in [
                (Edge::Bottom, (-8, -8, 1928, 1079)),
                (Edge::Top, (-8, 1, 1928, 1088)),
                (Edge::Left, (1, -8, 1928, 1088)),
                (Edge::Right, (-8, -8, 1919, 1088)),
            ] {
                assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[edge]), Some(expected));
                assert_eq!(short_of_autohide_taskbars(expected, monitor, monitor, &[edge]), None, "already trimmed");
            }
            assert_eq!(short_of_autohide_taskbars(rect, monitor, monitor, &[]), None, "no taskbar");
            // A taskbar that reserves its work area is the clamp's job.
            assert_eq!(short_of_autohide_taskbars(rect, monitor, (0, 0, 1920, 1032), &[Edge::Bottom]), None);
            assert_eq!(short_of_autohide_taskbars((0, 0, 960, 1080), monitor, monitor, &[Edge::Bottom]), None, "half-screen");
        }

        #[test]
        fn a_taskbar_sits_on_the_monitor_edge_nearest_its_centre() {
            let monitor = (0, 0, 1920, 1080);
            assert_eq!(taskbar_edge((0, 1032, 1920, 1080), monitor), Some(Edge::Bottom), "shown");
            assert_eq!(taskbar_edge((0, 1078, 1920, 1126), monitor), Some(Edge::Bottom), "auto-hidden");
            assert_eq!(taskbar_edge((0, -46, 1920, 2), monitor), Some(Edge::Top));
            assert_eq!(taskbar_edge((-60, 0, 2, 1080), monitor), Some(Edge::Left));
            assert_eq!(taskbar_edge((1918, 0, 1980, 1080), monitor), Some(Edge::Right));
            assert_eq!(taskbar_edge((1920, 1032, 3840, 1080), monitor), None, "other monitor");
            assert!(taskbar_edge((i32::MIN, i32::MIN, i32::MAX, i32::MAX), monitor).is_some(), "no overflow");
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::FrameGuard;

    pub fn service(_cc: &eframe::CreationContext<'_>, _custom_titlebar: bool) -> Option<FrameGuard> {
        None
    }
}
