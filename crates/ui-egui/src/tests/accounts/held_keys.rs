//! The smart card and system certificate dialog: what each certificate found says, and what a
//! frame of it costs with many found.

use super::*;

/// `n` certificates found, each Riley's, labelled `Key 0`…
fn found(n: usize) -> Vec<mailcraft_engine::model::HeldKey> {
    let p12 = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata/riley-aes.p12")).unwrap();
    let cert = mailcraft_engine::smime::read_p12(&p12, "secret").unwrap().chain[0].clone();
    (0..n).map(|i| mailcraft_engine::model::HeldKey { label: format!("Key {i}"), cert: cert.clone(), at: format!("{{\"n\":{i}}}") }).collect()
}

/// The dialog's frame: each text drawn that can be seen (not wholly outside its clip), where.
fn held_drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2)> {
    a.ui.dialog = Some(Dialog::SmartCard { account: "sample".into(), module: String::new(), pin: String::new(), system: true });
    frame(ctx, a, vec![]);
    drawn_text(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// Each certificate found shows its label and, under it, whose it is, who issued it and its key;
/// a new search's answer shows in place of the last.
#[test]
fn the_certificates_found_say_whose_they_are() {
    let (mut a, _) = app("held-keys");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.apply(MailEvent::HeldKeys { keys: found(2), error: None });
    let texts: Vec<String> = held_drawn(&ctx, &mut a).into_iter().map(|(t, _)| t).collect();
    let info = mailcraft_engine::smime::cert::info(&found(1)[0].cert).unwrap();
    let line = format!("riley@studio.example · {} · {}", info.issuer, info.key);
    assert!(texts.iter().any(|t| t == "Key 0") && texts.iter().any(|t| t == "Key 1"), "{texts:?}");
    assert_eq!(texts.iter().filter(|t| **t == line).count(), 2, "{texts:?}");
    // Looked again: what this one found, not the last.
    a.run("security.findKeys", json!({"keychain": true}));
    let mut one = found(1);
    one[0].label = "Riley (Keychain)".into();
    a.session.apply(MailEvent::HeldKeys { keys: one, error: None });
    let texts: Vec<String> = held_drawn(&ctx, &mut a).into_iter().map(|(t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "Riley (Keychain)") && !texts.iter().any(|t| t == "Key 0"), "{texts:?}");
    assert_eq!(texts.iter().filter(|t| **t == line).count(), 1, "{texts:?}");
}

/// A frame of the dialog with 40 certificates found, and a fingerprint of what it draws (the same
/// before and after a change that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_held_keys_dialog -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_held_keys_dialog() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-held-keys");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.apply(MailEvent::HeldKeys { keys: found(40), error: None });
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            frame(&ctx, a, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    frame(&ctx, &mut a, vec![]);
    let window = timed(&mut a);
    println!("MEASURE the window alone: {window:.2} ms a frame");
    let drawn = held_drawn(&ctx, &mut a);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (t, p) in &drawn {
        (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
    }
    let each = timed(&mut a);
    println!(
        "MEASURE held keys dialog, 40 found: {each:.2} ms a frame, the dialog {:.2} ms; drawn {} texts, {:016x}",
        each - window,
        drawn.len(),
        h.finish()
    );
}
