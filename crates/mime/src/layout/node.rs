//! What a layout reads off an element: its attributes and styles (widths, colours, room,
//! alignment), and what it holds.

use super::*;

/// The colour of a table's lines when `border="1"` is all that's said.
const TABLE_LINE: [u8; 3] = [128, 128, 128];

/// The words of a CSS value: split at spaces outside brackets, so `rgb(204, 204, 204)` is one.
fn words(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut word) = (0usize, String::new());
    for c in v.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                continue;
            }
            _ => {}
        }
        word.push(c);
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// One word of a `border` shorthand: its width, its style or its colour.
fn read_line_word(word: &str, width: &mut Option<u32>, kind: &mut Option<String>, colour: &mut Option<[u8; 3]>) {
    if matches!(word, "none" | "hidden" | "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset") {
        *kind = Some(word.to_string());
    } else if let Some(w) = line_width(word) {
        *width = Some(w);
    } else if let Some(c) = color(word) {
        *colour = Some(c);
    }
}

/// A line's width in pixels (`1px`, `thin`), `0` read as such.
fn line_width(v: &str) -> Option<u32> {
    match v.trim() {
        "thin" => Some(1),
        "medium" => Some(3),
        "thick" => Some(5),
        "0" => Some(0),
        v => length_px(v).filter(|_| v.ends_with(|c: char| c.is_ascii_alphabetic())).map(|n| n.round().clamp(0.0, 64.0) as u32),
    }
}

impl Node {
    pub(super) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// A property of the inline `style` (lower case, spaces removed from the value's ends).
    pub(super) fn style(&self, prop: &str) -> Option<String> {
        self.style_as_written(prop).map(|v| v.to_ascii_lowercase())
    }

    /// A property of the inline `style` as written (addresses keep their case).
    pub(super) fn style_as_written(&self, prop: &str) -> Option<String> {
        self.styles_as_written(prop).into_iter().next()
    }

    /// Every value the inline `style` gives a property, the one that counts first: as in CSS,
    /// the last marked `!important`, else the last (`display:none;display:block` shows it).
    /// The rest follow, for a value a reader can't read (`width:600px;width:calc(…)`): a
    /// browser takes the last it can.
    pub(super) fn styles_as_written(&self, prop: &str) -> Vec<String> {
        let Some(style) = self.attr("style") else { return Vec::new() };
        let (mut marked, mut plain) = (Vec::new(), Vec::new());
        for d in style.split(';').rev() {
            let Some((k, v)) = d.split_once(':') else { continue };
            if !k.trim().eq_ignore_ascii_case(prop) {
                continue;
            }
            let v = v.trim();
            match v.to_ascii_lowercase().find("!important") {
                Some(at) => marked.push(v.get(..at).unwrap_or(v).trim().to_string()),
                None => plain.push(v.to_string()),
            }
        }
        marked.extend(plain);
        marked
    }

    /// The picture behind it: the `background` attribute, or `url(…)` in `background-image` or
    /// `background`. Web, `cid:` and `data:` addresses only (nothing that runs).
    pub(super) fn background_image(&self) -> Option<String> {
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
    pub(super) fn placed(&self) -> bool {
        self.style("position").is_some_and(|p| p == "absolute" || p == "fixed")
    }

    /// A length in pixels (`12px`, `12`); `None` for anything else (`auto`, `50%`).
    pub(super) fn px(&self, prop: &str) -> Option<i32> {
        let v = self.style(prop)?;
        let n = v.trim().trim_end_matches("px").trim().parse::<f32>().ok()?;
        Some((n as i32).clamp(-MAX_OFFSET, MAX_OFFSET))
    }

    /// Where a placed box goes over its container.
    pub(super) fn overlay_at(&self) -> (i32, i32, bool, bool) {
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
    pub(super) fn has_overlays(&self) -> bool {
        self.style("position").is_some_and(|p| p == "relative") && self.kids.iter().any(|k| matches!(k, Kid::El(n) if n.placed() && !n.hidden()))
    }

    pub(super) fn hidden(&self) -> bool {
        self.attr("hidden").is_some()
            || self.style("display").is_some_and(|d| d == "none")
            || self.attr("style").is_some_and(crate::html::mso_hidden)
            || matches!(self.tag.as_str(), "head" | "script" | "style" | "title" | "template" | "noscript")
    }

    /// How wide it wants to be: `width`, held to `max-width` (a 600-pixel column that narrows on
    /// a phone is `width: 100%; max-width: 600px`).
    pub(super) fn width(&self) -> Option<Width> {
        // A style a browser can't read (a unit it doesn't know) leaves the attribute's; `auto`
        // and the like set none.
        let styled = self.style("width");
        let own = match styled.as_deref().map(str::trim) {
            Some("auto" | "inherit" | "initial" | "unset" | "fit-content" | "max-content" | "min-content") => None,
            Some(v) => parse_width(v).or_else(|| self.attr("width").and_then(parse_width)),
            None => self.attr("width").and_then(parse_width),
        };
        match (own, self.style("max-width").and_then(|v| parse_width(&v))) {
            (Some(Width::Px(w)), Some(Width::Px(m))) => Some(Width::Px(w.min(m))),
            (None | Some(Width::Percent(100)), Some(Width::Px(m))) => Some(Width::Px(m)),
            (own, _) => own,
        }
    }

    /// A box of a set number of pixels across (a message's centred 600-pixel column).
    pub(super) fn fixed_width(&self) -> bool {
        matches!(self.tag.as_str(), "div" | "center" | "section" | "article" | "main" | "header" | "footer")
            && matches!(self.width(), Some(Width::Px(_)))
    }

    pub(super) fn background(&self) -> Option<[u8; 3]> {
        // The first that reads: `background-color`, a colour in `background`, the `bgcolor`
        // attribute (what Outlook shows for a gradient it doesn't draw, issue #41), and last a
        // gradient's first colour.
        // Whole first: `rgb(34, 34, 34)`, as programs that write styles out put it, has spaces.
        // (The one that counts, of those that read: `#0a0` under a `var(--b)` after it.)
        let own = self
            .styles_as_written("background-color")
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .find_map(|v| color(&v).or_else(|| color(words(&v).first()?)));
        let shorthand = || {
            let v = self.style("background")?;
            if v.contains("gradient(") {
                return None;
            }
            words(&v).iter().find_map(|w| color(w))
        };
        let attr = || self.attr("bgcolor").map(str::to_ascii_lowercase).and_then(|v| color(&v));
        let gradient = || {
            let v = self.style("background").or_else(|| self.style("background-image"))?;
            let inner = v.get(v.find("gradient(")? + 9..)?;
            stops(inner).into_iter().find_map(|stop| color(stop.trim().split(" ").next()?).or_else(|| color(stop.trim())))
        };
        own.or_else(shorthand).or_else(attr).or_else(gradient)
    }

    pub(super) fn align(&self) -> Align {
        self.own_align().unwrap_or_default()
    }

    /// Whether it's a title (`<h1>` to `<h6>`).
    pub(super) fn is_title(&self) -> bool {
        matches!(self.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
    }

    /// The element without what lines it up (`align`, `text-align`): for inside the cell that
    /// does it instead.
    pub(super) fn unaligned(&self) -> Node {
        let attrs = self
            .attrs
            .iter()
            .filter(|(k, _)| k != "align")
            .map(|(k, v)| {
                if k == "style" {
                    let kept: Vec<&str> =
                        v.split(';').filter(|d| !d.split(':').next().is_some_and(|p| p.trim().eq_ignore_ascii_case("text-align"))).collect();
                    (k.clone(), kept.join(";"))
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        Node { tag: self.tag.clone(), attrs, kids: self.kids.iter().map(copy).collect() }
    }

    /// How what's in it lines up, when it says (`align`, `text-align`).
    pub(super) fn own_align(&self) -> Option<Align> {
        let v = self.attr("align").map(str::to_ascii_lowercase).or_else(|| self.style("text-align"))?;
        match v.trim() {
            "center" | "middle" => Some(Align::Center),
            "right" | "end" => Some(Align::Right),
            "left" | "start" => Some(Align::Left),
            _ => None,
        }
    }

    /// Where its content sits down its row (`valign`, `vertical-align`), when it says.
    pub(super) fn valign(&self) -> Option<VAlign> {
        let v = self.attr("valign").map(str::to_ascii_lowercase).or_else(|| self.style("vertical-align"))?;
        match v.trim() {
            "top" | "baseline" | "text-top" => Some(VAlign::Top),
            "middle" | "center" => Some(VAlign::Middle),
            "bottom" | "text-bottom" => Some(VAlign::Bottom),
            _ => None,
        }
    }

    /// How tall it asks to be at least (`height`, as a number of pixels or in its style); not a
    /// percentage, which says nothing without its row's own.
    pub(super) fn min_height(&self) -> Option<u16> {
        let v = self.style("height").or_else(|| self.attr("height").map(str::to_string))?;
        let v = v.trim();
        if v.ends_with('%') {
            return None;
        }
        let px = v.parse::<f32>().ok().or_else(|| length_px(v))?;
        (px.is_finite() && px >= 1.0).then(|| (px.round() as u16).min(MAX_CELL_HEIGHT))
    }

    /// The room its style sets inside it (`padding` and `padding-top`…), top, right, bottom,
    /// left in pixels; `None` when it sets none.
    pub(super) fn padding(&self) -> Option<[u16; 4]> {
        let px = |v: &str| length_px(v).map(|n| (n.round() as u16).min(MAX_ROOM));
        let mut out: Option<[u16; 4]> = None;
        if let Some(v) = self.style("padding") {
            let parts: Vec<u16> = v.split_whitespace().take(4).map(|p| px(p).unwrap_or(0)).collect();
            out = match parts.as_slice() {
                [a] => Some([*a; 4]),
                [a, b] => Some([*a, *b, *a, *b]),
                [a, b, c] => Some([*a, *b, *c, *b]),
                [a, b, c, d] => Some([*a, *b, *c, *d]),
                _ => None,
            };
        }
        for (i, side) in ["padding-top", "padding-right", "padding-bottom", "padding-left"].into_iter().enumerate() {
            if let Some(n) = self.style(side).as_deref().and_then(px) {
                let mut p = out.unwrap_or_default();
                if let Some(slot) = p.get_mut(i) {
                    *slot = n;
                }
                out = Some(p);
            }
        }
        out
    }

    /// The room a `<div>` asks for above it (`margin-top`, or `margin`'s first length), in pixels:
    /// `0` for none, and for any other box (a paragraph's is the empty line the text has for it).
    pub(super) fn room_above(&self) -> u16 {
        if self.tag != "div" {
            return 0;
        }
        let all = self.style("margin");
        let top = self.style("margin-top").or_else(|| all.as_deref().and_then(|v| v.split_whitespace().next()).map(str::to_string));
        top.as_deref().and_then(length_px).map_or(0, |n| (n.round() as u16).min(MAX_ROOM))
    }

    /// The line along one side of it (`top`, `right`, `bottom`, `left`) its style sets: from
    /// `border-top` or `border`, `border-top-width` and the like over them. None when its style is
    /// `none`, or says none, as CSS has it.
    pub(super) fn line(&self, side: &str) -> Option<Line> {
        let short = self.style(&format!("border-{side}")).or_else(|| self.style("border"));
        let (mut width, mut kind, mut colour): (Option<u32>, Option<String>, Option<[u8; 3]>) = (None, None, None);
        for word in words(short.as_deref().unwrap_or("")) {
            read_line_word(&word, &mut width, &mut kind, &mut colour);
        }
        let one = |prop: &str, all: &str| self.style(prop).or_else(|| self.style(all)).filter(|v| words(v).len() == 1);
        if let Some(v) = one(&format!("border-{side}-width"), "border-width") {
            width = line_width(&v);
        }
        if let Some(v) = one(&format!("border-{side}-style"), "border-style") {
            kind = Some(v);
        }
        if let Some(v) = one(&format!("border-{side}-color"), "border-color") {
            colour = color(&v).or(colour);
        }
        let shown = kind.is_some_and(|k| !matches!(k.as_str(), "none" | "hidden"));
        let width = width.unwrap_or(3).min(MAX_LINE) as u8;
        (shown && width > 0).then(|| Line { width, color: colour.unwrap_or([0, 0, 0]) })
    }

    /// The lines along its sides: top, right, bottom, left.
    pub(super) fn lines(&self) -> [Option<Line>; 4] {
        ["top", "right", "bottom", "left"].map(|side| self.line(side))
    }

    /// The line around a table (`border`, or the attribute `border="1"`).
    pub(super) fn table_line(&self) -> Option<Line> {
        self.line("top").or_else(|| {
            let n = self.attr("border")?.trim().parse::<u32>().ok().filter(|n| *n > 0)?;
            Some(Line { width: n.min(MAX_LINE) as u8, color: TABLE_LINE })
        })
    }

    /// A cell's lines: its own, its row's, and the one its table's `border="1"` gives every cell.
    pub(super) fn cell_lines(&self, tr: &Node, table: &Node) -> [Option<Line>; 4] {
        let all = table.attr("border").and_then(|b| b.trim().parse::<u32>().ok()).filter(|n| *n > 0).map(|_| Line { width: 1, color: TABLE_LINE });
        let mut lines = self.lines();
        for (i, side) in ["top", "right", "bottom", "left"].into_iter().enumerate() {
            if let Some(l) = lines.get_mut(i)
                && l.is_none()
            {
                *l = tr.line(side).or(all);
            }
        }
        lines
    }

    /// Whether there's nothing in it to see: no picture, and no text but spaces.
    pub(super) fn blank(&self) -> bool {
        !self.has_visible_text(0) && !self.has_picture(0)
    }

    fn has_visible_text(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && self.kids.iter().any(|k| match k {
                Kid::Text(t) => t.chars().any(|c| !c.is_whitespace() && !matches!(c, '\u{200a}' | '\u{200b}' | '\u{200c}' | '\u{feff}' | '\u{a0}')),
                Kid::El(n) => n.has_visible_text(depth + 1),
            })
    }

    /// Whether it's a box that draws something around what it holds: a colour other than the one
    /// it's on, or a line. Not a table, and not a picture behind it (those have their own ways).
    pub(super) fn draws_a_box(&self, on: Option<[u8; 3]>) -> bool {
        matches!(self.tag.as_str(), "div" | "section" | "article" | "aside" | "header" | "footer" | "main")
            && self.background_image().is_none()
            && (self.background().is_some_and(|c| Some(c) != on) || self.lines().iter().any(Option::is_some))
    }

    /// The line a box with nothing in it draws (`<p style="border-top: …">`, a one-cell table of
    /// that, or `<hr>` drawn so): a divider.
    pub(super) fn rule(&self) -> Option<Line> {
        if !matches!(self.tag.as_str(), "p" | "div" | "table") {
            return None;
        }
        let line = self.line("top").or_else(|| self.line("bottom")).filter(|_| self.line("left").is_none())?;
        let cells: usize = if self.tag == "table" { self.rows().iter().map(|(_, c)| c.len()).sum() } else { 0 };
        (cells <= 1 && self.background().is_none() && self.background_image().is_none() && self.blank()).then_some(line)
    }

    /// The height of a box with nothing in it that's there to part what's around it
    /// (`<div style="height:20px">&#8202;</div>`, as MJML writes a spacer), or to be a bar of colour.
    pub(super) fn spacer(&self) -> Option<u16> {
        let height = self.min_height().filter(|_| matches!(self.tag.as_str(), "div" | "p"))?;
        (self.background_image().is_none() && self.blank()).then_some(height)
    }

    /// The radius of its corners its style sets (`border-radius`: its first length, every corner
    /// drawn alike), in pixels; `None` for none, or for a share of its size (`50%`).
    pub(super) fn radius(&self) -> Option<u16> {
        let v = self.style("border-radius")?;
        let first = v.split(|c: char| c.is_whitespace() || c == '/').find(|p| !p.is_empty())?;
        let px = length_px(first)?;
        (px >= 0.5).then(|| (px.round() as u16).min(MAX_ROOM))
    }

    /// A link drawn as a button: a box of its own (`display: inline-block` or `block`) on a colour
    /// of its own, with room around its words, round corners or a size (a newsletter's "Shop
    /// now", or Outlook's VML one, made such a link as it's read).
    pub(super) fn button(&self) -> bool {
        self.tag == "a"
            && self.attr("href").is_some()
            && !self.hidden()
            && self.style("display").is_some_and(|d| d == "inline-block" || d == "block")
            && self.background().is_some()
            && self.background_image().is_none()
            && (self.padding().is_some() || self.radius().is_some() || self.width().is_some() || self.button_height().is_some())
    }

    /// How tall a button is at least: its `height`, or its one line's `line-height` in pixels or
    /// points (a number alone is a share of its text's size, not a height).
    pub(super) fn button_height(&self) -> Option<u16> {
        let line = self.style("line-height").filter(|v| v.ends_with("px") || v.ends_with("pt")).and_then(|v| length_px(&v));
        let line = line.filter(|px| *px >= 1.0).map(|px| (px.round() as u16).min(MAX_CELL_HEIGHT));
        self.min_height().max(line)
    }

    /// The one button it holds, through wrappers that hold nothing else and have no colour of
    /// their own (`<p>`, `<div>`, `<center>`, `<span>`, `<b>`…), and where the innermost of them
    /// that says puts it across.
    pub(super) fn only_button(&self, depth: usize) -> Option<(&Node, Align)> {
        if self.button() {
            return Some((self, Align::Left));
        }
        let wrapper = matches!(self.tag.as_str(), "div" | "p" | "center" | "span" | "font" | "b" | "strong" | "em" | "i");
        if depth > 4 || !wrapper || self.hidden() || self.background().is_some() || self.background_image().is_some() {
            return None;
        }
        let mut shown = self.kids.iter().filter(|k| match k {
            Kid::Text(t) => !t.trim().is_empty(),
            Kid::El(n) => !n.hidden(),
        });
        let (Some(Kid::El(only)), None) = (shown.next(), shown.next()) else { return None };
        let (b, inner) = only.only_button(depth + 1)?;
        let own = if self.tag == "center" { Align::Center } else { self.align() };
        Some((b, if inner == Align::Left { own } else { inner }))
    }

    /// A button's link without its box (its colour, room, corners and size are its cell's), and
    /// without a class or id a style sheet could give them back by: its words and their look.
    pub(super) fn as_link(&self) -> Node {
        let keep = |p: &str| matches!(p, "color" | "text-decoration" | "text-transform" | "letter-spacing") || p.starts_with("font");
        let style: Vec<&str> = self
            .attr("style")
            .unwrap_or("")
            .split(';')
            .filter(|d| d.split_once(':').is_some_and(|(p, _)| keep(&p.trim().to_ascii_lowercase())))
            .collect();
        let mut attrs: Vec<(String, String)> = self.attrs.iter().filter(|(k, _)| k == "href" || k == "title").cloned().collect();
        if !style.is_empty() {
            attrs.push(("style".into(), style.join(";")));
        }
        Node { tag: "a".into(), attrs, kids: self.kids.iter().map(copy).collect() }
    }

    /// A table's room between and around its cells (`cellspacing`; HTML's 2 when not said).
    pub(super) fn cell_spacing(&self) -> u16 {
        if self.style("border-collapse").is_some_and(|c| c == "collapse") {
            return 0;
        }
        let spacing = self.style("border-spacing").and_then(|v| v.split_whitespace().next().and_then(length_px));
        let attr = self.attr("cellspacing").and_then(length_px);
        spacing.or(attr).map_or(2, |n| (n.round() as u16).min(MAX_ROOM))
    }

    /// A table's room inside its cells (`cellpadding`; HTML's 1 when not said).
    pub(super) fn cell_padding(&self) -> u16 {
        self.attr("cellpadding").and_then(length_px).map_or(1, |n| (n.round() as u16).min(MAX_ROOM))
    }

    /// Where a table sits: `align`, or auto side margins.
    pub(super) fn table_align(&self) -> Align {
        self.own_table_align().unwrap_or_default()
    }

    /// [`Node::table_align`], when the table says.
    pub(super) fn own_table_align(&self) -> Option<Align> {
        let margin = self.style("margin").unwrap_or_default();
        if margin.contains("auto") || self.style("margin-left").is_some_and(|m| m == "auto") {
            return Some(Align::Center);
        }
        self.own_align()
    }

    /// Where the boxes in it sit across it, when its `align` attribute says (`<td
    /// align="right">`): HTML's `align` moves the tables in a cell too, where CSS's `text-align`
    /// moves its words alone.
    pub(super) fn boxes_align(&self) -> Option<Align> {
        match self.attr("align")?.trim().to_ascii_lowercase().as_str() {
            "center" | "middle" => Some(Align::Center),
            "right" => Some(Align::Right),
            "left" => Some(Align::Left),
            _ => None,
        }
    }

    /// Whether a table, side-by-side boxes or a box of a set width are somewhere inside.
    pub(super) fn has_table(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && !self.hidden()
            && self.kids.iter().any(|k| match k {
                Kid::El(n) => n.tag == "table" || n.columns().is_some() || n.fixed_width() || n.has_table(depth + 1),
                Kid::Text(_) => false,
            })
    }

    /// A box that sits beside its neighbours: `inline-block`, a table cell, or floated.
    pub(super) fn beside(&self) -> bool {
        self.style("display").is_some_and(|d| matches!(d.as_str(), "inline-block" | "table-cell" | "inline-flex" | "inline-table"))
            || self.style("float").is_some_and(|f| f == "left" || f == "right")
    }

    /// Boxes laid out side by side with CSS alone (flexbox in a row, a grid, or `inline-block` /
    /// floated / table-cell children), as rows of boxes; `None` when they stack.
    pub(super) fn columns(&self) -> Option<Vec<Vec<&Node>>> {
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
    pub(super) fn rows(&self) -> Vec<(&Node, Vec<&Node>)> {
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

impl Node {
    pub(super) fn has_block_inside(&self, depth: usize) -> bool {
        depth > MAX_DEPTH * 4 || self.kids.iter().any(|k| matches!(k, Kid::El(n) if block_tag(&n.tag) || n.has_block_inside(depth + 1)))
    }

    pub(super) fn has_picture(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4 && self.kids.iter().any(|k| matches!(k, Kid::El(n) if !n.hidden() && (n.tag == "img" || n.has_picture(depth + 1))))
    }

    pub(super) fn has_text(&self, depth: usize) -> bool {
        depth < MAX_DEPTH * 4
            && self.kids.iter().any(|k| match k {
                Kid::Text(t) => !t.trim().is_empty(),
                Kid::El(n) => n.has_text(depth + 1),
            })
    }

    /// Whether the only blocks inside it are titles (`<h1>`…`<h6>` holding text), as a paragraph
    /// sized as a title is made one.
    pub(super) fn holds_a_title(&self) -> bool {
        self.kids.iter().all(|k| match k {
            Kid::Text(_) => true,
            Kid::El(e) if !block_tag(&e.tag) => !e.has_block_inside(0),
            // A title may hold a box of its own (`<h2><div>`, as a template writes one).
            Kid::El(e) => e.tag.len() == 2 && e.tag.starts_with('h') && e.tag != "hr" && !e.has_table(0),
        })
    }

    /// The size of the one element it wraps, through inline wrappers (`<b><span style="font-size:
    /// 20pt">`, as Outlook writes a title), when that's all it holds.
    pub(super) fn wrapped_font_size(&self, depth: usize) -> Option<f32> {
        if depth > 4 || block_tag(&self.tag) {
            return None;
        }
        let mut kids = self.kids.iter().filter(|k| !matches!(k, Kid::Text(t) if t.trim().is_empty()));
        let (Some(Kid::El(only)), None) = (kids.next(), kids.next()) else { return None };
        only.font_size().or_else(|| only.wrapped_font_size(depth + 1))
    }

    /// Its size in pixels, when its style or `<font size>` sets one.
    /// The size of the text in it, in pixels, given the size of what's around it (`0`: not
    /// known). A size of nothing (`font-size: 0`, on a template's columns) is passed over.
    pub(super) fn text_size(&self, around: u16) -> u16 {
        let base = if around == 0 { 16.0 } else { f32::from(around) };
        let number = |v: &str, unit: &str| v.strip_suffix(unit).and_then(|n| n.trim().parse::<f32>().ok());
        let px = match self.style("font-size").as_deref().map(str::trim) {
            Some("smaller") => Some(base * 0.83),
            Some("larger") => Some(base * 1.2),
            // What's relative is to what's around it (the rest are the same anywhere).
            Some(v) if v.ends_with('%') => number(v, "%").map(|n| base * n / 100.0),
            Some(v) if v.ends_with("em") && !v.ends_with("rem") => number(v, "em").map(|n| base * n),
            _ => self.font_size(),
        };
        px.filter(|p| p.is_finite() && *p >= 1.0).map_or(around, |p| p.round().min(f32::from(u16::MAX)) as u16)
    }

    /// Whether the text in it is bold, given whether what's around it is: as its style's
    /// `font-weight` says, when it says. (What's bold by its tag, `<b>` or a heading, the text
    /// converter reads itself.)
    pub(super) fn bold(&self, around: bool) -> bool {
        match self.style("font-weight").as_deref().map(str::trim) {
            Some("bold" | "bolder") => true,
            Some("normal" | "lighter") => false,
            Some(v) => v.parse::<f32>().map_or(around, |w| w >= 600.0),
            None => around,
        }
    }

    pub(super) fn font_size(&self) -> Option<f32> {
        self.style("font-size")
            .as_deref()
            .and_then(font_px)
            .or_else(|| (self.tag == "font").then(|| self.attr("size").and_then(font_attr_px)).flatten())
    }
}
