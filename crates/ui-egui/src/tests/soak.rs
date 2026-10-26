//! The whole window played by a random hand, as deckcraft's soak test does: seeded clicks,
//! right-clicks, double-clicks, drags, keys (plain, Ctrl, Shift, Alt), typing and scrolling over
//! the sample mailbox, in a wide window and a narrow one. Nothing may panic, in egui or behind a
//! command (caught there and counted), and the window stays drawable and quick.
//!
//! `MAILCRAFT_SOAK=seeds,actions` plays longer (the default is 3 seeds of 300 actions).

use std::sync::atomic::Ordering;

use super::*;

/// A small, seeded random source (SplitMix64): the same seed plays the same hand.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pos(&mut self, size: egui::Vec2) -> egui::Pos2 {
        egui::pos2(self.below(size.x as u64) as f32, self.below(size.y as u64) as f32)
    }
}

const KEYS: [Key; 18] = [
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Delete,
    Key::Backspace,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::Home,
    Key::End,
    Key::PageDown,
    Key::A,
    Key::E,
    Key::R,
    Key::N,
    Key::Q,
    Key::F,
];

/// One action's input events.
fn action(d: &mut Dice, size: egui::Vec2) -> Vec<Event> {
    let at = d.pos(size);
    let button =
        |pressed: bool, button: egui::PointerButton, pos: egui::Pos2| Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE };
    match d.below(10) {
        0..=2 => vec![Event::PointerMoved(at), button(true, egui::PointerButton::Primary, at), button(false, egui::PointerButton::Primary, at)],
        3 => vec![Event::PointerMoved(at), button(true, egui::PointerButton::Secondary, at), button(false, egui::PointerButton::Secondary, at)],
        4 => {
            let click = [button(true, egui::PointerButton::Primary, at), button(false, egui::PointerButton::Primary, at)];
            std::iter::once(Event::PointerMoved(at)).chain(click.clone()).chain(click).collect()
        }
        5 => {
            let to = d.pos(size);
            vec![
                Event::PointerMoved(at),
                button(true, egui::PointerButton::Primary, at),
                Event::PointerMoved(to),
                button(false, egui::PointerButton::Primary, to),
            ]
        }
        6 => {
            let k = KEYS[d.below(KEYS.len() as u64) as usize];
            let modifiers = match d.below(4) {
                0 => Modifiers::COMMAND,
                1 => Modifiers::SHIFT,
                2 => Modifiers::ALT,
                _ => Modifiers::NONE,
            };
            vec![key(k, modifiers)]
        }
        7 => {
            let words = ["hello", "ana@x.example", " ", "is:unread", "שלום", "مرحبا", "日本語", "👋", "\u{202E}gpj.exe", "a\nb"];
            vec![Event::Text(words[d.below(words.len() as u64) as usize].to_string())]
        }
        8 => vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, d.below(7) as f32 - 3.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
                source: egui::MouseWheelSource::Wheel,
            },
        ],
        _ => vec![Event::PointerMoved(at)],
    }
}

/// Plays `actions` random actions with `seed`; any panic fails the test with the seed and step.
fn play(seed: u64, actions: usize) {
    let (mut a, dir) = app(&format!("soak-{seed}"));
    let ctx = egui::Context::default();
    let mut d = Dice(seed);
    let mut slowest = std::time::Duration::ZERO;
    let mut total = std::time::Duration::ZERO;
    let (mut dialogs, mut writing, mut selected) = (0, 0, 0);
    for step in 0..actions {
        // Now and then the window is narrow, or the theme changes.
        let size = if (step / 50) % 2 == 1 { egui::vec2(560.0, 700.0) } else { egui::vec2(1280.0, 800.0) };
        if step % 75 == 0 {
            a.ui.dark = Some(step % 150 == 0);
        }
        // Now and then Escape, as a person would, so the hand doesn't stay stuck in one place.
        let events = if step % 40 == 39 { vec![key(Key::Escape, Modifiers::NONE)] } else { action(&mut d, size) };
        let began = std::time::Instant::now();
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        // Let what was asked settle (a dialog opening, a pane laid out).
        frame(&ctx, &mut a, vec![]);
        let took = began.elapsed();
        if a.ui.dialog.is_some() {
            dialogs += 1;
        }
        if a.session.all_composes().next().is_some() {
            writing += 1;
        }
        if !a.session.selection.is_empty() {
            selected += 1;
        }
        slowest = slowest.max(took);
        total += took;
        if step % 10 == 9 {
            assert_eq!(PANICS_CAUGHT.load(Ordering::Relaxed), 0, "seed {seed}, step {step}: a command panicked (see the log)");
        }
    }
    assert_eq!(PANICS_CAUGHT.load(Ordering::Relaxed), 0, "seed {seed}: a command panicked");
    let mean = total / actions.max(1) as u32;
    eprintln!(
        "soak seed {seed}: {actions} actions, mean {mean:?}, slowest {slowest:?}; a dialog open {dialogs}, writing {writing}, selected {selected}"
    );
    // Debug builds on a busy CI machine: generous, but a frame that takes seconds is a bug.
    assert!(mean < std::time::Duration::from_millis(400), "seed {seed}: frames take {mean:?}");
    let _ = std::fs::remove_dir_all(dir);
}

use mailcraft_engine::PANICS_CAUGHT;

#[test]
fn the_window_survives_a_random_hand() {
    let (seeds, actions) = std::env::var("MAILCRAFT_SOAK")
        .ok()
        .and_then(|v| v.split_once(',').and_then(|(s, n)| Some((s.trim().parse().ok()?, n.trim().parse().ok()?))))
        .unwrap_or((3u64, 300usize));
    for seed in 1..=seeds {
        play(seed * 7919, actions);
    }
}
