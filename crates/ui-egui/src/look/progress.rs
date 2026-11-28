//! Long work going on in the background (Send / Receive, the search index's build, a data file's
//! import and its compaction, an update's download): a bar of how far it has come, in the
//! Activity panel and where the work is (a data file's import in the folder pane), and a ring
//! round the Activity button and beside an account updating.

use std::time::Duration;

use egui::{CornerRadius, Rect, Response, Sense, Ui, Vec2, vec2};

use crate::theme::Tokens;

/// An indeterminate bar moves at most this often: a long import doesn't keep the window drawing
/// at its full rate.
pub(crate) const STEP: Duration = Duration::from_millis(100);
/// It slides a segment this share of its length across in this many steps (1.6 s).
const SLIDE: f32 = 0.3;
const PERIOD: u64 = 16;

/// A bar `size` big, `permille` thousandths full, filled from where the interface's text starts.
/// `None` (how far isn't known): a segment sliding along it, ten steps a second at most and only
/// while it's drawn; with animations off, a still one.
pub(crate) fn bar(ui: &mut Ui, t: &Tokens, permille: Option<u16>, size: Vec2) -> Response {
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    let round = CornerRadius::same(if size.y >= 6.0 { 3 } else { 2 });
    ui.painter().rect_filled(r, round, t.border);
    let fill = match permille {
        Some(p) => Rect::from_min_size(r.min, vec2(r.width() * f32::from(p.min(1000)) / 1000.0, r.height())),
        None => {
            let moving = crate::motion::enabled(ui.ctx());
            let at = if moving {
                ui.ctx().request_repaint_after(STEP);
                slide_at(ui.input(|i| i.time))
            } else {
                (1.0 - SLIDE) / 2.0
            };
            Rect::from_min_size(r.min + vec2(r.width() * at, 0.0), vec2(r.width() * SLIDE, r.height()))
        }
    };
    ui.painter().rect_filled(crate::bidi::Mirror::new(r).rect(fill), round, t.accent);
    resp
}

/// How often a turning ring is drawn again: smooth (it stepped ten times a second, as a bar may,
/// and jerked round), yet not at the screen's full rate for as long as an index takes.
pub(crate) const TURN_STEP: Duration = Duration::from_millis(33);
/// How long a turning ring takes to go round once (seconds).
const TURN: f64 = 1.2;

/// A ring of `radius` round `center`, `share` of it (0 to 1) drawn from the top, clockwise (`None`:
/// how far isn't known, a quarter of it turning smoothly with the time; with animations off, a
/// still quarter), over a faint whole one, in `color`.
pub(crate) fn ring(ui: &Ui, center: egui::Pos2, radius: f32, share: Option<f32>, color: egui::Color32, time: f64) {
    use std::f32::consts::TAU;
    let width = (radius / 6.0).clamp(1.25, 2.0);
    let faint = color.gamma_multiply(0.3);
    ui.painter().circle_stroke(center, radius, egui::Stroke::new(width, faint));
    let (from, share) = match share {
        Some(s) => (0.0, s.clamp(0.0, 1.0)),
        None if crate::motion::enabled(ui.ctx()) => {
            ui.ctx().request_repaint_after(TURN_STEP);
            (turn_at(time), 0.25)
        }
        None => (0.0, 0.25),
    };
    if share <= 0.0 {
        return;
    }
    // From the top (a quarter turn back from egui's start, the right), clockwise, as a clock's.
    let start = -TAU / 4.0 + from * TAU;
    let n = ((share * 48.0).ceil() as usize).max(2);
    let points: Vec<egui::Pos2> =
        (0..=n).map(|i| start + share * TAU * i as f32 / n as f32).map(|a| center + radius * vec2(a.cos(), a.sin())).collect();
    ui.painter().add(egui::Shape::line(points, egui::Stroke::new(width, color)));
}

/// How far round a turning ring is at `time` (seconds), 0 to 1: with the time itself, not in steps.
pub(crate) fn turn_at(time: f64) -> f32 {
    if !time.is_finite() || time <= 0.0 {
        return 0.0;
    }
    (time.rem_euclid(TURN) / TURN) as f32
}

/// Where the sliding segment starts at `time` (seconds), as a share of the bar: moved in whole
/// steps of [`STEP`], so the frames asked for are all it needs.
pub(crate) fn slide_at(time: f64) -> f32 {
    if !time.is_finite() || time <= 0.0 {
        return 0.0;
    }
    // `as` saturates: a clock past u64's steps only stops the slide.
    let steps = (time / STEP.as_secs_f64()).floor() as u64;
    (steps % PERIOD) as f32 / PERIOD as f32 * (1.0 - SLIDE)
}

/// A percentage of `permille` thousandths, rounded down (99 at most: the end is said by the
/// work, not by its bar).
pub(crate) fn percent(permille: u16) -> u16 {
    (permille / 10).min(99)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentage_never_says_done() {
        assert_eq!([0, 9, 10, 425, 999, 1000, u16::MAX].map(super::percent), [0, 0, 1, 42, 99, 99, 99]);
    }

    /// The sliding segment moves in tenths of a second (a frame in between draws it where it
    /// was), stays on the bar, and goes round.
    #[test]
    fn an_indeterminate_bar_moves_ten_times_a_second_at_most() {
        assert_eq!(slide_at(0.0), slide_at(0.099));
        assert!(slide_at(0.1) > slide_at(0.0));
        assert_eq!(slide_at(0.25), slide_at(0.2));
        let mut seen = std::collections::BTreeSet::new();
        for ms in 0..10_000 {
            let at = slide_at(f64::from(ms) / 1000.0);
            assert!((0.0..=1.0 - SLIDE).contains(&at), "{at}");
            seen.insert(at.to_bits());
        }
        assert!(seen.len() <= 16, "{} places in 10 s", seen.len());
        assert_eq!(slide_at(1.65), slide_at(0.05), "round again");
        assert_eq!(slide_at(-5.0), 0.0);
        assert_eq!((slide_at(f64::INFINITY), slide_at(f64::NAN)), (0.0, 0.0));
    }

    /// The bar asks for the next frame no sooner than a tenth of a second: the window isn't
    /// drawn at its full rate for as long as an import runs.
    #[test]
    fn an_indeterminate_bar_asks_for_few_frames() {
        let ctx = egui::Context::default();
        let t = Tokens::light();
        // The frame after the bar is drawn (egui's own first frames ask for more at once).
        let delay = |permille: Option<u16>| {
            let mut delay = None;
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    bar(ui, &t, permille, vec2(120.0, 6.0));
                });
                out.textures_delta.clear();
                delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
            }
            delay
        };
        let d = delay(None);
        assert!(d.is_some_and(|d| d >= STEP && d < Duration::from_secs(1)), "{d:?}");
        // A bar that knows how far, or animations off: no frame asked for at all.
        let d = delay(Some(500));
        assert!(d.is_some_and(|d| d > Duration::from_secs(60)), "{d:?}");
        crate::motion::set_enabled(&ctx, false);
        let d = delay(None);
        assert!(d.is_some_and(|d| d > Duration::from_secs(60)), "{d:?}");
    }
}
