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
    ChevronUp,
    /// The calendar's views: a day, five days, a week, a month, the schedule (each told apart; one
    /// glyph stood for all five).
    ViewDay,
    ViewWorkWeek,
    ViewWeek,
    ViewMonth,
    ViewSchedule,
    /// A list (People's list view; it had a phone).
    ViewList,
    /// A password shown (an eye), and hidden (the eye struck through): its field's toggle.
    Eye,
    EyeOff,
    /// High importance (an exclamation mark) and low (an arrow down): a task's or a message's
    /// priority (a star and a "!" glyph stood for it).
    Priority,
    LowPriority,
    /// Each for one thing (a clock, a person, a pencil and a star each stood for several): a
    /// reminder's bell; a meeting's length (an hourglass); Show as (a time block, part taken);
    /// changing something (a pencil on a line; the Drafts folder keeps the pencil alone); saving
    /// (a disk); a signature (a pen's line on a rule); a person added; a download (an arrow down
    /// onto a line: an update fetched, which had the Send / Receive arrows).
    Bell,
    Timer,
    Availability,
    Edit,
    Save,
    Signature,
    PersonAdd,
    Download,
    /// Work going on in the background (the Activity button): a pulse, as a heart monitor's line.
    Activity,
    ChevronRight,
    ChevronLeft,
    Star,
    Image,
    File,
    Send,
    Plus,
    Layout,
    Person,
    Warning,
    Outbox,
    Pin,
    Clock,
    Tag,
    Print,
    Back,
    Info,
    Funnel,
    Calendar,
    People,
    Phone,
    Location,
    /// A window with an arrow leaving it (pop out), and the reverse (pop in).
    PopOut,
    PopIn,
    /// The message editor's ribbon.
    ListBullet,
    ListNumber,
    AlignLeft,
    AlignCenter,
    AlignRight,
    IndentMore,
    IndentLess,
    Link,
    ClearFormat,
    AddressBook,
    /// A sticky note (Notes).
    Note,
    /// A box with a tick, and lines beside it (Tasks).
    Task,
    /// Two chevrons running ahead (Quick Steps: several things in one go).
    Steps,
    /// Three lines, and an arrow leading off from them (Rules: mail sorted as it comes).
    Rules,
    /// A padlock (encrypted mail).
    Lock,
    /// A rosette with a tick (signed mail).
    Seal,
    /// A tick on its own (a follow-up marked complete).
    Done,
    /// A smiling face (reactions).
    Smile,
    /// Two speech bubbles, one behind the other (messages grouped by conversation).
    Conversation,
    /// A box with lines of text in it (a line of each message shown in the list).
    Preview,
    /// Rows packed close (a compact list).
    Compact,
    /// An arrow up beside an arrow down (the order a list is sorted in).
    Sort,
    /// Angle brackets around a slash (a message's source).
    Source,
    /// Two rows of squares (the classic ribbon's big buttons).
    Ribbon,
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
    fn dot(&self, x: f32, y: f32, rad: f32) {
        self.p.circle_filled(self.pt(x, y), rad / 16.0 * self.r.width(), self.s.color);
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
        Icon::ChevronUp => pen.line(&[(4.0, 10.0), (8.0, 6.0), (12.0, 10.0)]),
        Icon::ChevronRight => pen.line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::ChevronLeft => pen.line(&[(10.0, 4.0), (6.0, 8.0), (10.0, 12.0)]),
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
        Icon::ViewDay => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(1.5, 5.5), (14.5, 5.5)]);
            pen.rect(4.5, 7.5, 11.5, 11.0);
        }
        Icon::ViewWorkWeek | Icon::ViewWeek => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(1.5, 5.5), (14.5, 5.5)]);
            let n = if icon == Icon::ViewWeek { 7 } else { 5 };
            for i in 1..n {
                let x = 1.5 + 13.0 * i as f32 / n as f32;
                pen.line(&[(x, 5.5), (x, 13.5)]);
            }
        }
        Icon::ViewMonth => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            for y in [5.5, 8.2, 10.9] {
                pen.line(&[(1.5, y), (14.5, y)]);
            }
            for x in [4.75, 8.0, 11.25] {
                pen.line(&[(x, 5.5), (x, 13.5)]);
            }
        }
        Icon::ViewSchedule => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(4.0, 6.0), (10.0, 6.0)]);
            pen.line(&[(6.0, 9.0), (13.0, 9.0)]);
            pen.line(&[(3.5, 12.0), (8.0, 12.0)]);
        }
        Icon::Bell => {
            pen.arc(8.0, 7.0, 4.5, PI, 2.0 * PI);
            pen.line(&[(3.5, 7.0), (3.5, 10.5), (2.5, 11.5), (13.5, 11.5), (12.5, 10.5), (12.5, 7.0)]);
            pen.dot(8.0, 13.6, 1.3);
        }
        Icon::Timer => {
            pen.line(&[(4.0, 2.5), (12.0, 2.5)]);
            pen.line(&[(4.0, 13.5), (12.0, 13.5)]);
            pen.closed(&[(5.0, 2.5), (11.0, 2.5), (8.6, 8.0), (11.0, 13.5), (5.0, 13.5), (7.4, 8.0)]);
        }
        Icon::Availability => {
            pen.rect(2.5, 3.5, 13.5, 12.5);
            pen.filled(&[(2.5, 3.5), (6.5, 3.5), (6.5, 12.5), (2.5, 12.5)], pen.s.color);
        }
        Icon::Edit => {
            pen.closed(&[(10.5, 1.5), (13.0, 4.0), (6.5, 10.5), (3.5, 11.0), (4.0, 8.0)]);
            pen.line(&[(2.0, 14.0), (14.0, 14.0)]);
        }
        Icon::Save => {
            pen.closed(&[(2.5, 2.5), (11.0, 2.5), (13.5, 5.0), (13.5, 13.5), (2.5, 13.5)]);
            pen.line(&[(5.0, 2.5), (5.0, 6.0), (10.0, 6.0), (10.0, 2.5)]);
            pen.rect(4.5, 9.0, 11.5, 13.5);
        }
        Icon::Signature => {
            pen.line(&[(2.0, 11.0), (4.0, 5.5), (6.0, 10.5), (8.0, 6.5), (10.0, 10.5), (14.0, 8.5)]);
            pen.line(&[(2.0, 13.5), (14.0, 13.5)]);
        }
        Icon::PersonAdd => {
            pen.circle(6.5, 5.0, 2.8);
            pen.arc(6.5, 15.0, 5.5, PI, 2.0 * PI);
            pen.line(&[(12.5, 6.0), (12.5, 11.0)]);
            pen.line(&[(10.0, 8.5), (15.0, 8.5)]);
        }
        Icon::Activity => {
            pen.line(&[(1.5, 8.5), (4.5, 8.5), (6.5, 3.5), (9.5, 13.0), (11.5, 8.5), (14.5, 8.5)]);
        }
        Icon::Download => {
            pen.line(&[(8.0, 1.5), (8.0, 10.5)]);
            pen.line(&[(4.5, 7.0), (8.0, 10.5), (11.5, 7.0)]);
            pen.line(&[(2.5, 14.0), (13.5, 14.0)]);
        }
        Icon::Priority => {
            pen.line(&[(8.0, 2.5), (8.0, 10.0)]);
            pen.dot(8.0, 13.2, 1.3);
        }
        Icon::LowPriority => {
            pen.line(&[(8.0, 2.5), (8.0, 13.0)]);
            pen.line(&[(4.5, 9.5), (8.0, 13.0), (11.5, 9.5)]);
        }
        Icon::Eye | Icon::EyeOff => {
            // Two lids meeting at the corners (1.5, 8) and (14.5, 8), 4.5 above and below the
            // middle, round a pupil.
            let (r, a) = (6.94, 0.359);
            pen.arc(8.0, 10.44, r, PI + a, 2.0 * PI - a);
            pen.arc(8.0, 5.56, r, a, PI - a);
            pen.circle(8.0, 8.0, 2.2);
            if icon == Icon::EyeOff {
                pen.line(&[(2.5, 13.5), (13.5, 2.5)]);
            }
        }
        Icon::ViewList => {
            for y in [4.0, 8.0, 12.0] {
                pen.dot(2.5, y, 1.0);
                pen.line(&[(5.5, y), (14.5, y)]);
            }
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
        Icon::Pin => {
            pen.closed(&[(5.5, 2.0), (10.5, 2.0), (10.0, 6.5), (12.5, 9.0), (3.5, 9.0), (6.0, 6.5)]);
            pen.line(&[(8.0, 9.0), (8.0, 14.5)]);
        }
        Icon::Clock => {
            pen.circle(8.0, 8.5, 6.0);
            pen.line(&[(8.0, 5.0), (8.0, 8.5), (10.5, 10.0)]);
        }
        Icon::ListBullet => {
            for y in [4.0, 8.0, 12.0] {
                pen.dot(2.8, y, 1.1);
                pen.line(&[(6.0, y), (14.5, y)]);
            }
        }
        Icon::ListNumber => {
            pen.line(&[(2.0, 2.8), (3.2, 2.0), (3.2, 5.6)]);
            pen.line(&[(1.8, 7.8), (3.0, 7.0), (4.2, 7.8), (1.8, 10.2), (4.4, 10.2)]);
            pen.line(&[(1.8, 12.2), (4.2, 12.2), (3.0, 13.4), (4.2, 14.6), (1.8, 14.8)]);
            for y in [4.0, 9.0, 13.5] {
                pen.line(&[(6.5, y), (14.5, y)]);
            }
        }
        Icon::AlignLeft | Icon::AlignCenter | Icon::AlignRight => {
            for (i, y) in [3.0f32, 6.3, 9.6, 12.9].iter().enumerate() {
                let w = if i % 2 == 0 { 12.5 } else { 8.0 };
                let x0 = match icon {
                    Icon::AlignLeft => 1.8,
                    Icon::AlignCenter => 8.0 - w / 2.0,
                    _ => 14.2 - w,
                };
                pen.line(&[(x0, *y), (x0 + w, *y)]);
            }
        }
        Icon::IndentMore | Icon::IndentLess => {
            for y in [2.5, 13.5] {
                pen.line(&[(1.8, y), (14.5, y)]);
            }
            for y in [6.2, 9.8] {
                pen.line(&[(7.0, y), (14.5, y)]);
            }
            if icon == Icon::IndentMore {
                pen.line(&[(1.8, 5.5), (4.6, 8.0), (1.8, 10.5)]);
            } else {
                pen.line(&[(4.6, 5.5), (1.8, 8.0), (4.6, 10.5)]);
            }
        }
        Icon::Link => {
            pen.line(&[(7.0, 9.0), (9.0, 7.0)]);
            pen.closed(&[(5.6, 7.6), (3.0, 10.2), (3.0, 12.4), (3.6, 13.0), (5.8, 13.0), (8.4, 10.4)]);
            pen.closed(&[(7.6, 5.6), (10.2, 3.0), (12.4, 3.0), (13.0, 3.6), (13.0, 5.8), (10.4, 8.4)]);
        }
        Icon::ClearFormat => {
            pen.line(&[(3.0, 3.0), (11.5, 3.0)]);
            pen.line(&[(7.2, 3.0), (5.0, 13.0)]);
            pen.line(&[(9.5, 9.0), (14.0, 13.5)]);
            pen.line(&[(14.0, 9.0), (9.5, 13.5)]);
        }
        Icon::Note => {
            pen.closed(&[(2.5, 2.5), (13.5, 2.5), (13.5, 10.0), (10.0, 13.5), (2.5, 13.5)]);
            pen.line(&[(13.5, 10.0), (10.0, 10.0), (10.0, 13.5)]);
            pen.line(&[(5.0, 6.0), (11.0, 6.0)]);
            pen.line(&[(5.0, 9.0), (8.5, 9.0)]);
        }
        Icon::Task => {
            pen.rect(1.5, 3.0, 7.0, 8.5);
            pen.line(&[(2.8, 5.8), (4.2, 7.2), (6.0, 4.6)]);
            pen.line(&[(9.5, 5.75), (14.5, 5.75)]);
            pen.rect(1.5, 10.0, 7.0, 15.5);
            pen.line(&[(9.5, 12.75), (14.5, 12.75)]);
        }
        Icon::AddressBook => {
            pen.rect(3.0, 1.5, 13.5, 14.5);
            pen.line(&[(1.5, 4.5), (4.5, 4.5)]);
            pen.line(&[(1.5, 8.0), (4.5, 8.0)]);
            pen.line(&[(1.5, 11.5), (4.5, 11.5)]);
            pen.circle(8.5, 6.5, 1.8);
            pen.line(&[(5.8, 11.8), (6.6, 10.0), (10.4, 10.0), (11.2, 11.8)]);
        }
        Icon::PopOut => {
            pen.line(&[(7.0, 2.5), (2.5, 2.5), (2.5, 13.5), (13.5, 13.5), (13.5, 9.0)]);
            pen.line(&[(7.5, 8.5), (13.5, 2.5)]);
            pen.line(&[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)]);
        }
        Icon::Lock => {
            // The shackle, then the body.
            pen.line(&[(5.0, 7.0), (5.0, 4.8), (5.6, 3.0), (8.0, 1.8), (10.4, 3.0), (11.0, 4.8), (11.0, 7.0)]);
            pen.closed(&[(3.0, 7.0), (13.0, 7.0), (13.0, 14.5), (3.0, 14.5)]);
            pen.line(&[(8.0, 9.8), (8.0, 11.8)]);
        }
        Icon::Seal => {
            // A rosette (a circle with notches round it) and a tick inside.
            let pts: Vec<(f32, f32)> = (0..24)
                .map(|i| {
                    let a = i as f32 * PI / 12.0;
                    let r = if i % 2 == 0 { 6.8 } else { 5.8 };
                    (8.0 + r * a.cos(), 8.0 + r * a.sin())
                })
                .collect();
            pen.closed(&pts);
            pen.line(&[(5.4, 8.2), (7.2, 10.0), (10.8, 6.2)]);
        }
        Icon::Done => {
            // A tick, its short stroke down to the left of its foot.
            pen.line(&[(2.5, 8.6), (6.2, 12.3), (13.5, 3.7)]);
        }
        Icon::Smile => {
            // A round face: two eyes, and a smile below them.
            pen.circle(8.0, 8.0, 6.2);
            pen.dot(5.8, 6.4, 0.9);
            pen.dot(10.2, 6.4, 0.9);
            pen.line(&[(4.9, 9.4), (6.2, 10.9), (8.0, 11.5), (9.8, 10.9), (11.1, 9.4)]);
        }
        Icon::Conversation => {
            pen.closed(&[(1.5, 2.0), (10.5, 2.0), (10.5, 8.5), (5.0, 8.5), (2.5, 11.0), (2.5, 8.5), (1.5, 8.5)]);
            pen.line(&[(12.5, 5.0), (14.5, 5.0), (14.5, 11.5), (13.5, 11.5), (13.5, 14.0), (11.0, 11.5), (6.5, 11.5), (6.5, 10.5)]);
        }
        Icon::Preview => {
            pen.rect(1.5, 2.5, 14.5, 13.5);
            pen.line(&[(4.0, 6.0), (12.0, 6.0)]);
            pen.line(&[(4.0, 8.5), (10.0, 8.5)]);
            pen.line(&[(4.0, 11.0), (11.0, 11.0)]);
        }
        Icon::Compact => {
            for y in [3.0, 6.3, 9.7, 13.0] {
                pen.line(&[(2.0, y), (14.0, y)]);
            }
        }
        Icon::Sort => {
            pen.line(&[(5.0, 13.5), (5.0, 2.5)]);
            pen.line(&[(2.5, 5.0), (5.0, 2.5), (7.5, 5.0)]);
            pen.line(&[(11.0, 2.5), (11.0, 13.5)]);
            pen.line(&[(8.5, 11.0), (11.0, 13.5), (13.5, 11.0)]);
        }
        Icon::Source => {
            pen.line(&[(5.0, 4.0), (1.5, 8.0), (5.0, 12.0)]);
            pen.line(&[(11.0, 4.0), (14.5, 8.0), (11.0, 12.0)]);
            pen.line(&[(9.3, 3.0), (6.7, 13.0)]);
        }
        Icon::Ribbon => {
            for (x, y) in [(1.5, 2.5), (6.0, 2.5), (10.5, 2.5), (1.5, 9.0), (6.0, 9.0), (10.5, 9.0)] {
                pen.rect(x, y, x + 4.0, y + 4.5);
            }
        }
        Icon::Steps => {
            pen.line(&[(3.0, 4.0), (7.0, 8.0), (3.0, 12.0)]);
            pen.line(&[(8.5, 4.0), (12.5, 8.0), (8.5, 12.0)]);
        }
        Icon::Rules => {
            for y in [3.5, 8.0, 12.5] {
                pen.line(&[(1.5, y), (6.5, y)]);
            }
            pen.line(&[(8.5, 8.0), (14.5, 8.0)]);
            pen.line(&[(11.5, 5.0), (14.5, 8.0), (11.5, 11.0)]);
        }
        Icon::PopIn => {
            pen.line(&[(7.0, 2.5), (2.5, 2.5), (2.5, 13.5), (13.5, 13.5), (13.5, 9.0)]);
            pen.line(&[(7.5, 8.5), (13.5, 2.5)]);
            pen.line(&[(7.5, 4.5), (7.5, 8.5), (11.5, 8.5)]);
        }
        Icon::Back => {
            pen.circle(8.0, 8.0, 6.8);
            pen.line(&[(8.0, 4.5), (4.5, 8.0), (8.0, 11.5)]);
            pen.line(&[(4.5, 8.0), (11.5, 8.0)]);
        }
        Icon::Info => {
            pen.circle(8.0, 8.0, 6.5);
            pen.line(&[(8.0, 7.0), (8.0, 11.5)]);
            pen.line(&[(8.0, 4.4), (8.0, 4.9)]);
        }
        Icon::Calendar => {
            pen.rect(1.5, 3.0, 14.5, 14.5);
            pen.line(&[(1.5, 6.5), (14.5, 6.5)]);
            pen.line(&[(5.0, 1.5), (5.0, 4.5)]);
            pen.line(&[(11.0, 1.5), (11.0, 4.5)]);
            for (x, y) in [(5.0, 9.5), (8.0, 9.5), (11.0, 9.5), (5.0, 12.0), (8.0, 12.0)] {
                pen.line(&[(x - 0.6, y), (x + 0.6, y)]);
            }
        }
        Icon::People => {
            pen.circle(6.0, 5.5, 2.6);
            pen.arc(6.0, 14.5, 5.0, PI, 2.0 * PI);
            pen.arc(11.5, 6.0, 2.2, -PI * 0.6, PI * 0.6);
            pen.arc(11.5, 14.5, 4.0, -PI * 0.5, 0.0);
        }
        Icon::Phone => {
            pen.closed(&[
                (3.0, 2.0),
                (6.0, 2.0),
                (7.0, 5.5),
                (5.0, 7.0),
                (9.0, 11.0),
                (10.5, 9.0),
                (14.0, 10.0),
                (14.0, 13.0),
                (12.0, 14.5),
                (2.0, 4.0),
            ]);
        }
        Icon::Location => {
            pen.circle(8.0, 6.0, 2.0);
            pen.line(&[(3.6, 8.0), (8.0, 15.0), (12.4, 8.0)]);
            pen.arc(8.0, 6.0, 4.8, PI * 0.82, PI * 2.18);
        }
        Icon::Funnel => {
            pen.closed(&[(1.5, 2.5), (14.5, 2.5), (9.5, 8.5), (9.5, 14.0), (6.5, 12.5), (6.5, 8.5)]);
        }
        Icon::Print => {
            pen.rect(4.0, 1.5, 12.0, 5.5);
            pen.closed(&[(1.5, 5.5), (14.5, 5.5), (14.5, 11.5), (12.0, 11.5), (12.0, 9.5), (4.0, 9.5), (4.0, 11.5), (1.5, 11.5)]);
            pen.rect(4.0, 9.5, 12.0, 14.5);
        }
        Icon::Tag => {
            pen.closed(&[(1.5, 2.5), (8.0, 2.5), (14.5, 9.0), (9.0, 14.5), (1.5, 8.0)]);
            pen.circle(5.0, 6.0, 1.2);
        }
        Icon::Outbox => {
            pen.line(&[(1.5, 9.0), (4.0, 4.5), (5.5, 4.5)]);
            pen.line(&[(10.5, 4.5), (12.0, 4.5), (14.5, 9.0)]);
            pen.closed(&[(1.5, 9.0), (5.0, 9.0), (6.0, 11.0), (10.0, 11.0), (11.0, 9.0), (14.5, 9.0), (14.5, 13.5), (1.5, 13.5)]);
            pen.line(&[(8.0, 8.5), (8.0, 1.5)]);
            pen.line(&[(5.5, 4.0), (8.0, 1.5), (10.5, 4.0)]);
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

#[cfg(test)]
mod tests {
    /// Audit S8: each glyph stands for one thing. These are the meanings the audit found sharing
    /// a glyph with others (a clock for a reminder, a meeting's length and Show as; a pencil for
    /// Save draft, Signature and Edit; a person for Bcc and Add to contacts; a star for About and
    /// Priority; one grid for every calendar view; a bulleted list for Sort by beside List): none of them may go back to that glyph.
    #[test]
    fn a_glyph_stands_for_one_thing() {
        const NOT: [(&str, &[&str]); 8] = [
            ("Clock", &["Reminder", "Show as", "Meeting length"]),
            ("Drafts", &["Save draft", "Signature", "Edit", "Open"]),
            ("Person", &["Bcc", "Add to contacts", "New contact"]),
            ("Star", &["About", "Priority"]),
            ("Layout", &["Day", "Work week", "Week", "Month", "Schedule", "Cards"]),
            ("Phone", &["List"]),
            ("Sync", &["Download"]),
            ("ListBullet", &["Sort by", "Arrange by"]),
        ];
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if p.is_dir() && name != "tests" {
                    walk(&p, out);
                } else if name.ends_with(".rs") && name != "tests.rs" && name != "icons.rs" {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 100, "the window's code found: {}", files.len());
        // `Icon::Clock` itself, not `Icon::ClockSomething`.
        let uses =
            |line: &str, icon: &str| line.split(&format!("Icon::{icon}")).skip(1).any(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric()));
        let mut shared = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for (i, line) in text.lines().enumerate() {
                for (icon, meanings) in NOT {
                    if uses(line, icon) && meanings.iter().any(|m| line.contains(&format!("\"{m}\""))) {
                        shared.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
                    }
                }
            }
        }
        assert!(shared.is_empty(), "a glyph standing for something it doesn't show:\n{}", shared.join("\n"));
    }
}
