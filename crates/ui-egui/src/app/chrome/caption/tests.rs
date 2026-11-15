//! A pen or a finger moving and resizing the window by MailCraft's own title bar ([`pen`]).

use super::*;
use egui::{Event, Pos2, RawInput, Vec2, ViewportId};
use std::cell::Cell;
use std::rc::Rc;

/// A window with MailCraft's title bar and resize edges, on a screen where it obeys the move,
/// size and restore commands its frames send, as the system would.
struct Win {
    ctx: egui::Context,
    width: f32,
    height: f32,
    /// Where the window's content is on the screen, and its outer rect's offset from that.
    at: Pos2,
    frame_offset: Vec2,
    time: f64,
    maximized: bool,
    /// Windows moves the window asynchronously: with `lag`, a frame's touches are relative to
    /// where the window was before the previous frame's commands, while the viewport already
    /// says where it is now.
    lag: bool,
    before_move: Pos2,
    /// The system's cursor on the screen, when it follows the pen ([`note_os_pointer`]).
    cursor: Rc<Cell<Option<Pos2>>>,
    os_cursor: bool,
    cursor_follows: bool,
    /// A pen frame, by its index in the path, where egui's pointer also leaves the window.
    pointer_gone_at: Option<usize>,
}

impl Win {
    fn new(width: f32) -> Self {
        let at = pos2(200.0, 100.0);
        Self {
            ctx: egui::Context::default(),
            width,
            height: 600.0,
            at,
            frame_offset: vec2(-7.0, 0.0),
            time: 0.0,
            maximized: false,
            lag: false,
            before_move: at,
            cursor: Rc::default(),
            os_cursor: false,
            cursor_follows: true,
            pointer_gone_at: None,
        }
    }

    /// The system's cursor follows the pen, as Windows Ink moves it.
    fn with_os_cursor(mut self) -> Self {
        self.os_cursor = true;
        self
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.05;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(self.width, self.height));
        let mut input = RawInput { screen_rect: Some(screen), time: Some(self.time), events, ..Default::default() };
        if let Some(v) = input.viewports.get_mut(&ViewportId::ROOT) {
            v.maximized = Some(self.maximized);
            let inner = Rect::from_min_size(self.at, screen.size());
            v.inner_rect = Some(inner);
            v.outer_rect = Some(Rect::from_min_max(inner.min + self.frame_offset, inner.max - self.frame_offset));
        }
        let cursor = self.os_cursor.then(|| self.cursor.get()).flatten();
        let mut out = self.ctx.run_ui(input, |ui| {
            // What the main window's frame does first.
            note_os_pointer(ui.ctx(), || cursor.map(|c| [c.x.round() as i32, c.y.round() as i32]));
            let t = Tokens::themed(false, None, false);
            let bar = Rect::from_min_size(Pos2::ZERO, vec2(screen.width(), super::super::frame::TOP_BAR_H));
            title_bar(ui, &t, bar);
            resize_zones(ui);
        });
        out.textures_delta.clear();
        out
    }

    /// A pen stroke along `path`, in screen points, as winit and egui-winit deliver it on Windows:
    /// `Touch` events with an emulated pointer at the pen's spot in the window, which moves under
    /// the pen as the window does. The commands the frames sent.
    fn pen(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
        let touch = |phase, pos| Event::Touch { device_id: egui::TouchDeviceId(7), id: egui::TouchId(1), phase, pos, force: Some(0.5) };
        let mut all = vec![];
        let last = path.len().saturating_sub(1);
        for (i, p) in path.iter().enumerate() {
            if self.cursor_follows {
                self.cursor.set(Some(*p));
            }
            let local = self.local(*p);
            let mut events = match i {
                0 => vec![touch(egui::TouchPhase::Start, local), Event::PointerMoved(local), button(local, true)],
                _ => vec![touch(egui::TouchPhase::Move, local), Event::PointerMoved(local)],
            };
            if self.pointer_gone_at == Some(i) {
                events.push(Event::PointerGone);
            }
            all.extend(self.obey(events));
            if i == last {
                // Lifted where it last was, read as the frame's other touches are.
                let local = self.local(*p);
                all.extend(self.obey(vec![touch(egui::TouchPhase::End, local), button(local, false), Event::PointerGone]));
            }
        }
        all
    }

    /// Where the screen point `p` is in the window, as winit reads it for a touch this frame.
    fn local(&self, p: Pos2) -> Pos2 {
        p - if self.lag { self.before_move } else { self.at }.to_vec2()
    }

    /// One frame; the window then moves, resizes and restores as its commands say.
    fn obey(&mut self, events: Vec<Event>) -> Vec<ViewportCommand> {
        let cmds = self.frame(events).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        self.before_move = self.at;
        for c in &cmds {
            match c {
                ViewportCommand::OuterPosition(p) => self.at = *p - self.frame_offset,
                ViewportCommand::InnerSize(s) => (self.width, self.height) = (s.x, s.y),
                ViewportCommand::Maximized(false) => {
                    self.maximized = false;
                    (self.at, self.width, self.height) = (pos2(300.0, 200.0), 1000.0, 700.0);
                }
                _ => {}
            }
        }
        cmds
    }

    /// A mouse press at `path[0]`, moves along the rest, and the release, in the window.
    fn mouse(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
        let mut all = self.obey(vec![Event::PointerMoved(path[0]), button(path[0], true)]);
        for p in &path[1..] {
            all.extend(self.obey(vec![Event::PointerMoved(*p)]));
        }
        let end = path[path.len() - 1];
        all.extend(self.obey(vec![button(end, false)]));
        all
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Empty title bar, clear of the window's buttons and the resize edges.
const BAR: Pos2 = pos2(400.0, 20.0);

/// PhotoCraft's #7967: a pen on the title bar moves the window itself. Windows' move follows the
/// mouse button, which a pen never presses (winit reads it as `WM_POINTER`), and winit's
/// unfinished `drag_window` then stopped the mouse moving the window too.
#[test]
fn a_pen_moves_the_window_and_the_mouse_still_can_after() {
    let mut w = Win::new(1440.0);
    w.frame(vec![]);
    let start = w.at;
    let path: Vec<Pos2> = (0..=10).map(|i| start + BAR.to_vec2() + vec2(-30.0, 12.0) * i as f32).collect();
    let cmds = w.pen(&path);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag | ViewportCommand::BeginResize(_))), "no system move for a pen: {cmds:?}");
    assert_eq!(w.at, start + vec2(-300.0, 120.0), "the window followed the pen: {cmds:?}");
    // Lifting the pen ends it: a move of the pointer moves nothing.
    let cmds = w.obey(vec![Event::PointerMoved(pos2(500.0, 300.0))]);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::OuterPosition(_))), "{cmds:?}");
    // The mouse still hands the move to the system.
    let cmds = w.mouse(&[BAR, BAR + vec2(12.0, 4.0), BAR + vec2(30.0, 8.0)]);
    assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::OuterPosition(_))), "{cmds:?}");
}

/// A pen on an edge or a corner resizes the window itself, down to the smallest size, the left
/// and top edges without pushing the window along once it is that small.
#[test]
fn a_pen_resizes_the_window_from_its_edges() {
    let mut w = Win::new(1000.0);
    w.frame(vec![]);
    let screen = |w: &Win, p: Pos2| w.at + p.to_vec2();
    // South-east corner: 60 wider, 40 taller.
    let p = screen(&w, pos2(998.0, 598.0));
    let cmds = w.pen(&[p, p + vec2(30.0, 20.0), p + vec2(60.0, 40.0)]);
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{cmds:?}");
    assert_eq!((w.width, w.height), (1060.0, 640.0));
    let at = w.at;
    // West edge, 100 in: the window narrows and its left edge follows the pen.
    let p = screen(&w, pos2(2.0, 300.0));
    w.pen(&[p, p + vec2(50.0, 0.0), p + vec2(100.0, 0.0)]);
    assert_eq!((w.width, w.at), (960.0, at + vec2(100.0, 0.0)));
    // North edge, far down: stops at the smallest height with the bottom edge kept.
    let bottom = w.at.y + w.height;
    let p = screen(&w, pos2(480.0, 1.0));
    w.pen(&[p, p + vec2(0.0, 200.0), p + vec2(0.0, 900.0)]);
    assert_eq!((w.height, w.at.y + w.height), (MIN_WINDOW_SIZE.y, bottom));
    // The mouse still resizes through the system.
    let cmds = w.mouse(&[pos2(1.0, 300.0), pos2(8.0, 300.0)]);
    assert_eq!(cmds, vec![ViewportCommand::BeginResize(ResizeDirection::West)]);
}

/// A pen dragging a maximized window's title bar restores it and carries it, held where the pen
/// took it, as Windows does for the mouse.
#[test]
fn a_pen_drags_a_maximized_window_out_restored() {
    let mut w = Win::new(1920.0);
    w.maximized = true;
    w.at = Pos2::ZERO;
    w.frame(vec![]);
    let grab = pos2(1200.0, 20.0);
    let path: Vec<Pos2> = (0..=6).map(|i| grab + vec2(10.0, 20.0) * i as f32).collect();
    let cmds = w.pen(&path);
    assert!(cmds.contains(&ViewportCommand::Maximized(false)), "{cmds:?}");
    assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
    let held = path[path.len() - 1] - w.at;
    assert!((held.x - 1000.0 * 1200.0 / 1920.0).abs() < 0.01 && (held.y - 20.0).abs() < 0.01, "held at {held:?}, window at {:?}", w.at);
}

/// Windows moves the window asynchronously, so a frame's touches can be relative to where the
/// window was before the last move: added to where it is now they counted that move twice, and
/// the window ran away from the pen (PhotoCraft measured +687, +909 for a +200, +120 stroke). It
/// lands exactly under the pen whether it follows the system's cursor or the touches alone.
#[test]
fn a_pen_move_lands_under_the_pen_when_the_window_moves_late() {
    let stroke = |mut w: Win| {
        w.lag = true;
        w.frame(vec![]);
        let start = w.at;
        let path: Vec<Pos2> = (0..=10).map(|i| start + BAR.to_vec2() + vec2(20.0, 12.0) * i as f32).collect();
        w.pen(&path);
        w.at - start
    };
    assert_eq!(stroke(Win::new(1440.0).with_os_cursor()), vec2(200.0, 120.0));
    assert_eq!(stroke(Win::new(1440.0)), vec2(200.0, 120.0));
}

/// The same for a resize from the left edge, which moves the window as it sizes it.
#[test]
fn a_pen_resize_from_the_left_lands_under_the_pen_when_the_window_moves_late() {
    let mut w = Win::new(1200.0);
    w.lag = true;
    w.frame(vec![]);
    let (start, right) = (w.at, w.at.x + w.width);
    let p = start + vec2(2.0, 300.0);
    let path: Vec<Pos2> = (0..=10).map(|i| p + vec2(-15.0, 0.0) * i as f32).collect();
    w.pen(&path);
    assert_eq!((w.at.x, w.at.x + w.width), (start.x - 150.0, right));
}

/// A finger leaves the system's cursor behind: a cursor that isn't where the pointer is isn't
/// followed.
#[test]
fn a_cursor_away_from_the_pen_is_not_followed() {
    let mut w = Win::new(1440.0).with_os_cursor();
    w.cursor_follows = false;
    w.cursor.set(Some(pos2(5.0, 5.0)));
    w.frame(vec![]);
    let start = w.at;
    let path: Vec<Pos2> = (0..=10).map(|i| start + BAR.to_vec2() + vec2(-30.0, 12.0) * i as f32).collect();
    w.pen(&path);
    assert_eq!(w.at, start + vec2(-300.0, 120.0));
}

/// The mouse cursor, parked where the pen took the window, can end up outside the window as it
/// moves away: winit reports the mouse leaving, and egui's pointer is gone for the rest of the
/// stroke (PhotoCraft saw the move stop after 2 of 20 steps). The drag carries on.
#[test]
fn a_pen_move_survives_the_mouse_pointer_leaving_the_window() {
    for w in [Win::new(1440.0).with_os_cursor(), Win::new(1440.0)] {
        let mut w = w;
        w.lag = true;
        w.pointer_gone_at = Some(3);
        w.frame(vec![]);
        let start = w.at;
        let path: Vec<Pos2> = (0..=10).map(|i| start + BAR.to_vec2() + vec2(20.0, 12.0) * i as f32).collect();
        w.pen(&path);
        assert_eq!(w.at - start, vec2(200.0, 120.0), "following the cursor: {}", w.os_cursor);
    }
}

/// The edges a direction names move; the others stay; nothing gets smaller than the minimum.
#[test]
fn a_resize_moves_only_its_edges_and_stops_at_the_minimum() {
    let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(1100.0, 800.0));
    let min = vec2(760.0, 480.0);
    let d = vec2(40.0, -30.0);
    let got = |dir| pen::resized(r, dir, d, min);
    assert_eq!(got(ResizeDirection::East), Rect::from_min_max(r.min, pos2(1140.0, 800.0)));
    assert_eq!(got(ResizeDirection::North), Rect::from_min_max(pos2(100.0, 70.0), r.max));
    assert_eq!(got(ResizeDirection::SouthWest), Rect::from_min_max(pos2(140.0, 100.0), pos2(1100.0, 770.0)));
    // Far in from the left and the bottom: the right and top edges stay, at the minimum size.
    let small = pen::resized(r, ResizeDirection::SouthWest, vec2(900.0, -900.0), min);
    assert_eq!(small, Rect::from_min_max(pos2(340.0, 100.0), pos2(1100.0, 580.0)));
}
