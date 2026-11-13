//! HTML laid out as blocks: paragraphs, tables and their columns, colours, background pictures.

use super::*;

pub(super) struct Ctx {
    /// The message's `<style>` blocks, given to each piece of text (for colours and hidden parts).
    pub(super) css: String,
    pub(super) cells: usize,
    /// Links drawn as buttons so far ([`MAX_BUTTONS`]).
    pub(super) buttons: usize,
    /// The text colour the boxes around inherit to the text inside (a cell's `color`).
    pub(super) color: Option<String>,
    /// What's known of the box around: for telling a box that only wraps what it holds.
    pub(super) around: Around,
    /// Boxes taken for wrappers on the way here ([`MAX_UNWRAPPED`]).
    pub(super) unwrapped: usize,
    /// The size of text here, in pixels, as the boxes around set it (`0`: not set).
    pub(super) size: u16,
    /// Text here is bold, as the boxes around set it (a cell's `font-weight`).
    pub(super) bold: bool,
}

/// What's known of the box a part is in.
#[derive(Clone, Copy, Default)]
pub(super) struct Around {
    /// How wide it is inside, in pixels, when that's known.
    pub(super) room: Option<u32>,
    /// The colour it's on, when that's the box's own.
    pub(super) colour: Option<[u8; 3]>,
    /// How what's in it lines up.
    pub(super) align: Align,
    /// Where a table in it sits when the table doesn't say (the cell's `align` attribute).
    pub(super) boxes: Align,
}

impl Ctx {
    /// `f` with `around` as the box around.
    fn within<R>(&mut self, around: Around, f: impl FnOnce(&mut Ctx) -> R) -> R {
        let saved = std::mem::replace(&mut self.around, around);
        let r = f(self);
        self.around = saved;
        r
    }

    /// The box around a table's cell `c`: its share of the table's room, less its own.
    fn around_cell(&self, n: &Node, c: &Node, alone: bool) -> Around {
        let table = match n.width() {
            Some(Width::Px(w)) => Some(self.around.room.map_or(w, |r| r.min(w))),
            Some(Width::Percent(p)) => self.around.room.map(|r| r.saturating_mul(u32::from(p)) / 100),
            None => None,
        };
        let cell = match c.width() {
            Some(Width::Px(w)) => Some(table.map_or(w, |t| t.min(w))),
            Some(Width::Percent(p)) => table.map(|t| t.saturating_mul(u32::from(p)) / 100),
            None => table.filter(|_| alone),
        };
        let room = c.padding().unwrap_or([n.cell_padding(); 4]);
        let less = u32::from(room[1]).saturating_add(u32::from(room[3])).saturating_add(u32::from(n.cell_spacing()).saturating_mul(2));
        Around {
            room: cell.map(|w| w.saturating_sub(less)),
            colour: c.background().or(n.background()),
            align: c.own_align().unwrap_or(self.around.align),
            boxes: c.boxes_align().unwrap_or_default(),
        }
    }

    /// Whether a box of this colour, this wide, adds nothing to the box around: a colour it's
    /// on already, the whole of its room.
    fn adds_nothing(&self, colour: Option<[u8; 3]>, width: Option<Width>) -> bool {
        let full = match width {
            Some(Width::Percent(p)) => p >= 100,
            Some(Width::Px(w)) => self.around.room.is_some_and(|r| w >= r),
            None => false,
        };
        full && (colour.is_none() || colour == self.around.colour)
    }

    /// What a table holds, when all it does is wrap it (issue #47: a newsletter made with MJML
    /// has a table in a box in a table for each part of it, none of them drawing anything, and
    /// past the depth laid out its last parts were text tables): one row of one cell, as wide as
    /// the room there is, with no room, colour, corners or height of its own.
    fn wrapped<'a>(&self, n: &'a Node) -> Option<&'a Node> {
        if self.unwrapped >= MAX_UNWRAPPED || n.background_image().is_some() || n.radius().is_some() || n.cell_spacing() != 0 {
            return None;
        }
        let rows = n.rows();
        let [(tr, cells)] = rows.as_slice() else { return None };
        let [c] = cells.as_slice() else { return None };
        let c: &'a Node = c;
        let quiet = c.background_image().is_none()
            && c.radius().is_none()
            && c.min_height().is_none()
            && tr.min_height().is_none()
            && !c.has_overlays()
            && n.table_line().is_none()
            && c.cell_lines(tr, n).iter().all(Option::is_none);
        let no_room = c.padding().map_or(n.cell_padding() == 0, |p| p == [0; 4]);
        let colours = [n.background(), tr.background(), c.background()];
        let same_colour = colours.iter().flatten().all(|b| Some(*b) == self.around.colour);
        let in_line = c.own_align().is_none_or(|a| a == self.around.align);
        let full = self.adds_nothing(None, n.width()) && (c.width().is_none() || self.adds_nothing(None, c.width()));
        (quiet && no_room && same_colour && in_line && full).then_some(c)
    }
}

/// `f` with the text colour, size and weight `n` sets, if it sets them, as CSS inherits them.
pub(super) fn colored<R>(ctx: &mut Ctx, n: &Node, f: impl FnOnce(&mut Ctx) -> R) -> R {
    let size = n.text_size(ctx.size);
    let saved_size = std::mem::replace(&mut ctx.size, size);
    let bold = n.bold(ctx.bold);
    let saved_bold = std::mem::replace(&mut ctx.bold, bold);
    let r = colored_only(ctx, n, f);
    ctx.size = saved_size;
    ctx.bold = saved_bold;
    r
}

fn colored_only<R>(ctx: &mut Ctx, n: &Node, f: impl FnOnce(&mut Ctx) -> R) -> R {
    // Only what a colour is written with: it goes back into markup.
    let own = n
        .style_as_written("color")
        .or_else(|| (n.tag == "font").then(|| n.attr("color").map(str::to_string)).flatten())
        .filter(|c| c.len() < 40 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || "#(),. %".contains(ch)));
    match own {
        Some(c) => {
            let saved = ctx.color.replace(c);
            let r = f(ctx);
            ctx.color = saved;
            r
        }
        None => f(ctx),
    }
}

/// Whether a cell holds its place in its row: one with something in it or behind it, or one
/// that says how wide or tall it is (a spacer). An empty cell that says neither is left out; one
/// that does was too, and what came after it in the row moved into its place (a progress bar's
/// first step stood at its column's edge, its line gone from before it).
fn held(c: &Cell) -> bool {
    !c.blocks.is_empty() || c.background.is_some() || c.background_image.is_some() || c.width.is_some() || c.height.is_some()
}

/// The room inside a `table`'s cell: its own (`padding`), and above its first box the room that
/// box asks for (`<div style="margin-top: 20px">`: a footer's first line stood against the top
/// of its box).
fn room(c: &Node, table: &Node) -> Option<[u16; 4]> {
    let first = c.kids.iter().find(|k| !matches!(k, Kid::Text(t) if t.trim().is_empty()));
    let above = match first {
        Some(Kid::El(n)) if n.spacer().is_none() && n.rule().is_none() => n.room_above(),
        _ => 0,
    };
    if above == 0 {
        return c.padding();
    }
    let mut room = c.padding().unwrap_or([table.cell_padding(); 4]);
    room[0] = room[0].saturating_add(above).min(MAX_ROOM);
    Some(room)
}

/// A box with room above it (`margin-top`, a line's height or more) after text that leaves none
/// after it: an empty line between them, as a browser parts them. (Paragraphs and line breaks
/// leave their own, and a browser gives the larger of two, not both.)
fn parted(n: &Node, pending: &str) -> bool {
    let before = pending.trim_end();
    // What the box before it ends with, inside its own closing tags.
    let within = before.trim_end_matches("</div>").trim_end();
    n.room_above() >= 12
        && before.ends_with("</div>")
        && !["</p>", "</ul>", "</ol>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>", "</h6>", "<br>", "<br/>"].iter().any(|t| within.ends_with(t))
        && !matches!(n.kids.iter().find(|k| !matches!(k, Kid::Text(t) if t.trim().is_empty())), Some(Kid::El(c)) if c.tag == "p")
}

pub(super) fn blocks(kids: &[Kid], ctx: &mut Ctx, depth: usize) -> Vec<Block> {
    // Boxes side by side (`inline-block`, floated) straight in a cell or the body, with no box
    // around them: columns as they are in any box (issue #47: a newsletter's three
    // `inline-block` notes under its products stacked one under another).
    let boxes = kids.iter().filter(|k| matches!(k, Kid::El(n) if !n.hidden())).count();
    if boxes >= 2 && depth < MAX_DEPTH {
        let around = Node { tag: "div".into(), attrs: Vec::new(), kids: kids.iter().map(copy).collect() };
        if around.columns().is_some() {
            return blocks(&[Kid::El(around)], ctx, depth);
        }
    }
    let mut out = Vec::new();
    let mut pending = String::new();
    let flush = |pending: &mut String, out: &mut Vec<Block>, ctx: &Ctx| {
        if pending.chars().any(|c| !c.is_whitespace()) || pending.contains("<img") {
            let rich = match &ctx.color {
                Some(c) => crate::html_to_rich(&format!("{}<div style=\"color:{c}\">{}</div>", ctx.css, pending)),
                None => crate::html_to_rich(&format!("{}{}", ctx.css, pending)),
            };
            if !rich.is_empty() {
                out.push(Block::Text(rich));
            } else if pending.contains("<br") {
                // Line breaks alone between two boxes: the empty lines a browser leaves there.
                let n = pending.matches("<br").count().clamp(1, 4);
                out.push(Block::Text(vec![Vec::new(); n]));
            }
        }
        pending.clear();
    };
    for k in kids {
        match k {
            Kid::El(n) if n.hidden() => {}
            // A divider: a line as wide as the box says (one for each way of writing it that a
            // template gives, the one for Outlook and the one for the others).
            Kid::El(n) if n.rule().is_some() => {
                let Some(line) = n.rule() else { continue };
                flush(&mut pending, &mut out, ctx);
                if !out.last().is_some_and(|b| rule_of(b) == Some(line)) {
                    out.push(rule(line, n.width().or(Some(Width::Percent(100)))));
                }
            }
            // Room with nothing in it, or a bar of colour.
            Kid::El(n) if n.spacer().is_some() => {
                flush(&mut pending, &mut out, ctx);
                let cell =
                    Cell { background: n.background(), span: 1, padding: Some([0; 4]), height: n.spacer(), radius: n.radius(), ..Default::default() };
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), ..Default::default() }));
            }
            // A table that only wraps what it holds: what it holds, at this depth.
            Kid::El(n) if n.tag == "table" && ctx.wrapped(n).is_some() => {
                let Some(c) = ctx.wrapped(n) else { continue };
                flush(&mut pending, &mut out, ctx);
                ctx.unwrapped += 1;
                let inner = colored(ctx, n, |ctx| colored(ctx, c, |ctx| blocks(&c.kids, ctx, depth)));
                ctx.unwrapped -= 1;
                out.extend(inner);
            }
            Kid::El(n) if n.tag == "table" && depth < MAX_DEPTH => {
                flush(&mut pending, &mut out, ctx);
                let rows: Vec<Vec<Cell>> = colored(ctx, n, |ctx| {
                    n.rows()
                        .into_iter()
                        .map(|(tr, row)| {
                            // A row's colour (`<tr bgcolor>`, a striped row's class) is its cells', as
                            // is where their content sits and how tall it is at least.
                            let row_bg = tr.background();
                            let (row_valign, row_height) = (tr.valign(), tr.min_height());
                            let alone = row.len() == 1;
                            row.into_iter()
                                .map(|c| {
                                    ctx.cells += 1;
                                    let around = ctx.around_cell(n, c, alone);
                                    let mut content = if ctx.cells > MAX_CELLS {
                                        Vec::new()
                                    } else {
                                        colored(ctx, c, |ctx| ctx.within(around, |ctx| blocks(&c.kids, ctx, depth + 1)))
                                    };
                                    // A heading cell's words: bold unless it says otherwise.
                                    if c.tag == "th" && c.style("font-weight").is_none_or(|w| w != "normal" && w != "400") {
                                        each_span(&mut content, &mut |s| s.bold = true);
                                    }
                                    Cell {
                                        width: c.width().or_else(|| picture_width(&content)),
                                        blocks: content,
                                        background: c.background().or(row_bg),
                                        borders: c.cell_lines(tr, n),
                                        // Its own, its row's, and a heading cell's middle.
                                        align: c.own_align().or(tr.own_align()).unwrap_or(if c.tag == "th" { Align::Center } else { Align::Left }),
                                        span: c.attr("colspan").and_then(|s| s.trim().parse::<u16>().ok()).unwrap_or(1).clamp(1, 50),
                                        background_image: c.background_image(),
                                        overlays: Vec::new(),
                                        padding: room(c, n),
                                        valign: c.valign().or(row_valign).unwrap_or(VAlign::Middle),
                                        height: c.min_height().max(row_height),
                                        radius: c.radius(),
                                    }
                                })
                                .filter(held)
                                .collect::<Vec<Cell>>()
                        })
                        .filter(|r| !r.is_empty())
                        .collect()
                });
                if rows.is_empty() {
                    continue;
                }
                out.push(Block::Table(Table {
                    rows,
                    width: n.width(),
                    // Where it says, else where the cell it's in puts its boxes (`<td
                    // align="right">`: a header's link stood at the left of its cell).
                    align: n.own_table_align().unwrap_or(ctx.around.boxes),
                    background: n.background(),
                    background_image: n.background_image(),
                    spacing: Some(n.cell_spacing()),
                    padding: Some(n.cell_padding()),
                    // A table that sets no width is as wide as what it holds, as in a browser.
                    fit: n.width().is_none(),
                    radius: n.radius(),
                    border: n.table_line(),
                }));
            }
            // A box with others placed over it: its content, and theirs on top.
            Kid::El(n) if depth < MAX_DEPTH && n.has_overlays() => {
                flush(&mut pending, &mut out, ctx);
                let (placed, base): (Vec<&Kid>, Vec<&Kid>) = n.kids.iter().partition(|k| matches!(k, Kid::El(c) if c.placed()));
                let base: Vec<Kid> = base.into_iter().map(copy).collect();
                let overlays: Vec<Overlay> = colored(ctx, n, |ctx| {
                    placed
                        .into_iter()
                        .filter_map(|k| if let Kid::El(c) = k { Some(c) } else { None })
                        .filter(|c| !c.hidden())
                        .take(MAX_OVERLAYS)
                        .map(|c| {
                            let (x, y, from_right, from_bottom) = c.overlay_at();
                            // Its content: its children, or the element itself (a picture).
                            let content = colored(ctx, c, |ctx| {
                                ctx.within(Around::default(), |ctx| {
                                    if c.kids.is_empty() { blocks(&[Kid::El(c.unplaced())], ctx, depth + 1) } else { blocks(&c.kids, ctx, depth + 1) }
                                })
                            });
                            Overlay { x, y, from_right, from_bottom, width: c.width(), background: c.background(), blocks: content }
                        })
                        .filter(|o| !o.blocks.is_empty() || o.background.is_some())
                        .collect()
                });
                ctx.cells += 1;
                let cell = Cell {
                    blocks: colored(ctx, n, |ctx| ctx.within(Around::default(), |ctx| blocks(&base, ctx, depth + 1))),
                    background: n.background(),
                    background_image: n.background_image(),
                    align: n.align(),
                    span: 1,
                    overlays,
                    padding: n.padding(),
                    radius: n.radius(),
                    ..Default::default()
                };
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), ..Default::default() }));
            }
            // A box with a picture behind it (a banner): one cell that carries it.
            Kid::El(n) if depth < MAX_DEPTH && !matches!(n.tag.as_str(), "img" | "td" | "th" | "tr") && n.background_image().is_some() => {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                // Its content is the box itself without the picture, so columns in it stay columns;
                // else what it holds, so a box in it (its own colour, a VML banner's text) stays a box.
                let content = if n.columns().is_some() { vec![Kid::El(n.without_background())] } else { n.kids.iter().map(copy).collect() };
                let cell = Cell {
                    blocks: colored(ctx, n, |ctx| ctx.within(Around::default(), |ctx| blocks(&content, ctx, depth + 1))),
                    background: n.background(),
                    background_image: n.background_image(),
                    align: n.align(),
                    span: 1,
                    padding: n.padding(),
                    radius: n.radius(),
                    ..Default::default()
                };
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), ..Default::default() }));
            }
            // A link drawn as a button (in wrappers that hold nothing else): a box of its own, as
            // wide as its words and room unless it sets a width, as tall as it says at least.
            Kid::El(n) if depth < MAX_DEPTH && ctx.buttons < MAX_BUTTONS && n.only_button(0).is_some() => {
                let Some((b, align)) = n.only_button(0) else { continue };
                flush(&mut pending, &mut out, ctx);
                let width = b.width();
                let block = b.style("display").is_some_and(|d| d == "block");
                let cell = button_cell(b, ctx, depth);
                out.push(Block::Table(Table { rows: vec![vec![cell]], width, align, fit: width.is_none() && !block, ..Default::default() }));
            }
            // Boxes side by side with CSS: a table of one row per line of boxes.
            Kid::El(n) if depth < MAX_DEPTH && n.columns().is_some() => {
                flush(&mut pending, &mut out, ctx);
                let rows: Vec<Vec<Cell>> = colored(ctx, n, |ctx| {
                    n.columns()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|row| {
                            row.into_iter()
                                .map(|c| {
                                    ctx.cells += 1;
                                    // Buttons side by side: each its box, its words still a link.
                                    if c.button() && ctx.buttons < MAX_BUTTONS && ctx.cells <= MAX_CELLS {
                                        return Cell { width: c.width(), valign: c.valign().unwrap_or_default(), ..button_cell(c, ctx, depth) };
                                    }
                                    // A box's own content (its children when it has them, else itself).
                                    let content = if c.kids.is_empty() {
                                        vec![]
                                    } else {
                                        colored(ctx, c, |ctx| ctx.within(Around::default(), |ctx| blocks(&c.kids, ctx, depth + 1)))
                                    };
                                    Cell {
                                        blocks: if ctx.cells > MAX_CELLS { Vec::new() } else { content },
                                        width: c.width(),
                                        borders: c.lines(),
                                        background: c.background(),
                                        align: c.align(),
                                        span: 1,
                                        background_image: c.background_image(),
                                        overlays: Vec::new(),
                                        padding: c.padding(),
                                        valign: c.valign().unwrap_or_default(),
                                        height: c.min_height(),
                                        radius: c.radius(),
                                    }
                                })
                                .filter(held)
                                .collect::<Vec<Cell>>()
                        })
                        .filter(|r| !r.is_empty())
                        .collect()
                });
                if !rows.is_empty() {
                    out.push(Block::Table(Table {
                        rows,
                        width: n.width(),
                        align: n.table_align(),
                        background: n.background(),
                        background_image: n.background_image(),
                        ..Default::default()
                    }));
                }
            }
            // Pictures, or text, centred in a block of their own (a report's charts, a title
            // `text-align: center` in a newsletter's column, issue #41; a logo and a title over
            // a message that otherwise flows, issue #74): a centred cell.
            Kid::El(n)
                if depth < MAX_DEPTH
                    && (matches!(n.tag.as_str(), "div" | "p" | "center") || n.is_title())
                    && (n.tag == "center" || n.align() == Align::Center)
                    && (n.has_picture(0) || n.has_text(0))
                    && (!n.has_block_inside(0) || n.holds_a_title()) =>
            {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                // A title stays one: itself, without what centres it, in the cell.
                let title = n.is_title().then(|| [Kid::El(n.unaligned())]);
                let kids = title.as_ref().map_or(n.kids.as_slice(), |t| t.as_slice());
                let inner = colored(ctx, n, |ctx| ctx.within(Around::default(), |ctx| blocks(kids, ctx, depth + 1)));
                if !inner.is_empty() {
                    out.push(Block::Table(Table {
                        rows: vec![vec![Cell {
                            blocks: inner,
                            align: Align::Center,
                            background: n.background(),
                            span: 1,
                            radius: n.radius(),
                            ..Default::default()
                        }]],
                        width: None,
                        align: Align::Center,
                        background: None,
                        background_image: None,
                        ..Default::default()
                    }));
                }
            }
            // A box of a set width no wider than the room, that draws nothing: what it holds.
            Kid::El(n)
                if n.fixed_width()
                    && ctx.unwrapped < MAX_UNWRAPPED
                    && n.lines().iter().all(Option::is_none)
                    && ctx.adds_nothing(n.background(), n.width())
                    && n.background_image().is_none()
                    && n.radius().is_none()
                    && n.padding().is_none_or(|p| p == [0; 4]) =>
            {
                flush(&mut pending, &mut out, ctx);
                ctx.unwrapped += 1;
                let inner = colored(ctx, n, |ctx| blocks(&n.kids, ctx, depth));
                ctx.unwrapped -= 1;
                out.extend(inner);
            }
            // A column of a set width (`max-width: 600px; margin: 0 auto`): a one-cell table that
            // wide, so it isn't stretched across the window (issue #10). One that holds only a
            // divider between two parts is one too: its line ran the width of the window, past
            // the column it divides, or wasn't drawn.
            Kid::El(n)
                if depth < MAX_DEPTH
                    && n.fixed_width()
                    && (n.has_text(0)
                        || n.has_picture(0)
                        || n.has_table(0)
                        || n.kids.iter().any(|k| matches!(k, Kid::El(c) if c.rule().is_some()))) =>
            {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                let room = n.padding().unwrap_or_default();
                let wide = match n.width() {
                    Some(Width::Px(w)) => Some(ctx.around.room.map_or(w, |r| r.min(w))),
                    _ => None,
                };
                let around = Around {
                    room: wide.map(|w| w.saturating_sub(u32::from(room[1]).saturating_add(u32::from(room[3])))),
                    colour: n.background(),
                    align: n.own_align().unwrap_or(ctx.around.align),
                    boxes: n.boxes_align().unwrap_or(ctx.around.boxes),
                };
                let inner = colored(ctx, n, |ctx| ctx.within(around, |ctx| blocks(&n.kids, ctx, depth + 1)));
                if !inner.is_empty() {
                    let align = if n.tag == "center" { Align::Center } else { n.table_align() };
                    out.push(Block::Table(Table {
                        rows: vec![vec![Cell {
                            blocks: inner,
                            align: n.align(),
                            background: n.background(),
                            span: 1,
                            padding: n.padding(),
                            radius: n.radius(),
                            borders: n.lines(),
                            ..Default::default()
                        }]],
                        width: n.width(),
                        align,
                        background: None,
                        background_image: None,
                        ..Default::default()
                    }));
                }
            }
            // A box that draws something around what it holds (a card, a note, a round dot behind
            // a white check mark, issue #47): one cell that carries it. Only a box of a set width
            // kept its colour, and what was white on it couldn't be seen.
            Kid::El(n) if depth < MAX_DEPTH && n.draws_a_box(ctx.around.colour) && (n.has_text(0) || n.has_picture(0)) => {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                let room = n.padding().unwrap_or_default();
                let wide = match n.width() {
                    Some(Width::Px(w)) => Some(ctx.around.room.map_or(w, |r| r.min(w))),
                    Some(Width::Percent(p)) => ctx.around.room.map(|r| r.saturating_mul(u32::from(p)) / 100),
                    None => ctx.around.room,
                };
                let align = n.own_align().unwrap_or(ctx.around.align);
                let around = Around {
                    room: wide.map(|w| w.saturating_sub(u32::from(room[1]).saturating_add(u32::from(room[3])))),
                    colour: n.background(),
                    align,
                    boxes: n.boxes_align().unwrap_or(ctx.around.boxes),
                };
                let inner = colored(ctx, n, |ctx| ctx.within(around, |ctx| blocks(&n.kids, ctx, depth + 1)));
                let cell = Cell {
                    blocks: inner,
                    align,
                    background: n.background(),
                    span: 1,
                    padding: Some(room),
                    radius: n.radius(),
                    borders: n.lines(),
                    ..Default::default()
                };
                let fit = n.width().is_none() && n.style("display").is_some_and(|d| d == "inline-block");
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), fit, ..Default::default() }));
            }
            // A wrapper around tables (`<div>`, `<center>`): its parts in turn.
            Kid::El(n) if depth < MAX_DEPTH && n.has_table(0) => {
                flush(&mut pending, &mut out, ctx);
                let inner = colored(ctx, n, |ctx| blocks(&n.kids, ctx, depth));
                if n.tag == "center" || n.align() == Align::Center {
                    // Centred: as a one-cell table that sits in the middle.
                    out.push(Block::Table(Table {
                        rows: vec![vec![Cell { blocks: inner, align: Align::Center, span: 1, ..Default::default() }]],
                        width: n.width(),
                        align: Align::Center,
                        background: n.background(),
                        background_image: None,
                        ..Default::default()
                    }));
                } else {
                    out.extend(inner);
                }
            }
            // A table past what's laid out: read as blocks, one cell after another. A text table
            // here, of box-drawing lines in a monospace font, is no part of a layout.
            Kid::El(n) if n.tag == "table" || n.has_table(0) => write_flat(k, &mut pending, ctx.size, ctx.bold),
            k => {
                if matches!(k, Kid::El(n) if parted(n, &pending)) {
                    pending.push_str("<br>");
                }
                write_shown(k, &mut pending, ctx.size, ctx.bold);
            }
        }
    }
    flush(&mut pending, &mut out, ctx);
    out
}

/// A divider: a line along the top of a box that holds nothing.
fn rule(line: Line, width: Option<Width>) -> Block {
    let mut borders = [None; 4];
    borders[0] = Some(line);
    let cell = Cell { span: 1, padding: Some([0; 4]), height: Some(u16::from(line.width)), borders, ..Default::default() };
    Block::Table(Table { rows: vec![vec![cell]], width, align: Align::Center, ..Default::default() })
}

/// The line of a block that's a divider.
fn rule_of(b: &Block) -> Option<Line> {
    let Block::Table(t) = b else { return None };
    let [row] = t.rows.as_slice() else { return None };
    let [c] = row.as_slice() else { return None };
    if !c.blocks.is_empty() || c.background.is_some() {
        return None;
    }
    c.borders.first().copied().flatten().filter(|l| c.height == Some(u16::from(l.width)))
}

/// A link drawn as a button: one cell of its colour, with its room, corners and height, its words
/// (the link, without the box's own styles) in the middle of it.
fn button_cell(b: &Node, ctx: &mut Ctx, depth: usize) -> Cell {
    ctx.buttons += 1;
    let link = b.as_link();
    // Its words without the message's style sheets: their rules are in each element's style
    // already ([`inline_css`]), and read again for each of a newsletter's buttons they made it
    // a third slower to read.
    let sheets = std::mem::take(&mut ctx.css);
    let words = colored(ctx, b, |ctx| blocks(&[Kid::El(link)], ctx, depth + 1));
    ctx.css = sheets;
    Cell {
        blocks: words,
        background: b.background(),
        align: b.align(),
        span: 1,
        padding: Some(b.padding().unwrap_or_default()),
        valign: VAlign::Middle,
        height: b.button_height(),
        radius: b.radius(),
        ..Default::default()
    }
}

/// The width of a cell that holds a picture of a set width and nothing else (a business card's
/// photo beside its details): the picture's and a little room, as a browser fits the column to
/// it, rather than half the table.
fn picture_width(blocks: &[Block]) -> Option<Width> {
    let [Block::Text(lines)] = blocks else { return None };
    let mut widest = 0;
    for sp in lines.iter().flatten() {
        match (&sp.image, sp.image_size.and_then(|z| z.width)) {
            (Some(_), Some(Width::Px(w))) => widest = widest.max(w),
            (Some(_), _) => return None,
            (None, _) if !sp.text.trim().is_empty() => return None,
            (None, _) => {}
        }
    }
    (widest > 0).then(|| Width::Px(widest.saturating_add(16)))
}

/// Whether any table puts content side by side, or a picture behind it, or boxes over it, or is
/// a set number of pixels wide (a centred 600-pixel column), or a box of colour has round corners
/// (a button), or centres what it holds (a logo, a title): a layout worth keeping (otherwise the
/// text flows, the width of the window, from its start).
pub(super) fn has_columns(b: &[Block]) -> bool {
    b.iter().any(|b| match b {
        Block::Text(_) => false,
        Block::Table(t) => {
            matches!(t.width, Some(Width::Px(_)))
                || t.rows.iter().flatten().any(|c| c.align == Align::Center && !c.blocks.is_empty())
                || t.background_image.is_some()
                || t.rows.iter().any(|r| r.iter().filter(|c| !c.blocks.is_empty()).count() > 1)
                || (t.radius.is_some() && t.background.is_some())
                || t.rows.iter().flatten().any(|c| {
                    c.background_image.is_some() || !c.overlays.is_empty() || (c.radius.is_some() && c.background.is_some()) || has_columns(&c.blocks)
                })
        }
    })
}

/// The pictures behind boxes in a layout (to be shown, or held back, like the message's others).
pub fn background_pictures(b: &[Block]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |src: &Option<String>, out: &mut Vec<String>| {
        if let Some(s) = src
            && !out.contains(s)
            && out.len() < 200
        {
            out.push(s.clone());
        }
    };
    pub(super) fn walk(b: &[Block], out: &mut Vec<String>, add: &mut dyn FnMut(&Option<String>, &mut Vec<String>), depth: usize) {
        if depth > MAX_DEPTH * 2 {
            return;
        }
        for x in b {
            if let Block::Table(t) = x {
                add(&t.background_image, out);
                for c in t.rows.iter().flatten() {
                    add(&c.background_image, out);
                    walk(&c.blocks, out, add, depth + 1);
                    for o in &c.overlays {
                        walk(&o.blocks, out, add, depth + 1);
                    }
                }
            }
        }
    }
    walk(b, &mut out, &mut add, 0);
    out
}
