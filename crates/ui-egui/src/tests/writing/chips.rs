//! UI tests: a recipient box's people, a chip each (issue #269): shown by name, taken out by
//! their ×, by Backspace, moved between To, Cc and Bcc by their menu or a drag; the box's text,
//! what's sent, the same people.

use crate::dialogs::Recipients;
use crate::reading::compose_chips::{ChipDone, DragRecipient, apply};

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A new message in the reading pane, `to` in its To box.
fn writing_to(name: &str, to: &str) -> (MailApp, egui::Context) {
    let (mut a, _) = app(name);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": to}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, ctx)
}

/// The texts drawn in the message's boxes (right of its labels; the list, left of them, has a
/// Zoe Park of its own).
fn in_boxes(ctx: &egui::Context, a: &mut MailApp) -> Vec<Drawn> {
    let drawn = drawn_text(ctx, a, SIZE);
    let left = drawn.iter().find(|d| d.text == "To").map_or(0.0, |d| d.rect.right());
    drawn.into_iter().filter(|d| d.rect.left() > left).collect()
}

fn to(a: &MailApp) -> String {
    a.session.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default()
}

/// Each person is a chip by their name (an address alone by itself); what's still being typed
/// is the text box's. The ×'s of the first takes it out, and the box's text keeps the rest.
#[test]
fn recipients_are_chips_and_a_chips_x_takes_it_out() {
    let (mut a, ctx) = writing_to("chips-shown", "Zoe Park <zoe@x.example>; ops@x.example; jo");
    let drawn = in_boxes(&ctx, &mut a);
    let at = |what: &str| drawn.iter().find(|d| d.text == what).map(|d| d.rect);
    let zoe = at("Zoe Park").expect("Zoe's chip, by her name");
    let ops = at("ops@x.example").expect("an address alone");
    assert!(ops.left() > zoe.right(), "side by side");
    assert!(at("Zoe Park <zoe@x.example>").is_none(), "not written out");
    let jo = at("jo").expect("what's typed");
    assert!(jo.left() > ops.right() && (jo.center().y - ops.center().y).abs() < 6.0, "after them, on their line: {jo:?} {ops:?}");
    // Zoe's × (the right end of her chip).
    let named = named(&ctx, &mut a, SIZE);
    let x = named.iter().find(|n| n.name() == Some("Remove: Zoe Park")).and_then(Named::centre).expect("her ×");
    click(&ctx, &mut a, x);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "ops@x.example; jo");
}

/// Backspace with nothing typed after the chips takes the last one out, as Outlook's does;
/// an address typed and ended with `;` becomes a chip.
#[test]
fn backspace_takes_the_last_chip_and_typing_makes_one() {
    let (mut a, ctx) = writing_to("chips-typing", "Zoe Park <zoe@x.example>; ");
    let named = named(&ctx, &mut a, SIZE);
    let to_box = named.iter().find(|n| n.name() == Some("To")).and_then(Named::centre).expect("To");
    // Its text box, right of the chip: a click there, then typing.
    let zoe = in_boxes(&ctx, &mut a).into_iter().find(|d| d.text == "Zoe Park").map(|d| d.rect).expect("her chip");
    click(&ctx, &mut a, egui::pos2(zoe.right() + 120.0, to_box.y));
    frame(&ctx, &mut a, vec![Event::Text("ann@x.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; ann@x.example;");
    assert!(in_boxes(&ctx, &mut a).iter().any(|d| d.text == "ann@x.example"), "a chip now");
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; ");
    // A list pasted a person a line: a chip each.
    frame(&ctx, &mut a, vec![Event::Paste("bo@x.example\r\nCy Lee <cy@x.example>\n\ndi@x.example\n".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; bo@x.example; Cy Lee <cy@x.example>; di@x.example");
    let shown = in_boxes(&ctx, &mut a);
    assert!(["bo@x.example", "Cy Lee"].iter().all(|w| shown.iter().any(|d| d.text == *w)), "chips");
}

/// A chip dragged from To onto Cc moves there; the message's people are the same.
#[test]
fn a_chip_dragged_onto_cc_moves_there() {
    let (mut a, ctx) = writing_to("chips-drag", "Zoe Park <zoe@x.example>; ops@x.example; ");
    let from = in_boxes(&ctx, &mut a).into_iter().find(|d| d.text == "Zoe Park").map(|d| d.rect.center()).expect("her chip");
    let named = named(&ctx, &mut a, SIZE);
    let cc = named.iter().find(|n| n.name() == Some("Cc")).and_then(Named::centre).expect("Cc");
    let to_pt = egui::pos2(from.x + 200.0, cc.y);
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    for k in 1..=4 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(from + (to_pt - from) * (k as f32 / 4.0))]);
    }
    frame(&ctx, &mut a, vec![press(to_pt, false)]);
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().expect("the message");
    assert_eq!((c.to.as_str(), c.cc.as_str()), ("ops@x.example; ", "Zoe Park <zoe@x.example>; "));
}

/// What's done to a box's chips, on the message's boxes: taken out, moved (Bcc shown for one
/// moved there), put back to edit (what was being typed kept as a chip), dropped from another
/// box. Nothing else in the boxes changes.
#[test]
fn chips_done_on_the_boxes() {
    let mut c = mailcraft_engine::Compose { to: "Zoe <zoe@x.example>; ops@x.example; jo".into(), cc: "ann@x.example; ".into(), ..Default::default() };
    assert!(!apply(&mut c, Recipients::To, vec![ChipDone::Move(1, Recipients::Bcc)]));
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.show_bcc), ("Zoe <zoe@x.example>; jo", "ops@x.example; ", true));
    assert!(apply(&mut c, Recipients::To, vec![ChipDone::Edit(0)]));
    assert_eq!(c.to, "jo; Zoe <zoe@x.example>");
    let dropped = DragRecipient { from: Recipients::Cc, entry: "ann@x.example".into() };
    apply(&mut c, Recipients::Bcc, vec![ChipDone::Dropped(dropped)]);
    assert_eq!((c.cc.as_str(), c.bcc.as_str()), ("", "ops@x.example; ann@x.example; "));
    // Out of range, or onto its own box: nothing.
    let before = c.clone();
    apply(&mut c, Recipients::To, vec![ChipDone::Remove(9), ChipDone::Move(0, Recipients::To), ChipDone::Edit(5)]);
    apply(&mut c, Recipients::Bcc, vec![ChipDone::Dropped(DragRecipient { from: Recipients::Bcc, entry: "ann@x.example".into() })]);
    assert_eq!((c.to, c.cc, c.bcc), (before.to, before.cc, before.bcc));
}
