//! Long work going on in the background (the search index's build; a data file's import and
//! its compaction): the status bar gives it one segment, the most important job's, drawn the
//! same way for each: what it is, and a short bar of how far it has come.

use egui::{CornerRadius, Response, RichText, Sense, Ui, vec2};

use crate::theme::Tokens;

/// The bar's size.
const BAR: egui::Vec2 = vec2(56.0, 4.0);

/// A job's segment: its words, and a bar `permille` thousandths full (`None`: how far isn't
/// known, an empty bar). Hovering it says `hover`.
pub(crate) fn segment(ui: &mut Ui, t: &Tokens, text: &str, permille: Option<u16>, hover: &str) -> Response {
    let words = ui.label(RichText::new(text).color(t.dim).small());
    let (r, bar) = ui.allocate_exact_size(BAR, Sense::hover());
    let round = CornerRadius::same(2);
    ui.painter().rect_filled(r, round, t.border);
    if let Some(p) = permille {
        // Filled from where the interface's text starts.
        let part = f32::from(p.min(1000)) / 1000.0;
        let done = egui::Rect::from_min_size(r.min, vec2(r.width() * part, r.height()));
        ui.painter().rect_filled(crate::bidi::Mirror::new(r).rect(done), round, t.accent);
    }
    words.union(bar).on_hover_text(hover)
}

/// A percentage of `permille` thousandths, rounded down (99 at most: the end is said by the
/// work, not by its bar).
pub(crate) fn percent(permille: u16) -> u16 {
    (permille / 10).min(99)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_percentage_never_says_done() {
        assert_eq!([0, 9, 10, 425, 999, 1000, u16::MAX].map(super::percent), [0, 0, 1, 42, 99, 99, 99]);
    }
}
