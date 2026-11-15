//! UI tests: the people suggested in a recipient box while an address is typed.

use mailcraft_engine::model::pim::StoredCard;
use mailcraft_engine::model::vcard::{Card, Typed};

use super::*;

/// Each text the frame draws, where.
fn drawn(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(String, egui::Pos2)> {
    drawn_with(ctx, a, egui::vec2(1280.0, 800.0), events).into_iter().map(|d| (d.text, d.rect.min)).collect()
}

/// A new message in the reading pane, its To box with the focus and `typed` in it.
fn typing_in_to(ctx: &egui::Context, a: &mut MailApp, typed: &str) {
    a.session.prefs.new_mail_window = false;
    frame(ctx, a, vec![]);
    a.run("mail.new", json!({}));
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![Event::Text(typed.into())]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.to.as_str()), Some(typed));
    assert_eq!(a.ui.suggest_field, Some(crate::dialogs::Recipients::To));
}

/// What's suggested follows what's typed, and the address book as it changes while the same
/// text stays in the box; an address already in the box isn't suggested again.
#[test]
fn suggestions_follow_the_text_and_the_address_book() {
    let (mut a, _) = app("suggest-follow");
    let ctx = egui::Context::default();
    typing_in_to(&ctx, &mut a, "qu");
    let shows = |texts: &[(String, egui::Pos2)], what: &str| texts.iter().any(|(t, _)| t == what);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(!shows(&texts, "quinn.saved@example.org"), "no one yet");
    // Saved while "qu" stays typed: suggested on the next frame.
    a.run("contact.create", json!({"name": "Quinn Saved", "emails": "quinn.saved@example.org"}));
    drawn(&ctx, &mut a, vec![]);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(shows(&texts, "Quinn Saved") && shows(&texts, "quinn.saved@example.org"), "{texts:?}");
    // More typed that doesn't fit: gone.
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(!shows(&texts, "quinn.saved@example.org"), "{texts:?}");
    // Taken back: there again.
    frame(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)]);
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(shows(&texts, "quinn.saved@example.org"), "{texts:?}");
    // Already in the box, and "qu" typed after it: not suggested again.
    if let Some(c) = a.session.compose.as_mut() {
        c.to = "Quinn Saved <quinn.saved@example.org>, qu".into();
    }
    let texts = drawn(&ctx, &mut a, vec![]);
    assert!(!shows(&texts, "quinn.saved@example.org"), "{texts:?}");
}

/// A frame of a new message with an address being typed in To, the suggestions showing, with
/// 5,000 people saved, and a fingerprint of what it draws (the same before and after a change
/// that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_suggestions -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_suggestions() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-suggest");
    let ctx = egui::Context::default();
    let p = a.session.pim_mut("sample");
    let book = p.books.first().map(|b| b.id.clone()).unwrap_or_default();
    for i in 0..5000 {
        let card = Card {
            uid: format!("c{i}"),
            name: format!("Person {i}"),
            emails: vec![Typed { value: format!("person{i}@example.org"), ..Default::default() }],
            ..Default::default()
        };
        p.cards.push(StoredCard { book: book.clone(), card, ..Default::default() });
    }
    // The address book counted again, with them in it.
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    typing_in_to(&ctx, &mut a, "person1");
    // (The list is laid out unseen on its first frame.)
    frame(&ctx, &mut a, vec![]);
    let timed = |a: &mut MailApp, label: &str| {
        let texts = drawn(&ctx, a, vec![]);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &texts {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
        let suggested = texts.iter().filter(|(t, _)| t.starts_with("person1") && t.ends_with("@example.org")).count();
        let t = std::time::Instant::now();
        for _ in 0..100 {
            frame(&ctx, a, vec![]);
        }
        let each = t.elapsed().as_secs_f64() * 1000.0 / 100.0;
        println!("MEASURE {label}, 5000 people saved: {each:.3} ms a frame; {suggested} suggested; drawn {} texts, {:016x}", texts.len(), h.finish());
        each
    };
    let showing = timed(&mut a, "typing in To");
    // The same message with the box left (nothing suggested), to take away.
    a.ui.suggest_dismissed = a.session.compose.as_ref().map(|c| c.to.clone());
    let none = timed(&mut a, "the list put away");
    println!("MEASURE the suggestions' part of a frame: {:.3} ms", showing - none);
}
