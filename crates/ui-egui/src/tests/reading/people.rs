//! UI tests: the people above a message, by the contacts they're saved as.

use egui::accesskit::Role;
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
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9950 };
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(&raw, key.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
}

/// What the frame shows, as a screen reader is told it: each node's role, name and place.
fn shown(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(Role, String, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            let rect = egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32));
            Some((n.role(), n.label().or(n.value())?.to_string(), rect))
        })
        .collect()
}

/// A click with `button` on `at`, and the frame after it.
fn click_with(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, button: egui::PointerButton) -> Vec<(Role, String, egui::Rect)> {
    let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
    shown(ctx, a, vec![Event::PointerMoved(at)]);
    shown(ctx, a, vec![press(true)]);
    shown(ctx, a, vec![press(false)]);
    shown(ctx, a, vec![])
}

/// Someone saved as a contact gets "Edit contact…" and their card shows their saved name; someone
/// who isn't gets "Add to contacts…" (and the From line's button for it).
#[test]
fn people_above_a_message_are_found_among_the_contacts() {
    let (mut a, dir) = app("people-known");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    save_people(&mut a, 3);
    open_message(&mut a, &EmailAddress::new("Pat", "Person1@Example.org"), vec![EmailAddress::new("Quinn", "quinn@example.org")]);
    for _ in 0..2 {
        shown(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a, vec![]);
    assert!(!nodes.iter().any(|(_, name, _)| name == "Add to contacts"), "Pat is saved: {nodes:?}");
    let at = |nodes: &[(Role, String, egui::Rect)], who: &str| nodes.iter().find(|(_, name, _)| name == who).map(|n| n.2.center()).expect(who);
    // Right-clicked: what can be done with each.
    let pat = at(&nodes, "Pat");
    let menu = click_with(&ctx, &mut a, pat, egui::PointerButton::Secondary);
    assert!(menu.iter().any(|(_, n, _)| n == "Edit contact…") && !menu.iter().any(|(_, n, _)| n == "Add to contacts…"), "{menu:?}");
    shown(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    let nodes = shown(&ctx, &mut a, vec![]);
    let quinn = at(&nodes, "Quinn");
    let menu = click_with(&ctx, &mut a, quinn, egui::PointerButton::Secondary);
    assert!(menu.iter().any(|(_, n, _)| n == "Add to contacts…") && !menu.iter().any(|(_, n, _)| n == "Edit contact…"), "{menu:?}");
    shown(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // Clicked: the card, with the name Pat is saved as.
    let nodes = shown(&ctx, &mut a, vec![]);
    let card = click_with(&ctx, &mut a, at(&nodes, "Pat"), egui::PointerButton::Primary);
    assert!(card.iter().any(|(_, n, _)| n == "Person 1"), "{card:?}");
    assert!(card.iter().any(|(_, n, _)| n == "Edit contact…"), "{card:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame of a message to 50 people (the most named above it, about half of them saved), with no
/// contacts and with 5,000, and a fingerprint of what it draws (the same before and after a
/// change that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_people_above_a_message -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_people_above_a_message() {
    use std::hash::{Hash, Hasher};
    let (mut a, dir) = app("measure-people");
    let ctx = egui::Context::default();
    let to: Vec<EmailAddress> = (0..50).map(|i| EmailAddress::new(format!("Reader {i}"), format!("person{}@example.org", i * 197))).collect();
    open_message(&mut a, &EmailAddress::new("Pat", "pat@example.org"), to);
    let timed = |a: &mut MailApp, label: &str| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let texts: Vec<(String, egui::Pos2)> = out
            .shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some((t.galley.job.text.clone(), t.pos)) } else { None })
            .collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &texts {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
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
    let _ = std::fs::remove_dir_all(dir);
}
