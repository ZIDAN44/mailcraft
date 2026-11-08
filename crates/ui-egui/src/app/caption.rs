//! The window's own title bar on Windows (issue #22): MailCraft's top bar is the title bar, in
//! its colour, as Outlook's is, instead of a bar under the system's. Ported from PhotoCraft's own
//! title bar (`panels/titlebar.rs`), Windows' buttons only: the caption buttons at the bar's end,
//! the bar's free space that drags the window (a double-click maximizes it), and thin zones along
//! the window's edges that resize it. The system keeps the window borderless meanwhile
//! (`window_frame.rs` in the app, [`crate::Services::keep_frame`]). While a dialog holds the
//! window, the bar's drag and buttons are laid again above it ([`over_a_modal`], PhotoCraft's
//! `modal_drag_overlay`).

use egui::{Color32, CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand, pos2, vec2};

use crate::look::theme::Tokens;
use crate::*;

/// One caption button: as wide as Windows' own.
pub(crate) const BUTTON_W: f32 = 46.0;
/// The three of them.
pub(crate) const CAPTIONS_W: f32 = 3.0 * BUTTON_W;
/// Resize zone thickness along an edge, and the side of a corner zone (PhotoCraft found 5 pt hard
/// to find; 8 pt is about a native resize border).
const EDGE: f32 = 8.0;
const CORNER: f32 = 12.0;
/// The red Windows' Close button turns under the pointer, its glyph white.
const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Caption {
    Minimize,
    Maximize,
    Close,
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The window's Maximize or Restore, noted for the frame's end ([`take_zooming`]) so the window
/// is told first.
fn toggle_maximized(ctx: &egui::Context) {
    let max = maximized(ctx);
    ctx.data_mut(|d| d.insert_temp(zooming_id(), true));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
}

fn zooming_id() -> Id {
    Id::unique("title-bar-zooming")
}

/// Whether this frame's title bar asked to maximize or restore the window, the once: the command
/// runs after the frame, so what must come before it ([`crate::Services::before_zoom`]) goes here.
pub fn take_zooming(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(zooming_id())).unwrap_or(false)
}

/// The bar's free space, `rect`: a press drags the window (Windows snaps it at the screen's
/// edges), a double-click maximizes or restores it. Laid down before the bar's own controls, so
/// they keep their clicks; where it's laid over them (`except`, the bar's controls), a press
/// there does nothing. Not in the Tab order, as a system title bar isn't.
pub(crate) fn drag_area(ui: &mut Ui, rect: Rect, except: &[Rect]) {
    let resp = ui.interact(rect, ui.scope_id().with("title-bar-drag"), Sense::CLICK | Sense::DRAG);
    let at = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.press_origin()));
    if at.is_some_and(|p| except.iter().any(|r| r.contains(p))) {
        return;
    }
    if resp.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        toggle_maximized(ui.ctx());
    }
}

/// MailCraft's mark at the bar's start, where the window's icon sits in a title bar: white on
/// the coloured bar, the accent colour on a plain one.
pub(crate) fn mark(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
    let tile = egui::Rect::from_center_size(r.center(), vec2(20.0, 20.0));
    let (fill, ink) = if t.on_bar == Color32::WHITE { (Color32::WHITE, t.bar) } else { (t.accent, t.on_accent) };
    ui.painter().rect_filled(tile, 5.0, fill);
    icons::paint(ui.painter(), icons::centered(tile, 14.0), Icon::Unread, ink);
}

/// The minimize, maximize (restore) and close buttons at the right end of `bar` (the title
/// bar's outer rect), as tall as it and flush with the window's edge, so a maximized window's
/// corner hits Close. Close asks the window to close, as the system's button does (MailCraft then
/// asks about messages being written). Not in the Tab order, as the system's aren't; named to a
/// screen reader when `live` (not the ones a dialog has dimmed, laid again over it).
pub(crate) fn caption_buttons(ui: &mut Ui, t: &Tokens, bar: Rect, live: bool) {
    let max = maximized(ui.ctx());
    let right = bar.right().min(ui.ctx().content_rect().right());
    let mut clicked = None;
    for (i, c) in [Caption::Minimize, Caption::Maximize, Caption::Close].into_iter().enumerate() {
        let left = right - CAPTIONS_W + i as f32 * BUTTON_W;
        let r = Rect::from_min_max(pos2(left, bar.top()), pos2(left + BUTTON_W, bar.bottom()));
        let resp = ui.interact(r, ui.scope_id().with(("title-bar-caption", i)), Sense::CLICK);
        let close = c == Caption::Close;
        let fill = match (resp.is_pointer_button_down_on(), resp.hovered()) {
            (true, _) if close => CLOSE_RED.gamma_multiply(0.85),
            (_, true) if close => CLOSE_RED,
            (true, _) | (_, true) => widgets::bar_wash(t),
            _ => Color32::TRANSPARENT,
        };
        ui.painter().rect_filled(r, 0.0, fill);
        let ink = if close && resp.hovered() { Color32::WHITE } else { t.on_bar };
        paint_glyph(ui, c, max, r.center(), Stroke::new(1.0, ink));
        let tip = match c {
            Caption::Minimize => tl!("Minimize"),
            Caption::Maximize if max => tl!("Restore"),
            Caption::Maximize => tl!("Maximize"),
            Caption::Close => tl!("Close"),
        };
        if live {
            widgets::named(ui, &resp, tip, true, None);
        }
        if resp.on_hover_text(tip).clicked() {
            clicked = Some(c);
        }
    }
    match clicked {
        Some(Caption::Minimize) => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
        Some(Caption::Maximize) => toggle_maximized(ui.ctx()),
        Some(Caption::Close) => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
        None => {}
    }
}

/// Where the title bar was drawn: its whole rect (what drags the window), its top row (where the
/// window's buttons are) and its controls (tabs, the search box, its buttons), for
/// [`over_a_modal`].
fn bar_id() -> Id {
    Id::unique("title-bar-rects")
}

/// Note where the title bar was drawn this frame ([`over_a_modal`]).
pub(crate) fn note_bar(ctx: &egui::Context, whole: Rect, top: Rect, controls: Vec<Rect>) {
    ctx.data_mut(|d| d.insert_temp(bar_id(), (whole, top, controls)));
}

/// Where the title bar was last drawn: its whole rect, its top row and its controls.
#[cfg(test)]
pub(crate) fn noted_bar(ctx: &egui::Context) -> Option<(Rect, Rect, Vec<Rect>)> {
    ctx.data(|d| d.get_temp::<(Rect, Rect, Vec<Rect>)>(bar_id()))
}

/// The title bar laid again over a dialog holding the window, as last drawn: `None` without a
/// dialog, or when one, or a menu, reaches into the bar.
fn over_a_dialog(ctx: &egui::Context) -> Option<(Rect, Rect, Vec<Rect>)> {
    let modal = ctx.memory(|m| m.top_modal_layer())?;
    let (whole, top, controls) = ctx.data(|d| d.get_temp::<(Rect, Rect, Vec<Rect>)>(bar_id()))?;
    let clear = whole.is_positive() && !egui::Popup::is_any_open(ctx) && !ctx.memory(|m| m.area_rect(modal.id)).is_some_and(|r| r.intersects(whole));
    clear.then_some((whole, top, controls))
}

/// Whether the title bar is laid again over a dialog: the dimmed one under it is then not named
/// to a screen reader (one Close, not two).
pub(crate) fn laid_over_a_dialog(ctx: &egui::Context) -> bool {
    over_a_dialog(ctx).is_some()
}

/// While a dialog holds the window (an `egui::Modal`: the question as MailCraft closes, Add
/// account…), nothing under it takes a click, the title bar neither: the window couldn't be
/// moved, minimized or closed with the mouse. Its drag and buttons are laid again above
/// everything, as a system title bar stays usable over a dialog, unless the dialog itself, or a
/// menu, reaches into the bar. Ported from PhotoCraft's `modal_drag_overlay` (its #4913).
pub(crate) fn over_a_modal(ctx: &egui::Context, t: &Tokens) {
    let Some((whole, top, controls)) = over_a_dialog(ctx) else { return };
    egui::Area::new(Id::unique("title-bar-over-modal")).order(Order::Tooltip).fixed_pos(whole.min).movable(false).sense(Sense::hover()).show(
        ctx,
        |ui| {
            ui.allocate_rect(whole, Sense::hover());
            // Its free space only (PhotoCraft's gap): a double-click on a tab doesn't maximize.
            drag_area(ui, whole, &controls);
            caption_buttons(ui, t, top, true);
        },
    );
}

/// The 10 pt line glyphs of the caption buttons, drawn here (no system or Office artwork).
fn paint_glyph(ui: &Ui, c: Caption, maximized: bool, at: egui::Pos2, s: Stroke) {
    let p = ui.painter();
    let g = Rect::from_center_size(at, vec2(10.0, 10.0));
    match c {
        Caption::Minimize => {
            p.line_segment([g.left_center(), g.right_center()], s);
        }
        // Restore: a front square with the corner of the one behind it.
        Caption::Maximize if maximized => {
            let front = Rect::from_min_max(g.min + vec2(0.0, 2.0), g.max - vec2(2.0, 0.0));
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Middle);
            p.line(vec![g.min + vec2(2.0, 2.0), g.min + vec2(2.0, 0.0), g.right_top(), g.max - vec2(0.0, 2.0), g.max - vec2(2.0, 2.0)], s);
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Middle);
        }
        Caption::Close => {
            p.line_segment([g.left_top(), g.right_bottom()], s);
            p.line_segment([g.right_top(), g.left_bottom()], s);
        }
    }
}

/// The edge and corner zones of a window with content rect `w`: (zone, direction, cursor), the
/// corners last so they win where they overlap the edges.
fn zones(w: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (EDGE, CORNER);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeEast),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// Scroll bars kept clear of the window's resize edges: the reading pane's lay in the last 10 pt
/// of the window, under the right edge's zone, so pressing it resized the window instead
/// (found in review). Every scroll bar keeps the zone's width from its area's edge.
pub(crate) fn clear_scroll_bars(ctx: &egui::Context) {
    if ctx.global_style().spacing.scroll.bar_outer_margin != EDGE {
        ctx.all_styles_mut(|s| s.spacing.scroll.bar_outer_margin = EDGE);
    }
}

/// Invisible resize zones along the window's edges, above everything else; a press in one starts
/// a resize by the system in that direction. None while the window is maximized or full screen.
pub(crate) fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let layer = LayerId::new(Order::Foreground, Id::unique("title-bar-resize"));
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w).into_iter().enumerate() {
        let resp = zui.interact(r, Id::unique(("title-bar-resize", i)), Sense::DRAG).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
