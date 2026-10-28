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
use egui::{Color32, FontId, Galley, Key, Modifiers, Rect, RichText, Sense, Stroke, Ui, Vec2, vec2};
use mailcraft_engine::Compose;
use mailcraft_engine::mime::rich::{self, Align, List, PICTURE, Style, Styles};
use mailcraft_engine::model::dates::Clock;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

mod layout;
mod links;
mod pictures;
mod toolbar;
use layout::*;
pub(crate) use links::link_dialog;
use links::*;
pub use toolbar::Counts;
pub(crate) use toolbar::ribbon;
#[cfg(test)]
pub(crate) use toolbar::{count_words, date_choices};

/// Pixels per point.
const PX: f32 = 4.0 / 3.0;
/// One indent step (and the room for a bullet or number).
const INDENT: f32 = 28.0;
/// How tall a picture shows while its size isn't known.
const PICTURE_PX: f32 = 72.0;
/// Most undo steps kept.
const UNDO_MAX: usize = 200;

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
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    last_change: f64,
    /// The Insert link dialog, while open.
    link: Option<LinkForm>,
    /// Paste once the text has the keyboard back (asked from the right-click menu).
    paste: bool,
    /// The right-click menu was open last frame (closing it gives the text the keyboard back).
    menu_open: bool,
    pictures: HashMap<String, Option<egui::TextureHandle>>,
    /// Each picture's own size (`None`: it can't be read).
    sizes: HashMap<String, Option<Vec2>>,
    /// Give the keyboard back to the text (after a ribbon button took the click), with this
    /// selection.
    pub(crate) refocus: Option<Range<usize>>,
}

#[derive(Clone)]
struct Snap {
    text: String,
    styles: Styles,
    cursor: usize,
}

impl Editor {
    pub fn new(id: u64) -> Editor {
        Editor {
            id,
            typing: None,
            sel: 0..0,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: 0.0,
            link: None,
            paste: false,
            menu_open: false,
            pictures: HashMap::new(),
            sizes: HashMap::new(),
            refocus: None,
        }
    }

    fn remember(&mut self, c: &Compose, cursor: usize) {
        self.undo.push(Snap { text: c.body.clone(), styles: c.styles.clone(), cursor });
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

/// The message text box's id.
/// The id of the signature being edited (the signature editor is a message editor of its own).
pub(crate) const SIGNATURE_ID: u64 = u64::MAX;

pub(crate) fn body_id(c: &Compose) -> egui::Id {
    egui::Id::unique(("compose-body", c.id))
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

/// The look typing at `at` takes: the character before (or after, at a paragraph's start),
/// without a link the text doesn't continue.
fn typing_style(text: &str, styles: &Styles, at: usize) -> Style {
    let chars: Vec<char> = text.chars().collect();
    let before = at.checked_sub(1).filter(|i| chars.get(*i).is_some_and(|c| *c != '\n'));
    let i = before.or_else(|| chars.get(at).filter(|c| **c != '\n').map(|_| at));
    let Some(i) = i else { return Style::default() };
    let mut s = styles.style(i).for_typing();
    if s.link.is_some() && styles.style(at).link != s.link {
        s.link = None;
    }
    s
}

/// A change the ribbon or a shortcut asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    Bold,
    Italic,
    Underline,
    Strike,
    Size(u8),
    Grow,
    Shrink,
    Color(Option<[u8; 3]>),
    Highlight(Option<[u8; 3]>),
    Bullets,
    Numbers,
    Align(Align),
    Indent,
    Outdent,
    Clear,
    Link(String),
    Unlink,
    /// Text put in at the cursor (in place of the selection), as typing would.
    Insert(String),
}

/// Apply `op` to the selection (or to what's typed next, when nothing is selected).
pub(crate) fn apply(ed: &mut Editor, c: &mut Compose, op: Op) {
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    // Typed in, it takes the style typing would; a plain text message stays plain.
    if let Op::Insert(text) = &op {
        let style = ed.typing.clone().unwrap_or_else(|| typing_style(&c.body, &c.styles, sel.start));
        replace(ed, c, sel, text, style);
        return;
    }
    ed.remember(c, sel.end);
    let collapsed = sel.is_empty();
    let base = ed.typing.clone().unwrap_or_else(|| typing_style(&c.body, &c.styles, sel.start));
    let toggled = |get: fn(&Style) -> bool| -> bool { if collapsed { !get(&base) } else { !rich::all(&c.body, &c.styles, sel.clone(), get) } };
    type F = Box<dyn Fn(&Style) -> Style>;
    let char_op: Option<F> = match op.clone() {
        Op::Bold => {
            let on = toggled(|s| s.bold);
            Some(Box::new(move |s| Style { bold: on, ..s.clone() }))
        }
        Op::Italic => {
            let on = toggled(|s| s.italic);
            Some(Box::new(move |s| Style { italic: on, ..s.clone() }))
        }
        Op::Underline => {
            let on = toggled(|s| s.underline);
            Some(Box::new(move |s| Style { underline: on, ..s.clone() }))
        }
        Op::Strike => {
            let on = toggled(|s| s.strike);
            Some(Box::new(move |s| Style { strike: on, ..s.clone() }))
        }
        Op::Size(z) => Some(Box::new(move |s| Style { size: (z != rich::DEFAULT_SIZE).then_some(z), ..s.clone() })),
        Op::Grow | Op::Shrink => {
            let up = op == Op::Grow;
            Some(Box::new(move |s| {
                let z = rich::step_size(s.size(), up);
                Style { size: (z != rich::DEFAULT_SIZE).then_some(z), ..s.clone() }
            }))
        }
        Op::Color(col) => Some(Box::new(move |s| Style { color: col, ..s.clone() })),
        Op::Highlight(col) => Some(Box::new(move |s| Style { highlight: col, ..s.clone() })),
        Op::Clear => Some(Box::new(|s| Style { picture: s.picture.clone(), picture_width: s.picture_width, ..Style::default() })),
        Op::Link(url) => {
            let url = url.trim().to_string();
            Some(Box::new(move |s| Style { link: Some(url.clone()), ..s.clone() }))
        }
        Op::Unlink => Some(Box::new(|s| Style { link: None, ..s.clone() })),
        _ => None,
    };
    if let Some(f) = char_op {
        if collapsed {
            ed.typing = Some(f(&base));
        } else {
            c.styles.restyle(&c.body, sel.clone(), f);
        }
    }
    let (first, last) = rich::lines_of(&c.body, sel.clone());
    let all_list = |kind: List| (first..=last).all(|l| c.styles.para(l).list == kind);
    match op {
        Op::Bullets | Op::Numbers => {
            let kind = if op == Op::Bullets { List::Bullet } else { List::Number };
            let to = if all_list(kind) { List::None } else { kind };
            c.styles.relayout(&c.body, sel, |p| p.list = to);
        }
        Op::Align(a) => c.styles.relayout(&c.body, sel, |p| p.align = a),
        Op::Indent => c.styles.relayout(&c.body, sel, |p| p.indent = p.indent.saturating_add(1)),
        Op::Outdent => c.styles.relayout(&c.body, sel, |p| p.indent = p.indent.saturating_sub(1)),
        Op::Clear => c.styles.relayout(&c.body, sel, |p| *p = rich::Para { rule: p.rule, ..Default::default() }),
        _ => {}
    }
    c.plain = false;
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
    to.push(Snap { text: c.body.clone(), styles: c.styles.clone(), cursor: ed.sel.end });
    c.body = s.text;
    c.styles = s.styles;
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
    let line = rich::lines_of(&c.body, ed.sel.start..ed.sel.start).0;
    let para = c.styles.para(line);
    let line_start = c.body.split('\n').take(line).map(|l| l.chars().count() + 1).sum::<usize>();
    let line_empty = c.body.split('\n').nth(line).is_some_and(str::is_empty);
    let at_start = ed.sel.is_empty() && ed.sel.start == line_start;
    let listed = para.list != List::None;
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
    for op in ops {
        apply(ed, c, op);
    }
    if undo_k || redo_k {
        undo(ed, c, &ctx, redo_k);
    }
    if link_k {
        ed.link = Some(LinkForm::open(c, ed.sel.clone()));
    }
    if book_k && c.id != SIGNATURE_ID {
        acts.push(Action::Dialog(Dialog::select_names(c, "to")));
    }
}

/// Paste what another program put on the clipboard as HTML (a signature copied from Outlook,
/// part of a web page) with its look, in place of the selection. `false` when there is none,
/// or it holds no text (a picture alone is pasted as a picture).
fn paste_formatted(ui: &mut Ui, ed: &mut Editor, c: &mut Compose, id: egui::Id) -> bool {
    let Some(html) = crate::clipboard_html() else { return false };
    let (add, add_styles) = mailcraft_engine::mime::pasted_html(&html);
    if add.trim().is_empty() {
        return false;
    }
    ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_))));
    c.styles.fit(&c.body);
    ed.undo.push(Snap { text: c.body.clone(), styles: c.styles.clone(), cursor: ed.sel.start });
    if ed.undo.len() > UNDO_MAX {
        ed.undo.remove(0);
    }
    ed.redo.clear();
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
    let before = c.body.clone();
    let styles = c.styles.clone();
    let plain = c.plain;
    let tokens = *t;
    pictures::measure(ed, c);
    let sizes = &ed.sizes;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| layout(ui, buf.as_str(), &styles, plain, &tokens, wrap, sizes);
    let height = ui.available_height().max(220.0);
    // A right-click inside the selection keeps it (egui's text box moves the cursor on any
    // press); one outside it moves the cursor there, as Outlook does.
    let kept = ed.sel.clone();
    let right_press = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
    // The selection is drawn behind the text while the text box doesn't have the keyboard
    // (a menu, a dialog or the ribbon has it), so what's selected stays plain to see.
    let behind = ui.painter().add(egui::Shape::Noop);
    // No hint over a first line that's a list item or indented (its bullet is drawn there).
    let p0 = c.styles.para(0);
    let hint = match (p0.list == List::None && p0.indent == 0, c.id == SIGNATURE_ID) {
        (false, _) => "",
        (true, true) => tl!("Write your signature, or paste it…"),
        (true, false) => tl!("Write your message…"),
    };
    let out = egui::TextEdit::multiline(&mut c.body)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(hint)
        .lock_focus(true)
        .desired_width(f32::INFINITY)
        .min_size(vec2(ui.available_width(), height))
        .layouter(&mut layouter)
        .show(ui);
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
    if c.body != before {
        // The look of what was typed: the ribbon's choice, else the text's at the cursor.
        let typing = ed.typing.take().unwrap_or_else(|| typing_style(&before, &styles, ed.sel.start));
        let now = ui.input(|i| i.time);
        let breaks = c.body.matches('\n').count() != before.matches('\n').count();
        if now - ed.last_change > 1.0 || breaks || ed.undo.is_empty() {
            ed.undo.push(Snap { text: before.clone(), styles: styles.clone(), cursor: ed.sel.start });
            if ed.undo.len() > UNDO_MAX {
                ed.undo.remove(0);
            }
            ed.redo.clear();
        }
        ed.last_change = now;
        c.styles.edit_at(&before, &c.body, &typing, cursor.as_ref().map(|r| r.end));
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
    if !c.plain
        && let Some(r) = decorate(ui, t, ed, c, &out.galley, out.galley_pos)
    {
        pictures::resize(ed, c, r);
        ui.ctx().request_repaint();
    }
    if !out.response.has_focus() && !ed.sel.is_empty() {
        let color = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
        let shapes: Vec<egui::Shape> = selection_rects(&out.galley, ed.sel.clone())
            .into_iter()
            .map(|r| egui::Shape::rect_filled(r.translate(out.galley_pos.to_vec2()), 0.0, color))
            .collect();
        ui.painter().set(behind, egui::Shape::Vec(shapes));
    }
    text_menu(&out.response, ed, c, acts);
}

/// The rectangles a selection covers, one per row (galley coordinates).
fn selection_rects(g: &Galley, sel: Range<usize>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    // Long selections are drawn in full; past this many characters only their start is.
    for i in sel.start..sel.end.min(sel.start + 20_000) {
        let a = g.pos_from_cursor(CCursor::new(i));
        let b = g.pos_from_cursor(CCursor::new(i + 1));
        // A line's end: the selection runs a little past the text, as text boxes show it.
        let right = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + 5.0 };
        let r = Rect::from_min_max(egui::pos2(a.min.x, a.min.y), egui::pos2(right.max(a.min.x), a.max.y));
        match out.last_mut() {
            Some(last) if (last.min.y - r.min.y).abs() < 0.5 => *last = last.union(r),
            _ => out.push(r),
        }
    }
    out
}

/// The right-click menu on the text, as Outlook's: Cut, Copy, Paste and Select all; the link
/// (insert, edit, open, remove); and clearing the formatting.
fn text_menu(resp: &egui::Response, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>) {
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let chosen: String = c.body.chars().skip(sel.start).take(sel.end - sel.start).collect();
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
        if item(ui, "Cut", "Ctrl+X", some && !c.sending) {
            ui.ctx().copy_text(chosen.clone());
            let style = typing_style(&c.body, &c.styles, sel.start);
            replace(ed, c, sel.clone(), "", style);
            ed.refocus = Some(ed.sel.clone());
        }
        if item(ui, "Copy", "Ctrl+C", some) {
            ui.ctx().copy_text(chosen.clone());
            ed.refocus = Some(sel.clone());
        }
        if item(ui, "Paste", "Ctrl+V", !c.sending) {
            ed.paste = true;
            ed.refocus = Some(sel.clone());
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

/// Bullets and numbers, rules and pictures; a picture resized with its handle.
fn decorate(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &Compose, g: &Galley, origin: egui::Pos2) -> Option<pictures::Resize> {
    let painter = ui.painter().clone();
    let at = |i: usize| -> Rect { g.pos_from_cursor(CCursor::new(i)).translate(origin.to_vec2()) };
    let mut i = 0usize;
    let mut numbers: Vec<u32> = vec![0; 16];
    let right = ui.min_rect().right().max(origin.x + g.size().x);
    let mut resized = None;
    for (n, line) in c.body.split('\n').enumerate() {
        let p = c.styles.para(n);
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
                let style = c.styles.style(i);
                let px = f32::from(style.size()) * PX;
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
        // As wide as the layout let each picture be (`layout::job`).
        let most = g.job.wrap.max_width - paragraph_lead(&p);
        for (k, ch) in line.chars().enumerate() {
            if ch == PICTURE
                && let Some(r) = pictures::paint(ui, t, ed, c, i + k, at(i + k), most)
            {
                resized = Some(r);
            }
        }
        i += line.chars().count() + 1;
    }
    resized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_addresses() {
        let web = |a: &str| LinkForm { address: a.into(), ..Default::default() }.url();
        assert_eq!(web("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(web("http://x.example/a").as_deref(), Some("http://x.example/a"));
        assert_eq!(web("C:/Users/me/plan.docx").as_deref(), Some("file:///C:/Users/me/plan.docx"));
        assert_eq!(web("https://"), None);
        assert_eq!(web("  "), None);
        let mail = LinkForm { email: true, to: "zoe@studio.example".into(), subject: "Poster & sizes".into(), ..Default::default() };
        assert_eq!(mail.url().as_deref(), Some("mailto:zoe@studio.example?subject=Poster%20%26%20sizes"));
        assert_eq!(LinkForm { email: true, ..Default::default() }.url(), None);
        // An existing mailto link opens as an email address with its subject.
        let mut c = Compose { body: "write".into(), ..Default::default() };
        c.styles = Styles::plain(&c.body);
        c.styles.restyle(&c.body, 0..5, |s| Style { link: Some("mailto:ann@x.example?subject=Hi%20there".into()), ..s.clone() });
        let f = LinkForm::open(&c, 0..5);
        assert!(f.email && f.editing && !f.multi);
        assert_eq!((f.to.as_str(), f.subject.as_str(), f.text.as_str()), ("ann@x.example", "Hi there", "write"));
    }

    /// Right-to-left paragraphs in a message start against the editor's right edge, a short one
    /// and every row of a wrapped one, in rich and plain text; a left-to-right one stays left. (egui
    /// put them against the widest line, so a short Hebrew line sat in the middle.)
    #[test]
    fn right_to_left_paragraphs_start_on_the_right() {
        let text = "שלום עולם
English stays left.
זוהי פסקה ארוכה מאוד בעברית שתיכנס לכמה שורות כשהעורך צר, וכל שורה שלה צריכה להתחיל בצד ימין של העורך.";
        let wrap = 300.0;
        let ctx = egui::Context::default();
        let t = Tokens::light();
        for plain in [false, true] {
            let mut rows = Vec::new();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let g = layout::layout(ui, text, &Styles::plain(text), plain, &t, wrap, &HashMap::new());
                rows = g
                    .rows
                    .iter()
                    .map(|r| {
                        // Where its letters are (a row's rect takes in its leading space).
                        let ink =
                            |f: fn(f32, f32) -> f32, init: f32| r.row.glyphs.iter().filter(|g| !g.chr.is_whitespace()).map(|g| g.pos.x).fold(init, f);
                        let right = r.row.glyphs.iter().map(|g| g.max_x()).fold(0.0, f32::max);
                        (r.pos.x + ink(f32::min, f32::INFINITY), r.pos.x + right, r.ends_with_newline)
                    })
                    .collect();
            });
            out.textures_delta.clear();
            assert!(rows.len() > 3, "the long paragraph wraps: {rows:?}");
            // Row 0: the short Hebrew line; row 1: English; then the wrapped Hebrew paragraph.
            let (min0, max0, _) = rows[0];
            assert!((wrap - max0).abs() < 2.0 && min0 > 100.0, "plain={plain}: the short line against the right edge: {rows:?}");
            assert!(rows[1].0 < 2.0, "plain={plain}: English on the left: {rows:?}");
            for r in &rows[2..] {
                assert!((wrap - r.1).abs() < 2.0, "plain={plain}: each wrapped row against the right edge: {rows:?}");
            }
        }
    }

    /// A picture's room starts where its place in the text does, and is as wide at the start of
    /// a line as after text (letter spacing put it some 60 points right of its place after text,
    /// and none at a line's start, squeezing the picture into 36).
    #[test]
    fn a_picture_sits_where_it_is_in_the_text() {
        let ctx = egui::Context::default();
        let t = Tokens::light();
        for (body, at) in [("\u{FFFC}", 0), ("ab\u{FFFC}cd", 2), ("x\n\u{FFFC}\u{FFFC}", 2), ("x\n\u{FFFC}\u{FFFC}", 3)] {
            let mut st = Styles::plain(body);
            for (i, ch) in body.chars().enumerate() {
                if ch == PICTURE {
                    st.restyle(body, i..i + 1, |s| Style { picture: Some("cid".into()), ..s.clone() });
                }
            }
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let g = layout::layout(ui, body, &st, false, &t, 600.0, &HashMap::new());
                let (a, b) = (g.pos_from_cursor(CCursor::new(at)), g.pos_from_cursor(CCursor::new(at + 1)));
                let mut glyphs = g.rows.iter().flat_map(|r| r.row.glyphs.iter().map(move |gl| (r.pos.x + gl.pos.x, r.rect().center().y, gl.chr)));
                let here = glyphs.any(|(x, y, c)| c == PICTURE && (x - a.min.x).abs() < 1.0 && (y - a.center().y).abs() < 40.0);
                assert!(here, "{body:?}: the picture's character where its room starts");
                assert!(b.min.x - a.min.x >= PICTURE_PX * 1.2, "{body:?} at {at}: a room {} wide", b.min.x - a.min.x);
            });
            out.textures_delta.clear();
        }
    }
}
