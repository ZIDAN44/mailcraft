//! Long texts' tests: which texts keep their measurements, and what a frame of one costs.

use super::*;

fn id(n: u64) -> egui::Id {
    egui::Id::unique(("long-text-test", n))
}

/// Show text `n` at `now`, as `plain` and `rich` do: its state taken out, measured, put back.
/// Whether it still had its measurement from before.
fn show(st: &mut UiState, n: u64, now: f64) -> bool {
    let mut v = take(st, id(n));
    v.fit(100, 1);
    let had = v.measured.first() == Some(&Some(n as f32));
    if let Some(m) = v.measured.get_mut(0) {
        *m = Some(n as f32);
    }
    put_back(st, id(n), v, now);
    had
}

/// Nine long texts shown in the same frames (the reading pane and eight messages in windows)
/// keep their measurements from frame to frame: a ninth emptied the lot, so each was measured
/// again every frame and no scroll bar settled.
#[test]
fn nine_texts_shown_together_keep_their_measurements() {
    let mut st = UiState::default();
    for n in 0..9 {
        show(&mut st, n, 1.0);
    }
    for frame in 1..4 {
        let now = 1.0 + f64::from(frame) / 60.0;
        assert!((0..9).all(|n| show(&mut st, n, now)), "frame {frame}");
    }
}

/// Texts not shown for a while go once more than eight are kept, the latest eight staying;
/// however many are shown at once, no more than `MOST` are kept.
#[test]
fn texts_not_shown_for_a_while_go() {
    let mut st = UiState::default();
    // One a minute: the ninth sends the first away, the eight after it stay.
    for n in 0..9 {
        show(&mut st, n, 60.0 * n as f64);
    }
    assert_eq!(st.long_text.len(), KEPT);
    assert!(!st.long_text.contains_key(&id(0)) && (1..9).all(|n| st.long_text.contains_key(&id(n))));
    // A hundred at once: the latest `MOST` stay.
    let mut st = UiState::default();
    for n in 0..100 {
        show(&mut st, n, 1.0 + n as f64 / 1000.0);
    }
    assert_eq!(st.long_text.len(), MOST);
    assert!((100 - MOST as u64..100).all(|n| st.long_text.contains_key(&id(n))));
}

/// The pictures of a part not laid out are those drawing it shows, however deep its tables go:
/// a box placed over a cell holds what's one table deeper than the cell's own table, as drawn
/// (they were looked for a table deeper still, so a table in a box at the deepest drawn had its
/// pictures shown again after the text).
#[test]
fn a_parts_pictures_are_looked_for_as_deep_as_it_is_drawn() {
    use mime::layout::{Block, Cell, Overlay, Table};
    let table = |cell: Cell| Block::Table(Table { rows: vec![vec![cell]], ..Default::default() });
    let picture = |src: &str| Block::Text(vec![vec![mime::Span { image: Some(src.into()), ..Default::default() }]]);
    let session = Session::open(None, mailcraft_engine::model::dates::Clock::new(0, 0));
    let (key, parsed) = (MessageKey::default(), mime::Parsed::default());
    for depth in 12..=MAX_NESTED {
        // A table `depth` tables down, with a picture in a cell, and one in a table in a box over it.
        let boxed = Overlay { blocks: vec![table(Cell { blocks: vec![picture("cid:boxed")], ..Default::default() })], ..Default::default() };
        let mut b = table(Cell { blocks: vec![picture("cid:cell")], overlays: vec![boxed], ..Default::default() });
        for _ in 0..depth {
            b = table(Cell { blocks: vec![b], ..Default::default() });
        }
        let blocks = [b];
        let mut drawn = Vec::new();
        let ctx = egui::Context::default();
        let mut st = UiState::default();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let mut acts = Vec::new();
            let mut body = Body { s: &session, key: &key, p: &parsed, allowed: false, shown: &mut drawn, acts: &mut acts, cells: None, held: None };
            body.blocks(&mut st, ui, &blocks, 800.0, [255; 3], mime::layout::Align::Left, 0);
        });
        out.textures_delta.clear();
        let mut looked = Vec::new();
        pictures_of_blocks(&blocks, &mut looked, 0);
        drawn.sort();
        looked.sort();
        assert_eq!(looked, drawn, "{depth} tables down");
    }
}

/// A plain text shown where another as long was (the same message, read again) is cut for
/// itself and reads its own way: the first one's cuts were kept by its length, some of them
/// inside a letter of the second (those chunks drew as nothing), and its direction.
#[test]
fn a_text_as_long_as_the_one_before_is_cut_for_itself() {
    let first = "Quarterly figures, region by region.\n".repeat(1_000);
    // As many bytes: two-byte letters, right-to-left, in lines of another length.
    let line = format!("{}\n", "\u{5e9}".repeat(40));
    let mut second = line.repeat(first.len() / line.len());
    second.push_str(&"x".repeat(first.len() - second.len()));
    assert_eq!(first.len(), second.len());
    assert_ne!(stamp(&first), stamp(&second));
    let ctx = egui::Context::default();
    let mut st = UiState::default();
    let mut kept = |text: &str| {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| plain(&mut st, ui, id(1), text));
        });
        out.textures_delta.clear();
        st.long_text.get(&id(1)).and_then(|v| v.plain.as_ref()).map(|p| (p.ranges.clone(), p.rtl))
    };
    assert_eq!(kept(&first), Some((chunks(&first, CHUNK), false)));
    assert_eq!(kept(&first), Some((chunks(&first, CHUNK), false)), "kept for the same text");
    let (ranges, rtl) = kept(&second).unwrap();
    assert!(ranges.iter().all(|r| second.get(r.clone()).is_some()), "no cut inside a letter");
    assert_eq!((ranges, rtl), (chunks(&second, CHUNK), true));
    // Texts that differ in one byte, wherever it is, are told apart.
    let text = "a".repeat(100);
    for at in 0..text.len() {
        let mut other = text.clone().into_bytes();
        other[at] = b'b';
        assert_ne!(stamp(&text), stamp(&String::from_utf8(other).unwrap()), "{at}");
    }
}

/// A frame of a long plain text, in view at its top: how long it takes (a text of 4 MB was cut
/// in chunks and looked through for right-to-left letters on every frame).
#[test]
#[ignore = "a measurement: run in release"]
fn measure_a_frame_of_a_long_plain_text() {
    let line = "Quarterly figures, region by region, and what the numbers mean for next year.\n";
    let text = line.repeat(4_000_000 / line.len());
    let ctx = egui::Context::default();
    let mut st = UiState::default();
    let mut frame = || {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 700.0))), ..Default::default() };
        let start = std::time::Instant::now();
        let _ = ctx.run_ui(input, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| plain(&mut st, ui, id(1), &text));
        });
        start.elapsed()
    };
    for _ in 0..10 {
        frame();
    }
    let mut times: Vec<std::time::Duration> = (0..40).map(|_| frame()).collect();
    times.sort();
    let median = times.get(times.len() / 2).copied().unwrap_or_default();
    println!("MEASURE long plain text frame ({} bytes): median {median:?}", text.len());
}
