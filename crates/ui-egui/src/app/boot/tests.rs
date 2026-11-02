//! The card before MailCraft is in the window, drawn headless.

use egui::{Pos2, RawInput, Rect, vec2};

use super::*;

/// One frame at `time` seconds: whether the window may go on, and how many shapes it drew.
fn frame(ctx: &egui::Context, card: &mut BootCard, progress: &OpenProgress, ready: bool, time: f64) -> (bool, usize) {
    with(ctx, card, progress, ready, time, Vec::new()).0
}

/// One frame at `time` seconds with `events`: as [`frame`], and where `label` was drawn.
fn with(
    ctx: &egui::Context,
    card: &mut BootCard,
    progress: &OpenProgress,
    ready: bool,
    time: f64,
    events: Vec<egui::Event>,
) -> ((bool, usize), Option<Pos2>) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), time: Some(time), events, ..Default::default() };
    let mut go = false;
    let mut out = ctx.run_ui(input, |ui| go = card.ui(ui, progress, ready));
    out.textures_delta.clear();
    let at = out.shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text() == "Continue" => Some(t.visual_bounding_rect().center()),
        _ => None,
    });
    ((go, out.shapes.len()), at)
}

/// Press the Continue button where the last frame drew it, at `time` seconds.
fn press_continue(ctx: &egui::Context, card: &mut BootCard, progress: &OpenProgress, time: f64) {
    let at = with(ctx, card, progress, false, time, Vec::new()).1.expect("Continue drawn");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    with(ctx, card, progress, false, time + 0.01, vec![egui::Event::PointerMoved(at)]);
    with(ctx, card, progress, false, time + 0.02, vec![button(true)]);
    with(ctx, card, progress, false, time + 0.03, vec![button(false)]);
}

/// The text a frame drew.
fn text(ctx: &egui::Context, card: &mut BootCard, progress: &OpenProgress, ready: bool, time: f64) -> String {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), time: Some(time), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        card.ui(ui, progress, ready);
    });
    out.textures_delta.clear();
    let mut s = String::new();
    for c in &out.shapes {
        if let egui::Shape::Text(t) = &c.shape {
            s.push_str(t.galley.text());
            s.push('\n');
        }
    }
    s
}

/// A start with nothing to check draws nothing: no card for its first moments, and none at all
/// once the files are open.
#[test]
fn a_clean_start_draws_nothing() {
    let ctx = egui::Context::default();
    let progress = OpenProgress::default();
    let mut card = BootCard::new("auto", None);
    for time in [10.0, 10.1, 10.2, 10.29] {
        assert_eq!(frame(&ctx, &mut card, &progress, false, time), (false, 0), "drawn at {time}");
    }
    progress.finish();
    assert_eq!(frame(&ctx, &mut card, &progress, false, 10.29), (false, 0), "open, MailCraft not yet");
    assert_eq!(frame(&ctx, &mut card, &progress, true, 10.29), (true, 0));
    // Done before the first frame: straight on.
    let mut card = BootCard::new("auto", None);
    assert_eq!(frame(&ctx, &mut card, &OpenProgress::finished(), true, 0.0), (true, 0));
}

/// Opening that takes longer shows a card; a data file being checked says which, how far, and of
/// how many.
#[test]
fn opening_then_a_check_is_shown() {
    let ctx = egui::Context::default();
    let progress = OpenProgress::default();
    progress.start(1);
    progress.opening("zoe@studio.example.mailcraft");
    let mut card = BootCard::new("auto", Some(false));
    frame(&ctx, &mut card, &progress, false, 0.0);
    let (go, shapes) = frame(&ctx, &mut card, &progress, false, 0.31);
    assert!(!go && shapes > 0, "no card after a moment");
    assert!(text(&ctx, &mut card, &progress, false, 0.4).contains("Opening your mail…"));
    progress.checking("zoe@studio.example.mailcraft", 0.0);
    let s = text(&ctx, &mut card, &progress, false, 0.5);
    assert!(s.contains("Checking your data file") && s.contains("zoe@studio.example.mailcraft wasn't closed properly"), "{s}");
    assert!(!s.contains("data files"), "one file: no count");
    assert!(s.contains("0% checked"), "{s}");
    progress.checking("zoe@studio.example.mailcraft", 0.6);
    assert_eq!(progress.permille(), 600);
    assert!(text(&ctx, &mut card, &progress, false, 0.6).contains("60% checked"));
    // Dark, and several files.
    let several = OpenProgress::default();
    several.start(3);
    several.opening("a.mailcraft");
    several.opened();
    several.opening("b.mailcraft");
    several.checking("b.mailcraft", 0.3);
    let mut card = BootCard::new("auto", Some(true));
    frame(&ctx, &mut card, &several, false, 0.0);
    let s = text(&ctx, &mut card, &several, false, 1.0);
    assert!(s.contains("b.mailcraft wasn't closed properly") && s.contains("1 of 3 data files"), "{s}");
    several.opened();
    several.opened();
    several.finish();
    assert_eq!(frame(&ctx, &mut card, &several, true, 1.1), (true, 0));
}

/// A data file that couldn't be opened is said at once, and the window goes on when Continue is
/// pressed (once what MailCraft opens with has come).
#[test]
fn a_file_that_couldnt_be_opened_is_said() {
    let ctx = egui::Context::default();
    let progress = OpenProgress::default();
    progress.start(2);
    progress.fail("zoe@studio.example.mailcraft", "it is open in another copy of MailCraft".into());
    progress.finish();
    let mut card = BootCard::new("auto", None);
    let s = text(&ctx, &mut card, &progress, true, 0.0);
    assert!(s.contains("Some of your mail couldn't be opened"), "said at once: {s}");
    assert!(s.contains("zoe@studio.example.mailcraft: it is open in another copy of MailCraft"), "{s}");
    assert!(s.contains("each account says what's wrong"), "{s}");
    assert!(!frame(&ctx, &mut card, &progress, true, 0.1).0, "waits for Continue");
    assert!(!card.may_go(&progress, true));
    press_continue(&ctx, &mut card, &progress, 0.2);
    assert!(card.may_go(&progress, true) && !card.may_go(&progress, false), "pressed");
    assert!(!frame(&ctx, &mut card, &progress, false, 0.3).0, "MailCraft not there yet");
    assert_eq!(frame(&ctx, &mut card, &progress, true, 0.4), (true, 0));
}

/// Opening that failed altogether (its thread stopped) says MailCraft runs without its cache, in
/// the interface's words: not the panic's, and not that accounts will say why.
#[test]
fn opening_that_failed_altogether_is_said_plainly() {
    let ctx = egui::Context::default();
    let progress = OpenProgress::default();
    progress.fail("", "called `Option::unwrap()` on a `None` value".into());
    progress.finish();
    let mut card = BootCard::new("auto", None);
    let s = text(&ctx, &mut card, &progress, true, 0.0);
    assert!(s.contains("MailCraft couldn't open its data files, so it runs without its mail cache this time."), "{s}");
    assert!(!s.contains("unwrap") && !s.contains("each account"), "{s}");
}

/// The window can go on with nothing drawn (minimized, or hidden): once the files are open and
/// what MailCraft opens with has come.
#[test]
fn going_on_needs_no_frame() {
    let progress = OpenProgress::default();
    let card = BootCard::new("auto", None);
    assert!(!card.may_go(&progress, true), "still opening");
    progress.finish();
    assert!(!card.may_go(&progress, false), "MailCraft not there yet");
    assert!(card.may_go(&progress, true));
}

/// Every state, for the tour of the interface (the translation tests draw it in the pseudo-language).
pub(crate) fn tour(ctx: &egui::Context) {
    let progress = OpenProgress::default();
    progress.start(2);
    progress.opening("a.mailcraft");
    let mut card = BootCard::new("auto", None);
    frame(ctx, &mut card, &progress, false, 0.0);
    frame(ctx, &mut card, &progress, false, 1.0);
    progress.checking("a.mailcraft", 0.5);
    frame(ctx, &mut card, &progress, false, 1.1);
    progress.fail("a.mailcraft", "failed".into());
    progress.finish();
    frame(ctx, &mut card, &progress, false, 1.2);
    let whole = OpenProgress::default();
    whole.fail("", "stopped".into());
    whole.finish();
    frame(ctx, &mut BootCard::new("auto", None), &whole, false, 0.0);
}
