//! The layout of HTML mail built from tables: newsletters and receipts put their content in
//! columns with layout tables (a centred 600-pixel table, rows of two or three cells, coloured
//! bands). [`html_layout`] keeps that structure as [`Block`]s: tables with rows and cells (their
//! widths, colours and alignment) around formatted text, which is [`crate::html_to_rich`]'s.
//! Mail without tables of more than one column has no layout: it flows as text. Background
//! pictures (a banner behind a heading) and boxes positioned over others (`position: absolute`
//! inside `position: relative`, a label on a picture) are kept as well.
//!
//! The HTML is read with html5ever's tokenizer into a small tree of its own (a layout needs no
//! more than elements, attributes and text); like everything from mail it is hostile input, so
//! sizes, depth and counts are capped and nothing here panics.

use std::cell::RefCell;

use html5ever::TokenizerResult;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use serde::{Deserialize, Serialize};

use crate::RichText;

/// Tables nested deeper than this are flattened into text.
const MAX_DEPTH: usize = 12;
/// More cells than this in a message: no layout (it flows as text).
const MAX_CELLS: usize = 1_500;
/// Elements kept open at once while reading.
const MAX_OPEN: usize = 256;
/// Elements read at most.
const MAX_NODES: usize = 200_000;
/// Boxes placed over one container, at most.
const MAX_OVERLAYS: usize = 16;
/// How far a placed box may be moved, in pixels.
const MAX_OFFSET: i32 = 4_000;

/// A part of a laid-out message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    /// Formatted lines.
    Text(RichText),
    Table(Table),
}

/// How wide something wants to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Width {
    Px(u32),
    Percent(u8),
}

/// Where content sits across its space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub width: Option<Width>,
    /// Where the table sits (`align`, or `margin: 0 auto`).
    pub align: Align,
    pub background: Option<[u8; 3]>,
    /// A picture behind it (`background`, `background-image: url(…)`), covering it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub blocks: Vec<Block>,
    pub width: Option<Width>,
    pub background: Option<[u8; 3]>,
    /// How its content lines up.
    pub align: Align,
    /// Columns it spans.
    pub span: u16,
    /// A picture behind its content, covering the cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// Boxes placed over the content (`position: absolute` in a `position: relative` box).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
}

/// A box placed over its container: `x` pixels from its left edge (from the right when
/// `from_right`), `y` from its top (from the bottom when `from_bottom`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay {
    pub x: i32,
    pub y: i32,
    pub from_right: bool,
    pub from_bottom: bool,
    pub width: Option<Width>,
    pub background: Option<[u8; 3]>,
    pub blocks: Vec<Block>,
}

#[derive(Default)]
struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    kids: Vec<Kid>,
}

enum Kid {
    El(Node),
    Text(String),
}

/// Elements that never have content.
fn void(tag: &str) -> bool {
    matches!(tag, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "source" | "track" | "wbr")
}

/// Builds the tree from tokens.
struct Sink {
    stack: RefCell<Vec<Node>>,
    nodes: RefCell<usize>,
}

impl Sink {
    fn close_to(stack: &mut Vec<Node>, depth: usize) {
        while stack.len() > depth.max(1) {
            if let Some(n) = stack.pop()
                && let Some(parent) = stack.last_mut()
            {
                parent.kids.push(Kid::El(n));
            }
        }
    }

    /// Close the open elements down to (and including) the nearest `tag`, stopping at `stop`.
    fn close(stack: &mut Vec<Node>, tags: &[&str], stop: &[&str]) {
        let found = stack.iter().rposition(|n| tags.contains(&n.tag.as_str()));
        let barrier = stack.iter().rposition(|n| stop.contains(&n.tag.as_str()));
        if let Some(i) = found
            && barrier.is_none_or(|b| b < i)
        {
            Sink::close_to(stack, i);
        }
    }
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut stack = self.stack.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.to_string().to_ascii_lowercase();
                match tag.kind {
                    TagKind::StartTag => {
                        let mut count = self.nodes.borrow_mut();
                        *count += 1;
                        if *count > MAX_NODES {
                            return TokenSinkResult::Continue;
                        }
                        // What a new cell, row or table part implies: the open one ends.
                        match name.as_str() {
                            "td" | "th" => Sink::close(&mut stack, &["td", "th"], &["tr", "table"]),
                            "tr" => Sink::close(&mut stack, &["tr"], &["table"]),
                            "tbody" | "thead" | "tfoot" => Sink::close(&mut stack, &["tbody", "thead", "tfoot"], &["table"]),
                            "p" | "li" => Sink::close(&mut stack, &[name.as_str()], &["td", "th", "table", "ul", "ol", "div"]),
                            _ => {}
                        }
                        let node = Node {
                            tag: name.clone(),
                            attrs: tag.attrs.iter().map(|a| (a.name.local.to_string().to_ascii_lowercase(), a.value.to_string())).collect(),
                            kids: Vec::new(),
                        };
                        if void(&name) || tag.self_closing {
                            if let Some(parent) = stack.last_mut() {
                                parent.kids.push(Kid::El(node));
                            }
                        } else if stack.len() < MAX_OPEN {
                            stack.push(node);
                        }
                        // Their content is text, not markup.
                        return match name.as_str() {
                            "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed" | "noframes" => {
                                TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rawtext)
                            }
                            "script" => TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::ScriptData),
                            _ => TokenSinkResult::Continue,
                        };
                    }
                    TagKind::EndTag => {
                        let stop: &[&str] = match name.as_str() {
                            "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" => &["table"],
                            "table" => &[],
                            _ => &["td", "th", "table"],
                        };
                        Sink::close(&mut stack, &[name.as_str()], stop);
                    }
                }
            }
            Token::CharacterTokens(t) => {
                if let Some(top) = stack.last_mut() {
                    match top.kids.last_mut() {
                        Some(Kid::Text(s)) => s.push_str(&t),
                        _ => top.kids.push(Kid::Text(t.to_string())),
                    }
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn parse(html: &str) -> Node {
    let sink = Sink { stack: RefCell::new(vec![Node { tag: "#root".into(), ..Default::default() }]), nodes: RefCell::new(0) };
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(html));
    while let TokenizerResult::Script(()) = tok.feed(&queue) {}
    tok.end();
    let mut stack = tok.sink.stack.take();
    Sink::close_to(&mut stack, 1);
    stack.pop().unwrap_or_default()
}

impl Node {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// A property of the inline `style` (lower case, spaces removed from the value's ends).
    fn style(&self, prop: &str) -> Option<String> {
        self.style_as_written(prop).map(|v| v.to_ascii_lowercase())
    }

    /// A property of the inline `style` as written (addresses keep their case).
    fn style_as_written(&self, prop: &str) -> Option<String> {
        let style = self.attr("style")?;
        style.split(';').find_map(|d| {
            let (k, v) = d.split_once(':')?;
            k.trim().eq_ignore_ascii_case(prop).then(|| {
                let v = v.trim();
                let v = v.get(..v.to_ascii_lowercase().find("!important").unwrap_or(v.len())).unwrap_or(v);
                v.trim().to_string()
            })
        })
    }

    /// The picture behind it: the `background` attribute, or `url(…)` in `background-image` or
    /// `background`. Web, `cid:` and `data:` addresses only (nothing that runs).
    fn background_image(&self) -> Option<String> {
        let from_css = || {
            let v = self.style_as_written("background-image").or_else(|| self.style_as_written("background"))?;
            let at = v.to_ascii_lowercase().find("url(")?;
            let rest = v.get(at + 4..)?;
            let inner = rest.get(..rest.find(')')?)?.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            Some(inner.to_string())
        };
        let src = self.attr("background").map(|b| b.trim().to_string()).filter(|b| !b.is_empty()).or_else(from_css)?;
        let lower = src.to_ascii_lowercase();
        (src.len() < 4096 && (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("cid:"))).then_some(src)
    }

    /// `position: absolute` (or `fixed`, which a message can't fix to anything but its box).
    fn placed(&self) -> bool {
        self.style("position").is_some_and(|p| p == "absolute" || p == "fixed")
    }

    /// A length in pixels (`12px`, `12`); `None` for anything else (`auto`, `50%`).
    fn px(&self, prop: &str) -> Option<i32> {
        let v = self.style(prop)?;
        let n = v.trim().trim_end_matches("px").trim().parse::<f32>().ok()?;
        Some((n as i32).clamp(-MAX_OFFSET, MAX_OFFSET))
    }

    /// Where a placed box goes over its container.
    fn overlay_at(&self) -> (i32, i32, bool, bool) {
        let (left, right, top, bottom) = (self.px("left"), self.px("right"), self.px("top"), self.px("bottom"));
        let (x, from_right) = match (left, right) {
            (None, Some(r)) => (r, true),
            (l, _) => (l.unwrap_or(0), false),
        };
        let (y, from_bottom) = match (top, bottom) {
            (None, Some(b)) => (b, true),
            (t, _) => (t.unwrap_or(0), false),
        };
        (x.max(0), y.max(0), from_right, from_bottom)
    }

    /// A `position: relative` box with boxes placed over it.
    fn has_overlays(&self) -> bool {
        self.style("position").is_some_and(|p| p == "relative") && self.kids.iter().any(|k| matches!(k, Kid::El(n) if n.placed() && !n.hidden()))
    }

    fn hidden(&self) -> bool {
        self.attr("hidden").is_some()
            || self.style("display").is_some_and(|d| d == "none")
            || self.style("mso-hide").is_some_and(|d| d == "all")
            || matches!(self.tag.as_str(), "head" | "script" | "style" | "title" | "template" | "noscript")
    }

    fn width(&self) -> Option<Width> {
        let v = self.style("width").or_else(|| self.attr("width").map(str::to_string))?;
        let v = v.trim();
        if let Some(p) = v.strip_suffix('%') {
            return p.trim().parse::<f32>().ok().filter(|p| *p > 0.0).map(|p| Width::Percent(p.min(100.0) as u8));
        }
        let px = v.trim_end_matches("px").trim().parse::<f32>().ok()?;
        (px >= 1.0).then(|| Width::Px(px.min(4000.0) as u32))
    }

    fn background(&self) -> Option<[u8; 3]> {
        let v = self.style("background-color").or_else(|| self.style("background")).or_else(|| self.attr("bgcolor").map(str::to_ascii_lowercase))?;
        color(v.split_whitespace().next()?)
    }

    fn align(&self) -> Align {
        let v = self.attr("align").map(str::to_ascii_lowercase).or_else(|| self.style("text-align")).unwrap_or_default();
        match v.trim() {
            "center" | "middle" => Align::Center,
            "right" | "end" => Align::Right,
            _ => Align::Left,
        }
    }

    /// Where a table sits: `align`, or auto side margins.
    fn table_align(&self) -> Align {
        let margin = self.style("margin").unwrap_or_default();
        if margin.contains("auto") || self.style("margin-left").is_some_and(|m| m == "auto") {
            return Align::Center;
        }
        self.align()
    }

    /// Whether a table or side-by-side boxes are somewhere inside.
    fn has_table(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && !self.hidden()
            && self.kids.iter().any(|k| match k {
                Kid::El(n) => n.tag == "table" || n.columns().is_some() || n.has_table(depth + 1),
                Kid::Text(_) => false,
            })
    }

    /// A box that sits beside its neighbours: `inline-block`, a table cell, or floated.
    fn beside(&self) -> bool {
        self.style("display").is_some_and(|d| matches!(d.as_str(), "inline-block" | "table-cell" | "inline-flex" | "inline-table"))
            || self.style("float").is_some_and(|f| f == "left" || f == "right")
    }

    /// Boxes laid out side by side with CSS alone (flexbox in a row, a grid, or `inline-block` /
    /// floated / table-cell children), as rows of boxes; `None` when they stack.
    fn columns(&self) -> Option<Vec<Vec<&Node>>> {
        let kids: Vec<&Node> = self.kids.iter().filter_map(|k| if let Kid::El(n) = k { Some(n) } else { None }).filter(|n| !n.hidden()).collect();
        if kids.len() < 2 || matches!(self.tag.as_str(), "table" | "tr" | "tbody" | "thead" | "tfoot" | "ul" | "ol" | "p") {
            return None;
        }
        // Loose text between the boxes means they are inline in a paragraph.
        if self.kids.iter().any(|k| matches!(k, Kid::Text(t) if !t.trim().is_empty())) {
            return None;
        }
        let display = self.style("display").unwrap_or_default();
        if display == "flex" || display == "inline-flex" {
            let dir = self.style("flex-direction").unwrap_or_default();
            let wraps = self.style("flex-wrap").is_some_and(|w| w.starts_with("wrap"));
            return (!dir.starts_with("column") && !wraps).then(|| vec![kids]);
        }
        if display == "grid" || display == "inline-grid" {
            let template = self.style("grid-template-columns").unwrap_or_default();
            let tracks = if let Some(rest) = template.strip_prefix("repeat(") {
                rest.split(',').next().and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(1)
            } else {
                template.split_whitespace().count()
            }
            .clamp(1, 12);
            return (tracks > 1).then(|| kids.chunks(tracks).map(<[&Node]>::to_vec).collect());
        }
        kids.iter().all(|k| k.beside()).then(|| vec![kids])
    }

    /// Rows (through table sections), each its cells.
    fn rows(&self) -> Vec<Vec<&Node>> {
        let mut out = Vec::new();
        for k in &self.kids {
            let Kid::El(n) = k else { continue };
            match n.tag.as_str() {
                "tr" => out.push(
                    n.kids
                        .iter()
                        .filter_map(|k| if let Kid::El(c) = k { Some(c) } else { None })
                        .filter(|c| matches!(c.tag.as_str(), "td" | "th") && !c.hidden())
                        .collect(),
                ),
                "tbody" | "thead" | "tfoot" => out.extend(n.rows()),
                _ => {}
            }
        }
        out
    }
}

/// `#rgb`, `#rrggbb`, `rgb(r, g, b)` or a few colour names.
fn color(v: &str) -> Option<[u8; 3]> {
    let v = v.trim().trim_end_matches(';');
    if let Some(hex) = v.strip_prefix('#') {
        let n = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let c: Vec<u8> = hex.chars().filter_map(|c| c.to_digit(16)).map(|d| u8::try_from(d * 17).unwrap_or(0)).collect();
                Some([*c.first()?, *c.get(1)?, *c.get(2)?])
            }
            6 => Some([n(hex.get(0..2)?)?, n(hex.get(2..4)?)?, n(hex.get(4..6)?)?]),
            _ => None,
        };
    }
    if let Some(args) = v.strip_prefix("rgb(").or_else(|| v.strip_prefix("rgba(")) {
        let parts: Vec<u8> =
            args.trim_end_matches(')').split(',').take(3).filter_map(|p| p.trim().parse::<f32>().ok()).map(|f| f.clamp(0.0, 255.0) as u8).collect();
        return Some([*parts.first()?, *parts.get(1)?, *parts.get(2)?]);
    }
    match v {
        "white" => Some([255, 255, 255]),
        "black" => Some([0, 0, 0]),
        _ => None,
    }
}

/// Markup for a part of the tree (to hand to the text converter).
fn write(k: &Kid, out: &mut String) {
    match k {
        Kid::Text(t) => {
            for c in t.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    c => out.push(c),
                }
            }
        }
        Kid::El(n) => {
            if n.tag.starts_with('#') {
                for k in &n.kids {
                    write(k, out);
                }
                return;
            }
            out.push('<');
            out.push_str(&n.tag);
            for (k, v) in &n.attrs {
                if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':') {
                    continue;
                }
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&v.replace('&', "&amp;").replace('"', "&quot;"));
                out.push('"');
            }
            out.push('>');
            if !void(&n.tag) {
                for k in &n.kids {
                    write(k, out);
                }
                out.push_str("</");
                out.push_str(&n.tag);
                out.push('>');
            }
        }
    }
}

struct Ctx {
    /// The message's `<style>` blocks, given to each piece of text (for colours and hidden parts).
    css: String,
    cells: usize,
    /// The text colour the boxes around inherit to the text inside (a cell's `color`).
    color: Option<String>,
}

/// `f` with the text colour `n` sets, if it sets one, as CSS inherits it.
fn colored<R>(ctx: &mut Ctx, n: &Node, f: impl FnOnce(&mut Ctx) -> R) -> R {
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

fn blocks(kids: &[Kid], ctx: &mut Ctx, depth: usize) -> Vec<Block> {
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
                        .map(|row| {
                            row.into_iter()
                                .map(|c| {
                                    ctx.cells += 1;
                                    Cell {
                                        blocks: if ctx.cells > MAX_CELLS {
                                            Vec::new()
                                        } else {
                                            colored(ctx, c, |ctx| blocks(&c.kids, ctx, depth + 1))
                                        },
                                        width: c.width(),
                                        background: c.background(),
                                        align: c.align(),
                                        span: c.attr("colspan").and_then(|s| s.trim().parse::<u16>().ok()).unwrap_or(1).clamp(1, 50),
                                        background_image: c.background_image(),
                                        overlays: Vec::new(),
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
                    ..Default::default()
                };
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), ..Default::default() }));
            }
            // A box with a picture behind it (a banner): one cell that carries it.
            Kid::El(n) if depth < MAX_DEPTH && !matches!(n.tag.as_str(), "img" | "td" | "th" | "tr") && n.background_image().is_some() => {
                flush(&mut pending, &mut out, ctx);
                ctx.cells += 1;
                // Its content is the box itself without the picture, so columns in it stay columns.
                let cell = Cell {
                    blocks: colored(ctx, n, |ctx| blocks(&[Kid::El(n.without_background())], ctx, depth + 1)),
                    background: n.background(),
                    background_image: n.background_image(),
                    align: n.align(),
                    span: 1,
                    ..Default::default()
                };
                out.push(Block::Table(Table { rows: vec![vec![cell]], width: n.width(), align: n.table_align(), ..Default::default() }));
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
                    }));
                } else {
                    out.extend(inner);
                }
            }
            k => write(k, &mut pending),
        }
    }
    flush(&mut pending, &mut out, ctx);
    out
}

/// Whether any table puts content side by side, or a picture behind it, or boxes over it: a
/// layout worth keeping (otherwise the text flows).
fn has_columns(b: &[Block]) -> bool {
    b.iter().any(|b| match b {
        Block::Text(_) => false,
        Block::Table(t) => {
            t.background_image.is_some()
                || t.rows.iter().any(|r| r.iter().filter(|c| !c.blocks.is_empty()).count() > 1)
                || t.rows.iter().flatten().any(|c| c.background_image.is_some() || !c.overlays.is_empty() || has_columns(&c.blocks))
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
    fn walk(b: &[Block], out: &mut Vec<String>, add: &mut dyn FnMut(&Option<String>, &mut Vec<String>), depth: usize) {
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

/// A shallow copy of a part of the tree (its children too).
fn copy(k: &Kid) -> Kid {
    match k {
        Kid::Text(t) => Kid::Text(t.clone()),
        Kid::El(n) => Kid::El(Node { tag: n.tag.clone(), attrs: n.attrs.clone(), kids: n.kids.iter().map(copy).collect() }),
    }
}

impl Node {
    /// The element without its background (the cell around it carries that).
    fn without_background(&self) -> Node {
        let attrs = self
            .attrs
            .iter()
            .filter(|(k, _)| k != "background" && k != "bgcolor")
            .map(|(k, v)| {
                if k != "style" {
                    return (k.clone(), v.clone());
                }
                let kept: Vec<&str> = v
                    .split(';')
                    .filter(|d| d.split_once(':').is_none_or(|(p, _)| !p.trim().to_ascii_lowercase().starts_with("background")))
                    .collect();
                (k.clone(), kept.join(";"))
            })
            .collect();
        Node { tag: self.tag.clone(), attrs, kids: self.kids.iter().map(copy).collect() }
    }

    /// The element without its placement (to draw it inside the box placed over another).
    fn unplaced(&self) -> Node {
        let attrs = self.attrs.iter().filter(|(k, _)| k != "style").cloned().collect();
        Node { tag: self.tag.clone(), attrs, kids: self.kids.iter().map(copy).collect() }
    }
}

/// The layout of an HTML message: empty when it has no columns (the text flows instead).
pub fn html_layout(html: &str) -> Vec<Block> {
    let lower = html.to_ascii_lowercase();
    if !(lower.contains("<table")
        || lower.contains("flex")
        || lower.contains("grid")
        || lower.contains("inline-block")
        || lower.contains("float")
        || lower.contains("background")
        || lower.contains("absolute"))
    {
        return Vec::new();
    }
    let html = match html.char_indices().nth(crate::HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let built = std::panic::catch_unwind(|| {
        let root = parse(html);
        let mut css = String::new();
        collect_css(&root, &mut css, 0);
        let mut ctx = Ctx { css, cells: 0, color: None };
        let body = find(&root, "body", 0).unwrap_or(&root);
        let out = blocks(&body.kids, &mut ctx, 0);
        if ctx.cells > MAX_CELLS { Vec::new() } else { out }
    });
    match built {
        Ok(b) if has_columns(&b) => b,
        _ => Vec::new(),
    }
}

fn find<'a>(n: &'a Node, tag: &str, depth: usize) -> Option<&'a Node> {
    if n.tag == tag {
        return Some(n);
    }
    if depth > 8 {
        return None;
    }
    n.kids.iter().find_map(|k| if let Kid::El(e) = k { find(e, tag, depth + 1) } else { None })
}

fn collect_css(n: &Node, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH * 4 || out.len() > 256 * 1024 {
        return;
    }
    if n.tag == "style" {
        out.push_str("<style>");
        for k in &n.kids {
            if let Kid::Text(t) = k {
                out.push_str(&t.replace("</", ""));
            }
        }
        out.push_str("</style>");
        return;
    }
    for k in &n.kids {
        if let Kid::El(e) = k {
            collect_css(e, out, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &Block) -> String {
        match b {
            Block::Text(r) => r.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n"),
            Block::Table(_) => String::new(),
        }
    }

    #[test]
    fn a_newsletter_keeps_its_columns() {
        let html = r##"<html><head><style>.hide{display:none}</style></head><body>
            <div style="display:none">Preheader text</div>
            <center><table width="600" align="center" bgcolor="#f4f4f4"><tr><td align="center"><h1>Studio news</h1></td></tr>
            <tr><td><table width="100%"><tr>
                <td width="50%" style="background-color: #ffeedd"><p>Left <b>column</b></p><img src="cid:a@x" alt="poster"></td>
                <td width="50%"><p>Right column</p><a href="https://example.com/x">Read more</a></td>
            </tr></table></td></tr></table></center>
            <p class="hide">Hidden by a rule</p><p>Footer</p></body></html>"##;
        let b = html_layout(html);
        assert!(!b.is_empty());
        // Centre wrapper → table → second row's cell → the two columns.
        let Block::Table(center) = &b[0] else { panic!("{b:?}") };
        let Block::Table(outer) = &center.rows[0][0].blocks[0] else { panic!() };
        assert_eq!(outer.width, Some(Width::Px(600)));
        assert_eq!(outer.align, Align::Center);
        assert_eq!(outer.background, Some([0xf4, 0xf4, 0xf4]));
        assert_eq!(outer.rows[0][0].align, Align::Center);
        assert!(text(&outer.rows[0][0].blocks[0]).contains("Studio news"));
        let Block::Table(cols) = &outer.rows[1][0].blocks[0] else { panic!() };
        let row = &cols.rows[0];
        assert_eq!(row.len(), 2);
        assert_eq!(row[0].width, Some(Width::Percent(50)));
        assert_eq!(row[0].background, Some([0xff, 0xee, 0xdd]));
        let Block::Text(left) = &row[0].blocks[0] else { panic!() };
        assert!(left.iter().flatten().any(|s| s.bold && s.text.contains("column")));
        assert!(left.iter().flatten().any(|s| s.image.as_deref() == Some("cid:a@x")));
        let Block::Text(right) = &row[1].blocks[0] else { panic!() };
        assert!(right.iter().flatten().any(|s| s.link.as_deref() == Some("https://example.com/x")));
        // Hidden parts stay hidden; text after the table follows it.
        let all = format!("{b:?}");
        assert!(!all.contains("Preheader") && !all.contains("Hidden by a rule"), "{all}");
        assert!(text(b.last().unwrap()).contains("Footer"));
    }

    #[test]
    fn css_columns_are_columns_too() {
        // Flexbox, a grid and inline-block boxes, no tables.
        let flex = html_layout(
            r#"<div style="display:flex"><div style="width:30%"><p>Left</p></div><div style="background-color:#eee"><p>Right</p></div></div>"#,
        );
        let Block::Table(t) = &flex[0] else { panic!("{flex:?}") };
        assert_eq!(t.rows[0].len(), 2);
        assert_eq!(t.rows[0][0].width, Some(Width::Percent(30)));
        assert_eq!(t.rows[0][1].background, Some([0xee, 0xee, 0xee]));
        let grid = html_layout(r#"<div style="display: grid; grid-template-columns: repeat(2, 1fr)"><div>a</div><div>b</div><div>c</div></div>"#);
        let Block::Table(t) = &grid[0] else { panic!("{grid:?}") };
        assert_eq!(t.rows.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        let inline = html_layout(
            r#"<div><div style="display:inline-block;width:280px">One</div> <div style="display:inline-block;width:280px">Two</div></div>"#,
        );
        assert!(matches!(&inline[0], Block::Table(t) if t.rows[0].len() == 2), "{inline:?}");
        let floats = html_layout(r#"<div><div style="float:left">One</div><div style="float:right">Two</div></div>"#);
        assert!(!floats.is_empty());
        // Stacked flexbox, inline text with spans: no columns.
        assert!(html_layout(r#"<div style="display:flex;flex-direction:column"><div>a</div><div>b</div></div>"#).is_empty());
        assert!(
            html_layout(r#"<p>Hello <span style="display:inline-block">a</span> and <span style="display:inline-block">b</span></p>"#).is_empty()
        );
    }

    #[test]
    fn pictures_behind_boxes_are_kept() {
        // A banner: a cell with a picture behind its heading (the address keeps its case).
        let banner = html_layout(
            r##"<table width="600"><tr><td bgcolor="#2e7d6b" background="https://img.example/Hero.JPG" style="color:#fff"><h1>Open call</h1></td></tr></table>"##,
        );
        let Block::Table(t) = &banner[0] else { panic!("{banner:?}") };
        assert_eq!(t.rows[0][0].background_image.as_deref(), Some("https://img.example/Hero.JPG"));
        assert_eq!(t.rows[0][0].background, Some([0x2e, 0x7d, 0x6b]), "the colour stays, for when the picture isn't shown");
        // CSS forms, on a `<div>`: `background-image` and `background` with `url()`.
        for css in ["background-image: url('cid:Hero@x')", "background: #000 url(cid:Hero@x) no-repeat center / cover"] {
            let b = html_layout(&format!(r#"<div style="{css}"><p>Over the picture</p></div>"#));
            let Block::Table(t) = &b[0] else { panic!("{css}: {b:?}") };
            assert_eq!(t.rows[0][0].background_image.as_deref(), Some("cid:Hero@x"), "{css}");
            assert!(text(&t.rows[0][0].blocks[0]).contains("Over the picture"));
            assert_eq!(background_pictures(&b), ["cid:Hero@x"]);
        }
        // Columns inside a box with a picture behind it stay columns.
        let cols = html_layout(r#"<div style="display:flex;background-image:url(cid:b@x)"><div>One</div><div>Two</div></div>"#);
        let Block::Table(t) = &cols[0] else { panic!("{cols:?}") };
        assert!(matches!(&t.rows[0][0].blocks[0], Block::Table(inner) if inner.rows[0].len() == 2), "{cols:?}");
        // Nothing that runs, and nothing local.
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x"] {
            assert!(html_layout(&format!(r#"<div style="background-image:url({bad})"><p>x</p></div>"#)).is_empty(), "{bad}");
        }
    }

    #[test]
    fn text_takes_its_cells_colour() {
        // White on a dark band, set on the cell (or the table), as CSS inherits it.
        let colour_of = |b: &[Block], word: &str| {
            format!("{b:?}").contains(word).then(|| {
                fn spans(b: &[Block], out: &mut Vec<(String, Option<[u8; 3]>)>) {
                    for x in b {
                        match x {
                            Block::Text(r) => out.extend(r.iter().flatten().map(|s| (s.text.clone(), s.color))),
                            Block::Table(t) => t.rows.iter().flatten().for_each(|c| spans(&c.blocks, out)),
                        }
                    }
                }
                let mut v = Vec::new();
                spans(b, &mut v);
                v.into_iter().find(|(t, _)| t.contains(word)).and_then(|(_, c)| c)
            })
        };
        let cell = html_layout(r##"<table><tr><td style="color:#ffffff" bgcolor="#123456">Band</td><td>Other</td></tr></table>"##);
        assert_eq!(colour_of(&cell, "Band").flatten(), Some([255, 255, 255]), "{cell:?}");
        assert_eq!(colour_of(&cell, "Other").flatten(), None, "a neighbour keeps its own");
        let table = html_layout(r##"<table style="color:#00ff00"><tr><td>A</td><td>B</td></tr></table>"##);
        assert_eq!(colour_of(&table, "A").flatten(), Some([0, 255, 0]));
        // A colour can't smuggle markup in.
        let evil = html_layout(r##"<table><tr><td style="color:red&quot;><script>x</script>">A</td><td>B</td></tr></table>"##);
        assert!(!format!("{evil:?}").contains("script"), "{evil:?}");
    }

    #[test]
    fn boxes_placed_over_others_are_kept() {
        // A label over a picture, and a note in the bottom right corner.
        let b = html_layout(
            r##"<div style="position:relative; width:300px"><img src="cid:poster@x" alt="Poster">
               <span style="position:absolute; top:8px; left:10px; background:#c62828; color:#fff">NEW</span>
               <div style="position:absolute; right:4px; bottom:6px; width:120px">Ends Friday</div></div>"##,
        );
        let Block::Table(t) = &b[0] else { panic!("{b:?}") };
        assert_eq!(t.width, Some(Width::Px(300)));
        let cell = &t.rows[0][0];
        assert!(matches!(&cell.blocks[0], Block::Text(r) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:poster@x"))));
        assert_eq!(cell.overlays.len(), 2);
        let (new, ends) = (&cell.overlays[0], &cell.overlays[1]);
        assert_eq!((new.x, new.y, new.from_right, new.from_bottom), (10, 8, false, false));
        assert_eq!(new.background, Some([0xc6, 0x28, 0x28]));
        assert!(text(&new.blocks[0]).contains("NEW"));
        assert_eq!((ends.x, ends.y, ends.from_right, ends.from_bottom), (4, 6, true, true));
        assert_eq!(ends.width, Some(Width::Px(120)));
        // Placed with nothing to sit over: it flows; absurd offsets are capped.
        assert!(html_layout(r#"<p>Hi <span style="position:absolute;top:3px">there</span></p>"#).is_empty());
        let far = html_layout(r#"<div style="position:relative"><p>a</p><b style="position:absolute;top:99999999px;left:-50px">b</b></div>"#);
        let Block::Table(t) = &far[0] else { panic!("{far:?}") };
        assert_eq!((t.rows[0][0].overlays[0].x, t.rows[0][0].overlays[0].y), (0, MAX_OFFSET));
    }

    #[test]
    fn plain_mail_and_one_column_tables_have_no_layout() {
        assert!(html_layout("<p>Hello <b>there</b></p>").is_empty());
        assert!(html_layout("<table><tr><td>One</td></tr><tr><td>Two</td></tr></table>").is_empty());
        // A receipt (a data table) has columns.
        assert!(!html_layout("<table><tr><th>Item</th><th>Price</th></tr><tr><td>Poster</td><td>40</td></tr></table>").is_empty());
    }

    #[test]
    fn hostile_markup_is_survived() {
        // Unclosed, misnested, deep, huge.
        let deep = "<table><tr><td>".repeat(500) + "x";
        let _ = html_layout(&deep);
        let _ = html_layout("<table><td>a<td>b</tr></tr></table></td><tr><td><td>");
        let _ = html_layout("<table><tr><td><script>if (a < b) { document.write('<td>') }</script>z</td><td>y</td></tr></table>");
        let many = "<table><tr>".to_string() + &"<td>c</td>".repeat(5000) + "</tr></table>";
        assert!(html_layout(&many).is_empty(), "too many cells: flows as text");
        assert_eq!(color("#abc"), Some([0xaa, 0xbb, 0xcc]));
        assert_eq!(color("rgb(1, 2, 300)"), Some([1, 2, 255]));
        assert_eq!(color("#12"), None);
    }
}
