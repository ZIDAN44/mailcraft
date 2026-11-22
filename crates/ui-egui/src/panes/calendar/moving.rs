//! An event dragged to another time or day (issue #192), as in Outlook: while it's dragged, where
//! it would go is outlined with its new times; let go there, it moves (`event.move`), all of it,
//! as long as it was; of a repeating event, the occurrence dragged alone (issue #241). Esc while
//! dragging leaves it where it was.

use super::*;

/// A drag of an event's box, this frame.
pub(super) struct Drag {
    /// Where the pointer is.
    pub pointer: Pos2,
    /// Where the box's top left would be: the pointer, less where in the box it was grabbed.
    pub top_left: Pos2,
    /// Let go this frame.
    pub done: bool,
}

/// The box `resp` (drawn at `rect`) being dragged: where, and whether it was let go. `None` when
/// it isn't, or Esc called the drag off.
pub(super) fn dragged(ui: &Ui, resp: &egui::Response, rect: Rect) -> Option<Drag> {
    let grab_id = resp.id.with("grab");
    let off_id = resp.id.with("called-off");
    if resp.drag_started() {
        // Where the button went down: a drag starts only once the pointer has gone a little way,
        // and taken from there, the box jumped by that much (a quarter of an hour lost).
        let down = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos());
        let grab = down.map_or(Vec2::ZERO, |p| p - rect.min);
        ui.data_mut(|d| {
            d.insert_temp(grab_id, grab);
            d.insert_temp(off_id, false);
        });
    }
    if !(resp.dragged() || resp.drag_stopped()) {
        return None;
    }
    // egui ends a drag at Esc itself, as if it were let go there: that frame too is called off.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.data_mut(|d| d.insert_temp(off_id, true));
    }
    if ui.data(|d| d.get_temp::<bool>(off_id)).unwrap_or(false) {
        return None;
    }
    let pointer = ui.ctx().pointer_latest_pos().or_else(|| resp.interact_pointer_pos())?;
    let grab = ui.data(|d| d.get_temp::<Vec2>(grab_id)).unwrap_or(Vec2::ZERO);
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    Some(Drag { pointer, top_left: pointer - grab, done: resp.drag_stopped() })
}

/// The event `it` moved by `by` seconds: none when it stays where it was.
pub(super) fn moved(it: &AgendaItem, by: i64) -> Option<Action> {
    (by != 0).then(|| {
        let mut p = json!({"uid": it.uid, "by": by});
        if it.recurring {
            p["occurrence"] = json!(it.occurrence);
        }
        Action::Run("event.move", p)
    })
}

/// Seconds to the nearest quarter of an hour, as a dragged event steps.
pub(super) fn quarters(secs: f32) -> i64 {
    (secs / 900.0).round() as i64 * 900
}

/// Where the event being dragged would go: `r` outlined in the accent, over everything else, with
/// `label` (its new times, or its new day) in it.
pub(super) fn ghost(ui: &Ui, t: &Tokens, r: Rect, label: &str) {
    // Over the other events, within what's shown of the calendar (not over the ribbon above it
    // when dragged past the hours shown).
    let p =
        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("calendar-event-moved"))).with_clip_rect(ui.clip_rect());
    p.rect(r, 4.0, t.accent.gamma_multiply(0.25), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    p.with_clip_rect(r.shrink(1.0)).text(r.min + vec2(6.0, 3.0), Align2::LEFT_TOP, label, theme::bold(12.0), t.text);
}
