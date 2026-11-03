//! Searches of a big mailbox timed as the window runs them, the queries read from a file outside
//! the repo (`MC_QUERIES`, one a line), so a real file's words never enter it. An ignored test,
//! run by hand on a cache a data file was imported into (`mailcraft-net`'s `pst_bench`):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache MC_QUERIES=C:\mc-bench\queries.txt cargo test -p mailcraft-engine --release --lib bench::queries -- --ignored --nocapture
//! ```
//!
//! The account's index writer is opened first, as the app's indexer opens it: an index from a
//! build before has its words made again from the texts it keeps then (timed). Then the index's
//! segments' bytes, and each query by its number (never its words), as the window runs it: from
//! the frame that sets the query (the command takes none of the search, as the window's) to the
//! last busy one, each frame the session's tick and the search's units within 6 ms
//! (`search_step_within`). Each is searched in a session just opened (ticked once: its index
//! opened, a tick's worth of its summaries' texts made ahead), then again; in a session opened
//! a moment before (ticked until its summaries' texts are made ahead: the ticks, and the
//! longest); then the first time in a session the queries before it were searched in, then
//! again: the work all told, the frames and the longest, the index pages and text pages read,
//! the texts checked, and what the search spent its time on by kind (`SearchWork`). Then each
//! typed a character at a time in a session just opened, each prefix searched as the window
//! searches it once the typing pauses, the next typed once it's done or 30 frames on: how many
//! frames, and the longest, which fails it past 8 ms. `MC_TRUTH`, a file of what each query
//! should find (a line each, its count first, as an independent count of the mailbox says), fails
//! it when a search finds another number, or a line has no count or there's not one a query.
//! `MC_NO_PLACES` checks phrases against the texts kept instead, as a build before places did,
//! to compare.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::session::{SearchStats, SearchWork};
use crate::store::Store;

/// Most frames a prefix typed gets before the next character comes.
const TYPED_FRAMES: usize = 30;

/// Search `q` as the window does: the frame that sets the query, then a frame at a time (the
/// session's tick and 6 ms of the search) until it's done, or `most` frames: what each took.
fn frames(s: &mut Session, q: &str, most: usize) -> Vec<Duration> {
    frames_by_kind(s, q, most).0
}

/// [`frames`], with the time the search spent on each kind of its work, the most first (as the
/// window logs it when a search ends).
fn frames_by_kind(s: &mut Session, q: &str, most: usize) -> (Vec<Duration>, Vec<(SearchWork, Duration)>) {
    let began = Instant::now();
    s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
    let mut took = vec![began.elapsed()];
    let mut kinds: Vec<(SearchWork, Duration)> = Vec::new();
    while took.len() < most {
        let began = Instant::now();
        s.tick();
        let mut last = Instant::now();
        let p = s.search_step_within(4096, Duration::from_millis(6), &mut || began.elapsed(), &mut |w| {
            let now = Instant::now();
            match kinds.iter_mut().find(|(k, _)| *k == w) {
                Some((_, t)) => *t += now - last,
                None => kinds.push((w, now - last)),
            }
            last = now;
        });
        took.push(began.elapsed());
        if p.done {
            break;
        }
    }
    kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
    (took, kinds)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[test]
#[ignore]
fn queries() {
    let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE: a cache a data file was imported into"));
    let list = std::fs::read_to_string(std::env::var("MC_QUERIES").expect("MC_QUERIES: a file of queries, one a line")).unwrap();
    let queries: Vec<&str> = list.lines().map(str::trim).filter(|q| !q.is_empty()).collect();
    // What each should find, counted apart from the index: a line a query, each read or failing
    // (one skipped would hold every count after it to the wrong query).
    let truth: Option<Vec<usize>> = std::env::var("MC_TRUTH").ok().map(|f| {
        let t = std::fs::read_to_string(f).unwrap();
        let counts: Vec<usize> = t
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| l.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("MC_TRUTH line {}: no count", i + 1)))
            .collect();
        assert_eq!(counts.len(), queries.len(), "MC_TRUTH: a count for each query");
        counts
    });
    let mut wrong = Vec::new();
    let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    store
        .save_accounts(&[crate::model::AccountConfig { id: "d1".into(), name: "Big".into(), email: "big@local".into(), ..Default::default() }])
        .unwrap();
    let (before, places_before) = crate::store::search::segment_bytes(&store, "d1").unwrap();
    println!("segments before: {before} bytes, of them places pages {places_before}");
    let opened = Instant::now();
    drop(crate::store::search::Writer::open(&store, "d1").unwrap());
    println!("writer opened (words made again, if from a build before): {:.2} s", opened.elapsed().as_secs_f64());
    let (after, places) = crate::store::search::segment_bytes(&store, "d1").unwrap();
    println!("segments now: {after} bytes, of them places pages {places}");
    let fresh = || {
        let mut s = Session::open(Some(store.clone()), Clock::new(1_800_000_000, 0));
        s.take_jobs();
        s.set_background_search(true);
        s.set_search_first_units(0);
        // `MC_NO_PLACES`: phrases checked against the texts kept, as before places, to compare.
        if std::env::var_os("MC_NO_PLACES").is_some() {
            s.search_without_places();
        }
        s.tick();
        s
    };
    // A session ticked as the window ticks it once it's open, until its summaries' texts are
    // made ahead: how many ticks, and the longest.
    let settled = || {
        let mut s = fresh();
        let (mut ticks, mut longest) = (1, Duration::ZERO);
        while s.texts_ahead_due() && ticks < 100_000 {
            let began = Instant::now();
            s.tick();
            longest = longest.max(began.elapsed());
            ticks += 1;
        }
        (s, ticks, longest)
    };
    // Each query in a session just opened, then each in one session after the queries before
    // it (its summaries' texts made by the first: a query searched the first time in a session
    // in use).
    let mut used = fresh();
    for (i, q) in queries.iter().chain(&queries).enumerate() {
        let (i, alone) = (i % queries.len(), i < queries.len());
        let mut session = if alone { Some(fresh()) } else { None };
        let s = session.as_mut().unwrap_or(&mut used);
        let rounds: &[&str] = if alone { &["just opened", "again"] } else { &["first time in use", "again"] };
        for &round in rounds {
            let before = s.search_stats();
            let (took, kinds) = frames_by_kind(s, q, usize::MAX);
            let d = delta(before, s.search_stats());
            let worst = took.iter().max().copied().unwrap_or_default();
            println!(
                "query {} ({round}): {} found, {:.1} ms in {} frames, the longest {:.1} ms; index pages {}, text pages {}, texts checked {}",
                i + 1,
                s.list.len(),
                ms(took.iter().sum()),
                took.len(),
                ms(worst),
                d.reader_pages,
                d.text_pages,
                d.texts,
            );
            let kinds: Vec<String> = kinds.iter().map(|(w, t)| format!("{} {:.1} ms", w.name(), ms(*t))).collect();
            println!("  by kind: {}", kinds.join(", "));
            if let Some(want) = truth.as_ref().and_then(|t| t.get(i)).filter(|n| **n != s.list.len()) {
                wrong.push(format!("query {} ({round}): {} found, {want} there", i + 1, s.list.len()));
            }
        }
    }
    // Each the first time in a session opened a moment before.
    for (i, q) in queries.iter().enumerate() {
        let (mut s, ticks, longest) = settled();
        let before = s.search_stats();
        let (took, kinds) = frames_by_kind(&mut s, q, usize::MAX);
        let d = delta(before, s.search_stats());
        println!(
            "query {} (a moment after opening, {ticks} ticks, the longest {:.1} ms): {} found, {:.1} ms in {} frames, the longest {:.1} ms; index pages {}, text pages {}, texts checked {}",
            i + 1,
            ms(longest),
            s.list.len(),
            ms(took.iter().sum()),
            took.len(),
            ms(took.iter().max().copied().unwrap_or_default()),
            d.reader_pages,
            d.text_pages,
            d.texts,
        );
        let kinds: Vec<String> = kinds.iter().map(|(w, t)| format!("{} {:.1} ms", w.name(), ms(*t))).collect();
        println!("  by kind: {}", kinds.join(", "));
        if let Some(want) = truth.as_ref().and_then(|t| t.get(i)).filter(|n| **n != s.list.len()) {
            wrong.push(format!("query {} (a moment after opening): {} found, {want} there", i + 1, s.list.len()));
        }
    }
    // Each typed a character at a time, in a session just opened.
    let mut every = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        let mut s = fresh();
        let mut took = Vec::new();
        for (end, _) in q.char_indices().skip(1).chain([(q.len(), ' ')]) {
            took.extend(frames(&mut s, q.get(..end).unwrap_or(q), TYPED_FRAMES));
        }
        took.sort();
        let p99 = took.get(took.len() * 99 / 100).copied().unwrap_or_default();
        println!(
            "query {} typed: {} frames, p99 {:.1} ms, the longest {:.1} ms",
            i + 1,
            took.len(),
            ms(p99),
            ms(took.last().copied().unwrap_or_default())
        );
        every.extend(took);
    }
    every.sort();
    let longest = every.last().copied().unwrap_or_default();
    println!("typed, all: {} frames, the longest {:.1} ms", every.len(), ms(longest));
    assert!(wrong.is_empty(), "found other than there is: {wrong:?}");
    // A frame of the window's is 8 ms at most, typing as searching (run on a machine at rest).
    assert!(longest <= Duration::from_millis(8), "a typed frame of {:.1} ms", ms(longest));
}

fn delta(before: SearchStats, after: SearchStats) -> SearchStats {
    SearchStats {
        reader_pages: after.reader_pages.saturating_sub(before.reader_pages),
        text_pages: after.text_pages.saturating_sub(before.text_pages),
        texts: after.texts.saturating_sub(before.texts),
        ..SearchStats::default()
    }
}
