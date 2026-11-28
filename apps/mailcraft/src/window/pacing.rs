//! How often MailCraft's windows are drawn, on Linux and the BSDs, while more than one is open.
//!
//! Typing in a message written in a window of its own lagged where the reading pane kept up
//! (issue #287): every window of MailCraft's is drawn in the main window's pass (egui's immediate
//! viewports), and each waited for the screen's refresh by itself before the next was drawn
//! (egui#5836). With two windows open a pass took two refreshes, so the window typed in showed
//! half the frames it could, each key later. While a second window is open, frames go to the
//! screen without waiting for its refresh (`AutoNoVsync`: Mailbox or Immediate where the driver
//! has them, else waiting as before), and the pass is held to the screen's rate here instead,
//! once for all the windows, so a frame drawn again and again (an animation) doesn't run as fast
//! as the computer can. A pass after a pause isn't held: a key after a pause shows at once.
//!
//! Not on Windows, whose presents don't wait window by window (and where not waiting may tear),
//! nor on a Mac, where it hasn't been seen.

use std::time::{Duration, Instant};

use eframe::egui_wgpu::SurfaceConfig;
use eframe::wgpu::PresentMode;

/// The screen's refresh when the system doesn't say.
const DEFAULT_HZ: f64 = 60.0;
/// The rates believed (a screen saying 0 Hz, or a million, isn't).
const HZ_RANGE: std::ops::RangeInclusive<f64> = 24.0..=500.0;

/// The present mode for `windows` windows open: without waiting for the screen once there are
/// two or more.
pub(crate) fn present_mode(windows: usize) -> PresentMode {
    if windows > 1 { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync }
}

/// Whether this pass waits for the screen: with more than one window open, and only the first
/// pass of a frame (`pass`: egui's index of it). A pass egui runs again in the same frame (a
/// layout that moved) was held a second time, mid-frame, and a key read behind it.
pub(crate) fn holds(windows: usize, pass: usize) -> bool {
    windows > 1 && pass == 0
}

/// How long to wait before a pass that follows one begun `since` ago, for a screen refreshing
/// `hz` times a second: what's left of one refresh, nothing once one has gone by.
pub(crate) fn hold(since: Duration, hz: Option<f64>) -> Duration {
    let hz = hz.filter(|h| HZ_RANGE.contains(h)).unwrap_or(DEFAULT_HZ);
    Duration::from_secs_f64(1.0 / hz).saturating_sub(since)
}

/// The windows' pacing, followed every pass.
#[derive(Default)]
pub(crate) struct Pacing {
    /// When the last pass began.
    last: Option<Instant>,
}

impl Pacing {
    /// Every pass, before it's drawn: the present mode for the windows open, and with more than
    /// one, the pass held to the screen's rate.
    pub(crate) fn follow(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let windows = ctx.input(|i| i.raw.viewports.len()).max(1);
        if let Some(config) = frame.wgpu_surface_config() {
            let mode = present_mode(windows);
            if config.present_mode != mode {
                // Asked for: the driver may not have it (wgpu's OpenGL on Linux waits for the
                // screen whatever is asked; the adapter's backend is in the log at the start).
                log::info!("windows open: {windows}, asking to draw with {mode:?}");
                frame.set_wgpu_surface_config(SurfaceConfig { present_mode: mode, ..config });
            }
        }
        if holds(windows, ctx.current_pass_index())
            && let Some(last) = self.last
        {
            let hz =
                frame.winit_window().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map(|mhz| f64::from(mhz) / 1000.0);
            let wait = hold(last.elapsed(), hz);
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
        }
        if ctx.current_pass_index() == 0 {
            self.last = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One window waits for the screen as before; two or more don't, each by itself.
    #[test]
    fn a_second_window_draws_without_waiting_for_the_screen() {
        assert_eq!(present_mode(1), PresentMode::AutoVsync);
        assert_eq!(present_mode(2), PresentMode::AutoNoVsync);
        assert_eq!(present_mode(5), PresentMode::AutoNoVsync);
    }

    /// Only a frame's first pass waits, and only with two windows or more: one egui runs again in
    /// the same frame doesn't wait a second time (issue #287).
    #[test]
    fn only_a_frames_first_pass_waits() {
        assert!(holds(2, 0));
        assert!(!holds(2, 1), "a pass run again in the same frame");
        assert!(!holds(1, 0), "one window waits for the screen by itself");
    }

    /// The pass waits out the rest of one refresh at the screen's rate (60 Hz when it doesn't say,
    /// or says something unbelievable), and not at all after a pause.
    #[test]
    fn a_pass_waits_out_the_rest_of_a_refresh() {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        assert!((ms(hold(Duration::from_millis(4), Some(60.0))) - 12.67).abs() < 0.1);
        assert!((ms(hold(Duration::from_millis(4), None)) - 12.67).abs() < 0.1);
        assert!((ms(hold(Duration::from_millis(1), Some(144.0))) - 5.94).abs() < 0.1);
        assert!((ms(hold(Duration::from_millis(4), Some(0.0))) - 12.67).abs() < 0.1, "0 Hz isn't believed");
        assert!((ms(hold(Duration::from_millis(4), Some(1e7))) - 12.67).abs() < 0.1);
        assert!(hold(Duration::from_millis(17), Some(60.0)).is_zero());
        assert!(hold(Duration::from_secs(30), Some(60.0)).is_zero(), "a key after a pause isn't held");
    }
}
