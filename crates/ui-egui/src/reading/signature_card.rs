//! A signature written as HTML code, in the message being written (issue #90): drawn as the
//! reading pane draws it (its columns, colours, lines and pictures), where its text stands, so
//! what's written looks as it goes. Its text stays in the message, unseen (the editor's
//! [`rich_edit::Card`]): what's sent, and every place in the text, are what they were. A
//! signature changed in the message, or one in a message sent as plain text, is its text again.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use mailcraft_engine::Compose;

use super::*;

/// The signature's pictures, decoded, by the `cid:` its code names each under.
type OwnPictures = Arc<HashMap<String, egui::TextureHandle>>;

/// A signature's code laid out, for one message being written.
pub(crate) struct CardView {
    /// What it was laid out from: the code and what tells this message's pictures apart.
    stamp: u64,
    blocks: Vec<mime::layout::Block>,
    pictures: OwnPictures,
    /// How big it was drawn, in how wide a text.
    size: Option<(f32, egui::Vec2)>,
    /// It names pictures on the web, and whether they were asked for.
    web: bool,
    asked: bool,
}

/// Per message being written.
pub(crate) type SignatureCards = HashMap<u64, CardView>;

/// The account the signature's stand-in message is drawn under (none of the user's).
pub(super) const CARD_ACCOUNT: &str = "signature-card";

/// The room kept before it has been drawn once.
const FIRST_GUESS: egui::Vec2 = vec2(480.0, 96.0);

fn own_id() -> egui::Id {
    egui::Id::unique("signature-card-pictures")
}

/// One of the pictures of the signature being drawn, for the reading pane's drawing: `src` as
/// its code names it.
pub(super) fn own_picture(ctx: &egui::Context, src: &str) -> Option<egui::TextureHandle> {
    let cid = src.strip_prefix("cid:")?;
    let own: OwnPictures = ctx.data(|d| d.get_temp(own_id()))?;
    own.get(cid).cloned()
}

/// Before the editor is drawn: the part of the text the signature's code stands for, and the
/// room it took when it was last drawn.
pub(super) fn place(st: &mut UiState, ui: &Ui, c: &Compose, ed: &mut rich_edit::Editor) {
    ed.cards.retain(|k| k.what != rich_edit::CardFor::Signature);
    let Some(range) = c.signature_card() else {
        st.signature_cards.remove(&c.id);
        return;
    };
    let mut h = std::hash::DefaultHasher::new();
    (&c.signature_code, &c.signature_tag, c.id).hash(&mut h);
    let stamp = h.finish();
    if st.signature_cards.get(&c.id).is_none_or(|v| v.stamp != stamp) {
        let (html, pictures) = c.signature_card_parts();
        let blocks = mime::layout::html_layout(&html);
        let pictures = pictures
            .iter()
            .filter_map(|p| Some((p.cid.clone(), decode(ui.ctx(), &format!("signature-card-{}", p.cid), &p.data, 1200)?)))
            .collect::<HashMap<_, _>>();
        let web = mime::layout::text_blocks(&blocks)
            .iter()
            .flat_map(|(lines, _)| lines.iter().flatten())
            .filter_map(|span| span.image.as_deref())
            .any(|src| src.starts_with("https://") || src.starts_with("http://"));
        st.signature_cards.insert(c.id, CardView { stamp, blocks, pictures: Arc::new(pictures), size: None, web, asked: false });
    }
    // Code with nothing to lay out (a line of text): its text, as ever.
    let Some(view) = st.signature_cards.get(&c.id).filter(|v| !v.blocks.is_empty()) else { return };
    let Some(whole) = c.signature_place() else { return };
    ed.cards.push(rich_edit::Card { what: rich_edit::CardFor::Signature, range, whole, size: view.size.map_or(FIRST_GUESS, |(_, size)| size) });
}

/// After the editor: the signature drawn in the room the editor kept for it.
pub(super) fn show(s: &Session, st: &mut UiState, ui: &mut Ui, c: &Compose, ed: &mut rich_edit::Editor, acts: &mut Vec<Action>) {
    let rect = ed.card_rects.iter().find(|(what, _)| *what == rich_edit::CardFor::Signature).map(|(_, r)| *r);
    let card = ed.cards.iter().find(|k| k.what == rich_edit::CardFor::Signature).cloned();
    let (Some(rect), Some(card)) = (rect, card) else { return };
    let Some(mut view) = st.signature_cards.remove(&c.id) else { return };
    let t = st.tokens;
    // A message of its own to the drawing: none of its pictures is another message's.
    let key = MessageKey { account: CARD_ACCOUNT.into(), folder: String::new(), validity: 0, uid: u32::try_from(c.id & 0xFFFF_FFFF).unwrap_or(0) };
    let parsed = mime::Parsed::default();
    let mut shown: Vec<String> = Vec::new();
    ui.ctx().data_mut(|d| d.insert_temp(own_id(), view.pictures.clone()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(Align::Min)));
    // Within its room: drawn before its size is known, it isn't over the text under it.
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    {
        // The pictures it carries are shown; those it names on the web, once asked for (below).
        let mut body = Body { s, key: &key, p: &parsed, allowed: view.asked, shown: &mut shown, acts, cells: None, held: None };
        body.blocks(st, &mut child, &view.blocks, rect.width(), theme::surface_rgb(&t), mime::layout::Align::Left, 0);
    }
    ui.ctx().data_mut(|d| d.remove::<OwnPictures>(own_id()));
    let drawn = child.min_rect().size();
    // One thing in the text: a click puts the cursor after it.
    let over = ui
        .interact(Rect::from_min_size(rect.min, drawn), egui::Id::unique(("signature-card", c.id)), Sense::click())
        .on_hover_text(tl!("Your signature as it will be sent. To change it, pick Signature above the message."));
    if over.clicked() && !c.sending {
        ed.refocus = Some(card.range.end..card.range.end);
    }
    // Pictures on the web are fetched when the user asks, as for a message that's read.
    if view.web && !view.asked && !c.sending && child.link(tl!("Show pictures")).clicked() {
        view.asked = true;
        acts.push(Action::Run("compose.signaturePictures", json!({})));
    }
    let used = child.min_rect().size();
    let size = vec2(used.x.clamp(1.0, rect.width().max(1.0)), used.y.clamp(1.0, 4000.0));
    // Its room is its size from the next frame on.
    if (size - card.size).length() > 0.5 {
        ui.ctx().request_repaint();
    }
    st.signature_cards.insert(c.id, CardView { size: Some((rect.width(), size)), ..view });
}
