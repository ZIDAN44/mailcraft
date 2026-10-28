//! Pictures in the message being written, at the size they're sent (issue #10: every picture
//! was a thumbnail 72 points tall, whatever its size, and couldn't be made bigger or smaller).
//! A picture shows at its width (its own, else the one it was given) and keeps its shape; never
//! wider than the text. Clicked, it's selected (Delete removes it); its corner handle drags it
//! bigger or smaller (a double-click on it gives it its own size back), and its right-click menu
//! sets its size, as Outlook's do.

use super::*;

/// The widest a picture goes in at by itself (a page's width in Outlook, 6.5 inches); a wider one
/// is given this width as it's put in, and can be made bigger again.
pub(crate) const FIT_WIDTH: u32 = 624;
/// The narrowest a picture is dragged to.
const MIN_WIDTH: f32 = 16.0;
/// The largest a picture is decoded for showing in the text.
const DECODE_MAX: u32 = 1600;
/// Room kept between a picture and the text's right edge, where the scroll bar is (it took the
/// pointer from the corner handle of a picture as wide as the text).
const EDGE: f32 = 24.0;

/// Where a picture's character is in the laid-out text, and so where the picture goes.
pub(super) struct Place {
    /// The character's place (its row's height).
    pub(super) glyph: Rect,
    /// Its row's left edge, and the text's right edge.
    row_left: f32,
    right: f32,
    /// Last in its line: its room is before the character (see `layout::picture_format`)…
    last: bool,
    /// …unless the line wrapped there (egui leaves a row's leading room out): then it starts the row.
    pub(super) wrapped: bool,
}

impl Place {
    pub(super) fn of(g: &Galley, origin: egui::Pos2, at: usize, last: bool) -> Place {
        // Its own row when the line wraps just before it.
        let cursor = CCursor { prefer_next_row: true, ..CCursor::new(at) };
        let lc = g.layout_from_cursor(cursor);
        let starts_paragraph = lc.row == 0 || g.rows.get(lc.row - 1).is_none_or(|r| r.ends_with_newline);
        let row_left = g.rows.get(lc.row).map_or(0.0, |r| r.pos.x);
        Place {
            glyph: g.pos_from_cursor(cursor).translate(origin.to_vec2()),
            row_left: origin.x + row_left,
            right: origin.x + g.job.wrap.max_width,
            last,
            wrapped: lc.column.0 == 0 && !starts_paragraph,
        }
    }
}

/// A picture decoded for the text, with its own size (pixels).
pub(super) struct Pic {
    tex: egui::TextureHandle,
    size: egui::Vec2,
}

/// The own sizes of the message's pictures that can be shown (decoded once, then kept).
pub(super) fn own_sizes(ctx: &egui::Context, ed: &mut Editor, c: &Compose) -> HashMap<String, egui::Vec2> {
    let mut out = HashMap::new();
    for cid in rich::pictures(&c.body, &c.styles) {
        let pic = ed.pictures.entry(cid.clone()).or_insert_with(|| {
            let data = &c.inline.iter().find(|p| p.cid == cid)?.data;
            let size = own_size(data)?;
            let tex = crate::reading::decode(ctx, &format!("compose-{cid}"), data, DECODE_MAX)?;
            Some(Pic { tex, size: vec2(size.0 as f32, size.1 as f32) })
        });
        if let Some(p) = pic {
            out.insert(cid, p.size);
        }
    }
    out
}

/// A picture's own size in pixels, from its header (nothing decoded); `None` when it isn't a
/// picture that can be read.
pub(crate) fn own_size(data: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// The width a picture is given as it's put in: [`FIT_WIDTH`] when it's wider than that, else
/// none (its own).
pub(crate) fn fitted_width(size: Option<(u32, u32)>) -> Option<u32> {
    size.filter(|(w, _)| *w > FIT_WIDTH).map(|_| FIT_WIDTH)
}

/// A picture wider than a third of a page goes in on a line of its own, as a screenshot pasted
/// mid-sentence does in mail editors (egui wraps a row once per letter, so a second wide picture
/// after one went past the text's edge); smaller ones (icons) go in the line.
pub(crate) fn own_line(size: Option<(u32, u32)>) -> bool {
    size.is_some_and(|(w, _)| w > FIT_WIDTH / 3)
}

/// How big a picture shows in text `wrap` wide: at `width` (else its own), its shape kept, never
/// wider than the text. One that can't be read is a box of its own size.
pub(super) fn shown(own: Option<egui::Vec2>, width: Option<u16>, wrap: f32) -> egui::Vec2 {
    let own = own.filter(|s| s.x >= 1.0 && s.y >= 1.0).unwrap_or(vec2(96.0, 72.0));
    let room = if wrap.is_finite() { (wrap - EDGE).max(MIN_WIDTH) } else { f32::MAX };
    let w = width.map_or(own.x, f32::from).clamp(1.0, room);
    vec2(w, (w * own.y / own.x).clamp(1.0, 8000.0))
}

/// Picture `cid`'s style at character `at`, `width` wide.
fn set_width(c: &mut Compose, at: usize, width: Option<u16>) {
    let mut styles = std::mem::take(&mut c.styles);
    styles.restyle(&c.body, at..at + 1, |s| Style { picture_width: width, ..s.clone() });
    c.styles = styles;
}

/// Draw the picture at character `at` in its place; click, drag and right-click it.
#[allow(clippy::too_many_arguments)]
pub(super) fn picture(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &mut Compose, at: usize, cid: &str, place: Place) {
    let style = c.styles.style(at);
    let own = ed.pictures.get(cid).and_then(|p| p.as_ref().map(|p| p.size));
    let full = shown(own, style.picture_width, place.right - place.row_left);
    let left = match (place.last, place.wrapped) {
        (true, true) => place.row_left,
        (true, false) => place.glyph.min.x - full.x,
        (false, _) => place.glyph.min.x + 2.0,
    };
    // Where its room went past the text's edge (followed by text that wrapped, or after a
    // picture with no room for both), it stays within the text's width.
    let size = shown(own, style.picture_width, place.right - left + EDGE);
    let rect = Rect::from_min_size(egui::pos2(left, place.glyph.max.y - 2.0 - size.y), size);
    ed.placed.push((at, rect));
    let painter = ui.painter().clone();
    match ed.pictures.get(cid).and_then(Option::as_ref) {
        Some(p) => {
            painter.image(p.tex.id(), rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            icons::paint(&painter, icons::centered(rect, 22.0), Icon::Image, t.dim);
        }
    }
    if c.sending {
        return;
    }
    let body = body_id(c);
    let r = ui.interact(rect, egui::Id::unique(("compose-picture", c.id, at)), Sense::click());
    if r.clicked() {
        ed.sel = at..at + 1;
        ed.refocus = Some(at..at + 1);
    }
    let selected = ed.sel == (at..at + 1);
    if selected {
        painter.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
    }
    // The corner handle: dragged, the picture follows it (its shape kept). Over the picture, not
    // only hovered: with the button down on the handle the picture isn't hovered, and a drag
    // started without selecting the picture first lost its handle (#14's test found it).
    if selected || r.hovered() || ui.rect_contains_pointer(rect) || ed.resizing == Some(at) {
        // Just inside the corner, so it's on the picture wherever the picture is.
        let handle = Rect::from_center_size(rect.right_bottom() - vec2(4.0, 4.0), vec2(10.0, 10.0));
        let h = ui
            .interact(handle, egui::Id::unique(("compose-picture-size", c.id, at)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
        painter.rect(handle, 1.0, t.surface, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        // A double-click on it gives the picture its own size back (#14).
        if h.double_clicked() && c.styles.style(at).picture_width.is_some() {
            ed.remember(c, at + 1);
            set_width(c, at, None);
            ed.refocus = Some(at..at + 1);
        }
        if h.drag_started() {
            ed.remember(c, at + 1);
            ed.resizing = Some(at);
        }
        if h.dragged()
            && let Some(p) = h.interact_pointer_pos()
        {
            let w = (p.x + 4.0 - rect.left()).clamp(MIN_WIDTH, (place.right - place.row_left - EDGE).max(MIN_WIDTH));
            set_width(c, at, Some(w.round() as u16));
            ui.ctx().request_repaint();
        }
        if h.drag_stopped() {
            ed.resizing = None;
            ed.refocus = Some(at..at + 1);
        }
    }
    r.context_menu(|ui| {
        ui.set_min_width(160.0);
        let own_w = own.map_or(size.x, |s| s.x);
        let choices: [(String, Option<f32>); 4] =
            [(tl!("Original size").to_string(), None), ("75%".into(), Some(0.75)), ("50%".into(), Some(0.5)), ("25%".into(), Some(0.25))];
        for (label, part) in choices {
            if ui.button(label).clicked() {
                ed.remember(c, at + 1);
                set_width(c, at, part.map(|p| (own_w * p).round().clamp(MIN_WIDTH, f32::from(u16::MAX)) as u16));
                ed.refocus = Some(at..at + 1);
                ui.close();
            }
        }
    });
    if r.secondary_clicked() || r.clicked() {
        ui.memory_mut(|m| m.request_focus(body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture shows at its width, or its own, with its shape kept and never wider than the
    /// text; one wider than a page goes in at a page's width.
    #[test]
    fn pictures_show_at_their_width() {
        let own = Some(vec2(1200.0, 600.0));
        assert_eq!(shown(own, None, f32::INFINITY), vec2(1200.0, 600.0));
        assert_eq!(shown(own, Some(300), 800.0), vec2(300.0, 150.0));
        assert_eq!(shown(own, None, 624.0), vec2(600.0, 300.0), "the text's width at most");
        assert_eq!(shown(None, None, 800.0), vec2(96.0, 72.0), "unreadable: a box");
        assert!(shown(Some(vec2(1.0, 100_000.0)), None, 800.0).y <= 8000.0);
        assert_eq!(fitted_width(Some((2880, 1800))), Some(FIT_WIDTH));
        assert_eq!(fitted_width(Some((400, 300))), None);
        assert_eq!(fitted_width(None), None);
        assert!(own_line(Some((1200, 600))) && !own_line(Some((32, 32))) && !own_line(None));
        assert_eq!(own_size(&mailcraft_engine::sample::sample_png()).map(|s| s.0 > 0), Some(true));
        assert_eq!(own_size(b"not a picture"), None);
        let png = mailcraft_engine::sample::sample_png();
        assert_eq!(own_size(png.get(..20).unwrap_or_default()), None, "cut short (#14's case)");
    }
}
