//! Tables pasted into the message being written (issue #268): drawn as the reading pane draws a
//! table (its rows and cells, their lines and colours), where its text stands, so what's written
//! looks as it goes. Its text stays in the message, unseen (the editor's [`rich_edit::Card`]):
//! what's sent as text, and every place in the text, are what they were. Backspace after it takes
//! it out whole; a table whose text is changed (Undo past it) is that text.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use mailcraft_engine::Compose;
use rich_edit::CardFor;

use super::*;

/// A table laid out, and how big it was drawn, in how wide a text.
pub(crate) struct TableView {
    blocks: Vec<mime::layout::Block>,
    size: Option<(f32, egui::Vec2)>,
}

/// Per message being written: its tables laid out, by what tells their code apart.
pub(crate) type TableCards = HashMap<u64, HashMap<u64, TableView>>;

/// The room kept before a table has been drawn once.
const FIRST_GUESS: egui::Vec2 = vec2(480.0, 120.0);

fn stamp(code: &str) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    code.hash(&mut h);
    h.finish()
}

/// Before the editor is drawn: the text each table stands for, and the room it took when it was
/// last drawn.
pub(super) fn place(st: &mut UiState, c: &Compose, ed: &mut rich_edit::Editor) {
    ed.cards.retain(|k| !matches!(k.what, CardFor::Table(_)));
    let places = c.table_places();
    if places.is_empty() {
        st.table_cards.remove(&c.id);
        return;
    }
    let views = st.table_cards.entry(c.id).or_default();
    let mut used = Vec::new();
    for (i, range) in places {
        let Some(table) = c.tables.get(i) else { continue };
        let key = stamp(&table.code);
        used.push(key);
        let view = views.entry(key).or_insert_with(|| TableView { blocks: mime::layout::html_layout(&table.code), size: None });
        if view.blocks.is_empty() {
            continue;
        }
        let size = view.size.map_or(FIRST_GUESS, |(_, size)| size);
        ed.cards.push(rich_edit::Card { what: CardFor::Table(i), range: range.clone(), whole: range, size });
    }
    views.retain(|k, _| used.contains(k));
}

/// After the editor: each table drawn in the room the editor kept for it.
pub(super) fn show(s: &Session, st: &mut UiState, ui: &mut Ui, c: &Compose, ed: &mut rich_edit::Editor, acts: &mut Vec<Action>) {
    let rects: Vec<(usize, Rect)> = ed
        .card_rects
        .iter()
        .filter_map(|(what, r)| match what {
            CardFor::Table(i) => Some((*i, *r)),
            CardFor::Signature => None,
        })
        .collect();
    let t = st.tokens;
    for (i, rect) in rects {
        let (Some(table), Some(card)) = (c.tables.get(i), ed.cards.iter().find(|k| k.what == CardFor::Table(i)).cloned()) else { continue };
        let key = stamp(&table.code);
        let Some(mut view) = st.table_cards.get_mut(&c.id).and_then(|v| v.remove(&key)) else { continue };
        // A message of its own to the drawing: nothing of another message's is drawn in it.
        let msg =
            MessageKey { account: "table-card".into(), folder: String::new(), validity: 0, uid: u32::try_from(c.id & 0xFFFF_FFFF).unwrap_or(0) };
        let parsed = mime::Parsed::default();
        let mut shown: Vec<String> = Vec::new();
        // On a light page, as it will be read: a table's colours are for one (a header's yellow
        // under text in the page's colour), and the dark theme's white text on it didn't show.
        let page = if t.dark { theme::Tokens::light() } else { t };
        if t.dark {
            ui.painter().rect_filled(Rect::from_min_size(rect.min, card.size).expand(4.0), 4.0, page.surface);
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(Align::Min)));
        // Within its room: drawn before its size is known, it isn't over the text under it.
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        if t.dark {
            let v = child.visuals_mut();
            v.override_text_color = Some(page.text);
            v.hyperlink_color = page.accent;
            v.widgets.noninteractive.fg_stroke.color = page.text;
        }
        st.tokens = page;
        {
            let mut body = Body { s, key: &msg, p: &parsed, allowed: false, shown: &mut shown, acts, cells: None, held: None };
            body.blocks(st, &mut child, &view.blocks, rect.width(), theme::surface_rgb(&page), mime::layout::Align::Left, 0);
        }
        st.tokens = t;
        let drawn = child.min_rect().size();
        // One thing in the text: a click puts the cursor after it.
        let over = ui
            .interact(Rect::from_min_size(rect.min, drawn), egui::Id::unique(("table-card", c.id, i)), Sense::click())
            .on_hover_text(tl!("A table, as it will be sent. Backspace after it takes it out."));
        if over.clicked() && !c.sending {
            ed.refocus = Some(card.range.end..card.range.end);
        }
        let size = vec2(drawn.x.clamp(1.0, rect.width().max(1.0)), drawn.y.clamp(1.0, 8000.0));
        // Its room is its size from the next frame on.
        if (size - card.size).length() > 0.5 {
            ui.ctx().request_repaint();
        }
        view.size = Some((rect.width(), size));
        st.table_cards.entry(c.id).or_default().insert(key, view);
    }
}
