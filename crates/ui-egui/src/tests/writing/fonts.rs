//! A font of the computer's to write in (issue #166): the font box offers what's installed, the
//! text picked is in that font, drawn in it in the editor and sent with it.

use std::sync::Arc;

use egui::accesskit::Role;

use super::*;
use crate::mail_fonts::FontSource;

/// A computer with one font, "Testface" (egui's own sans, under another name).
struct OneFont;

impl FontSource for OneFont {
    fn families(&self) -> Arc<[String]> {
        Arc::from(vec!["Testface".to_string()])
    }

    fn face(&self, family: &str, _bold: bool) -> Option<(Arc<[u8]>, u32)> {
        let defs = egui::FontDefinitions::default();
        let data = defs.font_data.values().next()?;
        let bytes: &[u8] = data.font.as_ref().as_ref();
        family.eq_ignore_ascii_case("Testface").then(|| (Arc::<[u8]>::from(bytes), data.index))
    }
}

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A message being written in the window, its text "Hello" selected, the computer's fonts offered
/// as `source` has them.
fn writing(name: &str, source: Option<Arc<dyn FontSource>>) -> (MailApp, egui::Context, egui::Id) {
    let (mut a, _) = app(name);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    if let Some(source) = source {
        crate::mail_fonts::install(&ctx, source);
    }
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().expect("a message is written"));
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![]);
    (a, ctx, id)
}

/// The font box's node, and what it shows.
fn font_box(ctx: &egui::Context, a: &mut MailApp) -> Option<(egui::accesskit::NodeId, String)> {
    let out = sized_frame(ctx, a, SIZE, vec![]);
    let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
    nodes
        .into_iter()
        .find(|(_, n)| n.role() == Role::ComboBox && n.label() == Some("Font"))
        .map(|(id, n)| (id, n.value().unwrap_or_default().to_string()))
}

/// The font box offers the computer's fonts; one picked is the selected text's, drawn in it and
/// sent with it; the message's own font is picked back the same way.
#[test]
fn a_font_of_the_computers_is_picked_drawn_and_sent() {
    let (mut a, ctx, _) = writing("fonts-pick", Some(Arc::new(OneFont)));
    let (combo, shown) = font_box(&ctx, &mut a).expect("the font box, with the computer's fonts offered");
    assert_eq!(shown, "Default font");
    // Down picks the first of the computer's fonts, as a drop-down's arrows do.
    sized_frame(&ctx, &mut a, SIZE, vec![focus(combo)]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    sized_frame(&ctx, &mut a, SIZE, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    let c = a.session.compose.clone().expect("still written");
    assert_eq!(c.styles.style(0).font.as_deref(), Some("Testface"), "the selection is in the font picked");
    assert_eq!(font_box(&ctx, &mut a).map(|(_, s)| s).as_deref(), Some("Testface"), "and the box shows it");
    // Drawn in it: the text's run asks for that font's family, which the computer's font fills.
    let style = c.styles.style(0);
    let family = crate::mail_fonts::family("Testface", style.bold).expect("offered here");
    let fmt = crate::rich_edit::text_format(&style, None, &a.ui.tokens);
    assert_eq!(fmt.font_id.family, family);
    let drawn: Vec<String> = drawn_strings(&ctx, &mut a, SIZE);
    assert!(drawn.iter().any(|t| t.contains("Hello")), "the text is drawn, not left blank: {drawn:?}");
    // Sent with it.
    let (_, html, _) = c.bodies();
    assert!(html.is_some_and(|h| h.contains("font-family") && h.contains("Testface")), "sent in it");
    // Back to the message's own font, with Up.
    sized_frame(&ctx, &mut a, SIZE, vec![key(Key::ArrowUp, Modifiers::NONE)]);
    sized_frame(&ctx, &mut a, SIZE, vec![]);
    assert_eq!(a.session.compose.as_ref().and_then(|c| c.styles.style(0).font.clone()), None);
}

/// Without the computer's fonts (the browser), there's no font box, and text in a font from
/// elsewhere (pasted from Outlook) is drawn in the editor's own, never in a family nothing fills
/// (which would draw nothing). A font the computer hasn't is drawn the same way.
#[test]
fn without_the_computers_fonts_text_in_one_is_drawn_in_the_editors() {
    let (mut a, ctx, _) = writing("fonts-none", None);
    assert!(font_box(&ctx, &mut a).is_none(), "no font box");
    let style = mailcraft_engine::mime::rich::Style { font: Some("Tahoma".into()), ..Default::default() };
    assert_eq!(crate::rich_edit::text_format(&style, None, &a.ui.tokens).font_id.family, egui::FontFamily::Proportional);
    let (mut b, ctx_b, _) = writing("fonts-not-here", Some(Arc::new(OneFont)));
    assert!(font_box(&ctx_b, &mut b).is_some());
    assert_eq!(
        crate::rich_edit::text_format(&style, None, &b.ui.tokens).font_id.family,
        egui::FontFamily::Proportional,
        "Tahoma isn't on this computer"
    );
}

/// A screen reader's "focus this" for `id`.
fn focus(id: egui::accesskit::NodeId) -> Event {
    Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Focus,
        target_node: id,
        target_tree: egui::accesskit::TreeId::ROOT,
        data: None,
    })
}

/// File › Options › Mail's "Font for new messages" (issue #166) sets the font a new message is
/// written in: the font box shows it, and the editor draws the message's text in it.
#[test]
fn the_font_for_new_messages_is_set_in_options_and_written_in() {
    let (mut a, _) = app("fonts-new-messages");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::mail_fonts::install(&ctx, Arc::new(OneFont));
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tall = egui::vec2(1280.0, 2400.0);
    for _ in 0..3 {
        sized_frame(&ctx, &mut a, tall, vec![]);
    }
    let out = sized_frame(&ctx, &mut a, tall, vec![]);
    let nodes = out.platform_output.accesskit_update.expect("AccessKit is on").nodes;
    let row = nodes
        .iter()
        .find(|(_, n)| n.role() == Role::ComboBox && n.label() == Some("Font for new messages"))
        .map(|(id, n)| (*id, n.value().unwrap_or_default().to_string()))
        .expect("the row, with the computer's fonts offered");
    assert_eq!(row.1, "Default font");
    // Down picks the computer's first font.
    sized_frame(&ctx, &mut a, tall, vec![focus(row.0)]);
    sized_frame(&ctx, &mut a, tall, vec![]);
    sized_frame(&ctx, &mut a, tall, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    sized_frame(&ctx, &mut a, tall, vec![]);
    assert_eq!(a.session.prefs.compose_font.as_deref(), Some("Testface"));
    a.ui.backstage = None;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().expect("a message is written");
    assert_eq!(c.font.as_deref(), Some("Testface"));
    assert_eq!(font_box(&ctx, &mut a).map(|(_, s)| s).as_deref(), Some("Testface"), "the font box shows the message's font");
    let plain = mailcraft_engine::mime::rich::Style::default();
    let family = crate::mail_fonts::family("Testface", false).expect("offered here");
    assert_eq!(crate::rich_edit::text_format(&plain, c.font.as_deref(), &a.ui.tokens).font_id.family, family, "drawn in it");
}
