//! Tables pasted into a message (issue #268): cells copied in Excel, pasted, are a table in the
//! message, drawn as it will be sent where its text stands; Backspace after it takes it out
//! whole.

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Cells copied in Excel, as its clipboard HTML has them (lines and alignment in a `<style>`).
const EXCEL: &str = "<html><head><style>.xl65 {border:.5pt solid windowtext; text-align:right}</style></head><body><table>\
    <tr><td class=xl65 style='text-align:left'>Paper</td><td class=xl65>12.50</td></tr>\
    <tr><td class=xl65 style='text-align:left'>Ink</td><td class=xl65>8.00</td></tr></table></body></html>";

#[test]
fn cells_pasted_from_excel_are_a_table_in_the_message() {
    let (mut a, _) = app("paste-table");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Prices", "body": "Prices below"}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().expect("a message is written"));
    ctx.memory_mut(|m| m.request_focus(id));
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(12))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![]);

    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = Some(EXCEL.to_string()));
    frame(&ctx, &mut a, vec![Event::Paste("Paper\t12.50\nInk\t8.00".into())]);
    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = None);
    let c = a.session.compose.clone().expect("a message is written");
    assert_eq!(c.body, "Prices below\nPaper\t12.50\nInk\t8.00");
    assert_eq!(c.table_places().len(), 1, "kept as a table");
    assert!(c.bodies().1.is_some_and(|h| h.contains("<table") && h.contains("border-top:1px solid #000000")));

    // Drawn as a table: each cell's text on its own (its text in the message unseen under it).
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let drawn = drawn_strings(&ctx, &mut a, SIZE);
    for cell in ["Paper", "12.50", "Ink", "8.00"] {
        assert!(drawn.iter().any(|d| d.trim() == cell), "{cell} drawn in its cell: {drawn:?}");
    }

    // Backspace right after it takes it out whole.
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().expect("a message is written");
    assert_eq!(c.body, "Prices below");
    assert!(c.table_places().is_empty());
}

/// In the signature editor (a signature keeps its text, or code of its own) a table pasted is its
/// text, as before: only a message's own text keeps tables.
#[test]
fn a_table_pasted_into_a_signature_is_its_text() {
    let (mut a, _) = app("paste-table-signature");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Dialog(crate::Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false)))));
    frame(&ctx, &mut a, vec![]);
    let form = |a: &MailApp| match &a.ui.dialog {
        Some(crate::Dialog::Signature(f)) => Some(f.compose.clone()),
        _ => None,
    };
    let id = crate::rich_edit::body_id(&form(&a).expect("the signature editor"));
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = Some(EXCEL.to_string()));
    frame(&ctx, &mut a, vec![Event::Paste("Paper\t12.50\nInk\t8.00".into())]);
    crate::TEST_CLIPBOARD_HTML.with(|h| *h.borrow_mut() = None);
    let c = form(&a).expect("the signature editor");
    assert!(c.body.contains("Paper") && c.body.contains("8.00"), "{:?}", c.body);
    assert!(c.tables.is_empty(), "no table kept");
}
