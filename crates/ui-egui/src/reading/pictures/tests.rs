//! What the pictures kept cost to look through: a message's chips past the cap, and a burst of
//! textures let go.

use std::time::{Duration, Instant};

use super::*;

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid }
}

/// A texture to stand for a picture made (its size is what [`Slot::bytes`] says).
fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.load_texture("t", egui::ColorImage::filled([1, 1], egui::Color32::WHITE), egui::TextureOptions::default())
}

/// A message's chips past [`MAX_PER_MESSAGE`] get no slot; once its pictures are let go (it's
/// no longer shown), its chips are asked afresh, and another message has its own 200.
#[test]
fn a_messages_chips_are_counted_as_they_come_and_go() {
    let ctx = egui::Context::default();
    let mut p = Pictures::default();
    let (one, two) = (key(1), key(2));
    for i in 0..MAX_PER_MESSAGE + 5 {
        let got = p.get(&one, i, if i % 2 == 0 { Want::Thumb } else { Want::PdfFirst });
        assert_eq!(matches!(got, Got::Failed), i >= MAX_PER_MESSAGE, "chip {i}");
    }
    // A picture opened isn't a chip: it's asked past them.
    assert!(matches!(p.get(&one, MAX_PER_MESSAGE + 1, Want::Big), Got::Pending));
    assert!(matches!(p.get(&two, 0, Want::Thumb), Got::Pending));
    assert_eq!(p.chips.get(&one), Some(&MAX_PER_MESSAGE));
    // A frame in which they aren't shown: its pictures (waiting, or failed with no cache to read
    // them from) are forgotten, and their count with them.
    for _ in 0..2 {
        p.end_frame(&ctx, None, None, &Fonts::default(), &mut crate::NullServices(0));
    }
    assert!(p.chips.is_empty(), "{:?}", p.chips);
    assert!(matches!(p.get(&one, MAX_PER_MESSAGE + 3, Want::Thumb), Got::Pending));
    assert_eq!(p.chips.get(&one), Some(&1));
}

/// The textures let go, and the pictures past [`MAX_SLOTS`], are those the scan for the least
/// recently shown, one at a time, let go (as it was before, issue #488): over many kept sets,
/// with as many shown in each frame as not, the same ones are left.
#[test]
fn letting_go_leaves_what_it_did_one_at_a_time() {
    let ctx = egui::Context::default();
    let t = texture(&ctx);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n.max(1)
    };
    for round in 0..300 {
        let hasher = std::collections::hash_map::RandomState::new();
        let (mut now, mut before) = (Pictures::default(), Pictures::default());
        now.slots = HashMap::with_hasher(hasher.clone());
        before.slots = HashMap::with_hasher(hasher);
        let frame = 1 + next(20);
        let count = if round % 3 == 0 { MAX_SLOTS + next(300) as usize } else { next(400) as usize };
        for i in 0..count {
            let s = match next(4) {
                0 => State::Pending,
                1 => State::Failed,
                2 => State::Over,
                _ => State::Ready(t.clone()),
            };
            let bytes = if matches!(s, State::Ready(_)) { next(5_000) as usize } else { 0 };
            let used = next(frame + 1);
            let id = (key(next(6) as u32), i, if next(2) == 0 { Want::Thumb } else { Want::Body });
            let twin = match &s {
                State::Ready(t) => State::Ready(t.clone()),
                State::Pending => State::Pending,
                State::Failed => State::Failed,
                State::Over => State::Over,
            };
            now.slots.insert(id.clone(), Slot { state: s, bytes, used });
            before.slots.insert(id.clone(), Slot { state: twin, bytes, used });
            if id.2.chip() {
                *now.chips.entry(id.0).or_insert(0) += 1;
            }
        }
        for p in [&mut now, &mut before] {
            p.frame = frame;
            p.bytes = p.slots.values().map(|s| s.bytes).sum();
        }
        let target = next(now.bytes as u64 + 1) as usize;
        now.evict_to(target);
        evict_to_one_at_a_time(&mut before, target);
        let left = |p: &Pictures| {
            let mut ids: Vec<(u32, usize, bool)> = p.slots.keys().map(|(k, i, w)| (k.uid, *i, w.chip())).collect();
            ids.sort();
            ids
        };
        assert_eq!(left(&now), left(&before), "round {round}");
        assert_eq!(now.bytes, before.bytes, "round {round}");
        now.check_chips();
    }
}

/// [`Pictures::evict_to`] as it was: the least recently shown looked for again for each one let go.
fn evict_to_one_at_a_time(p: &mut Pictures, target: usize) {
    let frame = p.frame;
    while p.bytes > target {
        let oldest = p
            .slots
            .iter()
            .filter(|(_, s)| matches!(s.state, State::Ready(_)) && s.used < frame)
            .min_by_key(|(_, s)| s.used)
            .map(|(id, _)| id.clone());
        let Some(id) = oldest else { break };
        if let Some(s) = p.slots.remove(&id) {
            p.bytes = p.bytes.saturating_sub(s.bytes);
        }
    }
    while p.slots.len() > MAX_SLOTS {
        let oldest = p.slots.iter().filter(|(_, s)| s.used < frame).min_by_key(|(_, s)| s.used).map(|(id, _)| id.clone());
        let Some(id) = oldest else { break };
        if let Some(s) = p.slots.remove(&id) {
            p.bytes = p.bytes.saturating_sub(s.bytes);
        }
    }
}

#[test]
#[ignore = "a measurement: run in release"]
fn measure_chips_past_the_cap_and_a_burst_let_go() {
    // A message with 1,000 pictures among 2,048 remembered: each chip past the 200th is asked
    // every frame and never gets a slot.
    let mut p = Pictures::default();
    for i in 0..MAX_SLOTS - MAX_PER_MESSAGE {
        p.get(&key(2), i, Want::Body);
    }
    const CHIPS: usize = 1000;
    const FRAMES: u32 = 50;
    let k = key(1);
    for i in 0..CHIPS {
        p.get(&k, i, Want::Thumb);
    }
    let start = Instant::now();
    for _ in 0..FRAMES {
        for i in 0..CHIPS {
            std::hint::black_box(p.get(&k, i, Want::Thumb).ready());
        }
    }
    println!("MEASURE {CHIPS} chips of one message asked, {} pictures remembered: {:?} a frame", p.slots.len(), start.elapsed() / FRAMES);

    // Every texture kept let go at once (a big picture landing with no room).
    let ctx = egui::Context::default();
    let t = texture(&ctx);
    const ROUNDS: u32 = 20;
    let mut took = Duration::ZERO;
    for _ in 0..ROUNDS {
        let mut p = Pictures::default();
        for i in 0..MAX_SLOTS {
            p.slots.insert((key(3), i, Want::Thumb), Slot { state: State::Ready(t.clone()), bytes: 1, used: (i % 97) as u64 });
        }
        p.bytes = MAX_SLOTS;
        p.frame = 100;
        let start = Instant::now();
        p.evict_to(0);
        took += start.elapsed();
        assert!(p.slots.is_empty());
    }
    println!("MEASURE {MAX_SLOTS} textures let go at once: {:?}", took / ROUNDS);
}
