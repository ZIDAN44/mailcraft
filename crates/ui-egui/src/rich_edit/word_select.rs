//! A double-click in the message's text selects the word under the pointer, and only it. egui's own
//! reaches back to the word before and on to the next wherever the caret it hits sits at a word's
//! edge: double-clicking the left half of a one-letter "a" in "Take a calm walk" selected "Take a",
//! and what was typed then replaced both. A Shift-click after it extends from the word, as egui's
//! own double-click lets it. As PdfCraft's paragraph editor does.

use egui::text::{CCursor, CCursorRange};

/// The word a double-click at `caret` (a char index of `text`) means: the word whose letter the
/// caret sits before, or after it where the caret ends a word.
fn word_at_caret(text: &str, caret: usize) -> Option<std::ops::Range<usize>> {
    use egui::text_selection::text_cursor_state::{ccursor_next_word, ccursor_previous_word};
    let solid = |at: usize| text.chars().nth(at).is_some_and(|c| !c.is_whitespace());
    let (start, end) = if solid(caret) {
        (ccursor_previous_word(text, CCursor::new(caret + 1)), ccursor_next_word(text, CCursor::new(caret)))
    } else if caret > 0 && solid(caret - 1) {
        (ccursor_previous_word(text, CCursor::new(caret)), ccursor_next_word(text, CCursor::new(caret - 1)))
    } else {
        return None;
    };
    Some(usize::from(start.index)..usize::from(end.index))
}

/// The char index of the character the pointer (an offset in galley space) is over, when it's over
/// a glyph. `cursor_from_pos` takes any hit to the nearer caret, so the left half of the space after
/// "Take" and the right half of its "e" both come back as caret 4.
fn char_under(galley: &egui::Galley, pos: egui::Vec2) -> Option<usize> {
    let caret = usize::from(galley.cursor_from_pos(pos).index);
    // A glyph starts on the row that begins with it and ends on the row it's on, so at a soft wrap
    // its start is asked for with `prefer_next_row` and its end without.
    let start = |i: usize| galley.pos_from_cursor(CCursor { index: i.into(), prefer_next_row: true });
    let end = |i: usize| galley.pos_from_cursor(CCursor::new(i));
    [caret.checked_sub(1), Some(caret)].into_iter().flatten().find(|&i| {
        let (from, to) = (start(i), end(i + 1));
        // On the glyph's own row: a click in the blank after a short row hits the caret that starts
        // the next row, whose first glyph shares the x range.
        (from.center().y - to.center().y).abs() < 1.0 && from.y_range().contains(pos.y) && from.min.x <= pos.x && pos.x < to.min.x
    })
}

/// Narrow the selection a double-click just made in text box `id` to the word under the pointer,
/// and keep a following Shift-click extending from that word. Called once a pass, right after the
/// text box. The selection it set, as char indices, when it changed it (the box shows it from the
/// next pass).
pub(super) fn narrow_double_click(
    ui: &egui::Ui,
    id: egui::Id,
    text: &str,
    output: &egui::text_edit::TextEditOutput,
) -> Option<std::ops::Range<usize>> {
    let marker = id.with("narrowed word");
    // The word, and the selection as this left it: another one (a key moved it, text was typed,
    // the box was opened again) ends the anchor.
    let kept: Option<[usize; 4]> = ui.ctx().data(|d| d.get_temp(marker));
    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
    let sorted = |r: CCursorRange| r.sorted_cursors().map(|c| usize::from(c.index));
    let ends = |r: CCursorRange| (usize::from(r.primary.index), usize::from(r.secondary.index));
    let mut selected = state.cursor.char_range();
    let mut changed = false;
    // A press and its release can come in one frame (no button down at its end); egui still takes
    // that press, so it counts here too.
    let down = output.response.is_pointer_button_down_on() || (output.response.hovered() && ui.input(|i| i.pointer.any_pressed()));
    let stored = kept.filter(|m| down || selected.map(ends) == Some((m[2], m[3])));
    let narrowed = stored.map(|m| [m[0], m[1]]);
    let mut keep = narrowed;
    let shift = ui.input(|i| i.modifiers.shift);
    // egui keeps the word its double-click selected as the anchor of the Shift-clicks after it, so
    // each extends from that word, either way, and never drops it. That anchor can't be set from
    // outside (`set_char_range` clears it), so each click of the run is given the result egui's
    // gives: the pointer's end where egui put the primary cursor, the other the word's far end.
    if let (Some([start, end]), Some(range), true) = (narrowed, selected, down) {
        if shift {
            let pointer = usize::from(range.primary.index);
            let fixed = if pointer < start {
                CCursorRange { primary: range.primary, secondary: CCursor::new(end), h_pos: None }
            } else if pointer > end {
                CCursorRange { primary: range.primary, secondary: CCursor::new(start), h_pos: None }
            } else {
                CCursorRange::two(CCursor::new(start), CCursor::new(end))
            };
            if ends(fixed) != ends(range) {
                state.cursor.set_char_range(Some(fixed));
                selected = Some(fixed);
                changed = true;
            }
        } else {
            // A plain press starts a new selection.
            keep = None;
        }
    }
    // With Shift held, egui extends the selection instead (its own check is the same).
    let double_click = output.response.double_clicked() && !shift;
    // Only over a glyph: a double-click on the space after "Take" is left to egui.
    let target = double_click
        .then(|| output.response.interact_pointer_pos())
        .flatten()
        .and_then(|pointer| char_under(&output.galley, pointer - output.galley_pos))
        .filter(|&at| text.chars().nth(at).is_some_and(|c| !c.is_whitespace()))
        .and_then(|at| word_at_caret(text, at));
    if let Some(word) = target {
        if selected.map(sorted) != Some([word.start, word.end]) {
            let range = CCursorRange::two(CCursor::new(word.start), CCursor::new(word.end));
            state.cursor.set_char_range(Some(range));
            selected = Some(range);
            changed = true;
        }
        keep = Some([word.start, word.end]);
    }
    // The anchor lasts while the selection still holds the word; typing or a new selection ends it.
    let keep = keep.filter(|&[start, end]| selected.map(sorted).is_some_and(|[low, high]| low <= start && end <= high));
    let now = keep.zip(selected).map(|([start, end], range)| [start, end, ends(range).0, ends(range).1]);
    if now != kept {
        ui.ctx().data_mut(|d| match now {
            Some(m) => {
                d.insert_temp(marker, m);
            }
            None => {
                d.remove::<[usize; 4]>(marker);
            }
        });
    }
    if !changed {
        return None;
    }
    egui::TextEdit::store_state(ui.ctx(), id, state);
    ui.ctx().request_repaint();
    selected.map(|r| {
        let [low, high] = sorted(r);
        low..high
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word a double-click means, wherever in it the pointer is: never the one before it too.
    #[test]
    fn a_double_click_at_a_word_edge_means_that_word() {
        let text = "Take a calm walk";
        assert_eq!(word_at_caret(text, 5), Some(5..6), "before the a");
        assert_eq!(word_at_caret(text, 0), Some(0..4), "before Take");
        assert_eq!(word_at_caret(text, 4), Some(0..4), "after Take, before the space");
        assert_eq!(word_at_caret(text, 8), Some(7..11), "inside calm");
        assert_eq!(word_at_caret("  ", 1), None, "only spaces");
    }

    /// One pass of a text box holding `text`, narrowed as the message's is: where its text was
    /// laid out.
    fn pass(ctx: &egui::Context, text: &mut String, time: f64, events: Vec<egui::Event>) -> (std::sync::Arc<egui::Galley>, egui::Pos2) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut laid = None;
        let mut out = ctx.run_ui(input, |ui| {
            let id = egui::Id::unique("word-select");
            let out = egui::TextEdit::multiline(text).id(id).desired_width(600.0).show(ui);
            narrow_double_click(ui, id, text, &out);
            laid = Some((out.galley.clone(), out.galley_pos));
        });
        out.textures_delta.clear();
        laid.expect("laid out")
    }

    /// Double-clicking the left part of the one-letter "a" in "Take a calm walk" selects the "a"
    /// alone: egui's own double-click selected "Take a", and typing replaced both.
    #[test]
    fn double_clicking_a_one_letter_word_selects_only_that_word() {
        let ctx = egui::Context::default();
        let mut text = String::from("Take a calm walk");
        let (galley, at) = pass(&ctx, &mut text, 0.0, vec![]);
        let (from, to) = (galley.pos_from_cursor(CCursor::new(5)), galley.pos_from_cursor(CCursor::new(6)));
        let p = at + egui::vec2(from.min.x + 0.2 * (to.min.x - from.min.x), from.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut time = 0.1;
        for events in [vec![egui::Event::PointerMoved(p), button(true)], vec![button(false)], vec![button(true)], vec![button(false)], vec![], vec![]]
        {
            pass(&ctx, &mut text, time, events);
            time += 0.05;
        }
        let selected = egui::TextEdit::load_state(&ctx, egui::Id::unique("word-select")).and_then(|s| s.cursor.char_range()).map(|r| {
            let [a, b] = r.sorted_cursors().map(|c| usize::from(c.index));
            a..b
        });
        assert_eq!(selected, Some(5..6), "{:?}", selected.clone().map(|r| text.chars().skip(r.start).take(r.len()).collect::<String>()));
    }
}
