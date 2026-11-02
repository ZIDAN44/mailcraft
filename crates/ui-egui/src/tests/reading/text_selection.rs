//! A message's text selected as one text: Ctrl+A takes all of it, Shift with the arrows goes from
//! line to line, and a drag past the view's edge scrolls on (in the reading pane and in a
//! message's own window).

use super::*;

/// Formatted mail of every kind of block: paragraphs, bold words, a quote, a table's cells side
/// by side, a right-to-left line and a link.
fn mixed() -> Vec<u8> {
    let html = "<html><body><p>First paragraph with <b>bold words</b> in it.</p>\
        <blockquote>Quoted line one<br>Quoted line two</blockquote>\
        <table><tr><td>Left cell</td><td>Right cell</td></tr></table>\
        <p dir=rtl>\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}</p>\
        <p>Last paragraph, <a href=\"https://example.com/x\">a link</a> at its end.</p></body></html>";
    html_message(html)
}

fn html_message(html: &str) -> Vec<u8> {
    format!("From: Sam <sam@studio.example>\r\nSubject: Words\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n").into_bytes()
}

/// `n` lines, each a block of its own.
fn lines(n: usize) -> Vec<u8> {
    let body: String = (0..n).map(|i| format!("<div>Line {i:03} of the message.</div>")).collect();
    html_message(&format!("<html><body>{body}</body></html>"))
}

/// `raw` as the first message's, opened in the reading pane; where its text is drawn.
fn open(a: &mut MailApp, ctx: &egui::Context, raw: Vec<u8>) -> egui::Rect {
    frame(ctx, a, vec![]);
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    a.session.apply(MailEvent::Body { key, raw });
    for _ in 0..30 {
        frame(ctx, a, vec![]);
    }
    text_area(a, egui::Order::Background)
}

fn text_area(a: &MailApp, order: egui::Order) -> egui::Rect {
    a.ui.text_selection.shown().into_iter().find(|(_, o)| *o == order).map(|(r, _)| r).expect("the message's text is drawn")
}

/// The positions of `what` in `text`, which must all be there.
fn order_of(text: &str, what: &[&str]) -> Vec<usize> {
    what.iter().map(|w| text.find(w).unwrap_or_else(|| panic!("{w:?} not in {text:?}"))).collect()
}

/// Ctrl+A in the text of a message selects all of it, every block it's drawn as (it took the
/// paragraph under the selection), and a copy has it in order; the message list's selection is
/// left alone.
#[test]
fn ctrl_a_selects_the_whole_message_and_copies_it_in_order() {
    let (mut a, dir) = app("select-all-blocks");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, mixed());
    click(&ctx, &mut a, area.left_top() + egui::vec2(4.0, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(a.ui.text_selection.all());
    assert_eq!(a.session.selection.len(), 1, "the list's selection is as it was");
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    let at = order_of(
        &got,
        &[
            "First paragraph with bold words in it.",
            "Quoted line one",
            "Quoted line two",
            "Left cell",
            "Right cell",
            "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
            "Last paragraph, a link at its end.",
        ],
    );
    assert!(at.windows(2).all(|w| w[0] < w[1]), "in reading order: {got:?}");
    assert!(got.contains("in it.\n"), "a line break between blocks: {got:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// With the message list clicked after the text, Ctrl+A is the list's again: every message.
#[test]
fn ctrl_a_in_the_message_list_still_selects_its_messages() {
    let (mut a, dir) = app("select-all-list");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, mixed());
    click(&ctx, &mut a, area.left_top() + egui::vec2(4.0, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.session.selection.len(), 1);
    // A press on the list's empty end (below its rows) gives it the keyboard back.
    let list = egui::pos2(area.left() - 150.0, 790.0);
    let button = |pressed| Event::PointerButton { pos: list, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(list), button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    assert!(!a.ui.text_selection.all(), "the text lets go");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.selection.len() > 1, "{} selected", a.session.selection.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Shift with the arrows, End and Ctrl+End goes on across lines (each a block of its own, where
/// it stopped at the block's edge), and the message list doesn't move.
#[test]
fn shift_and_the_arrows_select_across_lines() {
    let (mut a, dir) = app("select-shift-arrows");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(40));
    let open_key = a.session.reading.as_ref().map(|r| r.key.clone());
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 8.0), Modifiers::NONE);
    let shift = |k| key(k, Modifiers::SHIFT);
    frame(&ctx, &mut a, vec![shift(Key::ArrowDown), shift(Key::ArrowDown), shift(Key::End)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "Line 000 of the message.\nLine 001 of the message.\nLine 002 of the message.");
    // On over the line's end: into the next line.
    frame(&ctx, &mut a, vec![shift(Key::ArrowRight), shift(Key::ArrowRight)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.ends_with("message.\nL"), "{got:?}");
    // Back with Shift+Left, a word at a time with Ctrl, past the line's start.
    let word = Modifiers { shift: true, ctrl: true, command: true, ..Modifiers::NONE };
    frame(&ctx, &mut a, vec![shift(Key::ArrowLeft), shift(Key::ArrowLeft), key(Key::ArrowLeft, word)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.ends_with("Line 002 of the "), "{got:?}");
    // Ctrl+Shift+End: to the end of the message.
    frame(&ctx, &mut a, vec![key(Key::End, word)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.starts_with("Line 000") && got.ends_with("Line 039 of the message."), "{got:?}");
    assert_eq!(a.session.selection.len(), 1, "the list's selection is as it was");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), open_key, "the same message is open");
    let _ = std::fs::remove_dir_all(dir);
}

/// Shift+Up from below goes back a line, and Shift+click moves the end to the line clicked.
#[test]
fn shift_up_and_shift_click_move_the_end() {
    let (mut a, dir) = app("select-shift-up");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(20));
    let row = area.height() / 20.0;
    // At the start of the fourth line, then Shift+Up twice: the second and third lines.
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 3.0 * row + 6.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::SHIFT), key(Key::ArrowUp, Modifiers::SHIFT)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "Line 001 of the message.\nLine 002 of the message.\n");
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 6.0 * row + 6.0), Modifiers::SHIFT);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "Line 003 of the message.\nLine 004 of the message.\nLine 005 of the message.\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A drag held below the reading pane scrolls the message on, and the selection grows with it
/// to lines that weren't in view (it stopped at the view's edge).
#[test]
fn a_drag_past_the_bottom_scrolls_and_keeps_selecting() {
    let (mut a, dir) = app("select-drag-scroll");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(200));
    let start = area.left_top() + egui::vec2(0.5, 8.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut a, vec![button(start, true)]);
    let below = egui::pos2(start.x + 40.0, 860.0);
    for _ in 0..40 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(below)]);
    }
    let moved = text_area(&a, egui::Order::Background);
    assert!(moved.top() < area.top() - 500.0, "scrolled: {} from {}", moved.top(), area.top());
    let held = copied(&ctx, &mut a, vec![Event::PointerMoved(below), Event::Copy]).expect("copied");
    for _ in 0..20 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(below)]);
    }
    let more = copied(&ctx, &mut a, vec![Event::PointerMoved(below), Event::Copy]).expect("copied");
    assert!(more.len() > held.len(), "the pointer still, it grows: {} then {}", held.len(), more.len());
    frame(&ctx, &mut a, vec![button(below, false)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.starts_with("Line 000") && got.contains("Line 060"), "{} lines", got.lines().count());
    // From a line in view up past the window's top: it scrolls back, selecting lines above.
    let mid = egui::pos2(start.x + 40.0, 500.0);
    let above = egui::pos2(start.x + 40.0, -60.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(mid), button(mid, true)]);
    for _ in 0..40 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(above)]);
    }
    let back = text_area(&a, egui::Order::Background).top();
    assert!(back > moved.top() + 500.0, "scrolled back: {back} from {}", moved.top());
    frame(&ctx, &mut a, vec![button(above, false)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.lines().count() > 30, "{got:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// In a message's own window: Ctrl+A and Shift with the arrows select its text as in the reading
/// pane, and the copy is its text.
#[test]
fn a_message_window_selects_its_text_the_same_way() {
    let (mut a, dir) = app("select-window");
    let ctx = egui::Context::default();
    let _ = open(&mut a, &ctx, lines(12));
    let k = a.session.reading.as_ref().map(|r| r.key.clone()).unwrap();
    let parsed = a.session.reading.as_ref().and_then(|r| r.parsed.clone());
    a.run("message.popOut", json!({"key": k.to_id()}));
    // The window shows the message as the reading pane has it (the store has the sample's).
    a.session.windows.iter_mut().for_each(|w| w.parsed = parsed.clone());
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    // Without a native backend the window is drawn inside the main one, over it.
    let area = text_area(&a, egui::Order::Middle);
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT), key(Key::End, Modifiers::SHIFT)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "Line 000 of the message.\nLine 001 of the message.");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.starts_with("Line 000") && got.ends_with("Line 011 of the message."), "{got:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A plain message, and one in Hebrew: Ctrl+A copies the text as written, every line.
#[test]
fn ctrl_a_takes_a_plain_message_whole_in_either_direction() {
    for body in [
        "Dear Riley,\n\nThe cut is ready.\nSee you Tuesday.\n",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd},\n\n\u{5d4}\u{5d2}\u{5e8}\u{5e1}\u{5d4} \u{5de}\u{5d5}\u{5db}\u{5e0}\u{5d4}.\n",
    ] {
        let (mut a, dir) = app("select-plain");
        let ctx = egui::Context::default();
        let d = mailcraft_engine::mime::Draft { subject: "Plain".into(), body: body.into(), ..Default::default() };
        let from = mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example");
        let area = open(&mut a, &ctx, mailcraft_engine::mime::build(&d, &from, NOW, "<plain@x>").unwrap());
        let text = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.text.clone()).unwrap();
        click(&ctx, &mut a, area.center_top() + egui::vec2(0.0, 6.0), Modifiers::NONE);
        frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
        let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
        assert_eq!(got, text);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The wheel turned while a selection is dragged scrolls the message, and the selection follows
/// the text that comes under the pointer.
#[test]
fn the_wheel_scrolls_while_a_selection_is_dragged() {
    let (mut a, dir) = app("select-drag-wheel");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(200));
    let start = area.left_top() + egui::vec2(0.5, 8.0);
    let mid = egui::pos2(start.x + 40.0, 400.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut a, vec![button(start, true)]);
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(mid)]);
    }
    let before = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    for _ in 0..20 {
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -3.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        };
        frame(&ctx, &mut a, vec![Event::PointerMoved(mid), wheel]);
    }
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(mid)]);
    }
    assert!(text_area(&a, egui::Order::Background).top() < area.top() - 200.0, "scrolled");
    frame(&ctx, &mut a, vec![button(mid, false)]);
    let after = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(after.starts_with("Line 000") && after.lines().count() > before.lines().count() + 10, "{before:?} then {after:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The scroll bar beside the text drags the view, not a selection, and what was selected stays
/// selected (a press there let go of it).
#[test]
fn the_scroll_bar_scrolls_and_leaves_the_selection() {
    let (mut a, dir) = app("select-scroll-bar");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(200));
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT), key(Key::End, Modifiers::SHIFT)]);
    let selected = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    // Hovered first, the bar widens to where it can be grabbed.
    let bar = egui::pos2(area.right() + 20.0, area.top() + 40.0);
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(bar)]);
    }
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![button(bar, true)]);
    for y in 1..=10 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(bar + egui::vec2(0.0, y as f32 * 20.0))]);
    }
    frame(&ctx, &mut a, vec![button(bar + egui::vec2(0.0, 200.0), false)]);
    assert!(text_area(&a, egui::Order::Background).top() < area.top() - 200.0, "the bar scrolled the message");
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, selected);
    let _ = std::fs::remove_dir_all(dir);
}

/// Shift+End goes to the end of the line on the screen, past a bold word in it (it stopped at the
/// end of the words before the bold ones), and Shift+Home back to its start.
#[test]
fn shift_end_and_home_take_a_line_of_several_runs() {
    let (mut a, dir) = app("select-shift-end");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, mixed());
    click(&ctx, &mut a, area.left_top() + egui::vec2(0.5, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::End, Modifiers::SHIFT)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "First paragraph with bold words in it.");
    // From the line's end, Shift+Home: all of it again, the other way round.
    click(&ctx, &mut a, area.left_top() + egui::vec2(area.width() - 2.0, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::SHIFT)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "First paragraph with bold words in it.");
    let _ = std::fs::remove_dir_all(dir);
}

/// A press and the drag on from it arriving in one frame (a quick hand): the selection starts
/// where it was pressed, not where the pointer had got to.
#[test]
fn a_quick_drag_starts_where_it_was_pressed() {
    let (mut a, dir) = app("select-quick-drag");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(40));
    let row = area.height() / 40.0;
    let start = area.left_top() + egui::vec2(0.5, 8.0);
    let to = area.left_top() + egui::vec2(0.5, 2.0 * row + 8.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(start), button(start, true), Event::PointerMoved(to)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(to), button(to, false)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert_eq!(got, "Line 000 of the message.\nLine 001 of the message.\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A drag held on the last line in view of the reading pane, short of its edge: the message
/// stays, and so does the selection (it scrolled within 30 points of the edge, the selection
/// running on under a still pointer).
#[test]
fn a_drag_held_on_the_last_line_in_view_stays() {
    let (mut a, dir) = app("select-drag-last-line");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(200));
    let view = a.ui.text_selection.view(egui::Order::Background).expect("the text is in view");
    let start = area.left_top() + egui::vec2(0.5, 8.0);
    let last = egui::pos2(start.x + 40.0, view.bottom() - 6.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(start)]);
    frame(&ctx, &mut a, vec![button(start, true)]);
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(last)]);
    }
    let held = copied(&ctx, &mut a, vec![Event::PointerMoved(last), Event::Copy]).expect("copied");
    for _ in 0..40 {
        frame(&ctx, &mut a, vec![Event::PointerMoved(last)]);
    }
    let top = text_area(&a, egui::Order::Background).top();
    assert!((top - area.top()).abs() < 0.5, "it stays: {top} from {}", area.top());
    let still = copied(&ctx, &mut a, vec![Event::PointerMoved(last), Event::Copy]).expect("copied");
    assert_eq!(still, held, "the selection stays too");
    frame(&ctx, &mut a, vec![button(last, false)]);
    let _ = std::fs::remove_dir_all(dir);
}

/// On a touch screen, a finger dragged up over the text scrolls the message, as over egui's
/// labels there, and selects nothing (the drag selected text, and scrolled only at the edge).
#[test]
fn a_finger_dragged_over_the_text_scrolls_it() {
    let (mut a, dir) = app("select-touch");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, lines(200));
    let start = egui::pos2(area.left() + 40.0, 600.0);
    let finger = |phase, pos| Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force: None };
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    // The screen is known to be one by touches before (egui learns it from the first).
    frame(&ctx, &mut a, vec![finger(egui::TouchPhase::Start, start), finger(egui::TouchPhase::End, start)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![finger(egui::TouchPhase::Start, start), Event::PointerMoved(start), button(start, true)]);
    let mut at = start;
    for _ in 0..20 {
        at.y -= 15.0;
        frame(&ctx, &mut a, vec![finger(egui::TouchPhase::Move, at), Event::PointerMoved(at)]);
    }
    frame(&ctx, &mut a, vec![finger(egui::TouchPhase::End, at), button(at, false), Event::PointerGone]);
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    let top = text_area(&a, egui::Order::Background).top();
    assert!(top < area.top() - 200.0, "the finger scrolled it: {top} from {}", area.top());
    let got = copied(&ctx, &mut a, vec![Event::Copy]);
    assert!(got.as_deref().is_none_or(str::is_empty), "nothing selected: {got:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Another message shown, then this one again by the keyboard (Down, Up in the list): its text
/// shows unselected and Ctrl+A is the list's (the old selection came back, keys and all).
#[test]
fn a_message_shown_again_starts_unselected() {
    let (mut a, dir) = app("select-shown-again");
    let ctx = egui::Context::default();
    let area = open(&mut a, &ctx, mixed());
    let first = a.session.reading.as_ref().map(|r| r.key.clone()).unwrap();
    click(&ctx, &mut a, area.left_top() + egui::vec2(4.0, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert!(a.ui.text_selection.all());
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_ne!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(first.clone()), "the next message is shown");
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::NONE)]);
    a.session.apply(MailEvent::Body { key: first.clone(), raw: mixed() });
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(first), "back on the first");
    let _ = text_area(&a, egui::Order::Background);
    assert!(!a.ui.text_selection.all(), "it shows unselected");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(!a.ui.text_selection.all(), "Ctrl+A isn't the text's");
    assert!(a.session.selection.len() > 1, "but the list's: {} selected", a.session.selection.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// A message popped out into its own window: Ctrl+A there, without a click in it first, selects
/// its whole text (its text had the keys only once clicked into).
#[test]
fn a_message_window_has_the_keys_from_the_start() {
    let (mut a, dir) = app("select-window-keys");
    let ctx = egui::Context::default();
    let _ = open(&mut a, &ctx, lines(12));
    let k = a.session.reading.as_ref().map(|r| r.key.clone()).unwrap();
    let parsed = a.session.reading.as_ref().and_then(|r| r.parsed.clone());
    a.run("message.popOut", json!({"key": k.to_id()}));
    a.session.windows.iter_mut().for_each(|w| w.parsed = parsed.clone());
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    let _ = text_area(&a, egui::Order::Middle);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let got = copied(&ctx, &mut a, vec![Event::Copy]).expect("copied");
    assert!(got.starts_with("Line 000") && got.ends_with("Line 011 of the message."), "{got:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A over a line with bold words in it, on a screen at 125%: each run's shading meets the
/// next's edge to edge (they overlapped by a fraction of a pixel where the runs' places round to
/// pixels, a darker line down the selection, the shading being translucent).
#[test]
fn the_shading_has_no_seam_between_runs_on_a_line() {
    let (mut a, dir) = app("select-seam");
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.25);
    let area = open(&mut a, &ctx, mixed());
    click(&ctx, &mut a, area.left_top() + egui::vec2(4.0, 8.0), Modifiers::NONE);
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let bg = ctx.global_style().visuals.selection.bg_fill;
    // The shading's rectangles (each two triangles of its colour, spanning it) across the first line.
    let y = area.top() + 8.0;
    let mut shades: Vec<egui::Rect> = Vec::new();
    for p in ctx.tessellate(out.shapes, out.pixels_per_point) {
        let egui::epaint::Primitive::Mesh(m) = p.primitive else { continue };
        for t in m.indices.as_chunks::<3>().0 {
            let v: Vec<_> = t.iter().filter_map(|i| m.vertices.get(*i as usize)).collect();
            if v.len() == 3 && v.iter().all(|v| v.color == bg) {
                let r = egui::Rect::from_points(&[v[0].pos, v[1].pos, v[2].pos]);
                if r.y_range().contains(y) && !shades.contains(&r) {
                    shades.push(r);
                }
            }
        }
    }
    shades.sort_by(|a, b| a.left().total_cmp(&b.left()));
    assert!(shades.len() >= 3, "the words, the bold words and the rest shaded: {shades:?}");
    for w in shades.windows(2) {
        assert!((w[1].left() - w[0].right()).abs() < 0.01, "edge to edge: {} then {}", w[0].right(), w[1].left());
    }
    let _ = std::fs::remove_dir_all(dir);
}
