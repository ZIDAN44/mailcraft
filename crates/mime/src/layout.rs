//! The layout of HTML mail built from tables: newsletters and receipts put their content in
//! columns with layout tables (a centred 600-pixel table, rows of two or three cells, coloured
//! bands). [`html_layout`] keeps that structure as [`Block`]s: tables with rows and cells (their
//! widths, colours and alignment) around formatted text, which is [`crate::html_to_rich`]'s.
//! Mail without tables of more than one column has no layout: it flows as text.
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
        let style = self.attr("style")?.to_ascii_lowercase();
        style.split(';').find_map(|d| {
            let (k, v) = d.split_once(':')?;
            (k.trim() == prop).then(|| v.trim().trim_end_matches("!important").trim().to_string())
        })
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

    fn has_table(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && !self.hidden()
            && self.kids.iter().any(|k| match k {
                Kid::El(n) => n.tag == "table" || n.has_table(depth + 1),
                Kid::Text(_) => false,
            })
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
}

fn blocks(kids: &[Kid], ctx: &mut Ctx, depth: usize) -> Vec<Block> {
    let mut out = Vec::new();
    let mut pending = String::new();
    let flush = |pending: &mut String, out: &mut Vec<Block>, ctx: &Ctx| {
        if pending.chars().any(|c| !c.is_whitespace()) || pending.contains("<img") {
            let rich = crate::html_to_rich(&format!("{}{}", ctx.css, pending));
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
                let rows: Vec<Vec<Cell>> = n
                    .rows()
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|c| {
                                ctx.cells += 1;
                                Cell {
                                    blocks: if ctx.cells > MAX_CELLS { Vec::new() } else { blocks(&c.kids, ctx, depth + 1) },
                                    width: c.width(),
                                    background: c.background(),
                                    align: c.align(),
                                    span: c.attr("colspan").and_then(|s| s.trim().parse::<u16>().ok()).unwrap_or(1).clamp(1, 50),
                                }
                            })
                            .filter(|c| !c.blocks.is_empty() || c.background.is_some())
                            .collect::<Vec<Cell>>()
                    })
                    .filter(|r| !r.is_empty())
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                out.push(Block::Table(Table { rows, width: n.width(), align: n.table_align(), background: n.background() }));
            }
            // A wrapper around tables (`<div>`, `<center>`): its parts in turn.
            Kid::El(n) if depth < MAX_DEPTH && n.has_table(0) => {
                flush(&mut pending, &mut out, ctx);
                let inner = blocks(&n.kids, ctx, depth);
                if n.tag == "center" || n.align() == Align::Center {
                    // Centred: as a one-cell table that sits in the middle.
                    out.push(Block::Table(Table {
                        rows: vec![vec![Cell { blocks: inner, align: Align::Center, span: 1, ..Default::default() }]],
                        width: n.width(),
                        align: Align::Center,
                        background: n.background(),
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

/// Whether any table puts content side by side.
fn has_columns(b: &[Block]) -> bool {
    b.iter().any(|b| match b {
        Block::Text(_) => false,
        Block::Table(t) => {
            t.rows.iter().any(|r| r.iter().filter(|c| !c.blocks.is_empty()).count() > 1) || t.rows.iter().flatten().any(|c| has_columns(&c.blocks))
        }
    })
}

/// The layout of an HTML message: empty when it has no columns (the text flows instead).
pub fn html_layout(html: &str) -> Vec<Block> {
    if !html.to_ascii_lowercase().contains("<table") {
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
        let mut ctx = Ctx { css, cells: 0 };
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
