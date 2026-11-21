//! A message's pictures: where each is had from (written into it, sent with it, or on the
//! web), the size it's shown at, and what takes its place when it can't be shown.

use std::hash::{Hash, Hasher};

use super::*;

/// The room a picture that can't be shown (held back, or lost) takes: the size it says, no wider
/// than the room, when it says its width in pixels (its height, or square when small, as an
/// icon is); `None` to write its description instead. (A description of 18 pixels' width came
/// out a letter to a line, and everything around it moved: issue #47.)
pub(crate) fn held_back_box(size: Option<mime::ImageSize>, width: f32) -> Option<egui::Vec2> {
    use mime::layout::Width;
    const SMALL: f32 = 64.0;
    let z = size?;
    let Some(Width::Px(w)) = z.width else { return None };
    let w = w as f32;
    let h = match z.height {
        Some(h) => h as f32,
        None if w <= SMALL => w,
        None => return None,
    };
    let shrink = (width / w).min(1.0);
    (w >= 0.5 && h >= 0.5).then(|| egui::vec2(w * shrink, (h * shrink).min(2000.0)))
}

/// What's written for a picture that can't be shown: its description in brackets, cut short (a
/// shop's is often the item's whole name).
pub(crate) fn held_back_words(alt: &str) -> String {
    const LONGEST: usize = 48;
    let alt = alt.trim();
    let alt = if alt.is_empty() { "picture" } else { alt };
    match alt.char_indices().nth(LONGEST) {
        Some((i, _)) => format!("[{}…]", alt.get(..i).unwrap_or(alt).trim_end()),
        None => format!("[{alt}]"),
    }
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

/// A message's picture, as far as it can be had now.
pub(crate) enum Pic {
    Ready(egui::TextureHandle),
    /// Being downloaded.
    Loading,
    /// Not in the message, not allowed yet, or not a picture.
    Missing,
}

/// Most web pictures kept decoded (those shown least recently go first, never one shown in this
/// frame; past them, one more is kept until it's no longer shown).
pub(crate) const WEB_PICTURES: usize = 200;

/// Whether a picture's address is on the web (`http://` or `https://`, in any case), read where
/// it starts (the address isn't copied to be lowered, for each picture every frame).
pub(crate) fn on_the_web(src: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| src.get(..scheme.len()).is_some_and(|s| s.eq_ignore_ascii_case(scheme)))
}

/// A picture of an HTML message: one inside it (`cid:`) always, one on the web once allowed.
pub(crate) fn picture_texture(
    s: &Session,
    st: &mut UiState,
    ctx: &egui::Context,
    key: &MessageKey,
    p: &mime::Parsed,
    src: &str,
    allowed: bool,
) -> Pic {
    // One of the signature drawn in the message being written (issue #90).
    if let Some(own) = super::signature_card::own_picture(ctx, src) {
        return Pic::Ready(own);
    }
    // One written into it (`data:`, issue #225).
    if let Some(pic) = written_in(st, ctx, src) {
        return pic;
    }
    // One sent with the message: by `cid:`, or by the name or address it was sent under.
    if !on_the_web(src) {
        let Some(index) = mime::picture_attachment(p, src) else { return Pic::Missing };
        // At a size to read it at (an attachment's card has a small thumbnail: a picture wider
        // than that came out that wide, issue #10), taken out with the message's others.
        return match st.pictures.get(key, index, crate::reading::Want::Body) {
            crate::reading::Got::Ready(t) => Pic::Ready(t),
            crate::reading::Got::Pending => Pic::Loading,
            crate::reading::Got::Failed => Pic::Missing,
        };
    }
    if !allowed {
        return Pic::Missing;
    }
    match s.pictures.get(src) {
        Some(mailcraft_engine::Picture::Ready(bytes)) => {
            let now = st.pictures.frame();
            if let Some(t) = st.web_pictures.get(src, now) {
                return t.map_or(Pic::Missing, Pic::Ready);
            }
            let t = decode(ctx, &format!("web-{src}"), bytes, 1200);
            st.web_pictures.keep_anyway(src.to_string(), t.clone(), now);
            t.map_or(Pic::Missing, Pic::Ready)
        }
        Some(mailcraft_engine::Picture::Loading) => Pic::Loading,
        _ => Pic::Missing,
    }
}

/// A picture covering `rect`, cropped to keep its shape (CSS's `background-size: cover`), its
/// corners `round` as its box's.
pub(crate) fn cover(tex: &egui::TextureHandle, rect: egui::Rect, round: egui::CornerRadius) -> egui::Shape {
    let sz = tex.size_vec2();
    if sz.x < 1.0 || sz.y < 1.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return egui::Shape::Noop;
    }
    let scale = (rect.width() / sz.x).max(rect.height() / sz.y);
    let part = vec2((rect.width() / (sz.x * scale)).min(1.0), (rect.height() / (sz.y * scale)).min(1.0));
    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), part);
    egui::Shape::Rect(egui::epaint::RectShape::filled(rect, round, Color32::WHITE).with_texture(tex.id(), uv))
}

/// The picture `src` is, when it's written into the message's HTML (`data:image/…;base64,…`,
/// issue #225: an Apple Mail rich link card's): part of the message, so shown as those sent with
/// it are, without asking as for one on the web; decoded once, and kept with those. `None` for
/// any other address.
fn written_in(st: &mut UiState, ctx: &egui::Context, src: &str) -> Option<Pic> {
    if !src.trim_start().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")) {
        return None;
    }
    // Kept by a digest of its address: the address is the picture, megabytes long at times.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    let name = format!("data-{:016x}", h.finish());
    let now = st.pictures.frame();
    if let Some(t) = st.web_pictures.get(&name, now) {
        return Some(t.map_or(Pic::Missing, Pic::Ready));
    }
    let t = mime::data_picture(src).and_then(|bytes| decode(ctx, &name, &bytes, 1200));
    st.web_pictures.keep_anyway(name, t.clone(), now);
    Some(t.map_or(Pic::Missing, Pic::Ready))
}
