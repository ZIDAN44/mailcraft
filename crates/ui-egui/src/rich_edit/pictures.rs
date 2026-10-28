//! Pictures in the message being written: each as big as it is (no wider than the text), with a
//! handle at its corner that resizes it, as Outlook's (issue #10: pasted pictures showed 72
//! points tall, whatever their size, and couldn't be resized).

use super::*;

/// The size the character a picture stands on is laid out in. Its row is as tall as the picture
/// (`TextFormat::line_height`), not its letters: a character as tall as a picture filled the font
/// atlas with a glyph that big.
const CHAR_PX: f32 = 14.0;
/// How big a picture shows while its size isn't known (one that can't be read).
const UNKNOWN: Vec2 = Vec2::new(PICTURE_PX * 4.0 / 3.0, PICTURE_PX);
/// The narrowest a picture can be made.
const MIN_WIDTH: f32 = 16.0;
/// Room left on each side of a picture.
const MARGIN: f32 = 2.0;
/// The most pixels a picture is decoded with for the editor (bigger ones are drawn from that).
const DECODE_PX: u32 = 1600;

/// A picture resized with its handle: its character, and how wide it now is (`None`: its own
/// width again). `started`: the first change of this drag (the one undo goes back to).
pub(super) struct Resize {
    pub at: usize,
    pub width: Option<u16>,
    pub started: bool,
}

/// A picture's own size in pixels, from its header (`None`: not a picture this reads).
fn own_size(data: &[u8]) -> Option<Vec2> {
    if data.len() > 30 << 20 {
        return None;
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    ((1..=12_000).contains(&w) && (1..=12_000).contains(&h)).then(|| vec2(w as f32, h as f32))
}

/// Each picture's own size, read once.
pub(super) fn measure(ed: &mut Editor, c: &Compose) {
    for p in &c.inline {
        if !ed.sizes.contains_key(&p.cid) {
            ed.sizes.insert(p.cid.clone(), own_size(&p.data));
        }
    }
}

/// How big the picture `style` stands for shows, with `most` points of width for it: as wide as
/// it was made, else its own width, never wider than `most`; as tall as that makes it.
pub(super) fn shown(style: &Style, sizes: &HashMap<String, Option<Vec2>>, most: f32) -> Vec2 {
    let Some(own) = style.picture.as_ref().and_then(|cid| sizes.get(cid).copied().flatten()) else { return UNKNOWN };
    let most = (most - 2.0 * MARGIN).max(MIN_WIDTH);
    let w = style.picture_width.map_or(own.x, f32::from).clamp(MIN_WIDTH, most);
    vec2(w, (w * own.y / own.x).max(1.0))
}

/// What the layout needs to make room for pictures: each one's own size, and how wide the
/// character a picture stands on is.
#[derive(Clone, Copy)]
pub(super) struct Rooms<'a> {
    pub sizes: &'a HashMap<String, Option<Vec2>>,
    pub advance: f32,
}

impl Rooms<'_> {
    /// How wide the picture's character is, for `ui`'s fonts. Laid out, as the text is: a font
    /// without the character draws another in its place (`glyph_width` had it narrower).
    pub fn advance(ui: &Ui) -> f32 {
        ui.fonts_mut(|f| f.layout_no_wrap(PICTURE.to_string(), theme::regular(CHAR_PX), Color32::TRANSPARENT).size().x)
    }

    /// The picture's character laid out as tall as the picture, invisible; and the room after
    /// it, put before what follows it. (As letter spacing it went before the character, and not
    /// at all at the start of a line.)
    pub fn room(&self, style: &Style, most: f32) -> (TextFormat, f32) {
        let size = shown(style, self.sizes, most);
        let format = TextFormat { font_id: theme::regular(CHAR_PX), line_height: Some(size.y), color: Color32::TRANSPARENT, ..Default::default() };
        (format, (size.x - self.advance).max(0.0) + 2.0 * MARGIN)
    }
}

/// The picture at character `at` drawn where its character is (`place`: the character's place),
/// sitting on the line. Hovered or selected, its edge shows with a handle at its corner: dragging
/// that resizes it (up to `most` wide), a double-click gives it its own size back.
pub(super) fn paint(ui: &mut Ui, t: &Tokens, ed: &mut Editor, c: &Compose, at: usize, place: Rect, most: f32) -> Option<Resize> {
    let style = c.styles.style(at);
    let cid = style.picture.clone()?;
    let size = shown(&style, &ed.sizes, most);
    let r = Rect::from_min_size(egui::pos2(place.min.x + MARGIN, place.max.y - size.y), size);
    let painter = ui.painter().clone();
    let tex = ed.pictures.entry(cid.clone()).or_insert_with(|| {
        c.inline.iter().find(|p| p.cid == cid).and_then(|p| crate::reading::decode(ui.ctx(), &format!("compose-{cid}"), &p.data, DECODE_PX))
    });
    let Some(tex) = tex else {
        painter.rect_stroke(r, 4.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        icons::paint(&painter, icons::centered(r, 22.0), Icon::Image, t.dim);
        return None;
    };
    painter.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    if c.sending {
        return None;
    }
    let handle = Rect::from_center_size(r.right_bottom() - vec2(4.0, 4.0), vec2(9.0, 9.0));
    let resp = ui.interact(handle.expand(3.0), egui::Id::unique(("picture-size", c.id, at)), Sense::click_and_drag());
    let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    let selected = ed.sel.start <= at && at < ed.sel.end;
    if selected || resp.hovered() || resp.dragged() || ui.rect_contains_pointer(r) {
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
        painter.rect_filled(handle, 2.0, t.accent);
        painter.rect_stroke(handle, 2.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
    }
    if resp.double_clicked() {
        return Some(Resize { at, width: None, started: true });
    }
    let p = resp.interact_pointer_pos().filter(|_| resp.dragged())?;
    // The corner stays as far from the pointer as where the handle was taken (it jumped to the
    // pointer by the handle's inset).
    let grab = if resp.drag_started() {
        let grab = ui.input(|i| i.pointer.press_origin()).map_or(0.0, |o| r.max.x - o.x);
        ui.data_mut(|d| d.insert_temp(resp.id, grab));
        grab
    } else {
        ui.data(|d| d.get_temp::<f32>(resp.id)).unwrap_or(0.0)
    };
    let w = (p.x + grab - r.min.x).round().clamp(MIN_WIDTH, (most - 2.0 * MARGIN).max(MIN_WIDTH).min(f32::from(u16::MAX)));
    Some(Resize { at, width: Some(w as u16), started: resp.drag_started() })
}

/// Make a picture as wide as it was resized to (a step undo goes back over, per drag).
pub(super) fn resize(ed: &mut Editor, c: &mut Compose, r: Resize) {
    if c.body.chars().nth(r.at) != Some(PICTURE) {
        return;
    }
    // A double-click on a picture at its own size changes nothing.
    let same = c.styles.style(r.at).picture_width == r.width;
    if r.started && !(same && r.width.is_none()) {
        ed.remember(c, ed.sel.start);
    }
    if !same {
        c.styles.restyle(&c.body, r.at..r.at + 1, |s| Style { picture_width: r.width, ..s.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> HashMap<String, Option<Vec2>> {
        HashMap::from([("pic".to_string(), Some(vec2(300.0, 200.0))), ("bad".to_string(), None)])
    }

    fn picture(width: Option<u16>) -> Style {
        Style { picture: Some("pic".into()), picture_width: width, ..Style::default() }
    }

    /// "a", a 300 × 200 picture and "b", as the editor lays them out `wrap` wide: the room from
    /// the picture's character to what follows it, and its row's height.
    fn room(width: Option<u16>, wrap: f32) -> Vec2 {
        let body = "a\u{FFFC}b";
        let mut st = Styles::plain(body);
        st.restyle(body, 1..2, |s| Style { picture: Some("pic".into()), picture_width: width, ..s.clone() });
        let ctx = egui::Context::default();
        let mut got = Vec2::ZERO;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let g = layout::layout(ui, body, &st, false, &Tokens::light(), wrap, &sizes());
            let (a, b) = (g.pos_from_cursor(CCursor::new(1)), g.pos_from_cursor(CCursor::new(2)));
            got = vec2(b.min.x - a.min.x, a.height());
        });
        out.textures_delta.clear();
        got
    }

    #[test]
    fn a_picture_shows_as_big_as_it_is_no_wider_than_the_text() {
        assert_eq!(shown(&picture(None), &sizes(), 1000.0), vec2(300.0, 200.0));
        assert_eq!(shown(&picture(None), &sizes(), 154.0), vec2(150.0, 100.0), "no wider than the text");
        assert_eq!(shown(&picture(Some(120)), &sizes(), 1000.0), vec2(120.0, 80.0), "as wide as it was made");
        assert_eq!(shown(&picture(Some(900)), &sizes(), 304.0), vec2(300.0, 200.0));
        assert_eq!(shown(&picture(Some(2)), &sizes(), 1000.0).x, MIN_WIDTH);
        let unreadable = Style { picture: Some("bad".into()), ..Style::default() };
        assert_eq!(shown(&unreadable, &sizes(), 1000.0), UNKNOWN);
        // Laid out: room for the picture and a margin each side, its row as tall as it.
        let close = |a: Vec2, b: Vec2| (a - b).length() < 1.5;
        let r = room(None, 600.0);
        assert!(close(r, vec2(304.0, 200.0)), "its own size: {r:?}");
        let r = room(Some(120), 600.0);
        assert!(close(r, vec2(124.0, 80.0)), "resized: {r:?}");
        let r = room(None, 154.0);
        assert!((r.y - 100.0).abs() < 1.5, "a narrow editor: {r:?}");
    }

    /// A drag is one step to undo; a double-click gives the picture its own size back.
    #[test]
    fn resizing_is_one_step_to_undo() {
        let body = "a\u{FFFC}b";
        let mut c = Compose { body: body.into(), styles: Styles::plain(body), ..Default::default() };
        c.styles.restyle(body, 1..2, |s| Style { picture: Some("pic".into()), ..s.clone() });
        let mut ed = Editor::new(1);
        resize(&mut ed, &mut c, Resize { at: 1, width: Some(200), started: true });
        resize(&mut ed, &mut c, Resize { at: 1, width: Some(180), started: false });
        assert_eq!((c.styles.style(1).picture_width, ed.undo.len()), (Some(180), 1));
        assert_eq!(c.styles.style(2).picture_width, None, "what follows isn't resized");
        resize(&mut ed, &mut c, Resize { at: 0, width: Some(50), started: true });
        assert_eq!((c.styles.style(0).picture_width, ed.undo.len()), (None, 1), "not a picture there");
        resize(&mut ed, &mut c, Resize { at: 1, width: None, started: true });
        resize(&mut ed, &mut c, Resize { at: 1, width: None, started: true });
        assert_eq!((c.styles.style(1).picture_width, ed.undo.len()), (None, 2));
    }

    #[test]
    fn a_pictures_own_size_comes_from_its_header() {
        let mut png = Vec::new();
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(30, 20));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(own_size(&png), Some(vec2(30.0, 20.0)));
        assert_eq!(own_size(b"not a picture"), None);
        assert_eq!(own_size(png.get(..20).unwrap_or_default()), None, "cut short");
    }
}
