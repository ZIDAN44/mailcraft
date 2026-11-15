//! Typing a search: the window steps it a part of each frame and never waits on it, and the
//! list and the status bar say how it's going (search.md §9).

use super::*;

/// A mailbox of `folders` folders of `per` messages each, in memory, indexed: every seventh
/// message's text says "zanzibar".
fn generated(folders: usize, per: u32) -> MailApp {
    use mailcraft_engine::model::{AccountConfig, Flags, Folder, FolderRole};
    let store = Store::memory();
    store.save_accounts(&[AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let list: Vec<Folder> = (0..folders)
        .map(|i| Folder {
            account: "a1".into(),
            path: if i == 0 { "INBOX".into() } else { format!("Folder {i}") },
            role: if i == 0 { FolderRole::Inbox } else { FolderRole::Other },
            selectable: true,
            ..Default::default()
        })
        .collect();
    store.save_folders("a1", &list).unwrap();
    let mut n = 0u32;
    for f in &list {
        let mut st = FolderState { validity: 1, ..Default::default() };
        for uid in 1..=per {
            n += 1;
            let key = MessageKey { account: "a1".into(), folder: f.path.clone(), validity: 1, uid };
            let text = if n.is_multiple_of(7) { "Ship the plan to zanzibar." } else { "Notes on the common plan." };
            store.write_body(&key, format!("From: a@x.example\r\nSubject: Notes {n}\r\n\r\n{text}\r\n").as_bytes()).unwrap();
            st.messages.push(
                Summary {
                    key,
                    subject: format!("Notes {n}"),
                    date: NOW - i64::from(n) * 60,
                    size: 200,
                    flags: Flags { seen: true, ..Default::default() },
                    ..Default::default()
                }
                .into(),
            );
        }
        mailcraft_engine::model::sort_newest_first(&mut st.messages);
        store.save_folder("a1", &f.path, &st).unwrap();
    }
    mailcraft_engine::sample::big::index(&store, "a1").unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW)))
}

/// The text drawn in a frame.
fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    drawn_strings(ctx, a, egui::vec2(1280.0, 800.0))
}

/// The text the message list draws, with nothing stepped first (as a frame between steps).
fn drawn_list(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| crate::panes::list::show(&a.session, &mut a.ui, ui, &mut Vec::new()));
    out.textures_delta.clear();
    texts_in(&out.shapes).into_iter().map(|d| d.text).collect()
}

/// "zanzibar" typed a key a frame into the box, then 192 frames more: for each frame that ran
/// some of the search, the units it ran and the time it spent on them.
fn typed_search(ctx: &egui::Context, a: &mut MailApp) -> Vec<(u64, std::time::Duration)> {
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(ctx, a, vec![]);
    assert!(a.ui.search_focused);
    let mut frames = Vec::new();
    for f in 0..200 {
        let typed = "zanzibar".chars().nth(f).map(|c| vec![Event::Text(c.to_string())]).unwrap_or_default();
        let before = a.session.search_stats().units;
        frame(ctx, a, typed);
        let ran = a.session.search_stats().units.saturating_sub(before);
        if ran > 0 {
            frames.push((ran, a.ui.search_spent));
        }
    }
    frames
}

/// A search typed a key a frame over twelve thousand messages is spread over frames rather than
/// holding one: each frame stops at its share of time (6 ms; a frame that waited for the search
/// ran all of it but the list put together, which a step begins with). The share is counted on
/// a clock of the test's, 2 ms a look (one after each unit), so no frame runs more than three
/// units whatever the machine and the build: on the machine's clock a quick one did the whole
/// search inside one share (2.6–4.8 ms in a release build), and a busy one's frames ran fewer
/// units. The times are in `a_typed_searchs_frames_measured`. It finds what a search that waits
/// finds.
#[test]
fn a_search_typed_never_holds_a_frame() {
    let ctx = egui::Context::default();
    let mut a = generated(6, 2000);
    a.ui.search_tick = Some(std::time::Duration::from_millis(2));
    let units: Vec<u64> = typed_search(&ctx, &mut a).into_iter().map(|(u, _)| u).collect();
    let (all, most) = (units.iter().sum::<u64>(), units.iter().max().copied().unwrap_or_default());
    assert!(all > 0, "the frames stepped the search");
    assert!(most <= 3, "each frame within its share: {units:?}");
    assert!(most * 2 < all, "no frame ran half the search: {units:?}");
    assert!(a.session.search_progress().is_some_and(|p| p.done), "done within 200 frames");
    assert_eq!(a.session.search.as_ref().map(|(q, _)| q.as_str()), Some("zanzibar"), "searched once typing paused");
    let found = a.session.list.len();
    assert_eq!(found, 12_000 / 7);
    let waited = a.session.run_search_with("zanzibar", mailcraft_engine::SearchScope::All, false);
    assert_eq!(waited, found, "the same as a search that waits");
}

/// The time each frame of the typed search spent on it (a release build's keeps to 8 ms: 6 ms,
/// and the unit it was in when the time ran out). Run by hand, in release, alone on the machine:
/// `cargo test --release -p mailcraft-ui-egui --lib a_typed_searchs_frames_measured -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run by hand"]
fn a_typed_searchs_frames_measured() {
    let ctx = egui::Context::default();
    let mut a = generated(6, 2000);
    let frames = typed_search(&ctx, &mut a);
    let units: Vec<u64> = frames.iter().map(|(u, _)| *u).collect();
    let mut spent: Vec<std::time::Duration> = frames.iter().map(|(_, t)| *t).collect();
    spent.sort();
    let at = |pct: usize| spent.get(spent.len().saturating_sub(1) * pct / 100).copied().unwrap_or_default();
    println!("MEASURE typed search: {} frames searching (units {units:?}), p50 {:?}, p99 {:?}, worst {:?}", spent.len(), at(50), at(99), at(100));
    assert!(a.session.search_progress().is_some_and(|p| p.done), "done within 200 frames");
}

/// The command that starts a search takes none of it in the window: run after the frame was
/// drawn, its four units held that frame for 16–20 ms on a real file. The next frame, asked for
/// at once, steps it within its share, and the frames after find it all.
#[test]
fn the_search_command_takes_none_of_the_search_in_the_window() {
    let ctx = egui::Context::default();
    let mut a = generated(3, 700);
    frame(&ctx, &mut a, vec![]);
    let before = a.session.search_stats().units;
    a.act(crate::Action::Run("search.run", json!({"query": "zanzibar", "scope": "all"})));
    assert_eq!(a.session.search_stats().units, before, "no unit of it in the command");
    assert!(a.session.search_progress().is_some_and(|p| !p.done));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.search_stats().units > before, "the next frame stepped it");
    for _ in 0..100 {
        if a.session.search_progress().is_some_and(|p| p.done) {
            break;
        }
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.search_progress().is_some_and(|p| p.done));
    assert_eq!(a.session.list.len(), 2_100 / 7);
}

/// While a search runs the list says so; mail the index doesn't have yet "may be missing", how
/// far the index has come with it, as the status bar shows; a date that isn't one is said; and
/// advanced search has the received dates.
#[test]
fn the_list_says_how_a_search_is_going() {
    let (mut a, _) = app("search-status");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let account = a.session.accounts[0].config.id.clone();
    // No index yet: what the summaries can't settle may be missing.
    a.run("search.run", json!({"query": "projector", "scope": "all"}));
    for _ in 0..20 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.search_progress().is_some_and(|p| p.done && p.missing > 0));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Some results may be missing: still indexing"), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("Indexing")), "no pass under way: {texts:?}");
    // A pass under way: how far, in the banner and the status bar.
    let work = mailcraft_engine::model::job::Work { done: 3, total: 10, ..Default::default() };
    a.session.apply(MailEvent::Indexed { account: account.clone(), work, generation: 1, failed: 0, finished: false });
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Some results may be missing: still indexing (30%)"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Indexing 30%"), "{texts:?}");
    // Still running: "Searching…" over an empty list, not "No results".
    a.run("search.run", json!({"query": "zzzz", "scope": "all"}));
    a.session.start_search("zzzz", mailcraft_engine::SearchScope::All, false);
    let texts = drawn_list(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Searching…"), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("No results")), "{texts:?}");
    a.session.apply(MailEvent::Indexed {
        account,
        work: mailcraft_engine::model::job::Work { done: 10, total: 10, ..Default::default() },
        generation: 1,
        failed: 0,
        finished: true,
    });
    // A date that isn't one.
    a.run("search.run", json!({"query": "received:2024-13 storyboard"}));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Couldn't read the date “received:2024-13”"), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("Indexing")), "the pass is over: {texts:?}");
    // A size that isn't one, and a search too long: said too.
    a.run("search.run", json!({"query": "larger:abc storyboard"}));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Couldn't read “larger:abc”"), "{texts:?}");
    let long: String = (0..70).map(|i| format!("w{i} ")).collect();
    a.run("search.run", json!({ "query": long }));
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "Part of a long search was left out"), "{texts:?}");
    // Advanced search's received dates, from the query and back to it.
    a.ui.search_text = "storyboard after:2024-03-01".into();
    let form = mailcraft_engine::model::search::Advanced::from_query(&a.ui.search_text);
    assert_eq!(form.received_after, "2024-03-01");
    a.ui.search_drop = crate::SearchDrop::Advanced(form);
    // (a new popup is drawn from its second frame)
    frame(&ctx, &mut a, vec![]);
    let texts = drawn(&ctx, &mut a);
    assert!(["Received after", "Received before", "2024-03-01"].iter().all(|w| texts.iter().any(|t| t == w)), "{texts:?}");
}
