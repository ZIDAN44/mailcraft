//! The message editor, as in Outlook's message window: the text shows as it will be sent (bold,
//! italic, underline, sizes, colours, highlights, lists, indents, rules, pictures) with no marks,
//! formatted from the Message ribbon or with Outlook's shortcuts.
//!
//! The text is an ordinary text box over the message's plain text; its look comes from the
//! message's [`Styles`], which follow the text after each change, and a layouter that draws them.
//! Undo and redo keep the formatting with the text.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, Key, Modifiers, Rect, RichText, Sense, Stroke, Ui, vec2};
use mailcraft_engine::mime::rich::{self, Align, List, PICTURE, Style, Styles};
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::{Compose, Quote};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

mod carets;
mod layout;
mod links;
mod ops;
mod pictures;
pub(crate) mod spelling;
mod spoken;
mod toolbar;
mod tracked;
mod undo;

pub(crate) use layout::Card;
use layout::*;
pub(crate) use links::link_dialog;
use links::*;
pub(crate) use ops::{Op, apply};
#[cfg(test)]
pub(crate) use pictures::FIT_WIDTH;
pub(crate) use pictures::{fitted_width, own_line, own_size, web_loaded};
pub use toolbar::Counts;
#[cfg(test)]
pub(crate) use toolbar::{count_words, date_choices};
pub(crate) use toolbar::{ribbon, signature_bar};
use tracked::Tracked;
use undo::{History, Snap};

/// Pixels per point.
const PX: f32 = 4.0 / 3.0;
/// One indent step (and the room for a bullet or number).
const INDENT: f32 = 28.0;

/// Outlook's standard colours, and its highlighter's.
pub(crate) const COLORS: [[u8; 3]; 12] = [
    [0xC0, 0x00, 0x00],
    [0xFF, 0x00, 0x00],
    [0xFF, 0xC0, 0x00],
    [0xFF, 0xFF, 0x00],
    [0x92, 0xD0, 0x50],
    [0x00, 0xB0, 0x50],
    [0x00, 0xB0, 0xF0],
    [0x00, 0x70, 0xC0],
    [0x00, 0x20, 0x60],
    [0x70, 0x30, 0xA0],
    [0x00, 0x00, 0x00],
    [0x7F, 0x7F, 0x7F],
];
pub(crate) const HIGHLIGHTS: [[u8; 3]; 8] = [
    [0xFF, 0xFF, 0x00],
    [0x00, 0xFF, 0x00],
    [0x00, 0xFF, 0xFF],
    [0xFF, 0x00, 0xFF],
    [0x00, 0x00, 0xFF],
    [0xFF, 0x00, 0x00],
    [0xC0, 0xC0, 0xC0],
    [0x80, 0x80, 0x80],
];

/// What the editor remembers between frames, for one message.
pub(crate) struct Editor {
    pub id: u64,
    /// The look the next typed text takes, set from the ribbon with nothing selected.
    typing: Option<Style>,
    /// The selection (characters) at the end of the last frame.
    pub sel: Range<usize>,
    undo: History,
    redo: History,
    last_change: f64,
    /// The Insert link dialog, while open.
    link: Option<LinkForm>,
    /// Paste once the text has the keyboard back (asked from the right-click menu).
    pub(crate) paste: bool,
    /// The right-click menu was open last frame (closing it gives the text the keyboard back).
    menu_open: bool,
    pictures: HashMap<String, Option<pictures::Pic>>,
    /// The picture (its character) whose size is being dragged.
    pub(crate) resizing: Option<usize>,
    /// Where each picture was drawn last (its character, its rectangle).
    pub(crate) placed: Vec<(usize, Rect)>,
    /// Give the keyboard back to the text (after a ribbon button took the click), with this
    /// selection.
    pub(crate) refocus: Option<Range<usize>>,
    /// The text box this tall at least, when not the room there is (a block of the quoted
    /// original, as tall as its text).
    pub(crate) min_height: Option<f32>,
    /// A part of the text drawn as something else, set for each frame by who draws it (a
    /// signature's HTML code, laid out), and where its room was in the last frame.
    pub(crate) card: Option<Card>,
    pub(crate) card_rect: Option<Rect>,
    /// The text as laid out last.
    laid: Laid,
    /// The misspelled words, and the one right-clicked (issue #174).
    spelling: spelling::Marks,
}

impl Editor {
    pub fn new(id: u64) -> Editor {
        Editor {
            id,
            typing: None,
            sel: 0..0,
            undo: History::default(),
            redo: History::default(),
            last_change: 0.0,
            link: None,
            paste: false,
            menu_open: false,
            pictures: HashMap::new(),
            resizing: None,
            placed: Vec::new(),
            refocus: None,
            min_height: None,
            card: None,
            card_rect: None,
            laid: Laid::default(),
            spelling: spelling::Marks::default(),
        }
    }

    /// Take an undo step: the message as it is, the cursor at `cursor`.
    fn remember(&mut self, c: &Compose, cursor: usize) {
        self.remember_snap(Snap::of(c, cursor));
    }

    /// Take an undo step back to `s` (what's to redo is gone).
    fn remember_snap(&mut self, s: Snap) {
        self.undo.push(s);
        self.redo.clear();
    }
}

/// The id of the signature being edited (the signature editor is a message editor of its own).
pub(crate) const SIGNATURE_ID: u64 = u64::MAX;

/// A look as the editor draws it (tests).
#[cfg(test)]
pub(crate) fn text_format(s: &Style, base: Option<&str>, t: &Tokens) -> TextFormat {
    layout::format(s, base, t)
}

/// The message text box's id.
pub(crate) fn body_id(c: &Compose) -> egui::Id {
    text_id(c.id)
}

/// The id of the text box of message `id` (or of a block of a quote's, under its own id).
pub(crate) fn text_id(id: u64) -> egui::Id {
    egui::Id::unique(("compose-body", id))
}

/// Select `range` of the text in `ed`'s box, as if it were selected there (drawn behind the
/// text while another box has the keyboard).
pub(crate) fn select(ctx: &egui::Context, ed: &mut Editor, range: Range<usize>) {
    set_selection(ctx, text_id(ed.id), range.clone());
    ed.sel = range;
}

/// Take away the text and the original quoted under it, all of it selected (issue #118): one
/// step to undo, which what's typed in its place joins.
pub(crate) fn clear_all(ctx: &egui::Context, ed: &mut Editor, c: &mut Compose) {
    let n = c.body.chars().count();
    let style = typing_style(&c.body, &c.styles, 0);
    replace(ed, c, 0..n, "", style.clone());
    c.quote = None;
    ed.typing = Some(style);
    ed.last_change = ctx.input(|i| i.time);
    set_selection(ctx, body_id(c), 0..0);
}

fn selection(ctx: &egui::Context, id: egui::Id, n: usize) -> Option<Range<usize>> {
    let state = egui::TextEdit::load_state(ctx, id)?;
    let r = state.cursor.char_range()?.as_sorted_char_range();
    Some(r.start.0.min(n)..r.end.0.min(n))
}

fn set_selection(ctx: &egui::Context, id: egui::Id, r: Range<usize>) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(r.start), CCursor::new(r.end))));
    state.store(ctx, id);
}

/// The characters of `text` in `range` (character positions, as the selection's are).
fn chars_in(text: &str, range: Range<usize>) -> impl Iterator<Item = char> + '_ {
    text.chars().skip(range.start).take(range.end.saturating_sub(range.start))
}

/// The look typing at `at` takes: the character before (or after, at a paragraph's start),
/// without a link the text doesn't continue.
fn typing_style(text: &str, styles: &Styles, at: usize) -> Style {
    // The characters either side of `at` (the ribbon asks every frame: the text isn't copied).
    let mut around = text.chars().skip(at.saturating_sub(1));
    let before = if at > 0 { around.next() } else { None };
    let after = around.next();
    let i = if before.is_some_and(|c| c != '\n') { at.checked_sub(1) } else { after.filter(|c| *c != '\n').map(|_| at) };
    let Some(i) = i else { return Style::default() };
    let mut s = styles.style(i).for_typing();
    if s.link.is_some() && styles.style(at).link != s.link {
        s.link = None;
    }
    s
}

/// The line (paragraph) character `at` is in, the character it starts at, and whether it's
/// empty: one look along the text as far as `at`.
fn line_at(text: &str, at: usize) -> (usize, usize, bool) {
    let mut chars = text.chars();
    let (mut line, mut start, mut i) = (0, 0, 0);
    while i < at {
        match chars.next() {
            Some('\n') => {
                line += 1;
                start = i + 1;
            }
            Some(_) => {}
            None => break,
        }
        i += 1;
    }
    // Empty: nothing between its start and `at`, and a break or the end after.
    (line, start, start == i && chars.next().is_none_or(|ch| ch == '\n'))
}

/// Put `text` in place of the characters in `range`, with style `style`.
fn replace(ed: &mut Editor, c: &mut Compose, range: Range<usize>, text: &str, style: Style) {
    ed.remember(c, range.end);
    let old = c.body.clone();
    let byte = |at: usize| old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    let (a, b) = (byte(range.start), byte(range.end.max(range.start)));
    c.body.replace_range(a..b, text);
    let n = text.chars().count();
    c.styles.edit_at(&old, &c.body, &style, Some(range.start + n));
    ed.sel = range.start + n..range.start + n;
}

fn undo(ed: &mut Editor, c: &mut Compose, ctx: &egui::Context, redo: bool) {
    let (from, to) = if redo { (&mut ed.redo, &mut ed.undo) } else { (&mut ed.undo, &mut ed.redo) };
    let Some(s) = from.pop() else { return };
    to.push(Snap::of(c, ed.sel.end));
    c.body = s.text;
    c.styles = s.styles;
    // The original quoted under the text comes back (or goes again) with the step that took it
    // away; otherwise it stays as it is, with what was changed in it since.
    if s.quote.is_some() != c.quote.is_some() {
        c.quote = s.quote;
    }
    let at = s.cursor.min(c.body.chars().count());
    ed.sel = at..at;
    ed.typing = None;
    set_selection(ctx, body_id(c), at..at);
}

/// Outlook's shortcuts in the text, taken before the text box sees them.
fn keys(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let id = body_id(c);
    let n = c.body.chars().count();
    if let Some(sel) = selection(&ctx, id, n) {
        ed.sel = sel;
    }
    let cmd = Modifiers::COMMAND;
    let cs = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut ops = Vec::new();
    let (mut undo_k, mut redo_k, mut link_k, mut book_k) = (false, false, false, false);
    // Lists: Enter on an empty item ends the list, Backspace at an item's start takes its bullet
    // away, Tab and Shift+Tab move it in and out.
    let (line, line_start, line_empty) = line_at(&c.body, ed.sel.start);
    let para = c.styles.para(line);
    let at_start = ed.sel.is_empty() && ed.sel.start == line_start;
    let listed = para.list != List::None;
    // A card is one thing: Backspace after it, or Delete before it, takes all of it (its
    // unseen letters one at a time would leave the rest of them as text).
    if let Some(card) = ed.card.clone().filter(|card| !c.sending && ed.sel.is_empty() && card.whole.end <= n) {
        let first = card.before(&c.body);
        let key = if ed.sel.start == card.whole.end {
            Some(Key::Backspace)
        } else if ed.sel.start == first {
            Some(Key::Delete)
        } else {
            None
        };
        let pressed = key.is_some_and(|key| {
            ui.input_mut(|i| [Modifiers::NONE, Modifiers::SHIFT, Modifiers::COMMAND, Modifiers::ALT].iter().any(|m| i.consume_key(*m, key)))
        });
        if pressed {
            let style = typing_style(&c.body, &c.styles, first);
            replace(ed, c, first..card.whole.end, "", style);
            set_selection(&ctx, id, ed.sel.clone());
            ed.card = None;
            return;
        }
    }
    ui.input_mut(|i| {
        // With Shift first, so Ctrl+Shift+Z isn't taken as Ctrl+Z.
        for (m, k, op) in [
            (cs, Key::L, Op::Bullets),
            (cs, Key::Period, Op::Grow),
            (cs, Key::Comma, Op::Shrink),
            (cs, Key::T, Op::Outdent),
            (cs, Key::Z, Op::Clear),
            (cmd, Key::B, Op::Bold),
            (cmd, Key::I, Op::Italic),
            (cmd, Key::U, Op::Underline),
            (cmd, Key::CloseBracket, Op::Grow),
            (cmd, Key::OpenBracket, Op::Shrink),
            (cmd, Key::L, Op::Align(Align::Left)),
            (cmd, Key::E, Op::Align(Align::Center)),
            (cmd, Key::R, Op::Align(Align::Right)),
            (cmd, Key::T, Op::Indent),
            (cmd, Key::Space, Op::Clear),
        ] {
            if i.consume_key(m, k) {
                ops.push(op);
            }
        }
        book_k = i.consume_key(cs, Key::B);
        undo_k = i.consume_key(cmd, Key::Z);
        redo_k = i.consume_key(cmd, Key::Y);
        link_k = i.consume_key(cmd, Key::K);
        if listed && line_empty && ed.sel.is_empty() && i.consume_key(Modifiers::NONE, Key::Enter) {
            ops.push(if para.list == List::Bullet { Op::Bullets } else { Op::Numbers });
        }
        if at_start && (listed || para.indent > 0) && i.consume_key(Modifiers::NONE, Key::Backspace) {
            ops.push(if listed { if para.list == List::Bullet { Op::Bullets } else { Op::Numbers } } else { Op::Outdent });
        }
        if listed && i.consume_key(Modifiers::SHIFT, Key::Tab) {
            ops.push(Op::Outdent);
        }
        if listed && i.consume_key(Modifiers::NONE, Key::Tab) {
            ops.push(Op::Indent);
        }
    });
    spoken::apply_keyed(ui.ctx(), ed, c, ops);
    if undo_k || redo_k {
        undo(ed, c, &ctx, redo_k);
    }
    if link_k {
        ed.link = Some(LinkForm::open(c, ed.sel.clone()));
    }
    if book_k && c.id != SIGNATURE_ID {
        acts.push(Action::Dialog(Dialog::select_names(c, crate::dialogs::Recipients::To)));
    }
}

/// Paste what another program put on the clipboard as HTML (a signature copied from Outlook,
/// part of a web page) with its look, in place of the selection, and the pictures written into
/// it (a logo copied from Word or Outlook, issue #11: they were left out). `false` when there is
/// none, or it holds no text (a picture alone is pasted as a picture).
fn paste_formatted(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) -> bool {
    let Some(html) = crate::clipboard_html() else { return false };
    let (add, mut add_styles, found) = mailcraft_engine::mime::signature_html(&html);
    if add.trim().is_empty() || add.chars().all(|ch| ch == PICTURE || ch.is_whitespace()) {
        return false;
    }
    pictures::take_pasted(c, &add, &mut add_styles, found);
    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
    c.styles.fit(&c.body);
    ed.remember(c, ed.sel.start);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    rich::splice(&mut c.body, &mut c.styles, sel.clone(), &add, &add_styles);
    let end = sel.start + add.chars().count();
    set_selection(ui.ctx(), id, end..end);
    ed.sel = end..end;
    ed.typing = None;
    ui.ctx().request_repaint();
    true
}

/// The message text: the text box, then bullets and numbers, rules and pictures drawn over it.
pub(crate) fn body(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, start_at: Option<usize>, acts: &mut Vec<Action>) {
    let id = body_id(c);
    c.styles.fit(&c.body);
    // Back from the ribbon: the cursor (or selection) where it was, typing in the text again.
    // Not on the frame of the click itself, when the text box gives the keyboard up because
    // it was clicked elsewhere.
    if ed.refocus.is_some() {
        if ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
            ui.ctx().request_repaint();
        } else if let Some(sel) = ed.refocus.take() {
            ui.memory_mut(|m| m.request_focus(id));
            set_selection(ui.ctx(), id, sel.clone());
            ed.sel = sel;
            // Paste from the right-click menu: the system sends the text to the text box now
            // that it has the keyboard.
            if std::mem::take(&mut ed.paste) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
        }
    }
    if ui.memory(|m| m.has_focus(id)) {
        keys(ui, ed, c, acts);
        if !c.plain && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)))) {
            paste_formatted(ui, ed, c, id);
        }
    }
    let plain = c.plain;
    let tokens = *t;
    let own = if plain { HashMap::new() } else { pictures::own_sizes(ui.ctx(), ed, c) };
    let card = ed.card.clone().filter(|_| !plain);
    // The text box changes the text in this frame, and lays it out at once: its look (and the
    // card's place) moved along with the change, as they are once the frame is over. Laid out
    // with the frame's first ones, all that followed the change was a letter off for a frame: a
    // picture after it was laid out as a letter, and the text under it jumped (issue #114).
    let was = c.body.clone();
    let font = c.font.clone();
    let (typing, at) = (ed.typing.clone(), ed.sel.start);
    let mut moved: Option<(String, Styles, Option<Card>)> = None;
    let mut text = Tracked::new(&mut c.body);
    let styles = &c.styles;
    let laid = &mut ed.laid;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let now = buf.as_str();
        if now == was {
            return layout_kept(laid, ui, now, Lay { st: styles, plain, t: &tokens, wrap, own: &own, card: card.as_ref(), font: font.as_deref() });
        }
        if moved.as_ref().is_none_or(|(laid_for, ..)| laid_for != now) {
            let mut st = styles.clone();
            let typing = typing.clone().unwrap_or_else(|| typing_style(&was, styles, at));
            st.edit_at(&was, now, &typing, None);
            moved = Some((now.to_string(), st, card.as_ref().and_then(|card| card.moved(&was, now))));
        }
        let Some((_, st, card)) = moved.as_ref() else {
            return layout_kept(laid, ui, now, Lay { st: styles, plain, t: &tokens, wrap, own: &own, card: None, font: font.as_deref() });
        };
        layout_kept(laid, ui, now, Lay { st, plain, t: &tokens, wrap, own: &own, card: card.as_ref(), font: font.as_deref() })
    };
    // As tall as the pane, or, above the original of a reply shown as it came, as its text.
    let height = ed.min_height.unwrap_or_else(|| if c.quote.is_some() { 96.0 } else { ui.available_height().max(220.0) });
    // A right-click inside the selection keeps it (egui's text box moves the cursor on any
    // press); one outside it moves the cursor there, as Outlook does.
    let kept = ed.sel.clone();
    let right_press = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
    // The selection is drawn behind the text while the text box doesn't have the keyboard
    // (a menu, a dialog or the ribbon has it), so what's selected stays plain to see.
    let behind = ui.painter().add(egui::Shape::Noop);
    // No hint over a first line that's a list item or indented (its bullet is drawn there).
    let p0 = c.styles.para(0);
    let hint = match (p0.list == List::None && p0.indent == 0 && ed.min_height.is_none(), c.id == SIGNATURE_ID) {
        (false, _) => "",
        (true, true) => tl!("Write your signature, or paste it…"),
        (true, false) => tl!("Write your message…"),
    };
    let out = egui::TextEdit::multiline(&mut text)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(hint)
        .lock_focus(true)
        .desired_width(f32::INFINITY)
        .min_size(vec2(ui.available_width(), height))
        .desired_rows(if ed.min_height.is_some() { 1 } else { 4 })
        .layouter(&mut layouter)
        .show(ui);
    // What the text was, when the text box changed it.
    let before = text.before();
    let mut cursor = out.cursor_range.map(|r| {
        let r = r.as_sorted_char_range();
        r.start.0..r.end.0
    });
    if right_press
        && !kept.is_empty()
        && let Some(p) = out.response.interact_pointer_pos().or_else(|| out.response.hover_pos())
    {
        let at = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
        if (kept.start..=kept.end).contains(&at) {
            set_selection(ui.ctx(), id, kept.clone());
            cursor = Some(kept.clone());
        }
    }
    // A card is passed as one thing: the cursor doesn't stop among its unseen letters (on the
    // way down it comes out after it, on the way up before it), and a selection that reaches
    // into it takes all of it.
    if let (Some(card), Some(r)) = (&card, cursor.as_mut())
        && before.is_none()
    {
        let (first, last) = (card.before(&c.body), card.whole.end);
        let inside = |p: usize| p > first && p < last;
        let moved = if r.start == r.end && inside(r.start) {
            let p = if ed.sel.start >= last { first } else { last };
            Some(p..p)
        } else if r.start != r.end && (inside(r.start) || inside(r.end)) {
            Some(if inside(r.start) { first } else { r.start }..r.end.max(last))
        } else {
            None
        };
        if let Some(to) = moved.filter(|to| to != r) {
            set_selection(ui.ctx(), id, to.clone());
            *r = to;
            ui.ctx().request_repaint();
        }
    }
    // Where its room is: its last line's row, the width of the text.
    ed.card_rect = card.as_ref().filter(|card| card.range.end <= c.body.chars().count()).map(|card| {
        let row = out.galley.pos_from_cursor(CCursor::new(card.last_line(&c.body)));
        let top = out.galley_pos.y + row.min.y + 2.0;
        Rect::from_min_size(egui::pos2(out.galley_pos.x, top), vec2(out.response.rect.width(), card.size.y))
    });
    if let Some(before) = before {
        // The look of what was typed: the ribbon's choice, else the text's at the cursor.
        let typing = ed.typing.take().unwrap_or_else(|| typing_style(&before, &c.styles, ed.sel.start));
        let now = ui.input(|i| i.time);
        let breaks = c.body.matches('\n').count() != before.matches('\n').count();
        let step = now - ed.last_change > 1.0 || breaks || ed.undo.is_empty();
        let styles = step.then(|| c.styles.clone());
        ed.last_change = now;
        c.styles.edit_at(&before, &c.body, &typing, cursor.as_ref().map(|r| r.end));
        if let Some(styles) = styles {
            ed.remember_snap(Snap { text: before, styles, quote: c.quote.clone(), cursor: ed.sel.start });
        }
        ui.ctx().request_repaint();
    } else if cursor.as_ref().is_some_and(|r| *r != ed.sel) {
        // The cursor moved: what the ribbon set for typing no longer applies.
        ed.typing = None;
    }
    if let Some(r) = cursor {
        ed.sel = r;
    }
    if let Some(at) = start_at {
        out.response.request_focus();
        set_selection(ui.ctx(), id, at..at);
        ed.sel = at..at;
    }
    if !c.plain {
        decorate(ui, t, ed, c, &out.galley, out.galley_pos);
    }
    spelling::marked(ui, t, ed, c, &out, right_press);
    if !out.response.has_focus() && !ed.sel.is_empty() {
        let color = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
        let shapes: Vec<egui::Shape> = carets::selection_rects(&out.galley, ed.sel.clone())
            .into_iter()
            .map(|r| egui::Shape::rect_filled(r.translate(out.galley_pos.to_vec2()), 0.0, color))
            .collect();
        ui.painter().set(behind, egui::Shape::Vec(shapes));
    }
    text_menu(&out.response, ed, c, acts);
}

/// The right-click menu on the text, as Outlook's: a misspelled word's suggestions, Ignore all
/// and Add to dictionary; Cut, Copy, Paste and Select all; the link (insert, edit, open,
/// remove); and clearing the formatting.
fn text_menu(resp: &egui::Response, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>) {
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    // What's selected, copied out only when it's cut or copied (not every frame).
    let chosen = |text: &str| -> String { chars_in(text, sel.clone()).collect() };
    let link = c.styles.style(sel.start).link.filter(|_| n > 0);
    let shown = resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        let item = |ui: &mut Ui, label: &str, keys: &str, enabled: bool| -> bool {
            let b = egui::Button::new(tl!(label)).shortcut_text(keys);
            let clicked = ui.add_enabled(enabled, b).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        let some = !sel.is_empty();
        if let Some(offer) = ed.spelling.offer.clone() {
            spelling::menu_items(ui, ed, c, &offer, acts);
        }
        if item(ui, "Cut", "Ctrl+X", some && !c.sending) {
            ui.ctx().copy_text(chosen(&c.body));
            let style = typing_style(&c.body, &c.styles, sel.start);
            replace(ed, c, sel.clone(), "", style);
            ed.refocus = Some(ed.sel.clone());
        }
        if item(ui, "Copy", "Ctrl+C", some) {
            ui.ctx().copy_text(chosen(&c.body));
            ed.refocus = Some(sel.clone());
        }
        if item(ui, "Paste", "Ctrl+V", !c.sending) {
            ed.paste = true;
            ed.refocus = Some(sel.clone());
            // Files copied in a file manager are attached instead (Explorer's: egui sends no
            // paste for them on Ctrl+V).
            if c.id != SIGNATURE_ID {
                acts.push(Action::PasteFiles);
            }
        }
        ui.separator();
        if item(ui, "Select all", "Ctrl+A", n > 0) {
            ed.refocus = Some(0..n);
        }
        if !c.plain {
            ui.separator();
            if item(ui, if link.is_some() { "Edit link…" } else { "Link…" }, "Ctrl+K", !c.sending) {
                ed.link = Some(LinkForm::open(c, sel.clone()));
            }
            if let Some(url) = &link {
                if item(ui, "Open link", "", true) {
                    acts.push(Action::OpenUrl(url.clone()));
                }
                if item(ui, "Remove link", "", !c.sending) {
                    apply(ed, c, Op::Unlink);
                    ed.refocus = Some(sel.clone());
                }
            }
            ui.separator();
            if item(ui, "Clear formatting", "Ctrl+Space", some && !c.sending) {
                apply(ed, c, Op::Clear);
                ed.refocus = Some(sel.clone());
            }
        }
    });
    // Closed without a choice (Escape, a click elsewhere in the window): the text gets the
    // keyboard back, with what was selected.
    let open = shown.is_some();
    if ed.menu_open && !open && ed.refocus.is_none() && ed.link.is_none() && !resp.ctx.input(|i| i.pointer.any_pressed() && !resp.hovered()) {
        ed.refocus = Some(sel.clone());
    }
    ed.menu_open = open;
}

/// Bullets and numbers, rules and pictures.
fn decorate(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, g: &Galley, origin: egui::Pos2) {
    let painter = ui.painter().clone();
    // Each line's start, in order: its row found walking on from the last line's (looked for
    // from the first row each time, a long message's lines took lines × rows a frame).
    let mut carets = carets::Carets::new(g);
    let mut at = |i: usize| -> Rect { carets.at(i).translate(origin.to_vec2()) };
    // A character's look, not copied.
    let look = |i: usize| c.styles.chars.get(i).and_then(|k| c.styles.palette.get(usize::from(*k)));
    let mut i = 0usize;
    let mut numbers: Vec<u32> = vec![0; 16];
    let right = ui.min_rect().right().max(origin.x + g.size().x);
    let mut pictures = Vec::new();
    ed.placed.clear();
    // (Nothing is drawn over a card's unseen letters: what stands for them is drawn there.)
    let card = ed.card.clone();
    let in_card = |i: usize| card.as_ref().is_some_and(|c| c.range.contains(&i));
    for (n, line) in c.body.split('\n').enumerate() {
        let p = if in_card(i) { Default::default() } else { c.styles.para(n) };
        let r = at(i);
        if p.rule {
            let y = r.center().y;
            painter.line_segment([egui::pos2(origin.x, y), egui::pos2(right, y)], Stroke::new(1.0, t.border));
        }
        let depth = usize::from(p.indent).min(15);
        match p.list {
            List::None => numbers.iter_mut().for_each(|x| *x = 0),
            List::Bullet | List::Number => {
                for x in numbers.iter_mut().skip(depth + 1) {
                    *x = 0;
                }
                let px = f32::from(look(i).map_or(rich::DEFAULT_SIZE, Style::size)) * PX;
                let mark = if p.list == List::Bullet {
                    if depth % 2 == 0 { "•".to_string() } else { "◦".to_string() }
                } else {
                    let k = numbers.get_mut(depth).map(|x| {
                        *x += 1;
                        *x
                    });
                    format!("{}.", k.unwrap_or(1))
                };
                let x = origin.x + f32::from(p.indent) * INDENT + INDENT - 8.0;
                painter.text(egui::pos2(x, r.center().y), egui::Align2::RIGHT_CENTER, mark, theme::regular(px), t.text);
            }
        }
        if p.list != List::Number
            && let Some(x) = numbers.get_mut(depth)
        {
            *x = 0;
        }
        let mut count = 0;
        for (k, ch) in line.chars().enumerate() {
            count = k + 1;
            if ch == PICTURE
                && !in_card(i + k)
                && let Some(cid) = look(i + k).and_then(|s| s.picture.clone())
            {
                pictures.push((i + k, cid));
            }
        }
        i += count + 1;
    }
    for (at, cid) in pictures {
        let last = c.body.chars().nth(at + 1).is_none_or(|ch| ch == '\n');
        let place = pictures::Place::of(g, origin, at, last);
        pictures::picture(ui, t, ed, c, at, &cid, place);
    }
}

#[cfg(test)]
mod tests;
