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
