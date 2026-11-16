//! Signatures (issue #11): several by name, made, renamed and deleted in the editor, with one
//! for new messages and one for replies; picked from the message's Signature menu; and as HTML
//! code, from a file saved as Outlook saves one, and back to the formatted editor.

use super::*;
use std::path::PathBuf;

/// Services whose file picker gives this file.
struct PicksFile(PathBuf);

impl crate::Services for PicksFile {
    stub_services!(clock, save_path, open_path);
    fn pick_files(&mut self) -> Vec<PathBuf> {
        vec![self.0.clone()]
    }
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
    a.services = Box::new(PicksFile(file));
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    a.act(crate::Action::SignatureFile);
    let f = form(&a);
    let code = f.code.clone().expect("in code");
    assert!(code.starts_with("<div class=WordSection1>") && code.contains("Andr\u{e9} Moss") && code.contains("Caf\u{e9} \u{2013} Sales"), "{code}");
    assert!(code.contains("src=\"picture:1\"") && !code.contains("v:imagedata") && !code.contains("<![if"), "{code}");
    assert_eq!(f.pictures.len(), 1, "the one beside it, not the one outside its folder");
    assert!(code.contains("<img src=\"file:../secret.png\">"), "left as written, as a file it may not read: {code}");

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
}

fn form_mut(a: &mut MailApp) -> &mut crate::dialogs::SignatureForm {
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!("the editor is open") };
    f
}

/// Issue #11: in the editor, a second signature is made (the first kept as it was), chosen for
/// replies, renamed (the defaults following it) and deleted; "(none)" is a choice too.
#[test]
fn signatures_are_made_renamed_and_chosen_in_the_editor() {
    let (mut a, _) = app("signatures-editor");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(form(&a).name, "Signature", "the one from before");
    assert!(form(&a).compose.body.contains("Riley Moreno"));
    click_named(&ctx, &mut a, "New");
    let f = form_mut(&mut a);
    assert_eq!((f.editing.as_deref(), f.name.as_str()), (None, "Signature 2"), "a name not taken");
    f.name = "Short".into();
    f.compose.set_text("Riley".into());
    f.for_reply = Some("Short".into());
    click_named(&ctx, &mut a, "Save");
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signatures.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Signature", "Short"]);
    assert_eq!(config.signature_named("Signature").map(|g| g.text.as_str()), Some("Riley Moreno · Studio Lantern"), "kept as it was");
    assert_eq!((config.signature_new.as_deref(), config.signature_reply.as_deref()), (Some("Signature"), Some("Short")));

    // Renamed, then another made: the rename is saved on switching, the defaults follow it.
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    form_mut(&mut a).name = "Full".into();
    click_named(&ctx, &mut a, "New");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_new.as_deref(), Some("Full"), "{:?}", config.signatures);
    assert_eq!(form(&a).names, ["Short", "Full"]);
    assert_eq!(form(&a).for_new.as_deref(), Some("Full"));
    // Deleted: gone, and none goes into replies.
    let Some(Dialog::Signature(f)) = a.ui.dialog.as_mut() else { panic!() };
    f.load(&a.session, Some("Short"));
    click_named(&ctx, &mut a, "Delete");
    assert!(a.session.account("sample").unwrap().config.signature_named("Short").is_none());
    assert_eq!(form(&a).for_reply, None);
    assert_eq!(form(&a).name, "Full", "the next one to edit");
    form_mut(&mut a).for_new = None;
    click_named(&ctx, &mut a, "Save");
    assert_eq!(a.session.account("sample").unwrap().config.signature_new, None, "(none) for new messages");
    a.run("mail.new", json!({}));
    assert_eq!(a.session.compose.as_ref().unwrap().body, "");
}

/// The message's Signature menu lists the account's signatures: one picked takes the place of
/// the one in the message.
#[test]
fn the_signature_menu_lists_the_accounts_signatures() {
    let (mut a, _) = app("signatures-menu");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.new_mail_window = false;
    a.run("account.setSignature", json!({"account": "sample", "name": "Short", "signature": "R."}));
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Signature");
    click_named(&ctx, &mut a, "Short");
    let c = a.session.compose.as_ref().unwrap();
    assert!(c.body.ends_with("-- \nR.") && c.body.matches("-- ").count() == 1, "{:?}", c.body);
    assert_eq!(c.signature_name, "Short");
    // From another account: its signature takes the place of the one picked (issue #11).
    a.run("account.add", json!({"email": "desk@studio.example", "password": "pw", "name": "Front desk", "deferSignIn": true}));
    let desk = a.session.accounts.iter().find(|x| x.config.email == "desk@studio.example").map(|x| x.config.id.clone()).unwrap();
    a.run("account.setSignature", json!({"account": desk, "name": "Desk", "signature": "Front desk"}));
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "From");
    click_named(&ctx, &mut a, "Front desk <desk@studio.example>");
    let c = a.session.compose.as_ref().unwrap();
    assert_eq!(c.account, desk);
    assert!(c.body.ends_with("-- \nFront desk") && c.body.matches("-- ").count() == 1, "{:?}", c.body);
}

/// Renamed to another signature's name, it isn't saved (it would take that one's place), nor
/// left behind by switching: the editor says why and waits.
#[test]
fn a_signature_cant_take_anothers_name() {
    let (mut a, _) = app("signatures-taken");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("account.setSignature", json!({"account": "sample", "name": "Short", "signature": "R."}));
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    frame(&ctx, &mut a, vec![]);
    form_mut(&mut a).name = "short".into();
    assert!(form(&a).problem().is_some());
    click_named(&ctx, &mut a, "New");
    assert_eq!(form(&a).name, "short", "still this one, to put right");
    frame(&ctx, &mut a, vec![]);
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_named("Short").map(|g| g.text.as_str()), Some("R."), "the other one untouched");
    assert_eq!(config.signatures.len(), 2);
    form_mut(&mut a).name = "Long".into();
    click_named(&ctx, &mut a, "Save");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!(config.signatures.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Long", "Short"]);
}

/// An account's first signature shows for new messages and replies alike, as it's saved; one
/// chosen for new messages only, "(none)" for replies, is saved so (it went into both).
#[test]
fn the_first_signature_is_saved_with_the_defaults_chosen() {
    let (mut a, _) = app("signatures-first");
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
    click_named(&ctx, &mut a, "Save");
    let config = a.session.account("sample").unwrap().config.clone();
    assert_eq!((config.signature_new.as_deref(), config.signature_reply.as_deref()), (Some("Signature"), None));
}

/// Issue #90: a signature written as HTML code shows in the message being written as it's sent,
/// its columns side by side, where its text stands; the text itself stays in the message (what's
/// sent is made from it) and isn't drawn. It was its text, one part under another. Sent as plain
/// text, or changed in the message, it's its text again.
#[test]
fn a_signature_in_columns_is_drawn_in_columns_while_writing() {
    const CODE: &str = r#"<table width="560" cellspacing="0" cellpadding="0" style="width:560px"><tr>
<td width="120" style="width:120px">STUDIO</td>
<td width="200" style="width:200px;padding:0 16px 0 12px"><b>Ms. Riley Moreno</b><br>riley@studio.example</td>
<td style="padding:0 0 0 16px;border-left:1px solid #E30613;font-size:11px">12 Harbour Road<br>Portside</td></tr></table>"#;
    let (mut a, _) = app("signature-card");
    let ctx = egui::Context::default();
    a.run("account.setSignature", json!({"account": "sample", "name": "Card", "signature": "", "code": CODE}));
    a.run("account.signatureDefaults", json!({"account": "sample", "new": "Card"}));
    a.run("mail.new", json!({}));
    let size = egui::vec2(1280.0, 800.0);
    // Where a piece of text was drawn by itself (the editor's text is one piece, all of it).
    let piece = |a: &mut MailApp, text: &str| {
        // (Its room is its size from the frame after it's first drawn.)
        for _ in 0..3 {
            drawn_text(&ctx, a, size);
        }
        drawn_text(&ctx, a, size).into_iter().find(|d| d.text.trim() == text).map(|d| d.ink)
    };
    let body = a.session.compose.as_ref().unwrap().body.clone();
    assert!(body.contains("Ms. Riley Moreno") && body.contains("12 Harbour Road"), "its text is in the message: {body:?}");
    assert!(a.session.compose.as_ref().unwrap().signature_card().is_some());
    let (name, address) = (piece(&mut a, "Ms. Riley Moreno").expect("the name, laid out"), piece(&mut a, "12 Harbour Road").expect("the address"));
    assert!(address.left() > name.right() && (address.top() - name.top()).abs() < 12.0, "side by side: {name:?} {address:?}");
    assert_eq!(a.session.compose.as_ref().unwrap().body, body, "drawing it changes nothing");
    // The cursor doesn't stop in its unseen text: put there, it's after the signature.
    let card = a.session.compose.as_ref().unwrap().signature_card().unwrap();
    assert_eq!(card.end, body.chars().count());
    // As plain text, the message goes with the signature's text: shown so.
    a.run("compose.format", json!({"plain": true}));
    assert!(a.session.compose.as_ref().unwrap().signature_card().is_none());
    assert!(piece(&mut a, "12 Harbour Road").is_none(), "its text, in the editor");
    a.run("compose.format", json!({"plain": false}));
    assert!(piece(&mut a, "12 Harbour Road").is_some());
    // Changed in the message, it's the writer's text from then on.
    let changed = body.replace("Harbour", "Harbor");
    a.run("compose.update", json!({"body": changed}));
    assert!(a.session.compose.as_ref().unwrap().signature_card().is_none());
    assert!(piece(&mut a, "12 Harbour Road").is_none());
}

/// The drawn signature is one thing in the text (issue #90): Backspace after it, or Delete
/// before it, takes all of it, its `-- ` line too (letter by letter, the rest of its unseen
/// text came back as text); Undo brings it back. In a reply it's above the original, and text
/// under the signature stays under it. Pictures it names on the web are fetched only once
/// asked for.
#[test]
fn the_drawn_signature_is_one_thing_in_the_text() {
    const CODE: &str = r#"<table width="560" cellspacing="0" cellpadding="0" style="width:560px"><tr>
<td width="120" style="width:120px"><img src="https://studio.example/logo.png" width="96" height="40" alt="Studio"></td>
<td width="200" style="width:200px"><b>Ms. Riley Moreno</b><br>riley@studio.example</td>
<td style="padding:0 0 0 16px;border-left:1px solid #E30613;font-size:11px">12 Harbour Road<br>Portside</td></tr></table>"#;
    let (mut a, _) = app("signature-card-keys");
    // (Written beside the list, where one editor draws it: a window of its own draws it the same.)
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    a.run("account.setSignature", json!({"account": "sample", "name": "Card", "signature": "", "code": CODE}));
    a.run("account.signatureDefaults", json!({"account": "sample", "new": "Card", "reply": "Card"}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"body": format!("Hello Ana,{}", a.session.compose.as_ref().unwrap().body)}));
    let size = egui::vec2(1280.0, 800.0);
    let draw = |a: &mut MailApp, events: Vec<egui::Event>| frame(&ctx, a, events);
    let compose = |a: &MailApp| a.session.compose.clone().unwrap();
    let whole = compose(&a).signature_place().expect("the signature, whole");
    let with_it = compose(&a).body;
    // The cursor after it, in the text box; then Backspace.
    let put_cursor = |a: &mut MailApp, at: usize| {
        draw(a, vec![]);
        a.ui.editor.as_mut().unwrap().refocus = Some(at..at);
        draw(a, vec![]);
        draw(a, vec![]);
    };
    put_cursor(&mut a, whole.end);
    draw(&mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    let without = compose(&a).body;
    assert_eq!(without, "Hello Ana,\n", "the signature and its line are gone, the text stays: {without:?}");
    assert!(compose(&a).signature_card().is_none());
    draw(&mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(compose(&a).body, with_it, "Undo brings it back");
    assert!(compose(&a).signature_card().is_some());
    // Delete before it does the same.
    put_cursor(&mut a, whole.start - 1);
    draw(&mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(compose(&a).body, "Hello Ana,\n");
    draw(&mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    // The cursor doesn't rest in it: put in its unseen text, it's after it.
    put_cursor(&mut a, whole.start + 6);
    assert_eq!(a.ui.editor.as_ref().unwrap().sel, whole.end..whole.end);

    // Its picture on the web: nothing is asked of the web until Show pictures.
    let pieces = |a: &mut MailApp| {
        for _ in 0..3 {
            drawn_text(&ctx, a, size);
        }
        drawn_text(&ctx, a, size)
    };
    assert!(pieces(&mut a).iter().any(|d| d.text == "Show pictures"));
    assert!(!a.session.take_jobs().iter().any(|j| matches!(j, mailcraft_engine::model::Job::FetchUrl { .. })));
    a.run("compose.signaturePictures", json!({}));
    let jobs = a.session.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [mailcraft_engine::model::Job::FetchUrl { url }] if url == "https://studio.example/logo.png"),
        "its one picture, asked for: {jobs:?}"
    );
    a.run("compose.discard", json!({}));

    // In a reply: the signature above the original, drawn in its columns.
    let key_of = a.session.list.iter().find(|m| m.subject.contains("Storyboard")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key_of.to_id()}));
    a.run("mail.reply", json!({}));
    assert!(compose(&a).signature_card().is_some(), "{:?}", compose(&a).body);
    let drawn = pieces(&mut a);
    let top = |text: &str| drawn.iter().find(|d| d.text.trim() == text).map(|d| d.ink.top());
    let (name, address) = (top("Ms. Riley Moreno").expect("the name"), top("12 Harbour Road").expect("the address"));
    assert!((name - address).abs() < 12.0, "side by side: {name} {address}");
    // The original (plain text, quoted in the message's own text) is after it: the text box
    // holds both, the signature's room between what's written and what's quoted.
    let (card, body) = (compose(&a).signature_card().unwrap(), compose(&a).body);
    let quoted = body.chars().skip(card.end).collect::<String>();
    assert!(quoted.contains("Storyboard review"), "{quoted:?}");
    let text_box = drawn.iter().find(|d| d.text.contains("Storyboard review") && d.text.contains("Ms. Riley Moreno")).map(|d| d.ink);
    assert!(text_box.is_some_and(|ink| ink.top() < name && ink.bottom() > name + 40.0), "the signature inside the text: {text_box:?} {name}");
}

/// Issue #91: HTML code with a picture from the web, switched to Formatted (which can't show it)
/// and back unchanged, is the code it was, picture and all; the editor says so. Changed there,
/// it's made from what the editor has.
#[test]
fn a_signatures_web_pictures_survive_a_look_at_formatted() {
    let (mut a, _) = app("signature-web-pictures");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Dialog::signature(&a.session);
    let code =
        r#"<table><tr><td><img style="border-style:none" src="https://studio.example/signature/logo.jpg"></td><td><b>Riley</b></td></tr></table>"#;
    form_mut(&mut a).switch(true);
    form_mut(&mut a).code = Some(code.into());
    form_mut(&mut a).switch(false);
    frame(&ctx, &mut a, vec![]);
    let note = "Its pictures from the web aren't shown here. Go back to HTML code without changing it here, and they're kept.";
    assert!(drawn_text(&ctx, &mut a, egui::vec2(1280.0, 900.0)).iter().any(|d| d.text == note), "the editor says so");
    form_mut(&mut a).switch(true);
    assert_eq!(form(&a).code.as_deref(), Some(code), "back as it was");

    // Changed in the editor: made from what's there (the picture can't come with it).
    form_mut(&mut a).switch(false);
    form_mut(&mut a).compose.body.push_str(" Moreno");
    form_mut(&mut a).switch(true);
    assert!(form(&a).code.as_deref().is_some_and(|c| !c.contains("logo.jpg") && c.contains("Moreno")), "{:?}", form(&a).code);

    assert!(crate::dialogs::has_web_pictures(r#"<IMG SRC='http://x.example/a.png'>"#));
    assert!(crate::dialogs::has_web_pictures(r#"<img alt="x" src = "//cdn.example/a.png">"#));
    assert!(!crate::dialogs::has_web_pictures(r#"<img src="cid:logo"><a href="https://x.example">x</a>"#));
}

/// Issue #114: typing above a signature with a picture in it leaves the text under it where it
/// is. The editor laid each frame's text out with the look it had at the frame's start, so in
/// the frame a letter was typed, the picture after it was a letter off and laid out as one: the
/// text under it went up a row and came back, at every letter.
#[test]
fn typing_above_a_signatures_picture_moves_nothing_under_it() {
    const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAGAAAAAoCAIAAACO8WhuAAAAWklEQVR4nO3QMQ0AIADAMJRw418FsnDAR9jRZAKWjj2XLo3vB/EAAQIECFA4QIAAAQIUDhAgQIAAhQMECBAgQOEAAQIECFA4QIAAAQIUDhAgQIAAhQMECNDTDuTJ0uI32OGAAAAAAElFTkSuQmCC";
    let (mut a, _) = app("signature-picture-typing");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let html = format!(r#"<p><b>Riley Moreno</b></p><p><img src="{PNG}" width="96" height="40"></p>"#);
    a.run("account.setSignature", json!({"account": "sample", "name": "Card", "signature": "Riley Moreno", "html": html}));
    a.run("account.signatureDefaults", json!({"account": "sample", "new": "Card", "reply": "Card"}));
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.run("mail.reply", json!({}));
    assert_eq!(a.session.compose.as_ref().unwrap().inline.len(), 1, "the signature's picture is in the reply");
    let size = egui::vec2(1280.0, 800.0);
    // The reply's text, signature and original under it, laid out as one piece: where it ends.
    let bottom =
        |drawn: &[crate::tests::Drawn]| drawn.iter().find(|d| d.text.contains("Riley Moreno") && d.text.contains("From:")).map(|d| d.rect.bottom());
    for _ in 0..4 {
        drawn_text(&ctx, &mut a, size);
    }
    a.ui.editor.as_mut().unwrap().refocus = Some(0..0);
    for _ in 0..3 {
        drawn_text(&ctx, &mut a, size);
    }
    let settled = bottom(&drawn_text(&ctx, &mut a, size)).expect("the reply's text is drawn");
    for letter in ["H", "i", "!"] {
        let typed = bottom(&drawn_with(&ctx, &mut a, size, vec![Event::Text(letter.into())]));
        let after = bottom(&drawn_text(&ctx, &mut a, size));
        assert_eq!((typed, after), (Some(settled), Some(settled)), "after {letter:?}: the text under the signature stays put");
    }
    assert!(a.session.compose.as_ref().unwrap().body.starts_with("Hi!"));
}
