//! A text field given the keyboard with what it holds selected, so what's typed replaces it: the
//! search box's query when it's asked for again (Ctrl+E, its key tip), a name a dialog offers to
//! change. A composition an input method has in progress in it is left alone: selecting over it
//! would lose what's being written. As PdfCraft's find field and AniCraft's prompts do.

/// When `request` is set: give field `r` the keyboard with all of `text` selected, and clear
/// `request`. Called on every pass right after the field is added (it follows the input method's
/// composition from the field's events).
pub(crate) fn focus_selecting_all(ui: &egui::Ui, r: &egui::Response, text: &str, request: &mut bool) {
    let composing = ime_composing(ui, r);
    if !std::mem::take(request) {
        return;
    }
    r.request_focus();
    if composing {
        return;
    }
    let mut state = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(text)));
    state.store(ui.ctx(), r.id);
}

/// Whether field `r` holds an input method's composition after this pass's events. A pre-edit with
/// text starts one; a commit or an empty pre-edit ends it, as does anything else that moves the
/// cursor off where the input method's last event left it (typing, a click), and losing the
/// keyboard. egui keeps its own record of this private.
fn ime_composing(ui: &egui::Ui, r: &egui::Response) -> bool {
    type Composition = Option<egui::text::CCursorRange>;
    let key = r.id.with("ime composition");
    let cursor = egui::TextEdit::load_state(ui.ctx(), r.id).and_then(|s| s.cursor.char_range());
    let mut composition: Composition = if r.has_focus() { ui.data(|d| d.get_temp::<Composition>(key)).flatten() } else { None };
    let mut saw_ime = false;
    if r.has_focus() {
        ui.input(|i| {
            for event in &i.events {
                if let egui::Event::Ime(ime) = event {
                    saw_ime = true;
                    match ime {
                        egui::ImeEvent::Preedit { text, .. } if text.is_empty() => composition = None,
                        egui::ImeEvent::Preedit { text, .. } if text != "\n" && text != "\r" => composition = cursor,
                        egui::ImeEvent::Commit(_) => composition = None,
                        _ => {}
                    }
                }
            }
        });
    }
    if let Some(range) = composition {
        if saw_ime {
            composition = cursor.or(Some(range));
        } else if cursor != Some(range) {
            composition = None;
        }
    }
    ui.data_mut(|d| d.insert_temp(key, composition));
    composition.is_some()
}
