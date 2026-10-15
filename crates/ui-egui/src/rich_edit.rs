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
use mailcraft_engine::Compose;
use mailcraft_engine::mime::rich::{self, Align, List, PICTURE, Style, Styles};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// Pixels per point.
const PX: f32 = 4.0 / 3.0;
/// One indent step (and the room for a bullet or number).
const INDENT: f32 = 28.0;
/// The tallest a picture shows in the text.
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
    /// The link box, while open: the address being typed.
    link: Option<String>,
    pictures: HashMap<String, Option<egui::TextureHandle>>,
    /// Give the keyboard back to the text (after a ribbon button took the click), with this
    /// selection.
    refocus: Option<Range<usize>>,
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
            pictures: HashMap::new(),
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
pub(crate) fn body_id(c: &Compose) -> egui::Id {
    egui::Id::new(("compose-body", c.id))
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
}

/// Apply `op` to the selection (or to what's typed next, when nothing is selected).
pub(crate) fn apply(ed: &mut Editor, c: &mut Compose, op: Op) {
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
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
        Op::Clear => Some(Box::new(|s| Style { picture: s.picture.clone(), ..Style::default() })),
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

/// Put `text` at the cursor with style `style` (a link typed into the link box with nothing
/// selected).
fn insert(ed: &mut Editor, c: &mut Compose, at: usize, text: &str, style: Style) {
    ed.remember(c, at);
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert_str(byte, text);
    let n = text.chars().count();
    c.styles.edit_at(&old, &c.body, &style, Some(at + n));
    ed.sel = at + n..at + n;
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
        ed.link = Some(c.styles.style(ed.sel.start).link.unwrap_or_else(|| "https://".into()));
    }
    if book_k {
        acts.push(Action::Dialog(Dialog::select_names(c, "to")));
    }
}

/// A look as text formatting (the person's colours adjusted to stay readable on the page).
fn format(s: &Style, t: &Tokens) -> TextFormat {
    let px = f32::from(s.size()) * PX;
    let font = if s.mono {
        FontId::monospace(px * 0.92)
    } else if s.bold {
        theme::bold(px)
    } else {
        theme::regular(px)
    };
    let (mut color, background) = theme::mail_colors(s.color, s.highlight, t, t.text);
    if s.link.is_some() && s.color.is_none() {
        color = t.accent;
    }
    let line = Stroke::new((px / 14.0).max(1.0), color);
    TextFormat {
        font_id: font,
        color,
        background: background.unwrap_or(Color32::TRANSPARENT),
        italics: s.italic,
        underline: if s.underline || s.link.is_some() { line } else { Stroke::NONE },
        strikethrough: if s.strike { line } else { Stroke::NONE },
        ..Default::default()
    }
}

/// Room for a picture: an invisible, wide character the picture is drawn over.
fn picture_format() -> TextFormat {
    TextFormat { font_id: theme::regular(PICTURE_PX), color: Color32::TRANSPARENT, extra_letter_spacing: PICTURE_PX * 0.8, ..Default::default() }
}

/// The text laid out with its styles; `shift` moves a paragraph right (alignment).
fn job(text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, shift: &[f32]) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let plain_fmt = TextFormat { font_id: theme::regular(14.5), color: t.text, ..Default::default() };
    let mut i = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i.saturating_sub(1)), t) };
            job.append("\n", 0.0, f);
            i += 1;
        }
        let p = st.para(n);
        let mut lead = if plain { 0.0 } else { f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { INDENT } };
        lead += shift.get(n).copied().unwrap_or(0.0);
        let mut run_start = 0usize;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for k in 0..chars.len() {
            let cur = st.chars.get(i + k).copied();
            let next = st.chars.get(i + k + 1).copied();
            let pic = chars.get(k).is_some_and(|(_, ch)| *ch == PICTURE) || chars.get(k + 1).is_some_and(|(_, ch)| *ch == PICTURE);
            if k + 1 == chars.len() || cur != next || pic {
                let from = chars.get(run_start).map_or(line.len(), |(b, _)| *b);
                let to = chars.get(k + 1).map_or(line.len(), |(b, _)| *b);
                let piece = line.get(from..to).unwrap_or("");
                let f = if plain {
                    plain_fmt.clone()
                } else if piece.starts_with(PICTURE) && st.style(i + run_start).picture.is_some() {
                    picture_format()
                } else {
                    format(&st.style(i + run_start), t)
                };
                job.append(piece, lead, f);
                lead = 0.0;
                run_start = k + 1;
            }
        }
        if chars.is_empty() && lead > 0.0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i), t) };
            job.append("", lead, f);
        }
        i += chars.len();
    }
    job
}

/// Lay the text out; centred and right-aligned paragraphs that fit on one line are moved over.
fn layout(ui: &Ui, text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32) -> Arc<Galley> {
    let first = ui.painter().layout_job(job(text, st, plain, t, wrap, &[]));
    // A right-to-left paragraph left as it was sits against the right edge.
    let lines: Vec<&str> = text.split('\n').collect();
    let align_of = |n: usize, a: Align| {
        if a == Align::Left && lines.get(n).is_some_and(|l| crate::bidi::has_rtl(l) && crate::bidi::paragraph_rtl(l)) { Align::Right } else { a }
    };
    if plain || st.paras.iter().enumerate().all(|(n, p)| align_of(n, p.align) == Align::Left) {
        return first;
    }
    let mut shift = vec![0.0; st.paras.len()];
    let mut row = 0;
    for (n, p) in st.paras.iter().enumerate() {
        // The rows this paragraph takes.
        let start = row;
        while let Some(r) = first.rows.get(row) {
            row += 1;
            if r.ends_with_newline {
                break;
            }
        }
        let align = align_of(n, p.align);
        if align != Align::Left
            && row == start + 1
            && let Some(r) = first.rows.get(start)
        {
            let room = (wrap - r.rect().max.x).max(0.0);
            if let Some(s) = shift.get_mut(n) {
                *s = if align == Align::Center { room / 2.0 } else { room };
            }
        }
    }
    ui.painter().layout_job(job(text, st, plain, t, wrap, &shift))
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
        }
    }
    if ui.memory(|m| m.has_focus(id)) {
        keys(ui, ed, c, acts);
    }
    let before = c.body.clone();
    let styles = c.styles.clone();
    let plain = c.plain;
    let tokens = *t;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| layout(ui, buf.as_str(), &styles, plain, &tokens, wrap);
    let height = ui.available_height().max(220.0);
    let out = egui::TextEdit::multiline(&mut c.body)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(tl!("Write your message…"))
        .lock_focus(true)
        .desired_width(f32::INFINITY)
        .min_size(vec2(ui.available_width(), height))
        .layouter(&mut layouter)
        .show(ui);
    let cursor = out.cursor_range.map(|r| {
        let r = r.as_sorted_char_range();
        r.start.0..r.end.0
    });
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
    if !c.plain {
        decorate(ui, t, ed, c, &out.galley, out.galley_pos);
    }
}

/// Bullets and numbers, rules and pictures.
fn decorate(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &Compose, g: &Galley, origin: egui::Pos2) {
    let painter = ui.painter().clone();
    let at = |i: usize| -> Rect { g.pos_from_cursor(CCursor::new(i)).translate(origin.to_vec2()) };
    let mut i = 0usize;
    let mut numbers: Vec<u32> = vec![0; 16];
    let right = ui.min_rect().right().max(origin.x + g.size().x);
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
        for (k, ch) in line.chars().enumerate() {
            if ch != PICTURE {
                continue;
            }
            let Some(cid) = c.styles.style(i + k).picture else { continue };
            let (a, b) = (at(i + k), at(i + k + 1));
            let room = Rect::from_min_max(egui::pos2(a.min.x + 2.0, a.min.y), egui::pos2(b.min.x.max(a.min.x + 8.0) - 2.0, a.max.y));
            let tex = ed.pictures.entry(cid.clone()).or_insert_with(|| {
                c.inline.iter().find(|p| p.cid == cid).and_then(|p| crate::reading::decode(ui.ctx(), &format!("compose-{cid}"), &p.data, 400))
            });
            match tex {
                Some(tex) => {
                    let size = tex.size_vec2();
                    let scale = (room.width() / size.x).min(room.height() / size.y).min(1.0);
                    let r = Rect::from_min_size(room.min, size * scale);
                    painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    painter.rect_stroke(room, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    icons::paint(&painter, icons::centered(room, 22.0), Icon::Image, t.dim);
                }
            }
        }
        i += line.chars().count() + 1;
    }
}

/// A ribbon button: an icon or a letter, lit when its format is on.
fn fmt_button(ui: &mut Ui, t: &Tokens, face: Face<'_>, on: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if on || resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, if on { t.accent_soft } else { t.hover });
    }
    match face {
        Face::Icon(ic) => icons::paint(ui.painter(), icons::centered(rect, 16.0), ic, t.text),
        Face::Letter(s, f) => {
            let galley = ui.painter().layout_job(LayoutJob::single_section(s.to_string(), f));
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
        }
    }
    resp.on_hover_text(tl!(tip))
}

enum Face<'a> {
    Icon(Icon),
    Letter(&'a str, TextFormat),
}

/// A colour button with its dropdown (font colour, highlight).
fn color_button(
    ui: &mut Ui,
    t: &Tokens,
    letter: &str,
    current: Option<[u8; 3]>,
    swatches: &[[u8; 3]],
    none: &str,
    tip: &str,
) -> Option<Option<[u8; 3]>> {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    ui.painter().text(rect.center() - vec2(4.0, 3.0), egui::Align2::CENTER_CENTER, letter, theme::regular(14.0), t.text);
    let bar = Rect::from_min_size(egui::pos2(rect.min.x + 6.0, rect.max.y - 7.0), vec2(16.0, 3.5));
    let shown = current.map_or(if letter == "A" { t.text } else { Color32::TRANSPARENT }, |c| Color32::from_rgb(c[0], c[1], c[2]));
    ui.painter().rect_filled(bar, 1.0, shown);
    icons::paint(ui.painter(), Rect::from_min_size(egui::pos2(rect.max.x - 11.0, rect.center().y - 4.0), vec2(8.0, 8.0)), Icon::ChevronDown, t.dim);
    let resp = resp.on_hover_text(tl!(tip));
    let mut picked = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(6.0 * 26.0);
            for c in swatches {
                let (r, s) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                // A stronger edge under the pointer.
                let edge = if s.hovered() { Stroke::new(2.0, t.text) } else { Stroke::new(1.0, t.border) };
                ui.painter().rect(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]), edge, egui::StrokeKind::Inside);
                if s.clicked() {
                    picked = Some(Some(*c));
                }
            }
        });
        if ui.button(tl!(none)).clicked() {
            picked = Some(None);
        }
    });
    picked
}

/// The Message ribbon: font size, bold / italic / underline / strikethrough, colours, lists,
/// alignment, indent, clear formatting; then names, files, links and pictures.
pub(crate) fn ribbon(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, acts: &mut Vec<Action>) {
    c.styles.fit(&c.body);
    let n = c.body.chars().count();
    let sel = ed.sel.start.min(n)..ed.sel.end.min(n);
    let cur =
        ed.typing.clone().unwrap_or_else(|| if sel.is_empty() { typing_style(&c.body, &c.styles, sel.start) } else { c.styles.style(sel.start) });
    let para = c.styles.para(rich::lines_of(&c.body, sel.start..sel.start).0);
    let mut ops: Vec<Op> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_enabled_ui(!c.plain, |ui| {
            egui::ComboBox::from_id_salt(("font-size", c.id)).width(48.0).selected_text(cur.size().to_string()).show_ui(ui, |ui| {
                for z in rich::SIZES {
                    if ui.selectable_label(cur.size() == z, z.to_string()).clicked() {
                        ops.push(Op::Size(z));
                    }
                }
            });
            ui.add_space(4.0);
            let letter = |bold: bool, italic: bool, under: bool, strike: bool| {
                let line = Stroke::new(1.2, t.text);
                TextFormat {
                    font_id: if bold { theme::bold(15.0) } else { theme::regular(15.0) },
                    color: t.text,
                    italics: italic,
                    underline: if under { line } else { Stroke::NONE },
                    strikethrough: if strike { line } else { Stroke::NONE },
                    ..Default::default()
                }
            };
            for (s, face, on, tip, op) in [
                ("B", letter(true, false, false, false), cur.bold, "Bold (Ctrl+B)", Op::Bold),
                ("I", letter(false, true, false, false), cur.italic, "Italic (Ctrl+I)", Op::Italic),
                ("U", letter(false, false, true, false), cur.underline, "Underline (Ctrl+U)", Op::Underline),
                ("ab", letter(false, false, false, true), cur.strike, "Strikethrough", Op::Strike),
            ] {
                if fmt_button(ui, t, Face::Letter(s, face), on, tip).clicked() {
                    ops.push(op);
                }
            }
            if let Some(h) = color_button(ui, t, "ab", cur.highlight, &HIGHLIGHTS, "No color", "Text highlight color") {
                ops.push(Op::Highlight(h));
            }
            if let Some(col) = color_button(ui, t, "A", cur.color, &COLORS, "Automatic", "Font color") {
                ops.push(Op::Color(col));
            }
            ui.separator();
            for (ic, on, tip, op) in [
                (Icon::ListBullet, para.list == List::Bullet, "Bullets (Ctrl+Shift+L)", Op::Bullets),
                (Icon::ListNumber, para.list == List::Number, "Numbering", Op::Numbers),
                (Icon::IndentLess, false, "Decrease indent (Ctrl+Shift+T)", Op::Outdent),
                (Icon::IndentMore, false, "Increase indent (Ctrl+T)", Op::Indent),
                (Icon::AlignLeft, para.align == Align::Left, "Align left (Ctrl+L)", Op::Align(Align::Left)),
                (Icon::AlignCenter, para.align == Align::Center, "Center (Ctrl+E)", Op::Align(Align::Center)),
                (Icon::AlignRight, para.align == Align::Right, "Align right (Ctrl+R)", Op::Align(Align::Right)),
                (Icon::ClearFormat, false, "Clear all formatting (Ctrl+Space)", Op::Clear),
            ] {
                if fmt_button(ui, t, Face::Icon(ic), on, tip).clicked() {
                    ops.push(op);
                }
            }
        });
        ui.separator();
        if crate::widgets::tool_button(ui, Icon::AddressBook, "Address Book", t, !c.sending).on_hover_text(tl!("Ctrl+Shift+B")).clicked() {
            acts.push(Action::Dialog(Dialog::select_names(c, "to")));
        }
        if crate::widgets::tool_button(ui, Icon::Attach, "Attach File", t, !c.sending).clicked() {
            acts.push(Action::AttachFiles);
        }
        if crate::widgets::tool_button(ui, Icon::Link, "Link", t, !c.sending && !c.plain).on_hover_text(tl!("Ctrl+K")).clicked() {
            ed.link = Some(cur.link.clone().unwrap_or_else(|| "https://".into()));
        }
        if crate::widgets::tool_button(ui, Icon::Image, "Pictures", t, !c.sending && !c.plain).clicked() {
            acts.push(Action::InsertPicture);
        }
    });
    // The link box.
    if let Some(mut url) = ed.link.take() {
        let mut keep = true;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Address")).color(t.dim));
            let r = ui.add(egui::TextEdit::singleline(&mut url).desired_width(320.0));
            if !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button(tl!("Insert")).clicked() || enter {
                let u = url.trim().to_string();
                if u.len() > "https://".len() || u.starts_with("mailto:") {
                    if sel.is_empty() {
                        let style = Style { link: Some(u.clone()), ..typing_style(&c.body, &c.styles, sel.start) };
                        insert(ed, c, sel.start, &u, style);
                    } else {
                        ops.push(Op::Link(u));
                    }
                }
                keep = false;
            }
            if cur.link.is_some() && ui.button(tl!("Remove link")).clicked() {
                ops.push(Op::Unlink);
                keep = false;
            }
            if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        if keep {
            ed.link = Some(url);
        } else {
            ed.refocus = Some(ed.sel.clone());
        }
    }
    let changed = !ops.is_empty();
    for op in ops {
        apply(ed, c, op);
    }
    if changed {
        ed.refocus = Some(ed.sel.clone());
    }
}
