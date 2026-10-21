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

mod blocks;
pub(crate) use blocks::background_pictures;
use blocks::*;

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

    /// Rows (through table sections), each its row element and cells.
    fn rows(&self) -> Vec<(&Node, Vec<&Node>)> {
        let mut out = Vec::new();
        for k in &self.kids {
            let Kid::El(n) = k else { continue };
            match n.tag.as_str() {
                "tr" if !n.hidden() => out.push((
                    n,
                    n.kids
                        .iter()
                        .filter_map(|k| if let Kid::El(c) = k { Some(c) } else { None })
                        .filter(|c| matches!(c.tag.as_str(), "td" | "th") && !c.hidden())
                        .collect(),
                )),
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

/// Elements whose content is text taken as it is, not markup.
fn raw_text(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "title" | "textarea" | "xmp" | "noembed" | "noframes" | "iframe")
}

/// How big text is, in pixels, from a CSS `font-size` (`20px`, `15pt`, `1.5em`, `x-large`).
pub(crate) fn font_px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|n| n.is_finite() && *n > 0.0);
    let px = if let Some(n) = v.strip_suffix("px") {
        num(n)?
    } else if let Some(n) = v.strip_suffix("pt") {
        num(n)? * 4.0 / 3.0
    } else if let Some(n) = v.strip_suffix("rem").or_else(|| v.strip_suffix("em")) {
        num(n)? * 16.0
    } else if let Some(n) = v.strip_suffix('%') {
        num(n)? * 0.16
    } else {
        match v.as_str() {
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            _ => return None,
        }
    };
    Some(px.min(400.0))
}

/// `<font size="…">` in pixels (1 to 7, as browsers size them; `+n` and `-n` from 3).
pub(crate) fn font_attr_px(v: &str) -> Option<f32> {
    let v = v.trim();
    let n: i32 = if let Some(d) = v.strip_prefix('+') {
        3i32.saturating_add(d.parse::<i32>().ok()?)
    } else if let Some(d) = v.strip_prefix('-') {
        3i32.saturating_sub(d.parse::<i32>().ok()?)
    } else {
        v.parse().ok()?
    };
    Some(match n.clamp(1, 7) {
        1 => 10.0,
        2 => 13.0,
        3 => 16.0,
        4 => 18.0,
        5 => 24.0,
        6 => 32.0,
        _ => 48.0,
    })
}

/// The heading level text of this size reads as (`0`: body text): mail sizes its titles with
/// CSS or `<font size>` rather than `<h1>`.
fn heading_for(px: f32) -> u8 {
    if px >= 26.0 {
        1
    } else if px >= 21.0 {
        2
    } else if px >= 18.0 {
        3
    } else {
        0
    }
}

/// Elements that start a block of their own (a heading can't hold them).
fn block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "table"
            | "tr"
            | "td"
            | "th"
            | "tbody"
            | "thead"
            | "tfoot"
            | "div"
            | "p"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "pre"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "form"
            | "dl"
    )
}

impl Node {
    fn has_block_inside(&self, depth: usize) -> bool {
        depth > MAX_DEPTH * 4 || self.kids.iter().any(|k| matches!(k, Kid::El(n) if block_tag(&n.tag) || n.has_block_inside(depth + 1)))
    }

    fn has_picture(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4 && self.kids.iter().any(|k| matches!(k, Kid::El(n) if !n.hidden() && (n.tag == "img" || n.has_picture(depth + 1))))
    }

    fn has_text(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && self.kids.iter().any(|k| match k {
                Kid::Text(t) => !t.trim().is_empty(),
                Kid::El(n) => n.has_text(depth + 1),
            })
    }

    /// Its size in pixels, when its style or `<font size>` sets one.
    fn font_size(&self) -> Option<f32> {
        self.style("font-size")
            .as_deref()
            .and_then(font_px)
            .or_else(|| (self.tag == "font").then(|| self.attr("size").and_then(font_attr_px)).flatten())
    }
}

/// The message with its style sheets' rules written into each element's `style` (inline
/// declarations still win, `!important` ones over both), and text sized as a title made a
/// heading, so the layout and the text converter both see them. Unchanged when it has no style
/// sheet and no sizes.
pub fn inline_css(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !(lower.contains("<style") || lower.contains("font-size") || lower.contains("<font")) {
        return html.to_string();
    }
    let capped = match html.char_indices().nth(crate::HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let done = std::panic::catch_unwind(|| {
        let mut root = parse(capped);
        let mut text = String::new();
        style_text(&root, &mut text, 0);
        let sheet = crate::css::Sheet::parse(&text);
        let mut chain = Vec::new();
        apply_css(&mut root, &sheet, &mut chain, 0);
        let mut out = String::with_capacity(capped.len() + capped.len() / 4);
        write(&Kid::El(root), &mut out);
        out
    });
    done.unwrap_or_else(|_| html.to_string())
}

/// The text of every `<style>` element.
fn style_text(n: &Node, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH * 4 || out.len() > 256 * 1024 {
        return;
    }
    for k in &n.kids {
        match k {
            Kid::Text(t) if n.tag == "style" => {
                out.push_str(t);
                out.push('\n');
            }
            Kid::El(e) => style_text(e, out, depth + 1),
            Kid::Text(_) => {}
        }
    }
}

/// An element as selectors see it, kept while its children are styled.
struct Ancestor {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
}

fn apply_css(n: &mut Node, sheet: &crate::css::Sheet, chain: &mut Vec<Ancestor>, depth: usize) {
    if depth > MAX_DEPTH * 4 || matches!(n.tag.as_str(), "head" | "style" | "script" | "title" | "template") {
        return;
    }
    for k in &mut n.kids {
        let Kid::El(kid) = k else { continue };
        chain.push(Ancestor {
            tag: kid.tag.clone(),
            id: kid.attr("id").map(str::to_string),
            classes: kid.attr("class").map(|c| c.split_whitespace().take(16).map(str::to_string).collect()).unwrap_or_default(),
        });
        if !sheet.is_empty() {
            let els: Vec<crate::css::El<'_>> = chain
                .iter()
                .map(|a| crate::css::El { tag: &a.tag, id: a.id.as_deref(), classes: a.classes.iter().map(String::as_str).collect() })
                .collect();
            let matched = sheet.matching(&els);
            if !matched.is_empty() {
                let mut style: Vec<(String, String)> = Vec::new();
                let set = |p: &str, v: &str, style: &mut Vec<(String, String)>| match style.iter_mut().find(|(k, _)| k == p) {
                    Some(e) => e.1 = v.to_string(),
                    None => style.push((p.to_string(), v.to_string())),
                };
                for d in matched.iter().filter(|d| !d.important) {
                    set(&d.prop, &d.value, &mut style);
                }
                for d in crate::css::declarations(kid.attr("style").unwrap_or("")) {
                    set(&d.prop, &d.value, &mut style);
                }
                for d in matched.iter().filter(|d| d.important) {
                    set(&d.prop, &d.value, &mut style);
                }
                let written = style.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ");
                match kid.attrs.iter_mut().find(|(k, _)| k == "style") {
                    Some(a) => a.1 = written,
                    None => kid.attrs.push(("style".into(), written)),
                }
            }
        }
        apply_css(kid, sheet, chain, depth + 1);
        chain.pop();
    }
    // Text sized as a title becomes a heading of that level: a block's content goes inside one;
    // a lone inline element (`<td><font size="5">Title</font></td>`) goes inside one itself.
    let only_child = n
        .kids
        .iter()
        .filter(|k| match k {
            Kid::Text(t) => !t.trim().is_empty(),
            Kid::El(_) => true,
        })
        .count()
        == 1;
    let parent_is_block = block_tag(&n.tag);
    for k in &mut n.kids {
        let Kid::El(kid) = k else { continue };
        let level = kid.font_size().map_or(0, heading_for);
        let is_heading = kid.tag.len() == 2 && kid.tag.starts_with('h') && kid.tag != "hr";
        if level == 0 || is_heading || !kid.has_text(0) || kid.has_block_inside(0) {
            continue;
        }
        let h = format!("h{level}");
        if block_tag(&kid.tag) {
            let inner = std::mem::take(&mut kid.kids);
            kid.kids = vec![Kid::El(Node { tag: h, attrs: Vec::new(), kids: inner })];
        } else if only_child && parent_is_block {
            let inner = std::mem::take(kid);
            *kid = Node { tag: h, attrs: Vec::new(), kids: vec![Kid::El(inner)] };
        }
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
                    match k {
                        // A style sheet or script goes back as it was (escaping would break it).
                        Kid::Text(t) if raw_text(&n.tag) => out.push_str(&t.replace("</", "")),
                        k => write(k, out),
                    }
                }
                out.push_str("</");
                out.push_str(&n.tag);
                out.push('>');
            }
        }
    }
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
mod tests;
