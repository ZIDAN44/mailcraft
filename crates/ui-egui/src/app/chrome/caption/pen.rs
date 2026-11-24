//! A window moved or resized by a pen or a finger, done by MailCraft rather than by the system
//! (Windows). Ported from PhotoCraft's title bar (its #7967, by FredApps).
//!
//! winit's `drag_window` and `drag_resize_window` post `WM_NCLBUTTONDOWN`, and Windows' move and
//! size loop follows the mouse button. winit takes pen and touch input as `WM_POINTER` messages
//! (`WindowEvent::Touch`) and answers them itself, so Windows makes no mouse input from them: the
//! loop saw no button down and never ran, and the window stayed put. winit only clears its
//! "dragging" flag when that loop ends (`WM_EXITSIZEMOVE`), so after the first try it ignored
//! every later `drag_window`, the mouse's too, until MailCraft was restarted. A pen or a finger
//! therefore never asks the system: the window follows it through `OuterPosition` and
//! `InnerSize`.
//!
//! Where the pen is on the screen comes from the system's cursor when it follows the pen
//! ([`crate::Services::pointer_on_screen`], [`note_os_pointer`]). A finger leaves the cursor
//! behind, so a touch goes by its own position in the window plus where the window is. winit moves
//! the window asynchronously (`SWP_ASYNCWINDOWPOS`): a touch read before a move landed, added to
//! the window's position read after it, counts that move twice, and the window runs away from the
//! finger (PhotoCraft measured +687, +909 for a +200, +120 stroke). PhotoCraft reads the pen's
//! screen position from a hook MailCraft can't have (it needs `unsafe`); here a touch is only read
//! in a frame that began with the window where the frame before began with it, so every touch of
//! the frame was converted against that same position ([`PenDrag::seen`]).

use super::*;
use egui::{Event, Pos2, Vec2};

/// What a pen or touch press on the title bar or an edge does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    Move,
    Resize(ResizeDirection),
}

/// A window move or resize after a pen or a finger.
#[derive(Clone, Copy, Debug)]
struct PenDrag {
    kind: Kind,
    /// The press in screen points, and the window's outer and inner rects then. `None` while a
    /// maximized window restores before it moves.
    start: Option<(Pos2, Rect, Rect)>,
    /// Where the press was in the maximized window, as a fraction of its width and points from
    /// its top: the restored window is placed so the pen holds it at the same spot.
    grab: (f32, f32),
    /// The last pass this was applied in, so the title bar and the edge zones can both call
    /// [`follow`].
    pass: u64,
    /// The system's cursor follows the pen ([`note_os_pointer`]): the drag follows that.
    os: bool,
    /// Where the window's content was at the start of the last pass: a touch is read only in a
    /// pass that starts with it still there, so it was converted against where the window is.
    seen: Option<Pos2>,
}

/// Only Windows needs it (elsewhere the system's move and size follow a pen), and Wayland lets no
/// window place itself. Tests run it everywhere.
const PEN_DRAG: bool = cfg!(any(target_os = "windows", test));

fn os_pointer_id() -> Id {
    Id::unique("title-bar-os-pointer")
}

fn pen_drag_id() -> Id {
    Id::unique("title-bar-pen-drag")
}

/// Read where the system's cursor is on the screen (`read`: physical pixels) for this pass, while
/// a pen or a finger is down or one drags the window. Call before the title bar and the edges.
pub(crate) fn note_os_pointer(ctx: &egui::Context, read: impl FnOnce() -> Option<[i32; 2]>) {
    let dragging = ctx.data(|d| d.get_temp::<PenDrag>(pen_drag_id()).is_some());
    if !PEN_DRAG || !(dragging || ctx.input(|i| i.any_touches())) {
        return;
    }
    let Some([x, y]) = read() else { return };
    let at = pos2(x as f32, y as f32) / ctx.pixels_per_point();
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(os_pointer_id(), (pass, at)));
}

/// The screen position [`note_os_pointer`] read this pass.
fn os_pointer(ctx: &egui::Context) -> Option<Pos2> {
    let (pass, at) = ctx.data(|d| d.get_temp::<(u64, Pos2)>(os_pointer_id()))?;
    (pass == ctx.cumulative_pass_nr()).then_some(at)
}

/// Where the last touch of this pass was, in the window: `None` without one.
fn touch_this_pass(ctx: &egui::Context) -> Option<Pos2> {
    ctx.input(|i| i.events.iter().rev().find_map(|e| if let Event::Touch { pos, .. } = e { Some(*pos) } else { None }))
}

/// Start a [`PenDrag`] if the press is a pen's or a finger's: true if it took the press.
pub(super) fn start(ctx: &egui::Context, kind: Kind) -> bool {
    if !PEN_DRAG || !ctx.input(|i| i.any_touches()) {
        return false;
    }
    let (inner, outer, maximized) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect, i.viewport().maximized.unwrap_or(false)));
    let (Some(inner), Some(outer), Some(press)) = (inner, outer, ctx.input(|i| i.pointer.press_origin().or(i.pointer.latest_pos()))) else {
        return false;
    };
    let pass = ctx.cumulative_pass_nr();
    // The cursor is trusted only where the pointer is: a finger leaves it behind.
    let latest = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(press);
    let os_now = os_pointer(ctx).filter(|s| (*s - (inner.min + latest.to_vec2())).length() <= 8.0);
    let os = os_now.is_some();
    let drag = if maximized {
        if kind != Kind::Move {
            return false;
        }
        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
        PenDrag { kind, start: None, grab: (press.x / inner.width().max(1.0), press.y), pass, os, seen: None }
    } else {
        let at = os_now.map_or(inner.min + press.to_vec2(), |s| s - (latest - press));
        PenDrag { kind, start: Some((at, outer, inner)), grab: (0.0, 0.0), pass, os, seen: Some(inner.min) }
    };
    ctx.data_mut(|d| d.insert_temp(pen_drag_id(), drag));
    true
}

/// Move or resize the window after the pen ([`PenDrag`]); ends as it's lifted. Every pass, from
/// the title bar and the edge zones.
pub(super) fn follow(ctx: &egui::Context) {
    let Some(mut drag) = ctx.data(|d| d.get_temp::<PenDrag>(pen_drag_id())) else { return };
    let pass = ctx.cumulative_pass_nr();
    if drag.pass == pass {
        return;
    }
    drag.pass = pass;
    // Down while egui has the press and a touch. egui's pointer can be gone meanwhile (the
    // mouse cursor, parked where the window was, leaves it as it moves away): it isn't needed.
    let down = ctx.input(|i| i.pointer.primary_down() && i.any_touches());
    let (inner, outer, maximized) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().outer_rect, i.viewport().maximized.unwrap_or(false)));
    let end = |ctx: &egui::Context| ctx.data_mut(|d| d.remove::<PenDrag>(pen_drag_id()));
    let (Some(inner), Some(outer)) = (inner, outer) else { return end(ctx) };
    let still = drag.seen == Some(inner.min);
    drag.seen = Some(inner.min);
    // Where the pen is on the screen, if that can be told this pass. The lift's own position
    // counts too: the window ends where the pen left it.
    let pen = if drag.os { os_pointer(ctx) } else { touch_this_pass(ctx).filter(|_| still).map(|p| inner.min + p.to_vec2()) };
    if let Some(pen) = pen {
        place(ctx, &mut drag, pen, inner, outer, maximized);
    }
    if down {
        ctx.data_mut(|d| d.insert_temp(pen_drag_id(), drag));
    } else {
        end(ctx);
    }
}

/// The window put after the pen at `pen` (screen points); `inner`, `outer` and `maximized`: where
/// it is now.
fn place(ctx: &egui::Context, drag: &mut PenDrag, pen: Pos2, inner: Rect, outer: Rect, maximized: bool) {
    let Some((press, outer0, inner0)) = drag.start else {
        // Restoring a maximized window: wait for its restored size, then put it under the pen.
        if !maximized {
            let at = pen - vec2(drag.grab.0 * inner.width(), drag.grab.1) - (inner.min - outer.min);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
            let shift = at - outer.min;
            drag.start = Some((pen, outer.translate(shift), inner.translate(shift)));
        }
        return;
    };
    let d = pen - press;
    match drag.kind {
        Kind::Move => {
            let at = outer0.min + d;
            if at != outer.min {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
            }
        }
        Kind::Resize(dir) => {
            // The least size stays as it is on the screen at any text size.
            let least = egui::Vec2::from(crate::app::at_zoom(MIN_WINDOW_SIZE.into(), ctx.zoom_factor()));
            let r = resized(inner0, dir, d, least);
            if r.size() != inner.size() {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(r.size()));
            }
            let at = outer0.min + (r.min - inner0.min);
            if at != outer.min {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(at));
            }
        }
    }
}

/// `r` with the edges `dir` names moved by `d`, no smaller than `min`: a left or top edge stops
/// where the window would be too small rather than pushing it along.
pub(super) fn resized(r: Rect, dir: ResizeDirection, d: Vec2, min: Vec2) -> Rect {
    use ResizeDirection::*;
    let mut r = r;
    if matches!(dir, West | NorthWest | SouthWest) {
        r.min.x = (r.min.x + d.x).min(r.max.x - min.x);
    }
    if matches!(dir, East | NorthEast | SouthEast) {
        r.max.x = (r.max.x + d.x).max(r.min.x + min.x);
    }
    if matches!(dir, North | NorthWest | NorthEast) {
        r.min.y = (r.min.y + d.y).min(r.max.y - min.y);
    }
    if matches!(dir, South | SouthWest | SouthEast) {
        r.max.y = (r.max.y + d.y).max(r.min.y + min.y);
    }
    r
}
