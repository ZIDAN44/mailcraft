//! Undo Send (issue #135) in the window: while a message sent waits in the Outbox, a bar says how
//! long it has and offers Undo, which opens it again as it was written.

use super::*;

#[test]
fn the_undo_bar_counts_down_and_takes_the_message_back() {
    let (mut a, _) = app("undo-bar");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    let id = a
        .session
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "imap.example.org", "smtpHost": "smtp.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.run("options.undoSend", json!({"seconds": 10}));
    a.run("mail.new", json!({"account": id}));
    a.run("compose.update", json!({"to": "ann@example.org", "subject": "Second thoughts", "body": "Hello"}));
    a.run("compose.send", json!({}));
    assert!(a.session.compose.is_none());
    for _ in 0..2 {
        let _ = drawn_strings(&ctx, &mut a, size);
    }
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(texts.iter().any(|t| t == "Sending · 10 s"), "{texts:?}");
    // Four seconds on (the window's clock is its services').
    a.services = Box::new(crate::services::NullServices(NOW + 4));
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(texts.iter().any(|t| t == "Sending · 6 s"), "it counts down: {texts:?}");
    click_named(&ctx, &mut a, "Undo");
    assert!(a.session.compose.as_ref().is_some_and(|c| c.subject == "Second thoughts"), "open again as it was");
    assert!(a.session.outbox.is_empty());
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(!texts.iter().any(|t| t.starts_with("Sending ·")), "the bar goes");
}
