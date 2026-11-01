//! Engine tests: searching a mailbox the size of a real file's, made up ([`sample::big`]: 50,000
//! messages in 40 folders, the Inbox 12,000), indexed as the indexer would (search.md §10–11).
//! The steps are bounded in what they read and check whatever the mailbox's size, and what's
//! found is what the mailbox was made with. How long they take is measured in a release build,
//! by hand (an ignored test, as `bench.rs`).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::*;
use crate::sample::big::{self, Shape, Truth};
use crate::session::SearchStats;

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

/// A session newly opened on the mailbox, searching in the background as the window does.
fn cold() -> (Session, Truth) {
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

/// Every query over every mailbox, as the window runs it: each `search_step(8)` reads at most 8
/// pages of folders' lists and 16 of the index, checks at most 8 × 512 summaries and 8 texts
/// (the command's first 4 units, half that), and reads no message; flags read nothing of the
/// index; and each finds what the mailbox holds, the list newest first.
#[test]
fn each_step_is_bounded_over_fifty_thousand_messages() {
    let (mut s, t) = cold();
    let reads = s.store.as_ref().unwrap().reads();
    for (q, want) in queries(&t) {
        let steps = search(&mut s, &q, "all");
        for (i, st) in steps.iter().enumerate() {
            let (d, n) = (st.read, if i == 0 { 4 } else { 8 });
            assert!(d.folder_pages <= n, "{q}: folder pages {}", d.folder_pages);
            assert!(d.reader_pages <= 2 * n, "{q}: index pages {}", d.reader_pages);
            assert!(d.summaries <= 512 * n, "{q}: summaries {}", d.summaries);
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

/// Hand-run, in release (`cargo test -p mailcraft-engine --release --lib search_big -- --ignored
/// --nocapture`; `MC_QUERY` for one query): how long each query takes over the mailbox, all
/// told and step by step, from a session just opened and again: in steps of 8 units, and in
/// frames as the window takes them (as many units as fit in 6 ms).
#[test]
#[ignore]
fn how_long_searches_take() {
    let started = Instant::now();
    let (mut s, t) = cold();
    eprintln!("made and indexed in {:?}", started.elapsed());
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
        }
    }
}
