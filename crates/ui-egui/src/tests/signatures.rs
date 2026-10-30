//! A signature as HTML code (issue #11): from a file saved as Outlook saves one, and back to the
//! formatted editor.

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
    let config = a.session.account("sample").unwrap().config.clone();
    assert!(config.signature_code.contains("data:image/png;base64,"), "kept with its picture");
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
