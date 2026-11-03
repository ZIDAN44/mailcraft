//! The six reactions offered (issue #39) drawn in colour, in code: the fonts MailCraft draws text
//! with have no colour emoji, and their outlines of the three faces looked all alike. Each is
//! MailCraft's own drawing on a 16-unit grid; any other emoji is drawn as text.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

const GOLD: Color32 = Color32::from_rgb(0xF7, 0xC9, 0x48);
const GOLD_EDGE: Color32 = Color32::from_rgb(0xB7, 0x86, 0x0B);
const INK: Color32 = Color32::from_rgb(0x4A, 0x32, 0x12);
const RED: Color32 = Color32::from_rgb(0xE0, 0x3B, 0x4C);
const TEAR: Color32 = Color32::from_rgb(0x4F, 0x9D, 0xE8);

/// Points on the grid, in the rect.
struct Grid {
    r: Rect,
}

impl Grid {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x / 16.0 * self.r.width(), self.r.min.y + y / 16.0 * self.r.height())
    }
    fn len(&self, d: f32) -> f32 {
        d / 16.0 * self.r.width()
    }
    fn poly(&self, p: &Painter, pts: &[(f32, f32)], fill: Color32, edge: Stroke) {
        p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| self.pt(x, y)).collect(), fill, edge));
    }
    fn line(&self, p: &Painter, pts: &[(f32, f32)], s: Stroke) {
        p.add(Shape::line(pts.iter().map(|&(x, y)| self.pt(x, y)).collect(), s));
    }
    /// Points along an arc (angles in degrees, 0 to the right, clockwise on screen).
    fn arc(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
        (0..=12).map(|i| from + (to - from) * i as f32 / 12.0).map(|a| (cx + r * a.to_radians().cos(), cy + r * a.to_radians().sin())).collect()
    }
}

/// Whether `emoji` is one MailCraft draws itself.
pub fn drawn(emoji: &str) -> bool {
    mailcraft_engine::mime::REACTIONS.contains(&emoji)
}

/// Draw `emoji` filling `r` (a square); `false` when it isn't one of the six (then draw it as text).
pub fn paint(p: &Painter, r: Rect, emoji: &str) -> bool {
    let g = Grid { r };
    let thin = Stroke::new(g.len(0.9).max(1.0), GOLD_EDGE);
    let ink = Stroke::new(g.len(1.2).max(1.0), INK);
    let face = |p: &Painter| {
        p.circle_filled(g.pt(8.0, 8.0), g.len(7.0), GOLD);
        p.circle_stroke(g.pt(8.0, 8.0), g.len(7.0), thin);
    };
    match emoji {
        // A thumb up from a closed hand.
        "👍" => {
            g.poly(p, &[(6.2, 7.0), (7.4, 2.2), (8.8, 2.0), (9.4, 3.2), (8.8, 7.0)], GOLD, thin);
            g.poly(p, &[(4.6, 7.0), (12.6, 7.0), (13.2, 8.2), (12.6, 13.6), (4.6, 13.6)], GOLD, thin);
            for y in [9.2, 11.0, 12.6] {
                g.line(p, &[(9.0, y), (12.8, y)], thin);
            }
            g.poly(p, &[(2.4, 7.2), (4.6, 7.2), (4.6, 13.6), (2.4, 13.6)], GOLD_EDGE, Stroke::NONE);
        }
        // A heart: two lobes and a point.
        "❤️" | "❤" => {
            p.circle_filled(g.pt(5.4, 6.0), g.len(3.3), RED);
            p.circle_filled(g.pt(10.6, 6.0), g.len(3.3), RED);
            g.poly(p, &[(2.25, 7.0), (13.75, 7.0), (8.0, 14.2)], RED, Stroke::NONE);
        }
        // A party popper: a cone, its streamers and confetti.
        "🎉" => {
            g.poly(p, &[(2.0, 14.0), (5.4, 5.6), (10.4, 10.6)], Color32::from_rgb(0xF5, 0x9E, 0x0B), thin);
            g.line(p, &[(7.4, 6.0), (8.6, 3.4), (10.4, 4.4), (12.0, 1.8)], Stroke::new(g.len(1.0), Color32::from_rgb(0xE0, 0x3B, 0x8A)));
            g.line(p, &[(10.2, 8.4), (12.6, 7.4), (13.6, 9.4), (15.0, 8.4)], Stroke::new(g.len(1.0), Color32::from_rgb(0x3B, 0x82, 0xF6)));
            for (x, y, c) in [
                (11.8, 4.8, Color32::from_rgb(0x22, 0xC5, 0x5E)),
                (13.6, 3.0, RED),
                (14.0, 12.0, Color32::from_rgb(0x8B, 0x5C, 0xF6)),
                (6.2, 2.4, TEAR),
            ] {
                p.circle_filled(g.pt(x, y), g.len(0.9), c);
            }
        }
        // Laughing: eyes shut tight, a wide open mouth.
        "😆" => {
            face(p);
            g.line(p, &[(4.2, 5.0), (6.6, 6.4), (4.2, 7.6)], ink);
            g.line(p, &[(11.8, 5.0), (9.4, 6.4), (11.8, 7.6)], ink);
            g.poly(p, &Grid::arc(8.0, 9.2, 4.2, 0.0, 180.0), Color32::from_rgb(0x7A, 0x1F, 0x2B), Stroke::NONE);
        }
        // Surprised: round eyes, a round open mouth.
        "😮" => {
            face(p);
            p.circle_filled(g.pt(5.6, 6.4), g.len(1.1), INK);
            p.circle_filled(g.pt(10.4, 6.4), g.len(1.1), INK);
            p.circle_filled(g.pt(8.0, 11.0), g.len(2.0), Color32::from_rgb(0x7A, 0x1F, 0x2B));
        }
        // Sad: a frown, and a tear under one eye.
        "😢" => {
            face(p);
            p.circle_filled(g.pt(5.6, 6.6), g.len(1.0), INK);
            p.circle_filled(g.pt(10.4, 6.6), g.len(1.0), INK);
            g.line(p, &Grid::arc(8.0, 13.6, 3.6, 220.0, 320.0), ink);
            p.circle_filled(g.pt(11.0, 10.4), g.len(1.2), TEAR);
            g.poly(p, &[(10.0, 10.0), (11.0, 7.8), (12.0, 10.0)], TEAR, Stroke::NONE);
        }
        _ => return false,
    }
    true
}
