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
//! segments' bytes, and each query by its number (never its words): searched in a session just
//! opened (its index opened by the window's first tick), then again; then each the first time in
//! a session the queries before it were searched in, then again; in frames of 6 ms of work as
//! the window steps it, from the frame that sets the query to the last busy one: the work all
//! told, the frames, the index pages and text pages read, and the texts checked. `MC_NO_PLACES`
//! checks phrases against the texts kept instead, as a build before places did, to compare.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::session::SearchStats;
use crate::store::Store;

#[test]
#[ignore]
fn queries() {
    let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE: a cache a data file was imported into"));
    let list = std::fs::read_to_string(std::env::var("MC_QUERIES").expect("MC_QUERIES: a file of queries, one a line")).unwrap();
    let queries: Vec<&str> = list.lines().map(str::trim).filter(|q| !q.is_empty()).collect();
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
        // `MC_NO_PLACES`: phrases checked against the texts kept, as before places, to compare.
        if std::env::var_os("MC_NO_PLACES").is_some() {
            s.search_without_places();
        }
        s.tick();
        s
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
            let mut all = Duration::ZERO;
            let began = Instant::now();
            s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
            all += began.elapsed();
            let mut frames = 1;
            loop {
                let began = Instant::now();
                let p = s.search_step_while(4096, &mut || began.elapsed() < Duration::from_millis(6));
                all += began.elapsed();
                frames += 1;
                if p.done {
                    break;
                }
            }
            let d = delta(before, s.search_stats());
            println!(
                "query {} ({round}): {} found, {:.1} ms in {frames} frames; index pages {}, text pages {}, texts checked {}",
                i + 1,
                s.list.len(),
                all.as_secs_f64() * 1000.0,
                d.reader_pages,
                d.text_pages,
                d.texts,
            );
        }
    }
}

fn delta(before: SearchStats, after: SearchStats) -> SearchStats {
    SearchStats {
        reader_pages: after.reader_pages.saturating_sub(before.reader_pages),
        text_pages: after.text_pages.saturating_sub(before.text_pages),
        texts: after.texts.saturating_sub(before.texts),
        ..SearchStats::default()
    }
}
