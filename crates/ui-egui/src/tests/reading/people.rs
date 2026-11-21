//! UI tests: the people above a message, by the contacts they're saved as.

use mailcraft_engine::model::EmailAddress;
use mailcraft_engine::model::pim::StoredCard;
use mailcraft_engine::model::vcard::{Card, Typed};

use super::*;

/// Save `n` people, `Person {i}` at `person{i}@example.org`, in the sample's address book.
fn save_people(a: &mut MailApp, n: usize) {
    let p = a.session.pim_mut("sample");
    let book = p.books.first().map(|b| b.id.clone()).unwrap_or_default();
    for i in 0..n {
        let card = Card {
            uid: format!("c{i}"),
            name: format!("Person {i}"),
            org: "Studio".into(),
            emails: vec![Typed { value: format!("person{i}@example.org"), ..Default::default() }],
            ..Default::default()
        };
        p.cards.push(StoredCard { book: book.clone(), card, ..Default::default() });
    }
}

/// A message from `from` to `to`, first in the Inbox, and opened.
fn open_message(a: &mut MailApp, from: &EmailAddress, to: Vec<EmailAddress>) {
    let d = mailcraft_engine::mime::Draft { subject: "Plans".into(), body: "See you there.".into(), to, ..Default::default() };
    let raw = mailcraft_engine::mime::build(&d, from, NOW, "<people@x>").unwrap();
    let key = with_raw(a, 9950, &raw);
    a.run("list.open", json!({"key": key.to_id()}));
}

/// What the frame shows, as a screen reader is told it: the named nodes on screen.
fn shown(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<Named> {
    let nodes = named_in(&sized_frame(ctx, a, egui::vec2(1280.0, 800.0), events));
    nodes.into_iter().filter(|n| n.rect.is_some() && n.name().is_some()).collect()
}

/// A click with `button` on `at`, and what the frame after it shows.
fn click_with(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, button: egui::PointerButton) -> Vec<Named> {
    click_at(ctx, a, at, button, Modifiers::NONE);
    shown(ctx, a, vec![])
}

/// Whether one of `nodes` is named `name`.
fn says(nodes: &[Named], name: &str) -> bool {
    nodes.iter().any(|n| n.name() == Some(name))
}

/// Someone saved as a contact gets "Edit contact…" and their card shows their saved name; someone
/// who isn't gets "Add to contacts…" (and the From line's button for it).
#[test]
fn people_above_a_message_are_found_among_the_contacts() {
    let (mut a, _) = app("people-known");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    save_people(&mut a, 3);
    open_message(&mut a, &EmailAddress::new("Pat", "Person1@Example.org"), vec![EmailAddress::new("Quinn", "quinn@example.org")]);
    for _ in 0..2 {
        shown(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a, vec![]);
    assert!(!says(&nodes, "Add to contacts"), "Pat is saved: {nodes:?}");
    let at = |nodes: &[Named], who: &str| nodes.iter().find(|n| n.name() == Some(who)).and_then(Named::centre).expect(who);
    // Right-clicked: what can be done with each.
    let pat = at(&nodes, "Pat");
    let menu = click_with(&ctx, &mut a, pat, egui::PointerButton::Secondary);
    assert!(says(&menu, "Edit contact…") && !says(&menu, "Add to contacts…"), "{menu:?}");
    shown(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    let nodes = shown(&ctx, &mut a, vec![]);
    let quinn = at(&nodes, "Quinn");
    let menu = click_with(&ctx, &mut a, quinn, egui::PointerButton::Secondary);
    assert!(says(&menu, "Add to contacts…") && !says(&menu, "Edit contact…"), "{menu:?}");
    shown(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // Clicked: the card, with the name Pat is saved as.
    let nodes = shown(&ctx, &mut a, vec![]);
    let card = click_with(&ctx, &mut a, at(&nodes, "Pat"), egui::PointerButton::Primary);
    assert!(says(&card, "Person 1"), "{card:?}");
    assert!(says(&card, "Edit contact…"), "{card:?}");
}

/// A frame of a message to 50 people (the most named above it, about half of them saved), with no
/// contacts and with 5,000, and a fingerprint of what it draws (the same before and after a
/// change that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_people_above_a_message -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_people_above_a_message() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-people");
    let ctx = egui::Context::default();
    let to: Vec<EmailAddress> = (0..50).map(|i| EmailAddress::new(format!("Reader {i}"), format!("person{}@example.org", i * 197))).collect();
    open_message(&mut a, &EmailAddress::new("Pat", "pat@example.org"), to);
    let timed = |a: &mut MailApp, label: &str| {
        let texts = drawn_text(&ctx, a, egui::vec2(1280.0, 800.0));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for d in &texts {
            (&d.text, d.rect.min.x.to_bits(), d.rect.min.y.to_bits()).hash(&mut h);
        }
        let t = std::time::Instant::now();
        for _ in 0..100 {
            frame(&ctx, a, vec![]);
        }
        let each = t.elapsed().as_secs_f64() * 1000.0 / 100.0;
        println!("MEASURE a message to 50 people, {label}: {each:.3} ms a frame; drawn {} texts, {:016x}", texts.len(), h.finish());
        each
    };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let none = timed(&mut a, "no contacts");
    save_people(&mut a, 5000);
    let many = timed(&mut a, "5000 contacts");
    println!("MEASURE the contacts' part of a frame: {:.3} ms", many - none);
}

/// Issue #223: a sender whose contact has a photo shows it, in the list and above the message,
/// where their initials were; a picture that can't be read leaves the initials.
#[test]
fn a_contacts_photo_takes_the_place_of_their_initials() {
    let (mut a, _) = app("people-photos");
    let ctx = egui::Context::default();
    // A small picture, as a contact's card has it.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 6, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let b64 = mailcraft_engine::model::base64::encode(&png);
    let p = a.session.pim_mut("sample");
    let book = p.books.first().map(|b| b.id.clone()).unwrap_or_default();
    for (uid, email, photo) in [("zoe", "zoe@studio.example", b64.as_str()), ("sam", "sam@studio.example", "bm90IGEgcGljdHVyZQ==")] {
        let card = Card {
            uid: uid.into(),
            name: uid.into(),
            emails: vec![Typed { value: email.into(), ..Default::default() }],
            extra: vec![format!("PHOTO;ENCODING=b;TYPE=PNG:{photo}")],
            ..Default::default()
        };
        p.cards.push(StoredCard { book: book.clone(), card, ..Default::default() });
    }
    a.session.pim_revision += 1;
    let named = |ctx: &egui::Context| -> Vec<String> {
        ctx.tex_manager().read().allocated().map(|(_, m)| m.name.clone()).filter(|n| n.starts_with("photo:")).collect()
    };
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(named(&ctx), ["photo:zoe@studio.example"], "Zoe's drawn; Sam's isn't a picture");
    assert_eq!(a.session.photo("zoe@studio.example").as_deref(), Some(&png[..]));
    // Opened: above the message too, the same picture.
    let zoe = a.session.list.iter().find(|m| m.from.email == "zoe@studio.example").map(|m| m.key.clone()).expect("mail from Zoe");
    a.run("list.open", json!({"key": zoe.to_id()}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(named(&ctx), ["photo:zoe@studio.example"]);
}
