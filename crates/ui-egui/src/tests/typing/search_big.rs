//! A mailbox the size of a real file's, made up ([`sample::big`]: 50,000 messages in 40
//! folders, the Inbox 12,000), in the window: a search typed over all of it never holds a frame
//! for more than its share, and Down through the Inbox builds no rows (search.md §11). The
//! times are checked in a release build only: a debug build's units are slower, and run beside
//! the other tests a frame of it can take a second. The two tests here run one at a time (side by
//! side, each one's frames measured the other's work).

use super::*;
use mailcraft_engine::sample::big;

/// Held by each test here while it runs.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The window on the mailbox, indexed, its Inbox shown.
fn big_app() -> (MailApp, big::Truth) {
    let store = Store::memory();
    let truth = big::generate(&store, NOW, big::Shape::default()).unwrap();
    big::index(&store, big::ACCOUNT).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), truth)
}

/// The `n`th value of `times` sorted, by a percentage (`99`: the 99th percentile).
fn percentile(times: &[std::time::Duration], pct: usize) -> std::time::Duration {
    let mut t = times.to_vec();
    t.sort();
    t.get((t.len().saturating_sub(1)) * pct / 100).copied().unwrap_or_default()
}

/// Queries typed a key a frame over 50,000 messages: the search is spread over frames and finds
/// what the mailbox holds; in a release build each frame spends at most its share of time on it
/// (6 ms, and the unit it was in when the time ran out: 8 ms in all), the frame that shows
/// 35,000 found at once 16 ms. One word, and one that finds 35,000.
#[test]
fn a_search_typed_over_fifty_thousand_never_holds_a_frame() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = egui::Context::default();
    let (mut a, t) = big_app();
    frame(&ctx, &mut a, vec![]);
    let (word, many) = (std::time::Duration::from_millis(8), std::time::Duration::from_millis(16));
    for (q, want, most) in [(big::RARE, t.rare, word), ("is:read", t.read, many)] {
        frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
        a.ui.search_text.clear();
        frame(&ctx, &mut a, vec![]);
        assert!(a.ui.search_focused);
        let mut spent = Vec::new();
        for f in 0..400 {
            let typed = q.chars().nth(f).map(|c| vec![Event::Text(c.to_string())]).unwrap_or_default();
            frame(&ctx, &mut a, typed);
            if !a.ui.search_spent.is_zero() {
                spent.push(a.ui.search_spent);
            }
            let searched = a.session.search.as_ref().is_some_and(|(s, _)| s == q);
            if searched && a.session.search_progress().is_some_and(|p| p.done) {
                break;
            }
        }
        let worst = spent.iter().max().copied().unwrap_or_default();
        eprintln!("{q}: {} frames searching, p50 {:?}, p99 {:?}, worst {worst:?}", spent.len(), percentile(&spent, 50), percentile(&spent, 99));
        assert!(spent.len() >= 2, "{q}: spread over frames");
        if !cfg!(debug_assertions) {
            assert!(worst <= most, "{q}: a frame spent {worst:?} on the search");
        }
        assert!(a.session.search_progress().is_some_and(|p| p.done), "{q}: done");
        assert_eq!(a.session.search.as_ref().map(|(s, _)| s.as_str()), Some(q));
        assert_eq!(a.session.list.len(), want, "{q}");
    }
}

/// Down through the 12,000 messages of the Inbox, a press a frame, each message read as it's
/// landed on: the list's rows are never built again (each Down built them all, and the list
/// with them), and a release build's frames keep to 30 ms (p99).
#[test]
fn down_through_twelve_thousand_builds_no_rows() {
    let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = egui::Context::default();
    let (mut a, _) = big_app();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.list.len(), 12_000);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    let (built, read) = (a.session.nav_structure(), a.session.messages_shown());
    let mut times = Vec::new();
    for _ in 0..60 {
        let started = std::time::Instant::now();
        frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
        times.push(started.elapsed());
    }
    let p99 = percentile(&times, 99);
    eprintln!("60 Downs: p50 {:?}, p99 {p99:?}, worst {:?}", percentile(&times, 50), times.iter().max().copied().unwrap_or_default());
    assert_eq!(a.session.messages_shown(), read + 60, "each one read");
    assert_eq!(a.session.nav_structure(), built, "no rows built");
    assert_eq!(a.session.row_keys().iter().position(|k| a.session.focus.as_ref() == Some(k)), Some(60));
    if !cfg!(debug_assertions) {
        assert!(p99 <= std::time::Duration::from_millis(30), "a Down's frame took {p99:?}");
    }
}
