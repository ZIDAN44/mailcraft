//! The original under a reply or forward, under the text being written, as the reading pane
//! shows it (its columns, colours, pictures, the boxes over them) and changed where it stands, as
//! Outlook lets it be: each block of its text a text box of its own, with the editor's keys,
//! ribbon and pictures. It goes out as it came until something in it is changed.

use std::collections::HashMap;

use mailcraft_engine::Compose;

use super::*;

/// The editors of the blocks of text of the quotes being written: per message, per block.
pub(crate) type QuoteEditors = HashMap<(u64, usize), rich_edit::Editor>;

/// The id a block of text of message `compose`'s quote is edited under (the editor's ids come
/// from it): never a message's own, nor the signature editor's.
pub(crate) fn cell_id(compose: u64, n: usize) -> u64 {
    (compose.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (u64::try_from(n).unwrap_or(0) + 1) << 1) | 1 << 62
}

/// Do `f` with block of text `n` of `c`'s quote standing in for its text, and its own editor:
/// the ribbon, when that block has the cursor. What `f` changed is kept in the quote.
pub(super) fn with_cell<R>(
    c: &mut Compose,
    eds: &mut QuoteEditors,
    n: usize,
    f: impl FnOnce(&mut Compose, &mut rich_edit::Editor) -> R,
) -> Option<R> {
    let id = c.id;
    let mut ed = eds.remove(&(id, n)).unwrap_or_else(|| cell_editor(id, n));
    let r = c.in_quote_cell(n, |c| {
        let own = std::mem::replace(&mut c.id, cell_id(id, n));
        let r = f(c, &mut ed);
        c.id = own;
        r
    });
    eds.insert((id, n), ed);
    r
}

fn cell_editor(compose: u64, n: usize) -> rich_edit::Editor {
    let mut ed = rich_edit::Editor::new(cell_id(compose, n));
    ed.min_height = Some(0.0);
    ed
}

/// The blocks of text drawn as text boxes, as the reading pane's drawing comes to them.
struct Cells<'a> {
    c: &'a mut Compose,
    /// Each block's text and styles as it is (read once a frame).
    texts: Vec<(String, mime::rich::Styles)>,
    /// Each block of text by where it is ([`block_at`]; the drawing walks the same blocks).
    index: HashMap<usize, usize>,
    eds: &'a mut QuoteEditors,
    tokens: Tokens,
    /// What was typed in each, and the one with the cursor.
    changed: Vec<(usize, String, mime::rich::Styles)>,
    focus: Option<usize>,
}

impl CellEdit for Cells<'_> {
    fn edit(&mut self, ui: &mut Ui, lines: &mime::RichText, page: [u8; 3], acts: &mut Vec<Action>) -> bool {
        let Some(&n) = self.index.get(&block_at(lines)) else { return false };
        let Some((text, styles)) = self.texts.get(n).cloned() else { return false };
        let id = self.c.id;
        let mut ed = self.eds.remove(&(id, n)).unwrap_or_else(|| cell_editor(id, n));
        // The block's text stands in for the message's while its box is drawn.
        let body = std::mem::replace(&mut self.c.body, text);
        let own = std::mem::replace(&mut self.c.styles, styles);
        let quote = self.c.quote.take();
        self.c.id = cell_id(id, n);
        // Its colours read against its cell, as the reading pane draws them.
        let tokens = Tokens { surface: egui::Color32::from_rgb(page[0], page[1], page[2]), ..self.tokens };
        rich_edit::body(ui, &tokens, &mut ed, self.c, None, acts);
        if ui.memory(|m| m.has_focus(rich_edit::body_id(self.c))) {
            self.focus = Some(n);
        }
        self.c.id = id;
        self.c.quote = quote;
        let text = std::mem::replace(&mut self.c.body, body);
        let styles = std::mem::replace(&mut self.c.styles, own);
        self.changed.push((n, text, styles));
        self.eds.insert((id, n), ed);
        true
    }
}

/// Where a block of text of the quote is, while it's drawn: the block's own (its lines' `Vec`),
/// one for each block. (Where its lines' buffer is told them apart before, and every empty one
/// has the same: two would have been one cell.)
#[allow(clippy::ptr_arg, reason = "the Vec itself is what's told apart, not its lines")]
pub(super) fn block_at(lines: &mime::RichText) -> usize {
    std::ptr::from_ref(lines).addr()
}

/// The quote's blocks of text by where each is ([`block_at`]), to their cell numbers.
pub(super) fn cell_index(blocks: &[mime::layout::Block]) -> HashMap<usize, usize> {
    mime::layout::text_blocks(blocks).into_iter().enumerate().map(|(n, (lines, _))| (block_at(lines), n)).collect()
}

/// `c`'s quote, under its text: drawn as the reading pane draws it, each block of its text a
/// text box.
pub(super) fn show(s: &Session, st: &mut UiState, ui: &mut Ui, c: &mut Compose, eds: &mut QuoteEditors, acts: &mut Vec<Action>) {
    let Some(q) = c.quote.clone() else { return };
    let t = st.tokens;
    ui.add_space(4.0);
    // Sent as plain text, it goes as its text: shown so.
    if c.plain {
        ui.add(egui::Label::new(RichText::new(q.text()).font(theme::regular(14.5)).color(t.text)).wrap());
        ui.add_space(8.0);
        return;
    }
    let index = cell_index(&q.blocks);
    let texts = q.cell_texts();
    let mut cells = Cells { c, texts, index, eds, tokens: t, changed: Vec::new(), focus: None };
    let mut shown: Vec<String> = Vec::new();
    let width = ui.available_width();
    let allowed = s.pictures_allowed();
    {
        let mut body = Body { s, key: &q.key, p: &q.parsed, allowed, shown: &mut shown, acts, cells: Some(&mut cells), held: None };
        body.blocks(st, ui, &q.blocks, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    let (changed, focus) = (std::mem::take(&mut cells.changed), cells.focus);
    drop(q);
    // Kept once the drawing let go of the quote (it isn't copied for each letter typed).
    c.set_quote_cells(changed);
    if let Some(n) = focus {
        st.quote_focus = Some((c.id, n));
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use mime::layout::{Block, Cell, Table};

    use super::*;

    /// Each block of text of a quote is a cell of its own, empty ones too (every empty one's
    /// lines are kept at the same place, so two were taken for one: found in audit, #480).
    #[test]
    fn every_block_of_text_is_a_cell_of_its_own() {
        let text = |t: &str| Block::Text(vec![vec![mime::Span { text: t.into(), ..Default::default() }]]);
        let cell = |blocks: Vec<Block>| Cell { blocks, span: 1, ..Default::default() };
        let blocks = vec![
            Block::Text(Vec::new()),
            text("Dear Sam,"),
            Block::Text(Vec::new()),
            Block::Table(Table { rows: vec![vec![cell(vec![Block::Text(Vec::new())]), cell(vec![text("Total")])]], ..Default::default() }),
        ];
        let found = mime::layout::text_blocks(&blocks);
        assert_eq!(found.len(), 5);
        let empty: Vec<&mime::RichText> = found.iter().map(|(lines, _)| *lines).filter(|l| l.is_empty()).collect();
        assert!(empty.len() == 3 && empty.iter().all(|l| l.as_ptr() == empty[0].as_ptr()), "their lines all in one place");
        let index = cell_index(&blocks);
        assert_eq!(index.len(), 5, "a cell each");
        let mut cells: Vec<usize> = found.iter().map(|(lines, _)| index[&block_at(lines)]).collect();
        assert_eq!(cells, [0, 1, 2, 3, 4], "each its own, in order");
        cells.dedup();
        assert_eq!(cells.len(), 5);
    }
}
