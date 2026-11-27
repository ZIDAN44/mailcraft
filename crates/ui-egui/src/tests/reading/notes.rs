//! The notes above a message (issue #276): what tracks its reader and the text it hides from
//! them, said unless turned off in File › Options › Safety; and when web pictures load, chosen
//! there.

use super::*;

/// A screen tall enough for File › Options' Safety card without scrolling.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 8000.0);

fn shows(ctx: &egui::Context, a: &mut MailApp, start: &str) -> bool {
    named(ctx, a, SCREEN).iter().any(|n| n.name().is_some_and(|x| x.starts_with(start)))
}

fn open(ctx: &egui::Context, a: &mut MailApp, key: &MessageKey) {
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
}

#[test]
fn the_notes_above_a_message_can_be_turned_off() {
    let _tall = windows::layout::Dressed::new(SCREEN, "");
    let (mut a, _) = app("message-notes");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let tracked = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).expect("in the Inbox");
    let words = "lorem ".repeat(40);
    let raw = format!(
        "From: Shop <news@shop.example>\r\nTo: riley@studio.example\r\nSubject: Sale\r\nContent-Type: text/html\r\n\r\n<p>Sale today</p><div style=\"display:none\">{words}</div>"
    );
    let hiding = with_raw(&mut a, 900, raw.as_bytes());
    let (tracking, hidden) = ("Tracking pictures, never loaded", "This message hides text from you");

    // Said, at first.
    open(&ctx, &mut a, &tracked);
    assert!(shows(&ctx, &mut a, tracking), "the tracking note");
    open(&ctx, &mut a, &hiding);
    assert!(shows(&ctx, &mut a, hidden), "the hidden-text note");

    // Turned off: neither.
    a.run("view.messageNotes", json!({"on": false}));
    frame(&ctx, &mut a, vec![]);
    assert!(!shows(&ctx, &mut a, hidden), "no hidden-text note");
    assert!(shows(&ctx, &mut a, "Sale today"), "the message itself");
    open(&ctx, &mut a, &tracked);
    assert!(!shows(&ctx, &mut a, tracking), "no tracking note");

    // File › Options › Safety: the notes' switch, and when web pictures load.
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    options_part(&mut a, crate::backstage::Section::Safety);
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = named(&ctx, &mut a, SCREEN);
    let switch = nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("Notes about tracking and hidden text") && n.role == egui::accesskit::Role::CheckBox)
        .and_then(Named::centre);
    click(&ctx, &mut a, switch.expect("its switch"));
    assert!(a.session.prefs.message_notes, "on again from Options");
    assert!(shows(&ctx, &mut a, "Show web pictures"));
    let unless = named(&ctx, &mut a, SCREEN).iter().find(|n| n.name() == Some("Unless suspicious")).and_then(Named::centre);
    click(&ctx, &mut a, unless.expect("the choice"));
    assert!(a.session.prefs.auto_pictures && !a.session.prefs.all_pictures, "unless suspicious");
}
