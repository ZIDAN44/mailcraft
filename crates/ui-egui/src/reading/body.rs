//! The reading pane: a message's text, laid out as its HTML lays it out, with its pictures.

use super::*;

/// Above an HTML message: whether its web pictures are hidden, and a switch to plain text.
pub(super) fn html_bar(s: &Session, t: &Tokens, ui: &mut Ui, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let web = s.remote_pictures();
    let formatted = s.prefs.show_html && !p.rich.is_empty();
    if !web.is_empty() && !s.pictures_allowed() && formatted {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Image, t.dim);
                    ui.label(RichText::new(tl!("Pictures from the web are hidden to protect your privacy.")).color(t.dim));
                    if ui.link(tl!("Show pictures")).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({})));
                    }
                    let who = p.from.email.clone();
                    if !who.is_empty() && ui.link(tf!("Always show from {sender}", sender = who)).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({"always": true})));
                    }
                });
            },
        );
        ui.add_space(6.0);
    }
    // What reports back to the sender: never-loaded pictures, links that count clicks.
    let tr = &p.trackers;
    if !tr.is_empty() {
        let line = match (tr.pictures.len(), tr.links) {
            (0, links) => tf!("Links that report clicks: {links}", links = links),
            (pictures, 0) => tf!("Tracking pictures, never loaded: {pictures}", pictures = pictures),
            (pictures, links) => {
                tf!("Tracking pictures, never loaded: {pictures} · Links that report clicks: {links}", pictures = pictures, links = links)
            }
        };
        let about = if s.prefs.clean_links {
            tl!("Links open without their tracking tags, and out of redirects that carry the real address")
        } else {
            tl!("Links open as they are (File › Options › Safety)")
        };
        ui.horizontal_wrapped(|ui| {
            let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Info, t.faint);
            ui.label(RichText::new(line).color(t.faint).small()).on_hover_text(about);
        });
    }
    ui.horizontal(|ui| {
        if formatted {
            if ui.link(RichText::new(tl!("Show as plain text")).small()).clicked() {
                acts.push(Action::Run("view.html", json!({"on": false})));
            }
            if !web.is_empty() && s.pictures_allowed() && s.prefs.trusted_senders.contains(&p.from.email.to_ascii_lowercase()) {
                ui.label(RichText::new("·").color(t.faint).small());
                if ui.link(RichText::new(tl!("Stop showing pictures from this sender")).small()).clicked() {
                    acts.push(Action::Run("message.blockPictures", json!({})));
                }
            }
        } else if !p.rich.is_empty() && ui.link(RichText::new(tl!("Show formatted")).small()).clicked() {
            acts.push(Action::Run("view.html", json!({"on": true})));
        } else if p.rich.is_empty() {
            ui.label(RichText::new(tl!("Shown as text.")).color(t.faint).small());
        }
    });
    ui.add_space(6.0);
}

/// A message that hides a lot of text from its reader: said, with the text to look at.
pub(super) fn hidden_bar(t: &Tokens, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed) {
    let id = egui::Id::unique(("hidden-text", key.to_id()));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.warn);
            ui.vertical(|ui| {
                ui.label(RichText::new(tf!("This message hides text from you ({n} words).", n = p.hidden_words)).color(t.text));
                ui.label(
                    RichText::new(tl!(
                        "Hidden text is a way to slip past spam filters, or to give instructions to tools and assistants that read mail for you."
                    ))
                    .color(t.dim)
                    .small(),
                );
                if ui.link(RichText::new(if open { tl!("Hide it again") } else { tl!("Show the hidden text") }).small()).clicked() {
                    open = !open;
                }
            });
        });
        if open {
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt(id).max_height(180.0).show(ui, |ui| {
                let mut text = p.hidden_text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text).font(egui::FontId::monospace(12.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, open));
    ui.add_space(8.0);
}

/// An HTML message, formatted: emphasis, code, links (opened in the browser on click) and
/// pictures: those inside the message always, those on the web only once allowed. Mail laid out
/// with tables keeps its columns ([`mime::layout`]).
pub(super) fn rich_body(s: &Session, st: &mut UiState, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let allowed = s.pictures_allowed();
    let mut shown: Vec<String> = Vec::new();
    let width = ui.available_width();
    let mut body = Body { s, key, p, allowed, shown: &mut shown, acts };
    if p.layout.is_empty() {
        body.lines(st, ui, &p.rich, width, theme::surface_rgb(&t), Align::Min);
    } else {
        body.blocks(st, ui, &p.layout, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    // Pictures without a description have no place in the text: show them after it.
    if allowed {
        let rest: Vec<&String> = p.images.iter().filter(|u| !shown.contains(u)).collect();
        if !rest.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for src in rest {
                    picture(s, st, ui, key, p, src, allowed, width, None);
                }
            });
        }
    }
}

/// What drawing a message body needs.
pub(super) struct Body<'a> {
    pub(super) s: &'a Session,
    pub(super) key: &'a MessageKey,
    pub(super) p: &'a mime::Parsed,
    pub(super) allowed: bool,
    /// Pictures drawn in place.
    pub(super) shown: &'a mut Vec<String>,
    pub(super) acts: &'a mut Vec<Action>,
}

/// Narrower than this, a table's columns go one under the other.
pub(super) const MIN_COLUMN: f32 = 140.0;

pub(super) fn span_font(sp: &mime::Span) -> egui::FontId {
    if sp.heading > 0 {
        theme::bold(match sp.heading {
            1 => 22.0,
            2 => 19.0,
            3 => 17.0,
            _ => 15.5,
        })
    } else if sp.code {
        egui::FontId::monospace(13.5)
    } else if sp.bold {
        theme::bold(14.5)
    } else {
        theme::regular(14.5)
    }
}

impl Body<'_> {
    /// Formatted lines `width` wide, on a background of `page` (for the text's contrast).
    pub(super) fn lines(&mut self, st: &mut UiState, ui: &mut Ui, lines: &mime::RichText, width: f32, page: [u8; 3], align: Align) {
        let t = st.tokens;
        for line in lines {
            if line.is_empty() {
                ui.add_space(9.0);
                continue;
            }
            // A line with right-to-left text is laid out whole, egui putting it in order (its links
            // show, but as text).
            if line.iter().any(|sp| crate::bidi::has_rtl(&sp.text)) && line.iter().all(|sp| sp.image.is_none()) {
                let mut job = egui::text::LayoutJob::default();
                for sp in line {
                    let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
                    let format = egui::TextFormat {
                        font_id: span_font(sp),
                        color: fg,
                        background: bg.unwrap_or(Color32::TRANSPARENT),
                        italics: sp.italic,
                        strikethrough: if sp.strike { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        underline: if sp.link.is_some() { egui::Stroke::new(1.0, fg) } else { egui::Stroke::NONE },
                        ..Default::default()
                    };
                    job.append(&sp.text, 0.0, format);
                }
                let r = ui
                    .scope(|ui| {
                        ui.set_max_width(width);
                        crate::bidi::add_selectable_label(ui, job)
                    })
                    .inner;
                let written: String = line.iter().map(|sp| sp.text.as_str()).collect();
                rtl_copy_menu(&r, Some(&written), &self.p.text);
                continue;
            }
            // Centred or right-aligned: pictures in the middle (or at the end), and a line that
            // fits moved over (a wrapped line keeps to the left).
            if align != Align::Min {
                if line.iter().all(|sp| sp.image.is_some() || sp.text.trim().is_empty()) {
                    ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(align), |ui| {
                        ui.set_width(width);
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
                let line_w: f32 =
                    line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
                if line.iter().all(|sp| sp.image.is_none()) && line_w < width {
                    let space = if align == Align::Center { (width - line_w) / 2.0 } else { width - line_w };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(space.max(0.0));
                        for sp in line {
                            self.span(st, ui, sp, width, page);
                        }
                    });
                    continue;
                }
            }
            let layout = egui::Layout::left_to_right(Align::BOTTOM).with_main_wrap(true);
            ui.allocate_ui_with_layout(vec2(width, 0.0), layout, |ui| {
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.x = 0.0;
                for sp in line {
                    self.span(st, ui, sp, width, page);
                }
            });
        }
    }

    /// One span: text (a link when it has one) or a picture.
    pub(super) fn span(&mut self, st: &mut UiState, ui: &mut Ui, sp: &mime::Span, width: f32, page: [u8; 3]) {
        let t = st.tokens;
        if let Some(src) = &sp.image {
            self.shown.push(src.clone());
            if !picture(self.s, st, ui, self.key, self.p, src, self.allowed, width, sp.image_size) {
                let alt = if sp.text.trim().is_empty() { "picture" } else { sp.text.trim() };
                ui.label(RichText::new(format!("[{alt}]")).color(t.faint));
            }
            return;
        }
        let mut rt = RichText::new(&sp.text).font(span_font(sp));
        if sp.italic {
            rt = rt.italics();
        }
        if sp.strike {
            rt = rt.strikethrough();
        }
        let (fg, bg) = theme::mail_colors_on(sp.color, sp.background, page, &t, if sp.link.is_some() { t.accent } else { t.text });
        if let Some(bg) = bg {
            rt = rt.background_color(bg);
        }
        match &sp.link {
            Some(link) => {
                // Hovered, a link says where it will open (cleaned, when links are).
                let shown = if self.s.prefs.clean_links { mime::clean_link(link) } else { link.clone() };
                let r = ui.add(egui::Link::new(rt.color(fg))).on_hover_text(shown);
                if r.clicked() {
                    self.acts.push(Action::OpenUrl(link.clone()));
                }
            }
            None => {
                ui.label(rt.color(fg));
            }
        }
    }

    /// Laid-out parts `width` wide.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn blocks(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        blocks: &[mime::layout::Block],
        width: f32,
        page: [u8; 3],
        align: mime::layout::Align,
        depth: usize,
    ) {
        use mime::layout::{Align as A, Block, Width};
        let main_align = |a: A| match a {
            A::Left => Align::Min,
            A::Center => Align::Center,
            A::Right => Align::Max,
        };
        for b in blocks {
            match b {
                Block::Text(lines) => self.lines(st, ui, lines, width, page, main_align(align)),
                Block::Table(table) if depth < 16 => {
                    let tw = match table.width {
                        Some(Width::Px(px)) => (px as f32).min(width),
                        Some(Width::Percent(pc)) => width * f32::from(pc) / 100.0,
                        None => width,
                    }
                    .max(1.0);
                    let indent = match table.align {
                        A::Center => (width - tw) / 2.0,
                        A::Right => width - tw,
                        A::Left => 0.0,
                    }
                    .max(0.0);
                    let tbg = theme::box_color(table.background, page, &st.tokens);
                    let page = tbg.unwrap_or(page);
                    let behind = table.background_image.as_deref().and_then(|src| self.background(st, ui.ctx(), src));
                    ui.horizontal_top(|ui| {
                        ui.add_space(indent);
                        ui.allocate_ui_with_layout(vec2(tw, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                            ui.set_width(tw);
                            let fill = tbg.map_or(Color32::TRANSPARENT, |c| Color32::from_rgb(c[0], c[1], c[2]));
                            let framed = egui::Frame::new().fill(fill).show(ui, |ui| {
                                // Its place in the drawing: above the fill, under the content.
                                let slot = ui.painter().add(egui::Shape::Noop);
                                ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
                                let columns = column_widths(&table.rows);
                                for row in &table.rows {
                                    self.row(st, ui, row, &columns, tw, page, depth);
                                }
                                slot
                            });
                            if let Some(tex) = &behind {
                                ui.painter().set(framed.inner, cover(tex, framed.response.rect));
                            }
                        });
                    });
                }
                Block::Table(_) => {}
            }
        }
    }

    /// A picture to put behind a box, when it can be shown (one inside the message, or one on the
    /// web once pictures are allowed); counted as shown either way, so it isn't added after the text.
    pub(super) fn background(&mut self, st: &mut UiState, ctx: &egui::Context, src: &str) -> Option<egui::TextureHandle> {
        if !self.shown.iter().any(|x| x == src) {
            self.shown.push(src.to_string());
        }
        match picture_texture(self.s, st, ctx, self.key, self.p, src, self.allowed) {
            Pic::Ready(tex) => Some(tex),
            Pic::Loading | Pic::Missing => None,
        }
    }

    /// A box placed over `over` (its container's rectangle); where it ends at the bottom.
    pub(super) fn overlay(&mut self, st: &mut UiState, ui: &mut Ui, o: &mime::layout::Overlay, over: egui::Rect, page: [u8; 3], depth: usize) -> f32 {
        use mime::layout::Width;
        const PAD: f32 = 4.0;
        let x = o.x as f32;
        let w = match o.width {
            Some(Width::Px(px)) => px as f32,
            Some(Width::Percent(pc)) => over.width() * f32::from(pc) / 100.0,
            // As wide as its text (CSS's shrink-to-fit), at most what's left of the box.
            None => natural_width(ui, &o.blocks).map_or(over.width() - x, |n| n + 2.0 * PAD + 1.0).min(over.width() - x).max(24.0),
        }
        .clamp(1.0, over.width().max(1.0));
        let left = if o.from_right { over.right() - x - w } else { over.left() + x }.max(over.left() - w);
        let bg = theme::box_color(o.background, page, &st.tokens);
        let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
        let draw = |this: &mut Self, st: &mut UiState, ui: &mut Ui, top: f32, measure: bool| -> f32 {
            let mut b = egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(left, top), vec2(w, f32::INFINITY)));
            if measure {
                b = b.sizing_pass().invisible();
            }
            let mut child = ui.new_child(b);
            egui::Frame::new().fill(fill).inner_margin(PAD).show(&mut child, |ui| {
                let inner = (w - 2.0 * PAD).max(1.0);
                ui.set_width(inner);
                this.blocks(st, ui, &o.blocks, inner, bg.unwrap_or(page), mime::layout::Align::Left, depth + 1);
            });
            child.min_rect().height()
        };
        // From the bottom: its height first, out of sight.
        let top = if o.from_bottom {
            let h = draw(self, st, ui, over.top(), true);
            (over.bottom() - o.y as f32 - h).max(over.top())
        } else {
            over.top() + o.y as f32
        };
        let h = draw(self, st, ui, top, false);
        top + h
    }

    /// A table row: its cells side by side, or one under the other when there's no room.
    /// `columns`: the widths the table's columns ask for (a width set in one row is the whole
    /// column's, as HTML sizes columns).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        row: &[mime::layout::Cell],
        columns: &[Option<mime::layout::Width>],
        width: f32,
        page: [u8; 3],
        depth: usize,
    ) {
        use mime::layout::Width;
        const PAD: f32 = 6.0;
        let px = |w: Width| match w {
            Width::Px(px) => px as f32,
            Width::Percent(pc) => width * f32::from(pc) / 100.0,
        };
        // Widths: what cells (or their columns) ask for, the rest shared by the others.
        let mut col = 0usize;
        let asked: Vec<Option<f32>> = row
            .iter()
            .map(|c| {
                let span = usize::from(c.span.max(1));
                let from_columns = || {
                    let parts = columns.get(col..col + span)?;
                    (parts.len() == span && parts.iter().all(Option::is_some)).then(|| parts.iter().flatten().map(|w| px(*w)).sum::<f32>())
                };
                let w = c.width.map(px).or_else(from_columns);
                col += span;
                w
            })
            .collect();
        let fixed: f32 = asked.iter().flatten().sum();
        let free = asked.iter().filter(|a| a.is_none()).count();
        let mut widths: Vec<f32> = if fixed > width || (free > 0 && width - fixed < MIN_COLUMN * free as f32) {
            // Asked for more than there is: shared out in proportion.
            let total: f32 = asked.iter().map(|a| a.unwrap_or(MIN_COLUMN)).sum::<f32>().max(1.0);
            asked.iter().map(|a| a.unwrap_or(MIN_COLUMN) / total * width).collect()
        } else {
            let each = if free > 0 { (width - fixed) / free as f32 } else { 0.0 };
            asked.iter().map(|a| a.unwrap_or(each)).collect()
        };
        // Cells that ask for nothing extra (spacers) next to wider ones.
        let used: f32 = widths.iter().sum();
        if used < width
            && let Some(last) = widths.last_mut()
        {
            *last += width - used;
        }
        let stack = row.len() > 1 && widths.iter().zip(row).any(|(w, c)| *w < MIN_COLUMN && !c.blocks.is_empty());
        let cell = |this: &mut Self, st: &mut UiState, ui: &mut Ui, c: &mime::layout::Cell, w: f32| {
            let bg = theme::box_color(c.background, page, &st.tokens);
            let fill = bg.map_or(Color32::TRANSPARENT, |b| Color32::from_rgb(b[0], b[1], b[2]));
            let behind = c.background_image.as_deref().and_then(|src| this.background(st, ui.ctx(), src));
            ui.allocate_ui_with_layout(vec2(w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(w);
                let framed = egui::Frame::new().fill(fill).inner_margin(PAD).show(ui, |ui| {
                    let slot = ui.painter().add(egui::Shape::Noop);
                    let inner = (w - 2.0 * PAD).max(1.0);
                    ui.set_width(inner);
                    this.blocks(st, ui, &c.blocks, inner, bg.unwrap_or(page), c.align, depth + 1);
                    slot
                });
                let rect = framed.response.rect;
                if let Some(tex) = &behind {
                    ui.painter().set(framed.inner, cover(tex, rect));
                }
                // Boxes placed over it; the cell grows to hold any that reach past its end.
                let mut bottom = rect.bottom();
                for o in &c.overlays {
                    bottom = bottom.max(this.overlay(st, ui, o, rect, bg.unwrap_or(page), depth));
                }
                if bottom > rect.bottom() {
                    ui.add_space(bottom - rect.bottom());
                }
            });
        };
        if stack {
            for c in row.iter().filter(|c| !c.blocks.is_empty()) {
                cell(self, st, ui, c, width);
            }
        } else {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (c, w) in row.iter().zip(&widths) {
                    cell(self, st, ui, c, *w);
                }
            });
        }
    }
}

/// Draw one picture of an HTML message, `width` the room it's in, at the size the message sets
/// (`size`); `false` when it can't be shown (yet).
#[allow(clippy::too_many_arguments)]
pub(super) fn picture(
    s: &Session,
    st: &mut UiState,
    ui: &mut Ui,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
    width: f32,
    size: Option<mime::ImageSize>,
) -> bool {
    let tex = match picture_texture(s, st, ui.ctx(), key, p, src, allowed) {
        Pic::Ready(tex) => tex,
        Pic::Loading => {
            ui.spinner();
            return true;
        }
        Pic::Missing => return false,
    };
    if let Some(at) = shown_size(tex.size_vec2(), size, width) {
        ui.add(egui::Image::new(&tex).fit_to_exact_size(at).maintain_aspect_ratio(false));
    }
    true
}

/// How big a picture of its own size `own` shows in room `width` wide: at the size the message
/// sets (both sides as set, as Outlook and browsers stretch it; one side, the other keeping its
/// shape), else its own (at most 900 wide); no wider than `max-width` or the room, its shape
/// kept. `None` for a tracking pixel or spacer that sets no size: nothing to see. (Every picture
/// showed at its own size: a picture twice as big as it says, for sharper screens, came out
/// twice as big, and what was laid out around it moved: issue #10.)
pub(crate) fn shown_size(own: egui::Vec2, size: Option<mime::ImageSize>, width: f32) -> Option<egui::Vec2> {
    use mime::layout::Width;
    let set = size.filter(|z| z.width.is_some() || z.height.is_some());
    if set.is_none() && own.x <= 3.0 && own.y <= 3.0 {
        return None;
    }
    let shape = own.y.max(1.0) / own.x.max(1.0);
    let w = set.and_then(|z| z.width).map(|w| match w {
        Width::Px(px) => px as f32,
        Width::Percent(pc) => width * f32::from(pc) / 100.0,
    });
    let h = set.and_then(|z| z.height).map(|h| h as f32);
    let (mut w, mut h) = match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * shape),
        (None, Some(h)) => (h / shape, h),
        (None, None) => (own.x, own.y),
    };
    let most = size.and_then(|z| z.max_width).map_or(f32::INFINITY, |m| m as f32).min(if set.is_some() { width } else { width.min(900.0) });
    if w > most {
        h *= most / w;
        w = most;
    }
    (w >= 0.5 && h >= 0.5).then(|| egui::vec2(w, h.min(8000.0)))
}

/// How wide formatted text is unwrapped (its widest line); `None` for anything but text.
pub(super) fn natural_width(ui: &Ui, blocks: &[mime::layout::Block]) -> Option<f32> {
    let mut widest: f32 = 0.0;
    for b in blocks {
        let mime::layout::Block::Text(lines) = b else { return None };
        for line in lines {
            if line.iter().any(|sp| sp.image.is_some()) {
                return None;
            }
            let w: f32 = line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
            widest = widest.max(w);
        }
    }
    Some(widest)
}

/// Right-click on right-to-left text (selectable too): copy the line, or the whole message, as
/// written.
pub(super) fn rtl_copy_menu(r: &egui::Response, line: Option<&str>, whole: &str) {
    r.interact(Sense::click()).context_menu(|ui| {
        if let Some(line) = line
            && ui.button(tl!("Copy")).clicked()
        {
            ui.ctx().copy_text(line.trim().to_string());
            ui.close();
        }
        if ui.button(tl!("Copy the whole message")).clicked() {
            ui.ctx().copy_text(whole.trim().to_string());
            ui.close();
        }
    });
}

/// A message's picture, as far as it can be had now.
pub(super) enum Pic {
    Ready(egui::TextureHandle),
    /// Being downloaded.
    Loading,
    /// Not in the message, not allowed yet, or not a picture.
    Missing,
}

/// The width each column of a table asks for: the first cell of one column that sets one, in any
/// row (HTML sizes a whole column so; only the first row set it in a report whose other rows then
/// lined up elsewhere).
pub(super) fn column_widths(rows: &[Vec<mime::layout::Cell>]) -> Vec<Option<mime::layout::Width>> {
    pub(super) const MAX_COLUMNS: usize = 64;
    let mut out: Vec<Option<mime::layout::Width>> = Vec::new();
    for row in rows {
        let mut col = 0usize;
        for c in row {
            let span = usize::from(c.span.max(1));
            if span == 1 && col < MAX_COLUMNS {
                if out.len() <= col {
                    out.resize(col + 1, None);
                }
                if let (Some(slot), Some(w)) = (out.get_mut(col), c.width)
                    && slot.is_none()
                {
                    *slot = Some(w);
                }
            }
            col = col.saturating_add(span);
        }
    }
    out
}

/// A picture of an HTML message: one inside it (`cid:`) always, one on the web once allowed.
pub(super) fn picture_texture(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
) -> Pic {
    // One sent with the message: by `cid:`, or by the name or address it was sent under.
    let lower = src.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        let Some(index) = mime::picture_attachment(p, src) else { return Pic::Missing };
        // At a size to read it at (an attachment's card has a thumbnail 480 wide: a picture
        // wider than that came out 480 wide, issue #10).
        let id = format!("cid-{}-{index}", key.to_id());
        if let Some(t) = st.web_pictures.get(&id) {
            return t.clone().map_or(Pic::Missing, Pic::Ready);
        }
        if st.web_pictures.len() > 200 {
            st.web_pictures.clear();
        }
        let t = s
            .store
            .as_ref()
            .and_then(|store| store.read_body(key))
            .and_then(|raw| mime::attachment_data(&raw, index))
            .and_then(|d| decode(ctx, &id, &d, 1200));
        st.web_pictures.insert(id, t.clone());
        return t.map_or(Pic::Missing, Pic::Ready);
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(mailcraft_engine::Picture::Ready(bytes)) => {
            if let Some(t) = st.web_pictures.get(src) {
                return t.clone().map_or(Pic::Missing, Pic::Ready);
            }
            if st.web_pictures.len() > 200 {
                st.web_pictures.clear();
            }
            let t = decode(ctx, &format!("web-{src}"), bytes, 1200);
            st.web_pictures.insert(src.to_string(), t.clone());
            t.map_or(Pic::Missing, Pic::Ready)
        }
        Some(mailcraft_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
}

/// A picture covering `rect`, cropped to keep its shape (CSS's `background-size: cover`).
pub(super) fn cover(tex: &egui::TextureHandle, rect: egui::Rect) -> egui::Shape {
    let sz = tex.size_vec2();
    if sz.x < 1.0 || sz.y < 1.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return egui::Shape::Noop;
    }
    let scale = (rect.width() / sz.x).max(rect.height() / sz.y);
    let part = vec2((rect.width() / (sz.x * scale)).min(1.0), (rect.height() / (sz.y * scale)).min(1.0));
    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), part);
    egui::Shape::image(tex.id(), rect, uv, Color32::WHITE)
}
