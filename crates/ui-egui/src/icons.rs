//! Icons, drawn in code on a 16-unit grid. All original: simple line drawings of everyday
//! objects (envelope, tray, arrow, flag…), no third-party or Microsoft artwork.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    NewMail,
    Delete,
    Archive,
    Junk,
    Reply,
    ReplyAll,
    Forward,
    Move,
    Read,
    Unread,
    Flag,
    Sync,
    Inbox,
    Sent,
    Drafts,
    Folder,
    Search,
    Attach,
    Gear,
    Sun,
    Moon,
    Close,
    ChevronDown,
    ChevronRight,
    Star,
    Image,
    File,
    Send,
    Plus,
    Layout,
    Person,
    Warning,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    s: Stroke,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x / 16.0 * self.r.width(), self.r.min.y + y / 16.0 * self.r.height())
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::line(v, self.s));
    }
    fn closed(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::closed_line(v, self.s));
    }
    fn filled(&self, pts: &[(f32, f32)], c: Color32) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.p.add(egui::Shape::convex_polygon(v, c, Stroke::NONE));
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.closed(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    fn circle(&self, x: f32, y: f32, rad: f32) {
        self.p.circle_stroke(self.pt(x, y), rad / 16.0 * self.r.width(), self.s);
    }
    fn arc(&self, cx: f32, cy: f32, rad: f32, a0: f32, a1: f32) {
        let n = 14;
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                (cx + rad * a.cos(), cy + rad * a.sin())
            })
            .collect();
        self.line(&pts);
    }
}

/// Paint `icon` into `rect` (square works best) in `color`.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let w = (rect.width() / 16.0 * 1.35).clamp(1.0, 2.6);
    let pen = Pen { p, r: rect, s: Stroke::new(w, color) };
    use std::f32::consts::PI;
    match icon {
        Icon::NewMail => {
            pen.line(&[(9.0, 4.0), (2.0, 4.0), (2.0, 13.0), (14.0, 13.0), (14.0, 9.0)]);
            pen.line(&[(2.0, 4.5), (8.0, 9.0), (10.0, 7.5)]);
            pen.line(&[(13.0, 1.5), (13.0, 6.5)]);
            pen.line(&[(10.5, 4.0), (15.5, 4.0)]);
        }
        Icon::Unread => {
            pen.rect(2.0, 4.0, 14.0, 13.0);
            pen.line(&[(2.0, 4.5), (8.0, 9.0), (14.0, 4.5)]);
        }
        Icon::Read => {
            pen.closed(&[(2.0, 7.0), (8.0, 3.0), (14.0, 7.0), (14.0, 14.0), (2.0, 14.0)]);
            pen.line(&[(2.0, 7.0), (8.0, 11.0), (14.0, 7.0)]);
        }
        Icon::Delete => {
            pen.line(&[(2.5, 4.0), (13.5, 4.0)]);
            pen.line(&[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)]);
            pen.closed(&[(3.8, 4.0), (4.8, 14.0), (11.2, 14.0), (12.2, 4.0)]);
            pen.line(&[(6.8, 6.5), (7.0, 11.5)]);
            pen.line(&[(9.2, 6.5), (9.0, 11.5)]);
        }
        Icon::Archive => {
            pen.rect(1.5, 2.5, 14.5, 5.5);
            pen.line(&[(2.5, 5.5), (2.5, 13.5), (13.5, 13.5), (13.5, 5.5)]);
            pen.line(&[(6.0, 8.0), (10.0, 8.0)]);
        }
        Icon::Junk => {
            pen.closed(&[(8.0, 1.5), (13.5, 3.5), (13.0, 9.0), (8.0, 14.5), (3.0, 9.0), (2.5, 3.5)]);
            pen.line(&[(5.5, 5.5), (10.5, 10.5)]);
            pen.line(&[(10.5, 5.5), (5.5, 10.5)]);
        }
        Icon::Reply => {
            pen.line(&[(6.0, 3.0), (2.0, 7.0), (6.0, 11.0)]);
            pen.line(&[(2.0, 7.0), (9.0, 7.0)]);
            pen.arc(9.0, 11.0, 4.5, -PI / 2.0, 0.0);
            pen.line(&[(13.5, 11.0), (13.5, 13.5)]);
        }
        Icon::ReplyAll => {
            pen.line(&[(5.0, 3.5), (1.5, 7.0), (5.0, 10.5)]);
            pen.line(&[(8.5, 3.0), (4.5, 7.0), (8.5, 11.0)]);
            pen.line(&[(4.5, 7.0), (10.0, 7.0)]);
            pen.arc(10.0, 11.0, 4.0, -PI / 2.0, 0.0);
            pen.line(&[(14.0, 11.0), (14.0, 13.5)]);
        }
        Icon::Forward => {
            pen.line(&[(10.0, 3.0), (14.0, 7.0), (10.0, 11.0)]);
            pen.line(&[(14.0, 7.0), (7.0, 7.0)]);
            pen.arc(7.0, 11.0, 4.5, -PI / 2.0, -PI);
            pen.line(&[(2.5, 11.0), (2.5, 13.5)]);
        }
        Icon::Move | Icon::Folder => {
            pen.closed(&[(1.5, 3.0), (6.0, 3.0), (7.5, 4.8), (14.5, 4.8), (14.5, 13.5), (1.5, 13.5)]);
            if icon == Icon::Move {
                pen.line(&[(5.0, 9.2), (11.0, 9.2)]);
                pen.line(&[(8.8, 7.0), (11.0, 9.2), (8.8, 11.4)]);
            }
        }
        Icon::Flag => {
            pen.line(&[(3.5, 1.5), (3.5, 15.0)]);
            pen.closed(&[(3.5, 2.5), (13.0, 2.5), (10.5, 5.8), (13.0, 9.0), (3.5, 9.0)]);
        }
        Icon::Sync => {
            pen.arc(8.0, 8.0, 5.5, -PI * 0.9, -PI * 0.05);
            pen.line(&[(13.5, 3.0), (13.5, 7.7), (9.0, 7.7)]);
            pen.arc(8.0, 8.0, 5.5, PI * 0.1, PI * 0.95);
            pen.line(&[(2.5, 13.0), (2.5, 8.3), (7.0, 8.3)]);
        }
        Icon::Inbox => {
            pen.closed(&[(1.5, 9.0), (4.0, 2.5), (12.0, 2.5), (14.5, 9.0), (14.5, 13.5), (1.5, 13.5)]);
            pen.line(&[(1.5, 9.0), (5.0, 9.0), (6.0, 11.0), (10.0, 11.0), (11.0, 9.0), (14.5, 9.0)]);
        }
        Icon::Sent | Icon::Send => {
            pen.closed(&[(1.5, 7.5), (14.5, 2.0), (10.5, 14.5), (7.5, 9.0)]);
            pen.line(&[(7.5, 9.0), (14.5, 2.0)]);
        }
        Icon::Drafts => {
            pen.closed(&[(10.5, 2.0), (14.0, 5.5), (6.0, 13.5), (2.0, 14.0), (2.5, 10.0)]);
            pen.line(&[(9.0, 3.5), (12.5, 7.0)]);
        }
        Icon::Search => {
            pen.circle(6.8, 6.8, 4.6);
            pen.line(&[(10.2, 10.2), (14.5, 14.5)]);
        }
        Icon::Attach => {
            pen.line(&[(11.5, 5.0), (11.5, 11.0)]);
            pen.arc(8.75, 11.0, 2.75, 0.0, PI);
            pen.line(&[(6.0, 11.0), (6.0, 4.0)]);
            pen.arc(7.75, 4.0, 1.75, PI, 2.0 * PI);
            pen.line(&[(9.5, 4.0), (9.5, 10.5)]);
        }
        Icon::Gear => {
            pen.circle(8.0, 8.0, 2.4);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                pen.line(&[(8.0 + 4.3 * a.cos(), 8.0 + 4.3 * a.sin()), (8.0 + 6.2 * a.cos(), 8.0 + 6.2 * a.sin())]);
            }
            pen.circle(8.0, 8.0, 4.4);
        }
        Icon::Sun => {
            pen.circle(8.0, 8.0, 3.2);
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                pen.line(&[(8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin()), (8.0 + 6.8 * a.cos(), 8.0 + 6.8 * a.sin())]);
            }
        }
        Icon::Moon => {
            pen.arc(8.0, 8.0, 6.0, PI * 0.35, PI * 1.85);
            pen.arc(10.6, 5.6, 4.6, PI * 0.62, PI * 1.55);
        }
        Icon::Close => {
            pen.line(&[(3.5, 3.5), (12.5, 12.5)]);
            pen.line(&[(12.5, 3.5), (3.5, 12.5)]);
        }
        Icon::ChevronDown => pen.line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Icon::ChevronRight => pen.line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -PI / 2.0 + i as f32 * PI / 5.0;
                    let rad = if i % 2 == 0 { 6.6 } else { 2.9 };
                    (8.0 + rad * a.cos(), 8.4 + rad * a.sin())
                })
                .collect();
            pen.closed(&pts);
        }
        Icon::Image => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.circle(5.5, 6.0, 1.3);
            pen.line(&[(1.5, 12.0), (6.0, 8.0), (9.0, 10.5), (11.0, 9.0), (14.5, 12.0)]);
        }
        Icon::File => {
            pen.closed(&[(3.0, 1.5), (10.0, 1.5), (13.0, 4.5), (13.0, 14.5), (3.0, 14.5)]);
            pen.line(&[(10.0, 1.5), (10.0, 4.5), (13.0, 4.5)]);
        }
        Icon::Plus => {
            pen.line(&[(8.0, 2.5), (8.0, 13.5)]);
            pen.line(&[(2.5, 8.0), (13.5, 8.0)]);
        }
        Icon::Layout => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(6.0, 2.5), (6.0, 13.5)]);
            pen.line(&[(6.0, 8.0), (14.5, 8.0)]);
        }
        Icon::Person => {
            pen.circle(8.0, 5.0, 3.0);
            pen.arc(8.0, 15.0, 6.0, PI, 2.0 * PI);
        }
        Icon::Warning => {
            pen.closed(&[(8.0, 1.5), (15.0, 14.0), (1.0, 14.0)]);
            pen.line(&[(8.0, 6.0), (8.0, 9.8)]);
            pen.filled(&[(7.3, 11.4), (8.7, 11.4), (8.7, 12.6), (7.3, 12.6)], color);
        }
    }
}

/// An icon-sized rect centred in `rect`.
pub fn centered(rect: Rect, size: f32) -> Rect {
    Rect::from_center_size(rect.center(), Vec2::splat(size))
}
