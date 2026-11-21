//! The reading pane: a message's text, laid out as its HTML lays it out, with its pictures.

use super::*;

mod marked;
pub(crate) use marked::{Marked, marked, marked_job, subject_marks, text_marks};
mod picture_menu;
mod picture_source;
#[cfg(test)]
pub(crate) use picture_menu::attach as picture_menu_for_tests;
#[cfg(test)]
pub(crate) use picture_source::on_the_web;
pub(super) use picture_source::{Pic, cover, held_back_box, held_back_words, picture_texture};
pub(crate) use picture_source::{WEB_PICTURES, shown_size};
mod table;
use table::{Room, Rows, column_widths, room_margin};

/// How deep tables are drawn inside one another, and measured inside the one measured (the
/// pictures of a long message's parts not laid out are looked for as deep).
pub(super) const MAX_NESTED: usize = 16;
/// Columns of a table sized and measured at most (those past it share what's left).
const MAX_COLUMNS: usize = 64;

/// The messages shown otherwise than the setting for all mail (File › Options, View › Plain
/// text) says: one message formatted though all mail is read as plain text, or the other way,
/// as Outlook shows one message as HTML from above it and leaves the setting as it was.
#[derive(Default)]
pub struct ShownAs {
    /// The setting these were chosen against: changed, they're forgotten.
    against: bool,
    /// Each message's key, and whether it's shown formatted.
    each: std::collections::HashMap<MessageKey, bool>,
}

/// More than this many messages switched, the oldest choices aren't worth keeping.
const SHOWN_AS_MAX: usize = 256;

impl ShownAs {
    /// Whether message `key` is shown formatted, `all` being the setting for all mail.
    pub fn formatted(&self, key: &MessageKey, all: bool) -> bool {
        if self.against != all {
            return all;
        }
        self.each.get(key).copied().unwrap_or(all)
    }

    /// Show message `key` formatted (`on`) or as plain text, whatever the setting `all` says.
    pub fn set(&mut self, key: &MessageKey, on: bool, all: bool) {
        if self.against != all || self.each.len() >= SHOWN_AS_MAX {
            self.each.clear();
            self.against = all;
        }
        if on == all {
            self.each.remove(key);
        } else {
            self.each.insert(key.clone(), on);
        }
    }
}

/// Whether message `key` (parsed as `p`) is shown formatted: it has formatting, and it's
/// shown that way, by the setting or for itself.
pub(crate) fn shown_formatted(s: &Session, st: &UiState, key: &MessageKey, p: &mime::Parsed) -> bool {
    !p.rich.is_empty() && st.shown_as.formatted(key, s.prefs.show_html)
}

/// Above an HTML message: whether its web pictures are hidden, and, shown as plain text, the way
/// back to its formatting (for this message; the setting stays). The way to plain text isn't
/// here, a click from the text: it's in More actions (⋯), View › Plain text and File › Options,
/// where Outlook keeps it too.
pub(super) fn html_bar(s: &Session, t: &Tokens, ui: &mut Ui, key: &MessageKey, p: &mime::Parsed, formatted: bool, acts: &mut Vec<Action>) {
    let web = s.remote_pictures();
    if !web.is_empty() && !s.pictures_allowed() && formatted {
        egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Image, t.dim);
                    // Shown by themselves elsewhere: why not on this one.
                    let why = if s.prefs.auto_pictures {
                        tl!("This message looks suspicious or its sender couldn't be confirmed, so its pictures stay hidden.")
                    } else {
                        tl!("Pictures from the web are hidden to protect your privacy.")
                    };
                    ui.label(RichText::new(why).color(t.dim));
                    if ui.link(tl!("Show pictures")).clicked() {
                        acts.push(Action::Run("message.showPictures", json!({})));
                    }
                    // Not for a message file opened to be looked at: who it says it's from may be
                    // anyone, so nothing is trusted on its word.
                    let who = if s.opened_file_name(key).is_some() { String::new() } else { p.from.email.clone() };
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
    let trusted = formatted && !web.is_empty() && s.pictures_allowed() && s.prefs.trusted_senders.contains(&p.from.email.to_ascii_lowercase());
    if !formatted || trusted {
        ui.horizontal(|ui| {
            if trusted {
                if ui.link(RichText::new(tl!("Stop showing pictures from this sender")).small()).clicked() {
                    acts.push(Action::Run("message.blockPictures", json!({})));
                }
            } else {
                ui.label(RichText::new(tl!("Shown as text.")).color(t.faint).small());
                if !p.rich.is_empty() && ui.link(RichText::new(tl!("Show formatted")).small()).clicked() {
                    acts.push(Action::ShowFormatted(key.clone(), true));
                }
            }
        });
        ui.add_space(6.0);
    }
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
    let mut body = Body { s, key, p, allowed, shown: &mut shown, acts, cells: None, held: None };
    if long_text::many_lines(p) {
        // Thousands of lines: laid out by what's in view.
        long_text::rich(&mut body, st, ui, ui.scope_id().with(("long-text", key.to_id())), width, theme::surface_rgb(&t));
    } else if p.layout.is_empty() {
        body.lines(st, ui, &p.rich, width, theme::surface_rgb(&t), Align::Min);
    } else {
        body.blocks(st, ui, &p.layout, width, theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    // Pictures without a description have no place in the text: show them after it.
    if allowed {
        let rest: Vec<&String> = p.images.iter().filter(|u| !body.shown.contains(u)).collect();
        if !rest.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for src in rest {
                    body.picture(st, ui, src, width, None, None);
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
    /// Blocks of text drawn as text boxes, to change (the original under a reply).
    pub(super) cells: Option<&'a mut dyn CellEdit>,
    /// What the table being drawn that's in no other holds (`None` between such tables): the
    /// tables inside it keep their measures by it.
    pub(super) held: Option<table::Held>,
}

/// Draws a block of text as a text box to change it (`false`: drawn as text after all).
pub(super) trait CellEdit {
    /// `page`: the colour it's on (its cell's), for its colours to read.
    fn edit(&mut self, ui: &mut Ui, lines: &mime::RichText, page: [u8; 3], acts: &mut Vec<Action>) -> bool;
}

/// Narrower than this, a table's columns go one under the other.
pub(super) const MIN_COLUMN: f32 = 140.0;

pub(crate) fn span_font(sp: &mime::Span) -> egui::FontId {
    let font = span_font_at_full_size(sp);
    match sp.rise {
        mime::Rise::Normal => font,
        _ => egui::FontId::new(font.size * SCRIPT_SIZE, font.family),
    }
}

/// How much smaller raised and lowered text is drawn.
const SCRIPT_SIZE: f32 = 0.72;

fn span_font_at_full_size(sp: &mime::Span) -> egui::FontId {
    // The size the message sets, one of its pixels a point here, as its layout is.
    let size = |unset: f32| sp.size.map_or(unset, f32::from);
    if sp.heading > 0 {
        theme::bold(size(match sp.heading {
            1 => 22.0,
            2 => 19.0,
            3 => 17.0,
            _ => 15.5,
        }))
    } else if sp.code {
        egui::FontId::monospace(size(13.5))
    } else if sp.bold {
        theme::bold(size(14.5))
    } else {
        theme::regular(size(14.5))
    }
}

/// How tall an empty line is: the gap a browser leaves between two paragraphs of text this
/// size (their `1em`), where a line of text is taller.
pub(super) const BLANK: f32 = 14.0;

impl Body<'_> {
    /// Formatted lines `width` wide, on a background of `page` (for the text's contrast).
    pub(super) fn lines(&mut self, st: &mut UiState, ui: &mut Ui, lines: &[Vec<mime::Span>], width: f32, page: [u8; 3], align: Align) {
        let t = st.tokens;
        for line in lines {
            if line.is_empty() {
                text_selection::blank(st);
                ui.add_space(BLANK);
                continue;
            }
            text_selection::line(st);
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
                        valign: if sp.rise == mime::Rise::Sup { Align::TOP } else { Align::BOTTOM },
                        ..Default::default()
                    };
                    job.append(&sp.text, 0.0, format);
                }
                let r = ui
                    .scope(|ui| {
                        ui.set_max_width(width);
                        text_selection::rtl_run(st, ui, job)
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
                let line_w = line_width(ui, line);
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
            if !self.picture(st, ui, src, width, sp.image_size, sp.link.as_deref()) {
                let alt = held_back_words(&sp.text);
                match held_back_box(sp.image_size, width) {
                    Some(size) => {
                        // The room it will take, with its description if that fits.
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        let galley = ui.painter().layout_no_wrap(alt, theme::regular(12.0), t.faint);
                        if galley.size().x <= rect.width() && galley.size().y <= rect.height() {
                            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.faint);
                        }
                        if size.x > 3.0 && size.y > 3.0 {
                            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.faint.gamma_multiply(0.4)), egui::StrokeKind::Inside);
                        }
                    }
                    None => {
                        ui.add(egui::Label::new(RichText::new(alt).color(t.faint)).selectable(false));
                    }
                }
            }
            return;
        }
        let mut rt = RichText::new(&sp.text).font(span_font(sp));
        // Raised: as tall as the line, so that it sits at the top of it beside the text.
        if sp.rise == mime::Rise::Sup {
            rt = rt.line_height(Some(ui.fonts_mut(|f| f.row_height(&span_font_at_full_size(sp))))).raised();
        }
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
                // Hovered, a link says where it will open (cleaned, when links are): written only
                // when it's shown, or a screen reader is told it (not for every link every frame).
                let clean = self.s.prefs.clean_links;
                let shown = || if clean { mime::clean_link(link) } else { link.clone() };
                let r = text_selection::run(st, ui, rt.color(fg), Some(fg));
                let r = if r.ctx.has_accesskit_node(r.id) {
                    r.on_hover_text(shown())
                } else {
                    // As `on_hover_text` draws it.
                    r.on_hover_ui(|ui| {
                        ui.set_max_width(ui.spacing().tooltip_width);
                        ui.add(egui::Label::new(shown()));
                    })
                };
                if r.clicked() {
                    self.acts.push(Action::OpenUrl(link.clone()));
                }
            }
            None => {
                text_selection::run(st, ui, rt.color(fg), None);
            }
        }
    }

    /// Laid-out parts `width` wide.
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
        // Blocks stacked straight one on another, as a browser stacks them: what parts them is
        // their own room (padding, spacing, a paragraph's lines), not the pane's (issue #41).
        let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
        for b in blocks {
            match b {
                Block::Text(lines) => {
                    // A text box where it's changed: not in a pass that only measures.
                    let edited = !ui.is_sizing_pass() && self.cells.as_deref_mut().is_some_and(|c| c.edit(ui, lines, page, self.acts));
                    if !edited {
                        self.lines(st, ui, lines, width, page, main_align(align));
                    }
                }
                Block::Table(table) if depth < MAX_NESTED => {
                    // What its columns hold (measured once): their widths, and its own when it
                    // sets none and is as wide as what it holds (a `<table>`, a button).
                    let (s, key, p, allowed) = (self.s, self.key, self.p, self.allowed);
                    let ctx = ui.ctx().clone();
                    // In no other table: all it holds is hashed here, once for those inside too.
                    let outer = self.held.is_none();
                    let held = self.held.take().unwrap_or_else(|| table::Held::of(ui, table));
                    let measures = table::measured_with(ui, key, Some(&held), table, allowed, &mut |sp| {
                        let Some(src) = &sp.image else { return table::PictureWidth::Alt };
                        match picture_texture(s, st, &ctx, key, p, src, allowed) {
                            Pic::Ready(tex) => {
                                table::PictureWidth::Is(shown_size(tex.size_vec2(), sp.image_size, f32::MAX / 2.0).map_or(0.0, |z| z.x))
                            }
                            Pic::Loading => table::PictureWidth::Coming,
                            Pic::Missing => table::PictureWidth::Alt,
                        }
                    });
                    self.held = Some(held);
                    let tw = match table.width {
                        Some(Width::Px(px)) => (px as f32).min(width),
                        Some(Width::Percent(pc)) => width * f32::from(pc) / 100.0,
                        None if table.fit => measures.whole.map_or(width, |f| f.max.min(width)),
                        None => width,
                    }
                    .max(1.0);
                    // Only one in no other table is scrolled sideways: a newsletter's columns,
                    // inside its page's table, still go one under the other in a narrow pane.
                    let too_wide = if depth == 0 && outer { table::sideways(table, measures.whole, width) } else { None };
                    let tw = too_wide.unwrap_or(tw);
                    // In a centred or right-aligned cell (`<td align="center">`), a table is
                    // too, as browsers do.
                    let at = table::table_place(table.align, align);
                    let indent = match at {
                        A::Center => (width - tw) / 2.0,
                        A::Right => width - tw,
                        A::Left => 0.0,
                    }
                    .max(0.0);
                    let tbg = theme::box_color(table.background, page, &st.tokens);
                    let page = tbg.unwrap_or(page);
                    let behind = table.background_image.as_deref().and_then(|src| self.background(st, ui.ctx(), src));
                    let mut draw = |ui: &mut Ui| {
                        ui.horizontal_top(|ui| {
                            ui.add_space(indent);
                            ui.allocate_ui_with_layout(vec2(tw, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                                ui.set_width(tw);
                                let fill = tbg.map_or(Color32::TRANSPARENT, |c| Color32::from_rgb(c[0], c[1], c[2]));
                                // The table's own room between and around its cells (`cellspacing`;
                                // none for a box of CSS), and inside them (`cellpadding`).
                                let spacing = f32::from(table.spacing.unwrap_or(0));
                                let room = Room { spacing, padding: f32::from(table.padding.unwrap_or(0)) };
                                let framed = egui::Frame::new().inner_margin(room_margin([spacing; 4])).show(ui, |ui| {
                                    // Its place in the drawing (its colour and picture): under the content.
                                    let slot = ui.painter().add(egui::Shape::Noop);
                                    ui.spacing_mut().item_spacing = vec2(spacing, spacing);
                                    let columns = column_widths(&table.rows);
                                    let width = (tw - 2.0 * spacing).max(1.0);
                                    let rows = Rows { columns: &columns, content: &measures.columns, width, page, room, depth };
                                    for row in &table.rows {
                                        self.row(st, ui, row, rows);
                                    }
                                    slot
                                });
                                ui.painter().set(framed.inner, table::backdrop(framed.response.rect, table.radius, fill, behind.as_ref()));
                                if let Some(l) = table.border {
                                    let stroke = egui::Stroke::new(f32::from(l.width), table::line_color(l, page, &st.tokens));
                                    ui.painter().rect_stroke(framed.response.rect, 0.0, stroke, egui::StrokeKind::Inside);
                                }
                            });
                        });
                    };
                    if too_wide.is_some() {
                        let id = ui.next_auto_id().with("wide-table");
                        // Its bar under it, not over its last row (a floating one hid its numbers).
                        ui.scope(|ui| {
                            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                            egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| draw(ui));
                        });
                    } else {
                        draw(ui);
                    }
                    if outer {
                        self.held = None;
                    }
                }
                Block::Table(_) => {}
            }
        }
        ui.spacing_mut().item_spacing.y = gap;
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

    /// Draw one picture of the message, `width` the room it's in, at the size the message sets
    /// (`size`), opening `link` when it's in one (issue #225); `false` when it can't be shown
    /// (yet).
    pub(super) fn picture(
        &mut self,
        st: &mut UiState,
        ui: &mut Ui,
        src: &str,
        width: f32,
        size: Option<mime::ImageSize>,
        link: Option<&str>,
    ) -> bool {
        let tex = match picture_texture(self.s, st, ui.ctx(), self.key, self.p, src, self.allowed) {
            Pic::Ready(tex) => tex,
            Pic::Loading => {
                ui.spinner();
                return true;
            }
            Pic::Missing => return false,
        };
        if let Some(at) = shown_size(tex.size_vec2(), size, width) {
            let resp = ui.add(egui::Image::new(&tex).fit_to_exact_size(at).maintain_aspect_ratio(false).sense(Sense::click()));
            picture_menu::attach(&resp, self.s, self.key, src, link, self.acts);
        }
        true
    }
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
            widest = widest.max(line_width(ui, line));
        }
    }
    Some(widest)
}

/// Line widths kept, by what each line says and in what fonts.
#[derive(Clone, Default)]
struct LineWidths(std::collections::HashMap<u64, f32>);

/// More line widths than this kept, they're let go and measured again as they're drawn.
const LINE_WIDTHS: usize = 8_192;

/// How wide a line of formatted text is unwrapped: its spans' words in their fonts, measured the
/// first time and kept by what they are (a newsletter's centred lines were each laid out twice a
/// frame, to measure and to draw, a sixth of its frame).
pub(crate) fn line_width(ui: &Ui, line: &[mime::Span]) -> f32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // The fonts themselves: installed again (another language's, one come for Chinese), the
    // same words are another width.
    (ui.ctx().pixels_per_point().to_bits(), table::fonts_stamp(ui)).hash(&mut h);
    for sp in line {
        // What its font is made of (span_font).
        (&sp.text, sp.heading, sp.code, sp.bold, sp.size, sp.rise == mime::Rise::Normal).hash(&mut h);
    }
    let key = h.finish();
    let store = egui::Id::unique("mailcraft-line-widths");
    if let Some(w) = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<LineWidths>(store).0.get(&key).copied()) {
        return w;
    }
    let w: f32 = line.iter().map(|sp| ui.fonts_mut(|f| f.layout_no_wrap(sp.text.clone(), span_font(sp), Color32::WHITE).size().x)).sum();
    ui.ctx().data_mut(|d| {
        let kept = d.get_temp_mut_or_default::<LineWidths>(store);
        if kept.0.len() >= LINE_WIDTHS {
            kept.0.clear();
        }
        kept.0.insert(key, w);
    });
    w
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
