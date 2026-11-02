//! Engine tests: searching a mailbox the size of a real file's, made up ([`sample::big`]: 50,000
//! messages in 40 folders, the Inbox 12,000), indexed as the indexer would (search.md §10–11).
//! The steps are bounded in what they read and check whatever the mailbox's size, and what's
//! found is what the mailbox was made with. How long they take is measured in a release build,
//! by hand (an ignored test, as `bench.rs`).

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::*;
use crate::sample::big::{self, Shape, Truth};
use crate::session::SearchStats;
use mailcraft_model::MessageKey;

/// The mailbox, made and indexed once for all the tests here (none of them changes it).
fn mailbox() -> &'static (Store, Truth) {
    static MADE: OnceLock<(Store, Truth)> = OnceLock::new();
    MADE.get_or_init(|| {
        let store = Store::memory();
        let truth = big::generate(&store, NOW, Shape::default()).unwrap();
        big::index(&store, big::ACCOUNT).unwrap();
        (store, truth)
    })
}

/// A session newly opened on the mailbox, searching in the background as the window does, and
/// ticked once, as the window ticks it before anyone can type: its index is open, nothing of it
/// read yet and no summary's text made.
fn cold() -> (Session, Truth) {
    let (mut s, truth) = unticked();
    s.tick();
    (s, truth)
}

/// [`cold`] before its tick.
fn unticked() -> (Session, Truth) {
    let (store, truth) = mailbox();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s.set_background_search(true);
    (s, *truth)
}

/// The queries measured, what each should find (`None`: too many words to count here).
fn queries(t: &Truth) -> Vec<(String, Option<usize>)> {
    vec![
        (big::RARE.to_string(), Some(t.rare)),
        (format!("\"{}\"", big::PHRASE), Some(t.phrase)),
        (big::DOMAIN.to_string(), Some(t.domain)),
        ("zurich".to_string(), Some(t.accented)),
        (big::CJK.to_string(), Some(t.cjk)),
        (big::FAR.to_string(), Some(t.far)),
        ("has:attachment".to_string(), Some(t.files)),
        ("is:unread".to_string(), Some(t.unread)),
        ("is:read".to_string(), Some(t.read)),
        ("k".to_string(), None),
    ]
}

/// What searches read in a step: `after` less `before`.
fn delta(before: SearchStats, after: SearchStats) -> SearchStats {
    SearchStats {
        folder_pages: after.folder_pages - before.folder_pages,
        reader_pages: after.reader_pages - before.reader_pages,
        summaries: after.summaries - before.summaries,
        texts: after.texts - before.texts,
        sources: after.sources - before.sources,
        units: after.units - before.units,
        summary_texts: after.summary_texts - before.summary_texts,
    }
}

/// One step of a search as the window takes it: what it read, how long it took, and how many
/// messages the list was built of, when it was built again.
struct Step {
    read: SearchStats,
    took: Duration,
    rebuilt: usize,
}

/// Search `q` as the window does (`search.run` in the background: its first units in the
/// command), then step it 8 units at a time until it's done: each step.
fn search(s: &mut Session, q: &str, scope: &str) -> Vec<Step> {
    search_by(s, q, scope, &mut |s| s.search_step(8).done)
}

/// [`search`], each step taken by `step`.
fn search_by(s: &mut Session, q: &str, scope: &str, by: &mut dyn FnMut(&mut Session) -> bool) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut step = |s: &mut Session, f: &mut dyn FnMut(&mut Session) -> bool| {
        let (before, structure) = (s.search_stats(), s.nav_structure());
        let started = Instant::now();
        let done = f(s);
        let took = started.elapsed();
        let rebuilt = if s.nav_structure() != structure { s.list.len() } else { 0 };
        steps.push(Step { read: delta(before, s.search_stats()), took, rebuilt });
        done
    };
    step(s, &mut |s| {
        s.run("search.run", &json!({"query": q, "scope": scope})).unwrap();
        false
    });
    for _ in 0..100_000 {
        if step(s, &mut *by) {
            return steps;
        }
    }
    panic!("{q}: a search that never ends");
}

/// Every query over every mailbox, as the window runs it: each `search_step(8)` reads at most
/// 16 store pages (folders' lists and the index together), checks at most 8 × 512 summaries and
/// 8 texts (the command's first 4 units, half that), makes at most 8 × 128 summaries' texts (in
/// a session just opened: 512 a unit took 11–15 ms a step), and reads no message; flags read
/// nothing of the index; and each finds what the mailbox holds, the list newest first.
#[test]
fn each_step_is_bounded_over_fifty_thousand_messages() {
    let (mut s, t) = cold();
    let reads = s.store.as_ref().unwrap().reads();
    for (q, want) in queries(&t) {
        let steps = search(&mut s, &q, "all");
        for (i, st) in steps.iter().enumerate() {
            let (d, n) = (st.read, if i == 0 { 4 } else { 8 });
            assert!(d.folder_pages + d.reader_pages <= 2 * n, "{q}: store pages {} + {}", d.folder_pages, d.reader_pages);
            assert!(d.summaries <= 512 * n, "{q}: summaries {}", d.summaries);
            assert!(d.summary_texts <= 128 * n, "{q}: summaries' texts made {}", d.summary_texts);
            assert!(d.texts <= n, "{q}: texts {}", d.texts);
            assert!(d.units <= n, "{q}: units {}", d.units);
        }
        let p = s.search_progress().unwrap();
        assert_eq!(p.missing, 0, "{q}: everything indexed");
        if let Some(n) = want {
            assert_eq!((p.found, s.list.len()), (n, n), "{q}");
        }
        assert!(s.list.windows(2).all(|w| w[0].when() >= w[1].when()), "{q}: newest first");
        if q.starts_with("is:") {
            assert_eq!(steps.iter().map(|st| st.read.reader_pages).sum::<u64>(), 0, "{q}: flags read nothing of the index");
        }
        assert!(steps.len() > 2, "{q}: {} steps", steps.len());
    }
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "no message read");
    assert_eq!(s.search_stats().sources, 0);
}

/// A common phrase found a few messages a step, over thousands of steps: the list is built
/// again only as often as its length allows, a few thousand messages' worth a step all told
/// (each step built all of it again: tens of thousands of messages a step by the end).
#[test]
fn a_common_phrase_doesnt_build_the_list_each_step() {
    let (mut s, t) = cold();
    let steps = search(&mut s, &format!("\"{}\"", big::PHRASE), "all");
    assert_eq!(s.list.len(), t.phrase);
    let rebuilt: usize = steps.iter().map(|st| st.rebuilt).sum();
    assert!(steps.len() > 1000, "{} steps", steps.len());
    assert!(rebuilt / steps.len() <= 6144, "{rebuilt} messages built into the list over {} steps", steps.len());
    // What's found still shows as it's found, not only at the end.
    let shown = steps.iter().filter(|st| st.rebuilt > 0).count();
    assert!(shown > 20, "built {shown} times");
}

/// A word searched again (from another view, or the same query run again) takes what the index
/// gave it the last time: nothing of the index is looked through for it, and it finds the same.
#[test]
fn a_word_searched_again_isnt_looked_up_again() {
    let (mut s, t) = cold();
    let look_ups = |s: &mut Session, q: &str| {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        let mut n = 0;
        while !s.search_step_timed(8, &mut || true, &mut |w| n += usize::from(w == crate::session::SearchWork::LookUp)).done {}
        n
    };
    let first = look_ups(&mut s, big::RARE);
    let again = look_ups(&mut s, big::RARE);
    assert_eq!(s.list.len(), t.rare);
    assert!(first > 2, "{first} units looking it up the first time");
    assert!(again <= 1, "{again} the second");
}

/// The common phrase searched again (typed on, or searched from another view) checks the texts
/// it folded the first time: nothing of them is read or folded again, and it finds the same. The
/// next search lets go of what their texts said a slice at a time.
#[test]
fn a_phrase_searched_again_reads_no_text() {
    let (mut s, t) = cold();
    let q = format!("\"{}\"", big::PHRASE);
    let first = search(&mut s, &q, "all");
    let second = search(&mut s, &q, "all");
    let pages = |steps: &[Step]| steps.iter().map(|st| st.read.reader_pages).sum::<u64>();
    assert_eq!(s.list.len(), t.phrase);
    assert!(pages(&first) > 50, "{} pages the first time", pages(&first));
    assert_eq!(pages(&second), 0, "nothing read the second time");
    assert!(second.len() * 3 < first.len() * 2, "{} steps the second time, {} the first", second.len(), first.len());
    // What each candidate's text said goes a slice a tick, not in the frame that searches next.
    s.run("search.run", &json!({"query": big::RARE, "scope": "all"})).unwrap();
    assert!(s.answers_letting_go() > 10_000, "{}", s.answers_letting_go());
    for _ in 0..40 {
        s.tick();
    }
    assert_eq!(s.answers_letting_go(), 0);
}

/// A phrase's candidates are checked in the index's order (each page of texts read once), the
/// folder on view still first (one whose mail was indexed late, a third of it with the phrase):
/// what's on screen is found first of what's checked against its text.
#[test]
fn a_phrase_checks_the_folder_on_view_first() {
    let (mut s, t) = cold();
    s.run("folder.open", &json!({"account": big::ACCOUNT, "folder": "Folder 37"})).unwrap();
    s.run("search.run", &json!({"query": format!("\"{}\"", big::PHRASE), "scope": "all"})).unwrap();
    let found = |s: &Session| -> HashSet<MessageKey> { s.list.iter().map(|m| m.key.clone()).chain(s.search_unshown()).collect() };
    // Every folder scanned (what their summaries say found): the texts checked from here.
    while s.search_stats().texts == 0 {
        assert!(!s.search_step(1).done);
    }
    let before = found(&s);
    let mut checked = HashSet::new();
    while checked.len() < 20 && !s.search_step(1).done {
        checked.extend(found(&s).difference(&before).cloned());
    }
    assert!(!checked.is_empty() && checked.iter().all(|k| k.folder == "Folder 37"), "{} checked first", checked.len());
    while !s.search_step(8).done {}
    assert_eq!(s.list.len(), t.phrase);
}

/// Nothing in the cache: a search of the mailbox reads each folder's list a page a unit (the
/// Inbox's 12,000 summaries over several), keeps them while it runs and finds what's there.
#[test]
fn a_mailbox_search_from_a_cold_cache() {
    let (mut s, t) = cold();
    s.set_cache_limit(0);
    let steps = search(&mut s, big::RARE, "mailbox");
    let pages: u64 = steps.iter().map(|st| st.read.folder_pages).sum();
    // (The Inbox, on view, stays.)
    assert!(pages >= 39, "each folder's list read: {pages} pages");
    assert!(steps.iter().skip(1).all(|st| st.read.folder_pages <= 8 && st.read.summaries <= 8 * 512));
    assert_eq!(s.list.len(), t.rare);
    // Kept for it, let go once it's done.
    assert_eq!(s.search_pinned(), 0);
}

/// Thirty-five thousand found (`is:read`): shown as they're found, in batches, the list never
/// shorter; once done, all of them newest first, the selection kept. The next search lets go of
/// them a slice a step or tick, not all in the frame that searches.
#[test]
fn thirty_five_thousand_found_stream_in() {
    let (mut s, t) = cold();
    s.run("search.run", &json!({"query": "is:read", "scope": "all"})).unwrap();
    // The first found show by the next step's start.
    s.search_step(1);
    let first = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.select", &json!({"key": first.to_id()})).unwrap();
    let (mut last, mut batches, mut steps) = (s.list.len(), 0, 0);
    loop {
        let p = s.search_step(8);
        steps += 1;
        assert!(s.list.len() >= last, "never fewer");
        batches += usize::from(s.list.len() > last);
        last = s.list.len();
        assert!(s.list.len() <= p.found);
        if p.done {
            break;
        }
    }
    assert_eq!(s.list.len(), t.read);
    assert!(batches >= 3, "{batches} batches over {steps} steps");
    assert!(s.list.windows(2).all(|w| w[0].when() >= w[1].when()), "newest first");
    assert!(s.list.iter().all(|m| m.flags.seen));
    assert_eq!(s.selection, [first], "the selection is kept");
    s.run("search.run", &json!({"query": big::RARE, "scope": "all"})).unwrap();
    assert!(s.letting_go() > 20_000, "{}", s.letting_go());
    for _ in 0..10 {
        s.tick();
    }
    assert_eq!(s.letting_go(), 0);
}

/// The list a run shows at its end is put together as a step's first unit, never after others:
/// begun late in the window's frame, showing 35,000 found ran past it by 5-8 ms.
#[test]
fn the_end_of_a_run_starts_a_step() {
    let (mut s, t) = cold();
    s.run("search.run", &json!({"query": "is:read", "scope": "all"})).unwrap();
    assert!(!s.search_step(u32::MAX).done, "everything read, the list not put together yet");
    assert!(s.list.len() < t.read, "{} shown", s.list.len());
    assert!(s.search_step(1).done);
    assert_eq!(s.list.len(), t.read);
}

/// Clearing a search of thirty-five thousand found, from the Inbox's 12,000: the Inbox's list
/// comes back as it was (not copied out of the cache, arranged and put into conversations
/// again), and what the search showed and held is let go of a slice a tick, not in the frame
/// that clears it (on a real file's 10,941-message folder that frame took 56–91 ms). A message
/// marked read meanwhile builds it again, with the mark.
#[test]
fn clearing_a_big_search_shows_the_folder_kept() {
    let (mut s, t) = cold();
    s.run("folder.open", &json!({"account": big::ACCOUNT, "folder": "INBOX"})).unwrap();
    let inbox: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    assert!(inbox.len() >= 10_000, "{}", inbox.len());
    let threadings = s.nav.threadings();
    let clear = |s: &mut Session| {
        let began = Instant::now();
        s.run("search.run", &json!({"query": ""})).unwrap();
        began.elapsed()
    };
    for round in 0..2 {
        s.run("search.run", &json!({"query": "is:read", "scope": "all"})).unwrap();
        while !s.search_step(64).done {}
        assert_eq!(s.list.len(), t.read);
        // What the last clear let go of is gone by now.
        assert_eq!(s.letting_go(), 0, "round {round}");
        let structure = s.nav_structure();
        let took = clear(&mut s);
        eprintln!("cleared {} found in {took:?}", t.read);
        assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), inbox, "the Inbox as it was");
        assert_ne!(s.nav_structure(), structure, "its rows are drawn again");
        assert_eq!(s.nav.threadings(), threadings, "its conversations not worked out again");
        assert!(s.letting_go() > 30_000, "{}", s.letting_go());
        // Each tick lets go of a slice.
        let before = s.letting_go();
        s.tick();
        assert!(s.letting_go() < before && before - s.letting_go() <= 2 * 4096);
        for _ in 0..40 {
            s.tick();
        }
        assert_eq!(s.letting_go(), 0);
        s.run("list.select", &json!({"key": inbox.first().unwrap().to_id()})).unwrap();
        assert_eq!(s.selection.len(), 1, "it's the list shown");
    }
}

/// A result flagged (and one not shown yet, as a server's change would) while the run still
/// reads the folder it's in, the list long enough to take what's found only every few steps:
/// what was found and not shown yet still shows (it was let go of when the flag was weighed
/// again), with its flags as they are.
#[test]
fn a_flag_mid_search_keeps_what_isnt_shown_yet() {
    let (mut s, t) = cold();
    s.run("search.run", &json!({"query": "is:read", "scope": "all"})).unwrap();
    s.search_step(8);
    s.search_step(8);
    let unshown = s.search_unshown();
    assert!(s.list.len() > 4096 && !unshown.is_empty(), "{} shown, {} not yet", s.list.len(), unshown.len());
    // One shown, one found and not shown yet: both flagged.
    let shown = s.list.first().map(|m| m.key.clone()).unwrap();
    let unshown = unshown.last().cloned().unwrap();
    s.run("list.select", &json!({"key": shown.to_id()})).unwrap();
    s.run("mail.flag", &json!({"on": true})).unwrap();
    s.apply_op(&unshown.account, Op::SetFlag { keys: vec![unshown.clone()], flag: Flag::Flagged, on: true });
    while !s.search_step(8).done {}
    assert_eq!((s.list.len(), s.search_progress().unwrap().found), (t.read, t.read));
    for k in [&shown, &unshown] {
        assert!(s.list.iter().any(|m| m.key == *k && m.flags.flagged), "flagged in the list");
    }
}

/// Hand-run, in release (`cargo test -p mailcraft-engine --release --lib search_big -- --ignored
/// --nocapture`; `MC_QUERY` for one query): how long each query takes over the mailbox, all
/// told and step by step, from a session just opened and again: in steps of 8 units, and in
/// frames as the window takes them (as many units as fit in 6 ms). It holds the budgets the
/// searches meet (search.md §11), so a slower one fails it: a word, warm, 20 ms all told (25 ms
/// before each summary was looked through once for it) and 4 ms a step at p99, each frame at
/// most 8 ms; a word in a session just opened ([`cold`]: its index opened by the window's first
/// tick, its summaries' texts made as it goes), alone or after the other queries, 90 ms all told
/// and 4 ms a step at p99; `is:unread`, warm or just opened, 15 ms all told; a one-letter word
/// warm 60 ms all told and 6 ms a step, at p99 and at worst, and just opened 300 ms all told;
/// and every frame of `is:read`, `is:unread`, the domain and the one-letter word, which find
/// 15,000 to 47,387, at most 8 ms, warm or just opened (the lists sharing the folders'
/// summaries, the run's sets holding no strings, and the list put together at a step's start:
/// each copied every summary found, hashed its key's strings and showed them past the frame's
/// end, 10-16 ms). Printed only: the common phrase searched again (150 ms in frames: 128-142
/// ms), and the tick that opens the index ahead, with what the index then holds in memory (and
/// after the queries). A machine busy with other builds slows every query alike and can fail
/// it: run it on one at rest. Not met yet (docs/gaps.md, Open): the common phrase searched the
/// first time (150 ms in frames: 357-454 ms).
#[test]
#[ignore]
fn how_long_searches_take() {
    let started = Instant::now();
    let (mut s, t) = unticked();
    eprintln!("made and indexed in {:?}", started.elapsed());
    let ticked = Instant::now();
    s.tick();
    eprintln!("its index opened ahead by a tick in {:?}, holding {} KiB", ticked.elapsed(), s.search_index_bytes(big::ACCOUNT) / 1024);
    let only = std::env::var("MC_QUERY").ok();
    let mut frame = |s: &mut Session| {
        let began = Instant::now();
        s.search_step_while(4096, &mut || began.elapsed() < Duration::from_millis(6)).done
    };
    for (q, _) in queries(&t).into_iter().filter(|(q, _)| only.as_ref().is_none_or(|o| o == q)) {
        for cache in ["cold", "warm", "cold, frames", "warm, frames"] {
            if cache.starts_with("cold") {
                (s, _) = cold();
            }
            let steps = if cache.ends_with("frames") { search_by(&mut s, &q, "all", &mut frame) } else { search(&mut s, &q, "all") };
            let all: Duration = steps.iter().map(|st| st.took).sum();
            let mut times: Vec<Duration> = steps.iter().map(|st| st.took).collect();
            times.sort();
            let p99 = times.get(times.len() * 99 / 100).copied().unwrap_or_default();
            let most = times.last().copied().unwrap_or_default();
            let at = steps.iter().position(|st| st.took == most).unwrap_or(0);
            let pages: u64 = steps.iter().map(|st| st.read.reader_pages).sum();
            let rebuilt: usize = steps.iter().map(|st| st.rebuilt).sum();
            eprintln!(
                "{q} ({cache}): {} found, {all:?} in {} steps, p99 {p99:?}, worst {most:?} (step {at}); index pages {pages}, list rows built {rebuilt}",
                s.list.len(),
                times.len(),
            );
            let word = [big::RARE, "zurich", big::CJK, big::FAR].contains(&q.as_str());
            let many = ["is:read", "is:unread", big::DOMAIN, "k"].contains(&q.as_str());
            let ms = Duration::from_millis;
            match cache {
                "warm" if word => assert!(all <= ms(20) && p99 <= ms(4), "{q}: {all:?}, p99 {p99:?}"),
                "warm, frames" if word => assert!(most <= ms(8), "{q}: a frame of {most:?}"),
                "cold" if word => assert!(all <= ms(90) && p99 <= ms(4), "{q}: {all:?}, p99 {p99:?}"),
                "warm" | "cold" if q == "is:unread" => assert!(all <= ms(15), "{q}: {all:?}"),
                "warm" if q == "k" => assert!(all <= ms(60) && p99 <= ms(6) && most <= ms(6), "{q}: {all:?}, p99 {p99:?}, worst {most:?}"),
                "cold" if q == "k" => assert!(all <= ms(300), "{q}: {all:?}"),
                "cold, frames" | "warm, frames" if many => assert!(most <= ms(8), "{q}: a frame of {most:?}"),
                _ => {}
            }
        }
    }
    eprintln!("its index held {} KiB after the queries", s.search_index_bytes(big::ACCOUNT) / 1024);
}
