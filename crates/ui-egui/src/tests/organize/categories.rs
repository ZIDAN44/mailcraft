//! The Categories dialog: the keywords on mail that aren't in the list, counted again as the mail
//! and the list change; and what a frame of it costs over a big mailbox.

use super::*;

/// The dialog's frame: each text drawn that can be seen (not wholly outside its clip), where.
fn categories_drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2)> {
    a.ui.dialog = Some(Dialog::Categories);
    frame(ctx, a, vec![]);
    drawn_text(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// A keyword on mail shows with how many messages have it, the count follows the mail while the
/// dialog stays open, and the keyword leaves "not in the list" once a category has it.
#[test]
fn the_categories_dialog_counts_keywords_on_mail_as_it_changes() {
    let (mut a, _) = app("categories-unlisted");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let keys: Vec<String> = a.session.list.iter().take(2).map(|m| m.key.to_id()).collect();
    let row = |texts: &[(String, egui::Pos2)]| texts.iter().find(|(t, _)| t.starts_with("projectxprojectx")).map(|(t, _)| t.clone());
    a.run("list.select", json!({"key": keys[0]}));
    a.run("mail.categorize", json!({"keyword": "projectx"}));
    let texts = categories_drawn(&ctx, &mut a);
    assert_eq!(row(&texts).as_deref(), Some("projectxprojectxMessages: 1"), "{texts:?}");
    // Another message gets it, the dialog open all the while.
    a.run("list.select", json!({"key": keys[1]}));
    a.run("mail.categorize", json!({"keyword": "projectx"}));
    let texts = categories_drawn(&ctx, &mut a);
    assert_eq!(row(&texts).as_deref(), Some("projectxprojectxMessages: 2"), "{texts:?}");
    // In the list now: not among those that aren't.
    a.run("category.create", json!({"name": "Project X", "keyword": "projectx"}));
    let texts = categories_drawn(&ctx, &mut a);
    assert_eq!(row(&texts), None, "{texts:?}");
    assert!(texts.iter().any(|(t, _)| t == "Project Xprojectx"), "{texts:?}");
}

/// A frame of the Categories dialog over the made-up big mailbox's Inbox (12,000 messages), each
/// with three keywords not in the list, and a fingerprint of what it draws (the same before and
/// after a change that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_categories_dialog -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_categories_dialog() {
    use std::hash::{Hash, Hasher};
    let store = Store::memory();
    mailcraft_engine::sample::big::generate(&store, NOW, mailcraft_engine::sample::big::Shape::default()).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW)));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.selectAll", json!({}));
    for k in ["projectx", "r=c3=bcckfrage", "Kunde_A"] {
        a.run("mail.categorize", json!({"keyword": k, "on": true}));
    }
    a.run("list.select", json!({"key": a.session.list[0].key.to_id()}));
    let listed = a.session.list.len();
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            frame(&ctx, a, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    frame(&ctx, &mut a, vec![]);
    let window = timed(&mut a);
    println!("MEASURE the window alone ({listed} listed): {window:.2} ms a frame");
    let drawn = categories_drawn(&ctx, &mut a);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (t, p) in &drawn {
        (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
    }
    let each = timed(&mut a);
    println!(
        "MEASURE categories dialog, {listed} listed, 3 keywords each: {each:.2} ms a frame, the dialog {:.2} ms; drawn {} texts, {:016x}",
        each - window,
        drawn.len(),
        h.finish()
    );
}
