//! Signatures (issue #11): several by name, made, renamed and deleted in the editor, with one
//! for new messages and one for replies; picked from the message's Signature menu; and as HTML
//! code, from a file saved as Outlook saves one, and back to the formatted editor.

use super::*;
use std::path::{Path, PathBuf};

/// Services whose file picker gives this file.
struct Picks(PathBuf);

impl crate::Services for Picks {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        vec![self.0.clone()]
    }
    fn save_path(&mut self, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
}

fn form(a: &MailApp) -> &crate::dialogs::SignatureForm {
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_ref() else { panic!("the editor is open") };
    f
}

/// Outlook's `.htm`: Western characters, VML around its picture, the picture in the folder
/// beside it; one that points out of that folder isn't read. Saved, it's the account's code.
#[test]
fn an_outlook_signature_file_becomes_the_code() {
    let (mut a, dir) = app("signature-file");
    let ctx = egui::Context::default();
    let folder = dir.join("Signatures");
    std::fs::create_dir_all(folder.join("Andr\u{e9} Moss_files")).unwrap();
    let png = mailcraft_engine::sample::sample_png();
    std::fs::write(folder.join("Andr\u{e9} Moss_files").join("image001.png"), &png).unwrap();
    std::fs::write(dir.join("secret.png"), &png).unwrap();
    let mut htm = b"<html xmlns:v=\"urn:schemas-microsoft-com:vml\"><head><meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\">\
        <style>p.MsoNormal{margin:0}</style></head><body lang=EN-US><div class=WordSection1><table><tr><td>\
        <!--[if gte vml 1]><v:shape><v:imagedata src=\"Andr%E9%20Moss_files/image001.png\"/></v:shape><![endif]-->\
        <![if !vml]><img width=80 height=80 src=\"Andr%E9%20Moss_files/image001.png\"><![endif]></td>\
        <td><p class=MsoNormal><b>Andr\xe9 Moss</b></p><p class=MsoNormal>Caf\xe9 \x96 Sales</p>\
        <img src=\"../secret.png\"></td></tr></table></div></body></html>"
        .to_vec();
    htm.retain(|b| *b != b'\r');
    let file = folder.join("Andr\u{e9} Moss.htm");
    std::fs::write(&file, &htm).unwrap();
    a.services = Box::new(Picks(file));
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    a.act(crate::Action::SignatureFile);
    let f = form(&a);
    let code = f.code.clone().expect("in code");
    assert!(code.starts_with("<div class=WordSection1>") && code.contains("Andr\u{e9} Moss") && code.contains("Caf\u{e9} \u{2013} Sales"), "{code}");
    assert!(code.contains("src=\"picture:1\"") && !code.contains("v:imagedata") && !code.contains("<![if"), "{code}");
    assert_eq!(f.pictures.len(), 1, "the one beside it, not the one outside its folder");
    assert!(code.contains("src=\"file:../secret.png\"") || code.contains("src=\"../secret.png\""), "left as written: {code}");

    // To the formatted editor and back: the text and look, then the code again.
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!() };
    let code = mailcraft_engine::mime::code_with_pictures(f.code.as_deref().unwrap_or(""), &f.pictures);
    a.run("account.setSignature", json!({"account": "sample", "signature": "", "code": code}));
    let config = a.session.default_signature("sample", false).unwrap();
    assert!(config.code.contains("data:image/png;base64,"), "kept with its picture");
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!() };
    f.switch(false);
    assert!(f.code.is_none() && f.compose.body.contains("Andr\u{e9} Moss") && f.compose.inline.len() == 1, "{:?}", f.compose.body);
    f.switch(true);
    assert!(f.code.as_deref().is_some_and(|c| c.contains("picture:1")), "{:?}", f.code);
    // Opened again later: in code, as saved.
    a.ui.dialog = Dialog::signature(&a.session);
    assert!(form(&a).code.as_deref().is_some_and(|c| c.contains("WordSection1")));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Click the button a screen reader names `label` (the frame after shows what it did).
fn click(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
    let at = nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label) && n.bounds().is_some() && !n.is_disabled())
        .and_then(|(_, n)| n.bounds())
        .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        .unwrap_or_else(|| panic!("no {label} to click"));
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
    frame(ctx, a, vec![]);
}

fn form_mut(a: &mut MailApp) -> &mut crate::dialogs::SignatureForm {
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!("the editor is open") };
    f
}

/// Issue #11: in the editor, a second signature is made (the first kept as it was), chosen for
/// replies, renamed (the defaults following it) and deleted; "(none)" is a choice too.
#[test]
fn signatures_are_made_renamed_and_chosen_in_the_editor() {
    let (mut a, dir) = app("signatures-editor");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&a).name, "Signature", "the one from before");
    assert!(form(&a).compose.body.contains("Riley Moreno"));
    click(&ctx, &mut a, "New");
    let f = form_mut(&mut a);
    assert_eq!((f.editing.as_deref(), f.name.as_str()), (None, "Signature 2"), "a name not taken");
    f.name = "Short".into();
    f.compose.set_text("Riley".into());
    f.for_reply = Some("Short".into());
    click(&ctx, &mut a, "Save");
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signatures.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Signature", "Short"]);
    assert_eq!(config.signature_named("Signature").map(|g| g.text.as_str()), Some("Riley Moreno · Studio Lantern"), "kept as it was");
    assert_eq!((config.signature_new.as_deref(), config.signature_reply.as_deref()), (Some("Signature"), Some("Short")));

    // Renamed, then another made: the rename is saved on switching, the defaults follow it.
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    form_mut(&mut a).name = "Full".into();
    click(&ctx, &mut a, "New");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_new.as_deref(), Some("Full"), "{:?}", config.signatures);
    assert_eq!(form(&a).names, ["Short", "Full"]);
    assert_eq!(form(&a).for_new.as_deref(), Some("Full"));
    // Deleted: gone, and none goes into replies.
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!() };
    f.load(&a.session, Some("Short"));
    click(&ctx, &mut a, "Delete");
    assert!(a.session.account("sample").unwrap().config.signature_named("Short").is_none());
    assert_eq!(form(&a).for_reply, None);
    assert_eq!(form(&a).name, "Full", "the next one to edit");
    form_mut(&mut a).for_new = None;
    click(&ctx, &mut a, "Save");
    assert_eq!(a.session.account("sample").unwrap().config.signature_new, None, "(none) for new messages");
    a.run("mail.new", json!({}));
    assert_eq!(a.session.compose.as_ref().unwrap().body, "");
    let _ = std::fs::remove_dir_all(dir);
}

/// The message's Signature menu lists the account's signatures: one picked takes the place of
/// the one in the message.
#[test]
fn the_signature_menu_lists_the_accounts_signatures() {
    let (mut a, dir) = app("signatures-menu");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.new_mail_window = false;
    a.run("account.setSignature", json!({"account": "sample", "name": "Short", "signature": "R."}));
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    click(&ctx, &mut a, "Signature");
    click(&ctx, &mut a, "Short");
    let c = a.session.compose.as_ref().unwrap();
    assert!(c.body.ends_with("-- \nR.") && c.body.matches("-- ").count() == 1, "{:?}", c.body);
    assert_eq!(c.signature_name, "Short");
    // From another account: its signature takes the place of the one picked (issue #11).
    a.run("account.add", json!({"email": "desk@studio.example", "password": "pw", "name": "Front desk", "deferSignIn": true}));
    let desk = a.session.accounts.iter().find(|x| x.config.email == "desk@studio.example").map(|x| x.config.id.clone()).unwrap();
    a.run("account.setSignature", json!({"account": desk, "name": "Desk", "signature": "Front desk"}));
    frame(&ctx, &mut a, vec![]);
    click(&ctx, &mut a, "From");
    click(&ctx, &mut a, "Front desk <desk@studio.example>");
    let c = a.session.compose.as_ref().unwrap();
    assert_eq!(c.account, desk);
    assert!(c.body.ends_with("-- \nFront desk") && c.body.matches("-- ").count() == 1, "{:?}", c.body);
    let _ = std::fs::remove_dir_all(dir);
}

/// Renamed to another signature's name, it isn't saved (it would take that one's place), nor
/// left behind by switching: the editor says why and waits.
#[test]
fn a_signature_cant_take_anothers_name() {
    let (mut a, dir) = app("signatures-taken");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("account.setSignature", json!({"account": "sample", "name": "Short", "signature": "R."}));
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    form_mut(&mut a).name = "short".into();
    assert!(form(&a).problem().is_some());
    click(&ctx, &mut a, "New");
    assert_eq!(form(&a).name, "short", "still this one, to put right");
    frame(&ctx, &mut a, vec![]);
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_named("Short").map(|g| g.text.as_str()), Some("R."), "the other one untouched");
    assert_eq!(config.signatures.len(), 2);
    form_mut(&mut a).name = "Long".into();
    click(&ctx, &mut a, "Save");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signatures.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Long", "Short"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// An account's first signature shows for new messages and replies alike, as it's saved; one
/// chosen for new messages only, "(none)" for replies, is saved so (it went into both).
#[test]
fn the_first_signature_is_saved_with_the_defaults_chosen() {
    let (mut a, dir) = app("signatures-first");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("account.removeSignature", json!({"account": "sample", "name": "Signature"}));
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    assert_eq!((form(&a).for_new.clone(), form(&a).for_reply.clone()), (None, None), "nothing written yet");
    form_mut(&mut a).compose.set_text("Riley".into());
    frame(&ctx, &mut a, vec![]);
    let f = form(&a);
    assert_eq!((f.for_new.as_deref(), f.for_reply.as_deref()), (Some("Signature"), Some("Signature")), "shown as it'll be saved");
    // Replies: "(none)", as picked.
    let f = form_mut(&mut a);
    f.for_reply = None;
    f.picked = true;
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&a).for_reply, None, "kept as picked");
    click(&ctx, &mut a, "Save");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!((config.signature_new.as_deref(), config.signature_reply.as_deref()), (Some("Signature"), None));
    let _ = std::fs::remove_dir_all(dir);
}
