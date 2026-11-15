//! A picture in a message, right-clicked (issue #110): copied, or saved as a file, as a browser
//! offers it.

use super::*;

/// The picture's menu on a right-click: Copy picture (its pixels, for another app to paste),
/// Save picture as…
pub(crate) fn attach(resp: &egui::Response, s: &Session, key: &MessageKey, src: &str, acts: &mut Vec<Action>) {
    // Not the signature drawn in a message being written: it's no message open here.
    if key.account == super::super::signature_card::CARD_ACCOUNT {
        return;
    }
    resp.context_menu(|ui| {
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
