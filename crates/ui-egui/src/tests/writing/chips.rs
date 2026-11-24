//! UI tests: a recipient box's people, a chip each (issue #269): shown by name, taken out by
//! their ×, selected by clicks and arrows and taken out together, moved between To, Cc and Bcc
//! by their menu or a drag, put in another order by a drag, folded to a line with how many more;
//! the box's text, what's sent, the same people.

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

/// The chips selected in the box that has the keyboard.
fn selected_in(ctx: &egui::Context) -> Vec<usize> {
    let id = ctx.memory(|m| m.focused()).expect("a box has the keyboard");
    crate::reading::compose_chips::selected(ctx, id, 99).chips.into_iter().collect()
}

/// A click on the To box's chip with `label`, with `modifiers` held.
fn click_chip(ctx: &egui::Context, a: &mut MailApp, label: &str, modifiers: Modifiers) {
    let at = in_boxes(ctx, a).into_iter().find(|d| d.text == label).map(|d| d.rect.center()).expect("the chip");
    click_at(ctx, a, at, egui::PointerButton::Primary, modifiers);
}

/// A click in the To box's text, right of its chip `last`.
fn into_the_text(ctx: &egui::Context, a: &mut MailApp, last: &str) {
    let named = named(ctx, a, SIZE);
    let to_box = named.iter().find(|n| n.name() == Some("To")).and_then(Named::centre).expect("To");
    let chip = in_boxes(ctx, a).into_iter().find(|d| d.text == last).map(|d| d.rect).expect("the chip");
    click(ctx, a, egui::pos2(chip.right() + 30.0, to_box.y));
    // The frame a click draws: the text box's keys its own from then on (egui keeps the
    // arrows for a text box from the frame after it takes the keyboard).
    frame(ctx, a, vec![]);
}

/// A drag with the primary button from `from` to `to`, a frame at each step.
fn drag(ctx: &egui::Context, a: &mut MailApp, from: egui::Pos2, to: egui::Pos2) {
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(from)]);
    frame(ctx, a, vec![press(from, true)]);
    for k in 1..=4 {
        frame(ctx, a, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    frame(ctx, a, vec![press(to, false)]);
    frame(ctx, a, vec![]);
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

/// Backspace with nothing typed after the chips selects the last, and a second takes it out;
/// an address typed and ended with `;` becomes a chip, and a list pasted a person a line a chip
/// each.
#[test]
fn backspace_takes_the_last_chip_and_typing_makes_one() {
    let (mut a, ctx) = writing_to("chips-typing", "Zoe Park <zoe@x.example>; ");
    into_the_text(&ctx, &mut a, "Zoe Park");
    frame(&ctx, &mut a, vec![Event::Text("ann@x.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; ann@x.example;");
    assert!(in_boxes(&ctx, &mut a).iter().any(|d| d.text == "ann@x.example"), "a chip now");
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; ann@x.example;", "selected, not gone");
    assert_eq!(selected_in(&ctx), [1]);
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@x.example>; ");
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
    drag(&ctx, &mut a, from, egui::pos2(from.x + 200.0, cc.y));
    let c = a.session.compose.clone().expect("the message");
    assert_eq!((c.to.as_str(), c.cc.as_str()), ("ops@x.example; ", "Zoe Park <zoe@x.example>; "));
}

/// Several chips selected (Ctrl+click adds one, Shift+click those between) go together:
/// Delete takes them all out.
#[test]
fn chips_selected_by_clicks_go_together() {
    let (mut a, ctx) = writing_to("chips-select", "a@x.example; b@x.example; c@x.example; d@x.example; ");
    click_chip(&ctx, &mut a, "a@x.example", Modifiers::NONE);
    click_chip(&ctx, &mut a, "c@x.example", Modifiers::COMMAND);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(selected_in(&ctx), [0, 2]);
    click_chip(&ctx, &mut a, "b@x.example", Modifiers::NONE);
    click_chip(&ctx, &mut a, "d@x.example", Modifiers::SHIFT);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(selected_in(&ctx), [1, 2, 3]);
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "a@x.example; ");
}

/// With nothing typed, ← selects the last chip and goes along them, Shift+← takes in more, →
/// past the last goes back to the text; Backspace takes those selected out; typing lets them go,
/// and Esc too (the text box gives up the keyboard, as egui's do). Short addresses, so the text
/// box is on the chips' line.
#[test]
fn the_arrows_go_along_the_chips() {
    let (mut a, ctx) = writing_to("chips-arrows", "a@x.io; b@x.io; c@x.io; ");
    into_the_text(&ctx, &mut a, "c@x.io");
    let to_box = ctx.memory(|m| m.focused()).expect("the To box has the keyboard");
    let sel = || -> Vec<usize> { crate::reading::compose_chips::selected(&ctx, to_box, 99).chips.into_iter().collect() };
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    assert_eq!(sel(), [2]);
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    assert_eq!(sel(), [1]);
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::SHIFT)]);
    assert_eq!(sel(), [0, 1]);
    frame(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::NONE)]);
    assert_eq!(sel(), [1]);
    frame(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::ArrowRight, Modifiers::NONE)]);
    assert!(sel().is_empty(), "past the last: back to the text");
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "a@x.io; ");
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    assert_eq!(sel(), [0]);
    frame(&ctx, &mut a, vec![Event::Text("z".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "a@x.io; z");
    assert!(sel().is_empty());
    // Esc: the keyboard gone, those selected go.
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::ArrowLeft, Modifiers::NONE)]);
    assert_eq!(sel(), [0]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(sel().is_empty());
}

/// A chip dragged within its box goes where it's let go: here the first between the second and
/// the third; then none is selected.
#[test]
fn a_chip_dragged_within_its_box_changes_place() {
    let (mut a, ctx) = writing_to("chips-order", "a@x.io; b@x.io; c@x.io; ");
    let drawn = in_boxes(&ctx, &mut a);
    let rect = |w: &str| drawn.iter().find(|d| d.text == w).map(|d| d.rect).expect("a chip");
    let from = rect("a@x.io").center();
    drag(&ctx, &mut a, from, egui::pos2(rect("c@x.io").left() + 2.0, from.y));
    assert_eq!(to(&a), "b@x.io; a@x.io; c@x.io; ");
    // None stays selected: the one dragged was, by its place, now another's.
    into_the_text(&ctx, &mut a, "c@x.io");
    assert!(selected_in(&ctx).is_empty(), "{:?}", selected_in(&ctx));
}

/// A box not being written in, holding more people than fit on its line, shows those that fit
/// and how many more; a click on that shows them all, the box taking the keyboard. What's sent
/// is the same.
#[test]
fn a_long_box_is_one_line_with_how_many_more() {
    let people: String = (1..=30).map(|n| format!("Person Number {n} <p{n}@x.example>; ")).collect();
    let (mut a, ctx) = writing_to("chips-fold", &people);
    ctx.memory_mut(|m| {
        if let Some(id) = m.focused() {
            m.surrender_focus(id);
        }
    });
    frame(&ctx, &mut a, vec![]);
    let drawn = in_boxes(&ctx, &mut a);
    let chips = drawn.iter().filter(|d| d.text.starts_with("Person Number")).count();
    let more = drawn.iter().find(|d| d.text.starts_with('+')).expect("how many more");
    assert!(chips > 0 && chips < 30, "{chips} shown");
    assert_eq!(more.text, format!("+{} more", 30 - chips));
    click(&ctx, &mut a, more.rect.center());
    frame(&ctx, &mut a, vec![]);
    let drawn = in_boxes(&ctx, &mut a);
    assert_eq!(drawn.iter().filter(|d| d.text.starts_with("Person Number")).count(), 30, "all of them");
    assert!(!drawn.iter().any(|d| d.text.starts_with('+')));
    assert_eq!(to(&a), people, "what's sent the same");
}

/// What's done to a box's chips, on the message's boxes: taken out, moved (Bcc shown for those
/// moved there), put back to edit (what was being typed kept as a chip), dropped from another
/// box or within their own, several at once. Nothing else in the boxes changes.
#[test]
fn chips_done_on_the_boxes() {
    let mut c = mailcraft_engine::Compose { to: "Zoe <zoe@x.example>; ops@x.example; jo".into(), cc: "ann@x.example; ".into(), ..Default::default() };
    assert!(!apply(&mut c, Recipients::To, vec![ChipDone::Move(vec![1], Recipients::Bcc)]));
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.show_bcc), ("Zoe <zoe@x.example>; jo", "ops@x.example; ", true));
    assert!(apply(&mut c, Recipients::To, vec![ChipDone::Edit(0)]));
    assert_eq!(c.to, "jo; Zoe <zoe@x.example>");
    // From Cc onto Bcc, before its first.
    let dropped = DragRecipient { from: Recipients::Cc, entries: vec!["ann@x.example".into()] };
    apply(&mut c, Recipients::Bcc, vec![ChipDone::Dropped(dropped, 0)]);
    assert_eq!((c.cc.as_str(), c.bcc.as_str()), ("", "ann@x.example; ops@x.example; "));
    // Several in their own box, to another place: before the one that was there.
    c.to = "a@x.example; b@x.example; c@x.example; d@x.example; ".into();
    let within = |entries: &[&str], at| {
        ChipDone::Dropped(DragRecipient { from: Recipients::To, entries: entries.iter().map(|e| e.to_string()).collect() }, at)
    };
    apply(&mut c, Recipients::To, vec![within(&["c@x.example", "d@x.example"], 0)]);
    assert_eq!(c.to, "c@x.example; d@x.example; a@x.example; b@x.example; ");
    apply(&mut c, Recipients::To, vec![within(&["c@x.example"], 4)]);
    assert_eq!(c.to, "d@x.example; a@x.example; b@x.example; c@x.example; ");
    apply(&mut c, Recipients::To, vec![ChipDone::Move(vec![3, 0], Recipients::Cc)]);
    assert_eq!((c.to.as_str(), c.cc.as_str()), ("a@x.example; b@x.example; ", "d@x.example; c@x.example; "));
    apply(&mut c, Recipients::To, vec![ChipDone::Remove(vec![1, 0, 1])]);
    assert_eq!(c.to, "");
    // Out of range, onto its own box, or no longer there: nothing.
    c.to = "a@x.example; ".into();
    let before = c.clone();
    apply(&mut c, Recipients::To, vec![ChipDone::Remove(vec![9]), ChipDone::Move(vec![0], Recipients::To), ChipDone::Edit(5)]);
    apply(&mut c, Recipients::Cc, vec![ChipDone::Dropped(DragRecipient { from: Recipients::To, entries: vec!["gone@x.example".into()] }, 0)]);
    assert_eq!((c.to, c.cc, c.bcc), (before.to, before.cc, before.bcc));
}
