//! Issue #304: a message shown in the reading pane is marked read after the wait chosen (File ›
//! Options › Mail), at its time: the window wakes for it. It waited for the window's next look,
//! up to 20 seconds on with nothing else going on.

use super::*;

/// How soon the frame asks to be drawn again.
fn wakes_in(ctx: &egui::Context, a: &mut MailApp) -> std::time::Duration {
    let out = sized_frame(ctx, a, egui::vec2(1280.0, 800.0), vec![]);
    out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).expect("the main window's output")
}

#[test]
fn the_window_wakes_when_a_message_shown_is_due_to_be_read() {
    let (mut a, _) = app("read-wait");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let key = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.session.selection = vec![key.clone()];
    a.run("mail.markUnread", json!({}));
    a.run("view.readingOptions", json!({"markRead": true, "when": "shown", "after": 2}));
    // Until nothing animates any more.
    let settled = |ctx: &egui::Context, a: &mut MailApp| {
        (0..200).map(|_| wakes_in(ctx, a)).find(|d| *d > std::time::Duration::from_millis(500)).unwrap_or_default()
    };
    let idle = settled(&ctx, &mut a);
    a.session.open_message(&key);
    let waiting = settled(&ctx, &mut a);
    assert!(!a.session.summary(&key).is_some_and(|m| m.flags.seen), "not read yet");
    assert!(waiting <= std::time::Duration::from_millis(2100), "woken within its 2 seconds: {waiting:?} (idle: {idle:?})");
    assert!(idle > std::time::Duration::from_millis(2100), "nothing waiting, it isn't woken that soon: {idle:?}");
}
