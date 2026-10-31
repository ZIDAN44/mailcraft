//! UI tests: a message's source in a window of its own (View message source, issue #27).

use super::*;

/// From the reading pane's More menu's command, a window with the source; asked for again it
/// stays one window, brought to the front; Escape closes it.
#[test]
fn a_source_window_opens_once_and_closes() {
    let (mut a, dir) = app("source-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.open", json!({"key": k.to_id()}));
    a.act(crate::Action::Run("message.source", json!({})));
    assert_eq!(a.ui.raise_source.as_ref(), Some(&k));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.raise_source.is_none(), "brought to the front once");
    a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.sources.len(), 1);
    assert!(!a.session.sources[0].pending && !a.session.sources[0].lines.is_empty());
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.sources.is_empty(), "Escape closes it");
    // Nothing chosen: said, not a window.
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.act(crate::Action::Run("message.source", json!({})));
    assert!(a.session.sources.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// A source of 30 MB: its window draws quickly, scrolled through and its header alone, without
/// laying out all of it.
#[test]
fn a_huge_source_draws_quickly() {
    let (mut a, dir) = app("source-huge");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    let mut raw = b"Subject: big\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec();
    let line = [b'Q'; 76];
    while raw.len() < 30 << 20 {
        raw.extend_from_slice(&line);
        raw.extend_from_slice(b"\r\n");
    }
    // And a line of 2 MB with nothing to break it.
    raw.extend(std::iter::repeat_n(b'x', 2 << 20));
    a.session.store.as_ref().unwrap().write_body(&k, &raw).unwrap();
    let began = std::time::Instant::now();
    a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
    let opened = began.elapsed();
    assert!(a.session.sources[0].truncated);
    let began = std::time::Instant::now();
    for i in 0..20 {
        let at = egui::pos2(640.0, 400.0);
        frame(
            &ctx,
            &mut a,
            vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Page,
                    delta: egui::vec2(0.0, -3.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                },
            ],
        );
        if i == 10 {
            a.ui.source_headers.insert(k.clone());
        }
    }
    let drawn = began.elapsed();
    eprintln!("30 MB source: prepared in {opened:?}, 20 frames in {drawn:?}");
    assert!(opened.as_secs() < 5 && drawn.as_secs() < 20, "{opened:?} {drawn:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The window is filled with lines to its foot: the rows laid out are counted at the height
/// they're drawn at (they were counted with the spacing under each, which the lines don't
/// have, and the window's lower third stayed empty).
#[test]
fn a_source_fills_its_window() {
    let v =
        mailcraft_engine::SourceView { lines: (0..400).map(|i| format!("Line {i}")).collect(), header_lines: 3, size: 4000, ..Default::default() };
    let t = crate::theme::Tokens::light();
    let ctx = egui::Context::default();
    let height = 600.0;
    let mut shown = 0..0;
    let mut row = 0.0;
    for _ in 0..2 {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, height))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
                row = ui.fonts_mut(|f| f.row_height(&egui::FontId::monospace(12.5)));
                shown = crate::app::source::source_text(ui, &t, &v, false);
            });
        });
        out.textures_delta.clear();
    }
    assert_eq!(shown.start, 0);
    assert!(shown.len() as f32 * row >= height - row, "{} lines of {row} in {height}", shown.len());
}
