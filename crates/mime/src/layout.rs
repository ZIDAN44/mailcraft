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

use serde::{Deserialize, Serialize};

use crate::RichText;

mod blocks;
mod flat;
mod node;
mod parse;
mod ready;
mod write;
pub(crate) use blocks::background_pictures;
use blocks::*;
pub(crate) use flat::each_span;
pub use flat::{cell_text, flatten, text_blocks};
pub(crate) use parse::void;
use parse::{parse, parse_as};
pub(crate) use ready::ready;
use ready::{How, write_as};
pub use write::to_html;

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

/// Where a cell's content sits down its row (`valign`, `vertical-align`): a table cell's in its
/// middle unless it says otherwise, as HTML has it; a box laid out with CSS at its top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
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
    /// Room between its cells, and around them, in pixels (`cellspacing`, HTML's 2 when not
    /// said; none with `border-collapse: collapse`); `None` for a box laid out with CSS: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u16>,
    /// Room inside each of its cells that sets none of its own (`cellpadding`, HTML's 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<u16>,
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
    /// Room inside it, in pixels (top, right, bottom, left), when its style sets some
    /// (`padding`, as mail sets each block's): else its table's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding: Option<[u16; 4]>,
    /// Where its content sits down the row (the row as tall as its tallest cell).
    #[serde(default, skip_serializing_if = "is_top")]
    pub valign: VAlign,
    /// How tall it is at least, in pixels (`height`, its own or its row's): a cell taller than
    /// that grows, as in a browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u16>,
}

fn is_top(v: &VAlign) -> bool {
    *v == VAlign::Top
}

/// The tallest a cell is asked to be at least, in pixels.
pub const MAX_CELL_HEIGHT: u16 = 2_000;

/// The most room kept inside or between boxes, in pixels.
pub const MAX_ROOM: u16 = 200;

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
        apply_css(&mut root, &sheet, &mut Vec::new(), 0);
        make_headings(&mut root, 0);
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
}

/// Text sized as a title made a heading of that level (once the styles are written in): a
/// block's content goes inside one; a lone inline element (`<td><font size="5">Title</font></td>`)
/// goes inside one itself.
fn make_headings(n: &mut Node, depth: usize) {
    if depth > MAX_DEPTH * 4 || matches!(n.tag.as_str(), "head" | "style" | "script" | "title" | "template") {
        return;
    }
    for k in &mut n.kids {
        if let Kid::El(kid) = k {
            make_headings(kid, depth + 1);
        }
    }
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
        let level = kid.font_size().or_else(|| kid.wrapped_font_size(0)).map_or(0, heading_for);
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
    write_as(k, out, How { leave_hidden: false, blocks: false, styles: true, in_values: false }, false);
}

/// [`write`], without what's hidden from Outlook (a layout's text, as it's shown).
fn write_shown(k: &Kid, out: &mut String) {
    write_as(k, out, How { leave_hidden: true, blocks: false, styles: true, in_values: false }, false);
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
    html_layout_within(html, crate::HTML_MAX)
}

/// [`html_layout`], up to `max` characters of `html` (as [`crate::html::html_to_rich_within`]).
pub(crate) fn html_layout_within(html: &str, max: usize) -> Vec<Block> {
    // A table's footer at its end, as in the message's text (`html::footers_last`).
    let html = &*crate::html::footers_last(html);
    let lower = html.to_ascii_lowercase();
    if !(lower.contains("<table")
        || lower.contains("flex")
        || lower.contains("grid")
        || lower.contains("inline-block")
        || lower.contains("float")
        || lower.contains("background")
        || lower.contains("absolute")
        || lower.contains("width"))
    {
        return Vec::new();
    }
    let html = crate::capped(html, max);
    let built = std::panic::catch_unwind(|| {
        let root = parse(html);
        let mut css = String::new();
        collect_css(&root, &mut css, 0);
        let mut ctx = Ctx { css, cells: 0, color: None };
        let body = find(&root, "body", 0).unwrap_or(&root);
        // A body of a set width is a column of that width, as a box would be.
        let out = if body.tag == "body" && matches!(body.width(), Some(Width::Px(_))) {
            let column = Node { tag: "div".into(), attrs: body.attrs.clone(), kids: body.kids.iter().map(copy).collect() };
            blocks(&[Kid::El(column)], &mut ctx, 0)
        } else {
            blocks(&body.kids, &mut ctx, 0)
        };
        if ctx.cells > MAX_CELLS { Vec::new() } else { out }
    });
    match built {
        Ok(b) if has_columns(&b) => b,
        _ => Vec::new(),
    }
}

/// A CSS or attribute width: `600`, `600px`, `450pt` (as Outlook writes them), `37.5em`, `80%`.
fn parse_width(v: &str) -> Option<Width> {
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().filter(|p| p.is_finite() && *p > 0.0).map(|p| Width::Percent(p.min(100.0) as u8));
    }
    let px = length_px(v)?;
    (px >= 1.0).then(|| Width::Px(px.min(4000.0) as u32))
}

/// A CSS length in pixels: a bare number or `px`, and the absolute units (`pt`, `pc`, `in`,
/// `cm`, `mm`) and `em` / `rem` (of a 16-pixel font), as mail written by Outlook and by email
/// frameworks sizes its columns; `None` for anything else (`auto`, `50%`, `10vw`).
pub(crate) fn length_px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let units: [(&str, f32); 8] =
        [("px", 1.0), ("pt", 4.0 / 3.0), ("pc", 16.0), ("in", 96.0), ("cm", 96.0 / 2.54), ("mm", 96.0 / 25.4), ("rem", 16.0), ("em", 16.0)];
    let (number, scale) = units.iter().find_map(|(u, k)| v.strip_suffix(u).map(|n| (n, *k))).unwrap_or((v.as_str(), 1.0));
    let n = number.trim().parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0)?;
    Some((n * scale).min(100_000.0))
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
