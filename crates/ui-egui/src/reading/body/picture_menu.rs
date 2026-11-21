//! A picture in a message, right-clicked (issue #110): copied, or saved as a file, as a browser
//! offers it; and one in a link (issue #225: an Apple Mail rich link card), clicked to open it.

use super::*;

/// The picture's menu on a right-click: Copy picture (its pixels, for another app to paste),
/// Save picture as…; in a link, Open link and Copy link too. In a link, a click opens it, and
/// pointed at, it says where to (cleaned, when links are), as a text link does.
pub(crate) fn attach(resp: &egui::Response, s: &Session, key: &MessageKey, src: &str, link: Option<&str>, acts: &mut Vec<Action>) {
    // Not the signature drawn in a message being written: it's no message open here.
    if key.account == super::super::signature_card::CARD_ACCOUNT {
        return;
    }
    let shown = link.map(|l| if s.prefs.clean_links { mime::clean_link(l) } else { l.to_string() });
    if let (Some(link), Some(shown)) = (link, &shown) {
        let r = resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(shown);
        if r.clicked() {
            acts.push(Action::OpenUrl(link.to_string()));
        }
    }
    resp.context_menu(|ui| {
        if let (Some(link), Some(shown)) = (link, &shown) {
            if ui.button(tl!("Open link")).clicked() {
                acts.push(Action::OpenUrl(link.to_string()));
                ui.close();
            }
            if ui.button(tl!("Copy link")).clicked() {
                ui.ctx().copy_text(shown.clone());
                ui.close();
            }
            ui.separator();
        }
        if ui.button(tl!("Copy picture")).clicked() {
            match s.picture_bytes(key, src).and_then(|b| crate::reading::picture_pixels(&b, COPIED_SIDE, COPIED_SIDE)) {
                Some((image, _)) => ui.ctx().copy_image(image),
                None => acts.push(Action::Notice(tl!("The picture couldn't be copied").to_string())),
            }
            ui.close();
        }
        if ui.button(tl!("Save picture as…")).clicked() {
            acts.push(Action::SavePicture { key: key.clone(), src: src.to_string() });
            ui.close();
        }
    });
}

/// The longest side a copied picture keeps, as a picture is drawn at most (a bigger one is
/// scaled down to it).
const COPIED_SIDE: u32 = 8192;
