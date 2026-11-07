//! What a layout reads off an element: its attributes and styles (widths, colours, room,
//! alignment), and what it holds.

use super::*;

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
            || self.style("mso-hide").is_some_and(|d| d == "all")
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
        let own = self.style("background-color").and_then(|v| color(v.split_whitespace().next()?));
        let shorthand = || {
            let v = self.style("background")?;
            if v.contains("gradient(") {
                return None;
            }
            v.split_whitespace().find_map(color)
        };
        let attr = || self.attr("bgcolor").map(str::to_ascii_lowercase).and_then(|v| color(&v));
        let gradient = || {
            let v = self.style("background").or_else(|| self.style("background-image"))?;
            let inner = v.get(v.find("gradient(")? + 9..)?;
            inner.split(',').find_map(|stop| color(stop.split_whitespace().next()?))
        };
        own.or_else(shorthand).or_else(attr).or_else(gradient)
    }

    pub(super) fn align(&self) -> Align {
        let v = self.attr("align").map(str::to_ascii_lowercase).or_else(|| self.style("text-align")).unwrap_or_default();
        match v.trim() {
            "center" | "middle" => Align::Center,
            "right" | "end" => Align::Right,
            _ => Align::Left,
        }
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
        let margin = self.style("margin").unwrap_or_default();
        if margin.contains("auto") || self.style("margin-left").is_some_and(|m| m == "auto") {
            return Align::Center;
        }
        self.align()
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
            Kid::El(e) => e.tag.len() == 2 && e.tag.starts_with('h') && e.tag != "hr" && !e.has_block_inside(0),
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
    pub(super) fn font_size(&self) -> Option<f32> {
        self.style("font-size")
            .as_deref()
            .and_then(font_px)
            .or_else(|| (self.tag == "font").then(|| self.attr("size").and_then(font_attr_px)).flatten())
    }
}
