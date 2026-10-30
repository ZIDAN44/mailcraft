//! Typing: Alt+numpad character codes (issue #23).

use super::*;

/// A frame whose input goes through the Alt codes first, as the app's windows' does.
fn alt_frame(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) {
    let mut input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    crate::alt_codes(ctx, &mut input, (1252, 850));
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// Alt held, the digits, Alt let go: one character.
fn code(digits: &str) -> Vec<Event> {
    let alt = Modifiers::ALT;
    let mut v = vec![Event::ModifiersChanged(alt)];
    for d in digits.chars() {
        let k = Key::from_name(&d.to_string()).unwrap();
        v.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: alt });
        v.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: alt });
    }
    v.push(Event::ModifiersChanged(Modifiers::NONE));
    v
}

/// In a message's text and its subject, as French typists type É, È and Ç (Alt+0201…); a code
/// another key interrupts types nothing, and neither does Alt alone.
#[test]
fn alt_codes_type_their_characters() {
    let (mut a, dir) = app("alt-codes");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let body = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(body));
    frame(&ctx, &mut a, vec![]);
    let mut events = Vec::new();
    for c in ["0201", "0200", "0199", "130"] {
        events.extend(code(c));
    }
    alt_frame(&ctx, &mut a, events);
    frame(&ctx, &mut a, vec![]);
    let body_text = |a: &MailApp| a.session.compose.as_ref().unwrap().body.clone();
    assert!(body_text(&a).contains("ÉÈÇé"), "{:?}", body_text(&a));
    // Interrupted (Alt+0 then Tab), or Alt alone: nothing.
    let mut cut = code("0201");
    cut.insert(3, Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
    ctx.memory_mut(|m| m.request_focus(body));
    alt_frame(&ctx, &mut a, cut);
    alt_frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::ALT), Event::ModifiersChanged(Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    // (the Tab is the editor's; with the rest of the code dropped, as Windows drops it)
    assert!(body_text(&a).ends_with("ÉÈÇé\t") && !body_text(&a).contains('╔'), "nothing more: {:?}", body_text(&a));
    // A code spread over two frames (Alt held across them).
    let both = code("0199");
    let (first, second) = both.split_at(4);
    ctx.memory_mut(|m| m.request_focus(body));
    alt_frame(&ctx, &mut a, first.to_vec());
    alt_frame(&ctx, &mut a, second.to_vec());
    frame(&ctx, &mut a, vec![]);
    assert!(body_text(&a).ends_with("ÉÈÇé\tÇ"), "{:?}", body_text(&a));
    let _ = std::fs::remove_dir_all(dir);
}
