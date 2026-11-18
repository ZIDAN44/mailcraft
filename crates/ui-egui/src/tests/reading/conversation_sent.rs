//! Issue #148 in the reading pane: a conversation's list has the user's own reply from Sent Items,
//! its folder's name beside its time, and a click opens it in a window of its own.

use super::*;

#[test]
fn my_reply_from_sent_items_is_in_the_conversation() {
    let (mut a, _) = app("conversation-sent-pane");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("view.conversations", json!({}));
    let subject = "Your proof for the festival poster is ready";
    let k = a.session.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    a.run("compose.update", json!({"body": "Approved, thanks."}));
    a.run("compose.send", json!({}));
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": k.to_id()}));
    for _ in 0..3 {
        let _ = drawn_text(&ctx, &mut a, size);
    }
    let texts = drawn_text(&ctx, &mut a, size);
    assert!(texts.iter().any(|d| d.text == "2 messages in this conversation"), "{:?}", texts.iter().map(|d| &d.text).collect::<Vec<_>>());
    let row = texts.iter().find(|d| d.text.starts_with("Sent Items · ")).map(|d| d.rect).expect("my reply, its folder named");
    click(&ctx, &mut a, row.center());
    let sent = a.session.reading.as_ref().unwrap().aside[0].key.clone();
    assert!(a.session.windows.iter().any(|w| w.key == sent), "opened in a window of its own");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(k), "the conversation stays open here");
}
