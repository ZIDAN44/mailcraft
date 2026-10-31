//! How an event shows (issue #32): its title when a shared calendar gives only its time, and
//! the strip at its left that tells how it shows its time.

use egui::{Color32, Rect, Ui, pos2};
use mailcraft_engine::model::ical::{Privacy, ShowAs};

/// How an event shows its time, as the event form's Show as list names it.
pub fn show_as_label(s: ShowAs) -> &'static str {
    match s {
        ShowAs::Free => tl!("Free"),
        ShowAs::WorkingElsewhere => tl!("Working elsewhere"),
        ShowAs::Tentative => tl!("Tentative"),
        ShowAs::Busy => tl!("Busy"),
        ShowAs::OutOfOffice => tl!("Out of office"),
    }
}

/// What an event is called in the calendar: its subject; one a shared calendar gives without
/// it (its time only) is a private appointment, or what its time shows as, never a blank box.
pub fn event_title(summary: &str, privacy: Privacy, show_as: ShowAs) -> String {
    if !summary.trim().is_empty() {
        summary.to_string()
    } else if privacy != Privacy::Public {
        tl!("Private appointment").to_string()
    } else {
        show_as_label(show_as).to_string()
    }
}

/// Out of office's colour, the same in light and dark (as its meaning is).
pub(crate) const AWAY: Color32 = Color32::from_rgb(0x88, 0x4C, 0xC7);

/// The strip at an event's left, telling how it shows its time: its colour (busy), hollow
/// (free), in stripes (tentative), in dots (working elsewhere), out of office's purple.
pub(crate) fn time_strip(ui: &Ui, r: Rect, c: Color32, show_as: ShowAs) {
    let p = ui.painter();
    match show_as {
        ShowAs::Busy => {
            p.rect_filled(r, 2.0, c);
        }
        ShowAs::OutOfOffice => {
            p.rect_filled(r, 2.0, AWAY);
        }
        ShowAs::Free => {
            p.rect_stroke(r, 1.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Inside);
        }
        ShowAs::Tentative | ShowAs::WorkingElsewhere => {
            p.rect_stroke(r, 1.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Inside);
            let step = if show_as == ShowAs::Tentative { 6.0 } else { 4.0 };
            let mut y = r.min.y;
            while y < r.max.y {
                p.rect_filled(Rect::from_min_max(pos2(r.min.x, y), pos2(r.max.x, (y + step / 2.0).min(r.max.y))), 0.0, c);
                y += step;
            }
        }
    }
}
