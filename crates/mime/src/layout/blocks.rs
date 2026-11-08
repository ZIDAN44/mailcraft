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
}

/// `f` with the text colour `n` sets, if it sets one, as CSS inherits it.
pub(super) fn colored<R>(ctx: &mut Ctx, n: &Node, f: impl FnOnce(&mut Ctx) -> R) -> R {
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
                            row.into_iter()
                                .map(|c| {
                                    ctx.cells += 1;
                                    let content =
                                        if ctx.cells > MAX_CELLS { Vec::new() } else { colored(ctx, c, |ctx| blocks(&c.kids, ctx, depth + 1)) };
                                    Cell {
                                        width: c.width().or_else(|| picture_width(&content)),
                                        blocks: content,
                                        background: c.background().or(row_bg),
                                        align: c.align(),
                                        span: c.attr("colspan").and_then(|s| s.trim().parse::<u16>().ok()).unwrap_or(1).clamp(1, 50),
                                        background_image: c.background_image(),
                                        overlays: Vec::new(),
                                        padding: c.padding(),
                                        valign: c.valign().or(row_valign).unwrap_or(VAlign::Middle),
                                        height: c.min_height().max(row_height),
                                        radius: c.radius(),
                                    }
                                })
                                .filter(|c| !c.blocks.is_empty() || c.background.is_some() || c.background_image.is_some())
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
                    align: n.table_align(),
                    background: n.background(),
                    background_image: n.background_image(),
                    spacing: Some(n.cell_spacing()),
                    padding: Some(n.cell_padding()),
                    // A table that sets no width is as wide as what it holds, as in a browser.
                    fit: n.width().is_none(),
                    radius: n.radius(),
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
                                if c.kids.is_empty() { blocks(&[Kid::El(c.unplaced())], ctx, depth + 1) } else { blocks(&c.kids, ctx, depth + 1) }
                            });
                            Overlay { x, y, from_right, from_bottom, width: c.width(), background: c.background(), blocks: content }
                        })
                        .filter(|o| !o.blocks.is_empty() || o.background.is_some())
                        .collect()
                });
                ctx.cells += 1;
                let cell = Cell {
                    blocks: colored(ctx, n, |ctx| blocks(&base, ctx, depth + 1)),
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
                    blocks: colored(ctx, n, |ctx| blocks(&content, ctx, depth + 1)),
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
                                    let content = if c.kids.is_empty() { vec![] } else { colored(ctx, c, |ctx| blocks(&c.kids, ctx, depth + 1)) };
                                    Cell {
                                        blocks: if ctx.cells > MAX_CELLS { Vec::new() } else { content },
                                        width: c.width(),
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
                                .filter(|c| !c.blocks.is_empty() || c.background.is_some() || c.background_image.is_some())
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
            // `text-align: center` in a newsletter's column, issue #41): a centred cell. Laid-out
            // mail only: text that flows has no alignment of its own.
            Kid::El(n)
                if depth < MAX_DEPTH
                    && matches!(n.tag.as_str(), "div" | "p" | "center")
                    && (n.tag == "center" || n.align() == Align::Center)
                    && (n.has_picture(0) || (depth > 0 && n.has_text(0)))
                    && (!n.has_block_inside(0) || n.holds_a_title()) =>
            {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                let inner = colored(ctx, n, |ctx| blocks(&n.kids, ctx, depth + 1));
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
            // A column of a set width (`max-width: 600px; margin: 0 auto`): a one-cell table that
            // wide, so it isn't stretched across the window (issue #10).
            Kid::El(n) if depth < MAX_DEPTH && n.fixed_width() && (n.has_text(0) || n.has_picture(0)) => {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                let inner = colored(ctx, n, |ctx| blocks(&n.kids, ctx, depth + 1));
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
            k => write_shown(k, &mut pending),
        }
    }
    flush(&mut pending, &mut out, ctx);
    out
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
/// (a button): a layout worth keeping (otherwise the text flows, the width of the window).
pub(super) fn has_columns(b: &[Block]) -> bool {
    b.iter().any(|b| match b {
        Block::Text(_) => false,
        Block::Table(t) => {
            matches!(t.width, Some(Width::Px(_)))
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
