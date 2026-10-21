//! Motion: how things move, kept short and calm, as a mail client's should be. Things coming in
//! decelerate into place, things leaving accelerate away, and a change of state (a hover, a
//! selection) eases both ways; nothing takes longer than a quarter of a second. With Play
//! animations off (File › Options) everything lands at once.
//!
//! The curves are cubic Béziers like CSS's `cubic-bezier()`, solved here: x(s) = t is found by
//! Newton's method, with bisection when the slope is too flat to trust, and y(s) is the eased
//! value.

use egui::{Context, Id};

/// A cubic Bézier easing curve from (0, 0) to (1, 1) through control points (x1, y1), (x2, y2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl Curve {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Curve {
        Curve { x1, y1, x2, y2 }
    }

    /// One coordinate of the curve at parameter `s` (its end points are 0 and 1).
    fn coord(a: f32, b: f32, s: f32) -> f32 {
        // B(s) = 3(1-s)²s·a + 3(1-s)s²·b + s³, written as a polynomial in s.
        let (c3, c2, c1) = (1.0 + 3.0 * a - 3.0 * b, 3.0 * b - 6.0 * a, 3.0 * a);
        ((c3 * s + c2) * s + c1) * s
    }

    fn slope(a: f32, b: f32, s: f32) -> f32 {
        let (c3, c2, c1) = (1.0 + 3.0 * a - 3.0 * b, 3.0 * b - 6.0 * a, 3.0 * a);
        (3.0 * c3 * s + 2.0 * c2) * s + c1
    }

    /// The eased value at time `t` (0 to 1; outside, held at the ends).
    pub fn at(&self, t: f32) -> f32 {
        if t.is_nan() || t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        // Newton: a few steps from s = t usually land within a thousandth.
        let mut s = t;
        for _ in 0..8 {
            let err = Self::coord(self.x1, self.x2, s) - t;
            if err.abs() < 1e-5 {
                return Self::coord(self.y1, self.y2, s);
            }
            let d = Self::slope(self.x1, self.x2, s);
            if d.abs() < 1e-6 {
                break;
            }
            s = (s - err / d).clamp(0.0, 1.0);
        }
        // Bisection: x(s) rises from 0 to 1 for any curve whose x controls lie in 0..=1.
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        s = t;
        for _ in 0..30 {
            let x = Self::coord(self.x1, self.x2, s);
            if (x - t).abs() < 1e-5 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = 0.5 * (lo + hi);
        }
        Self::coord(self.y1, self.y2, s)
    }
}

/// Coming in: fast at first, settling gently into place.
pub const DECELERATE: Curve = Curve::new(0.1, 0.9, 0.2, 1.0);
/// Leaving: starting gently, gone quickly.
pub const ACCELERATE: Curve = Curve::new(0.9, 0.1, 1.0, 0.2);
/// A change of state either way (hover, selection, a moving marker).
pub const STANDARD: Curve = Curve::new(0.33, 0.0, 0.67, 1.0);

pub fn decelerate(t: f32) -> f32 {
    DECELERATE.at(t)
}

pub fn accelerate(t: f32) -> f32 {
    ACCELERATE.at(t)
}

pub fn standard(t: f32) -> f32 {
    STANDARD.at(t)
}

/// Durations, in seconds.
pub const FAST: f32 = 0.10;
pub const NORMAL: f32 = 0.18;
pub const GENTLE: f32 = 0.25;

fn off_id() -> Id {
    Id::unique("motion-off")
}

/// Whether things move (Play animations).
pub fn enabled(ctx: &Context) -> bool {
    !ctx.data(|d| d.get_temp::<bool>(off_id())).unwrap_or(false)
}

/// Turn motion on or off: egui's own (panels sliding, windows fading, smooth scrolling) too.
pub fn set_enabled(ctx: &Context, on: bool) {
    if enabled(ctx) == on && ctx.global_style().animation_time == if on { NORMAL } else { 0.0 } {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(off_id(), !on));
    ctx.global_style_mut(|s| {
        s.animation_time = if on { NORMAL } else { 0.0 };
        s.scroll_animation = if on { egui::style::ScrollAnimation::default() } else { egui::style::ScrollAnimation::none() };
    });
}

/// 0 to 1 easing toward `on` over `secs` (a hover, a selection); at once with motion off.
pub fn toward(ctx: &Context, id: Id, on: bool, secs: f32) -> f32 {
    if !enabled(ctx) {
        return if on { 1.0 } else { 0.0 };
    }
    ctx.animate_bool_with_time_and_easing(id, on, secs, standard)
}

/// A value gliding to `target` over `secs` (a marker moving between tabs); at once with motion off.
pub fn glide(ctx: &Context, id: Id, target: f32, secs: f32) -> f32 {
    ctx.animate_value_with_time(id, target, if enabled(ctx) { secs } else { 0.0 })
}

/// 0 to 1 since `id` came into view (drawn this frame but not the last), decelerating: for
/// something appearing. 1 at once with motion off.
pub fn entrance(ctx: &Context, id: Id, secs: f32) -> f32 {
    if !enabled(ctx) {
        return 1.0;
    }
    let (now, frame) = (ctx.input(|i| i.time), ctx.cumulative_frame_nr());
    let key = id.with("entrance");
    let start = ctx.data_mut(|d| {
        let (start, last) = d.get_temp::<(f64, u64)>(key).unwrap_or((now, frame));
        // Not drawn last frame: it's coming in again.
        let start = if frame > last.saturating_add(1) { now } else { start };
        d.insert_temp(key, (start, frame));
        start
    });
    let t = ((now - start) as f32 / secs.max(0.001)).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    decelerate(t)
}

/// A colour between `a` and `b` (`k` from 0 to 1).
pub fn mix(a: egui::Color32, b: egui::Color32, k: f32) -> egui::Color32 {
    a.lerp_to_gamma(b, k.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The curves agree with what browsers compute for the same `cubic-bezier()`.
    #[test]
    fn curves_ease_as_css_does() {
        // linear is the identity.
        let linear = Curve::new(0.0, 0.0, 1.0, 1.0);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            assert!((linear.at(t) - t).abs() < 1e-3, "{t}");
        }
        // CSS `ease-out` (0, 0, 0.58, 1) at 0.5 is about 0.6847.
        assert!((Curve::new(0.0, 0.0, 0.58, 1.0).at(0.5) - 0.6847).abs() < 2e-3);
        // `ease-in-out` is symmetric: 0.5 at 0.5.
        assert!((Curve::new(0.42, 0.0, 0.58, 1.0).at(0.5) - 0.5).abs() < 1e-3);
        // Decelerating is well ahead early on; accelerating well behind; both reach the ends.
        assert!(DECELERATE.at(0.2) > 0.6 && ACCELERATE.at(0.2) < 0.1);
        for c in [DECELERATE, ACCELERATE, STANDARD] {
            assert_eq!((c.at(0.0), c.at(1.0)), (0.0, 1.0));
            assert_eq!((c.at(-3.0), c.at(9.0), c.at(f32::NAN)), (0.0, 1.0, 0.0));
            // Never runs backwards.
            let mut last = 0.0;
            for i in 0..=200 {
                let v = c.at(i as f32 / 200.0);
                assert!(v + 1e-4 >= last, "{c:?} at {i}");
                last = v;
            }
        }
    }

    /// Off: everything lands at once.
    #[test]
    fn off_means_at_once() {
        let ctx = Context::default();
        set_enabled(&ctx, false);
        assert_eq!(toward(&ctx, Id::unique("x"), true, NORMAL), 1.0);
        assert_eq!(entrance(&ctx, Id::unique("y"), NORMAL), 1.0);
        assert_eq!(ctx.global_style().animation_time, 0.0);
        set_enabled(&ctx, true);
        assert!(enabled(&ctx));
        assert_eq!(ctx.global_style().animation_time, NORMAL);
    }
}
