//! Calendars subscribed to by their link, in the Calendar's side pane.

use super::*;

/// A frame of the Calendar with `events`: the texts it draws.
fn texts(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<Drawn> {
    drawn_with(ctx, a, windows::layout::SIZE.with(std::cell::Cell::get), events)
}

/// Where the text `what` is drawn, seen.
fn at(texts: &[Drawn], what: &str) -> egui::Pos2 {
    texts.iter().find(|d| d.text == what && d.seen()).map(|d| d.rect.center()).unwrap_or_else(|| {
        panic!("{what} drawn: {:?}", texts.iter().map(|d| d.text.as_str()).collect::<Vec<_>>());
    })
}

fn subscribed(a: &MailApp) -> usize {
    a.session.pim.values().map(|p| p.subscribed.len()).sum()
}

/// The Calendar with one calendar subscribed to, named Holidays (never read: there's no network
/// here), where its name is drawn, and the right end of the pane it's in.
fn with_holidays(name: &str) -> (MailApp, egui::Context, egui::Pos2, f32) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    a.run("view.calendar", json!({}));
    a.run("calendar.subscribe", json!({"url": "https://calendar.example/holidays.ics", "name": "Holidays"}));
    assert_eq!(subscribed(&a), 1);
    for _ in 0..2 {
        texts(&ctx, &mut a, vec![]);
    }
    let drawn = texts(&ctx, &mut a, vec![]);
    let pane = drawn.iter().find(|d| d.text == "Holidays").map_or(0.0, |d| d.clip.right());
    (a, ctx, at(&drawn, "Holidays"), pane)
}

/// Issue #119: a subscribed calendar couldn't be removed. Remove is on its
/// right-click menu, but its name, a text that could be selected, took the right-click, and the
/// row was only as wide as its name and tag: the menu opened from nowhere one would click. It
/// opens from the name and from the rest of the row, and Remove takes the calendar away.
#[test]
fn a_right_click_on_a_subscribed_calendar_removes_it() {
    let (mut a, ctx, name, pane) = with_holidays("calendar-unsubscribe");
    // The row's far end, past its name and tag, then its name.
    assert!(pane > name.x + 100.0, "{pane}");
    for p in [egui::pos2(pane - 12.0, name.y), name] {
        click_at(&ctx, &mut a, p, egui::PointerButton::Secondary, Modifiers::NONE);
        let menu = texts(&ctx, &mut a, vec![]);
        at(&menu, "Refresh");
        at(&menu, "Remove this calendar");
        assert_eq!(subscribed(&a), 1, "the menu is open, nothing done yet");
        if p != name {
            texts(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
            texts(&ctx, &mut a, vec![]);
        }
    }
    let remove = at(&texts(&ctx, &mut a, vec![]), "Remove this calendar");
    click(&ctx, &mut a, remove);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(subscribed(&a), 0, "removed");
    let left: Vec<String> = texts(&ctx, &mut a, vec![]).into_iter().map(|d| d.text).collect();
    assert!(!has(&left, "Holidays"), "{left:?}");
}

/// On a Mac, a Control-click is the right-click (issue #119: the reporter's was a Mac). It came as
/// a click, so no right-click menu opened on a Mac with no second button or two-finger click; now
/// it opens the calendar's, its release read with it though Control was let go first.
#[test]
fn a_control_click_on_a_mac_opens_a_calendars_menu() {
    let (mut a, ctx, name, _) = with_holidays("calendar-control-click");
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    // A frame whose input is read as each window's is on a Mac.
    let mac = |a: &mut MailApp, events: Vec<Event>| {
        let mut raw = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        crate::control_click(&ctx, &mut raw, true);
        let mut out = ctx.run_ui(raw, |ui| a.ui(ui));
        out.textures_delta.clear();
        texts_in(&out.shapes)
    };
    let control = Modifiers { ctrl: true, ..Default::default() };
    let press = |pressed, modifiers| Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers };
    mac(&mut a, vec![Event::ModifiersChanged(control), Event::PointerMoved(name)]);
    mac(&mut a, vec![press(true, control)]);
    mac(&mut a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    mac(&mut a, vec![press(false, Modifiers::NONE)]);
    let menu = mac(&mut a, vec![]);
    at(&menu, "Remove this calendar");
    assert_eq!(subscribed(&a), 1, "the menu is open, nothing done yet");
}
