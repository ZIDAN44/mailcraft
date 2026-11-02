//! A big mailbox, on demand: what opening it, its biggest folder, a search and the address book
//! cost. An ignored test, run by hand on a cache a data file was imported into (see
//! `mailcraft-net`'s `pst_bench`):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache-many cargo test -p mailcraft-engine --release --lib bench::big_mailbox -- --ignored --nocapture
//! ```
//!
//! Or `MC_DATA`, a data folder as the app's `--data` keeps it (a copy: Down marks mail read),
//! whose first account is the big one. With neither, the made-up mailbox of the search tests
//! ([`crate::sample::big`], 50,000 messages), in memory.
//!
//! Each step prints how long it took, and `memory` lines what the summaries kept in memory take
//! (and the process, on Windows) as the folder, a 500-Down walk and a search hold them. Steps
//! are a few seconds apart so a watcher outside sees each. `MC_DATA_CACHE_MB` opens the
//! account's data file with that cache instead of the usual one; built with
//! `-F mailcraft-store/cache-metrics`, `cache` lines
//! say how it was used before the searches and at the end.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::sample::big;
use crate::store::Store;

mod memory;

fn step<T>(what: &str, f: impl FnOnce() -> T) -> T {
    std::thread::sleep(Duration::from_secs(3));
    let t = Instant::now();
    let r = f();
    println!("{what}: {:.2} s", t.elapsed().as_secs_f64());
    r
}

/// The domain most of `folder`'s mail is from.
fn common_domain(s: &mut Session, account: &str, folder: &str) -> String {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in &s.folder_state(account, folder).messages {
        if let Some((_, d)) = m.from.email.rsplit_once('@') {
            *counts.entry(d).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(d, n)| (*n, *d)).map(|(d, _)| d.to_string()).unwrap_or_else(|| "example".into())
}

/// Down 500 times from the top, each message opened as it's landed on, as one holds the key
/// down: the worst and the middle step. Then the largest message in the list opened.
fn walk(s: &mut Session) {
    let Some(first) = s.list.first().map(|m| m.key.clone()) else { return };
    let _ = s.run("list.open", &json!({"key": first.to_id()}));
    std::thread::sleep(Duration::from_secs(3));
    let mut steps: Vec<f64> = (0..500)
        .map(|_| {
            let t = Instant::now();
            drop(s.run("list.next", &json!({})));
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    steps.sort_by(f64::total_cmp);
    let at = |q: usize| steps.get(steps.len().saturating_sub(1) * q / 100).copied().unwrap_or_default();
    println!("Down x500: median {:.2} ms, p90 {:.2}, max {:.2}, {:.2} s in all", at(50), at(90), at(100), steps.iter().sum::<f64>() / 1000.0);
    let at = s.reading.as_ref().and_then(|r| s.list.iter().position(|m| m.key == r.key));
    println!("reading the message at {at:?}");
    if let Some(m) = s.list.iter().max_by_key(|m| m.size) {
        let (key, size) = (m.key.clone(), m.size);
        step(&format!("open the largest message ({} KB)", size / 1024), || drop(s.run("list.open", &json!({"key": key.to_id()}))));
    }
}

#[test]
#[ignore]
fn big_mailbox() {
    let (store, account) = if let Ok(data) = std::env::var("MC_DATA") {
        let data = PathBuf::from(data);
        let store = Store::open_packed(data.clone(), data.join("MailCraft Files")).unwrap();
        let account = store.load_accounts().unwrap().first().map(|a| a.id.clone()).expect("an account in MC_DATA");
        (store, account)
    } else if let Ok(cache) = std::env::var("MC_CACHE") {
        let cache = PathBuf::from(cache);
        let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
        store
            .save_accounts(&[crate::model::AccountConfig { id: "d1".into(), name: "Big".into(), email: "big@local".into(), ..Default::default() }])
            .unwrap();
        (store, "d1".to_string())
    } else {
        let store = Store::memory();
        big::generate(&store, 1_800_000_000, big::Shape::default()).unwrap();
        (store, big::ACCOUNT.to_string())
    };
    // The words searched for: on the made-up mailbox, words it was made with (an everyday word
    // there finds nothing, measuring nothing).
    let made_up = account == big::ACCOUNT;
    let pick = |real: &str, made: &str| if made_up { made.to_string() } else { real.to_string() };
    let (here, everywhere, again) = (pick("budget review", big::RARE), pick("storyboard deadline", big::FAR), pick("festival", "zurich"));
    let phrase = pick("\"budget review\"", &format!("\"{}\"", big::PHRASE));
    if let Some(mb) = std::env::var("MC_DATA_CACHE_MB").ok().and_then(|v| v.parse::<usize>().ok()) {
        // A bulk write's cache, held for the whole run: the way to give the file another one.
        store.bulk_begin(&account, mb << 20);
    }
    let mut s = step("open", || Session::open(Some(store), Clock::new(1_800_000_000, 0)));
    let folders: Vec<(String, u32)> = s.account(&account).map(|a| a.folders.iter().map(|f| (f.path.clone(), f.total)).collect()).unwrap_or_default();
    println!("folders: {}", folders.len());
    let biggest = folders.iter().max_by_key(|(_, n)| *n).map(|(p, _)| p.clone()).unwrap_or_else(|| "INBOX".into());
    step("open the biggest folder", || s.run("folder.open", &json!({"account": account, "folder": biggest})).unwrap());
    println!("list: {} messages", s.list.len());
    step("refresh the list", || s.refresh_list());
    memory::report(&s, "with the biggest folder open");
    walk(&mut s);
    memory::report(&s, "after the walk and the largest message");
    // Down through the list, each message opened (and marked read) as it's landed on: the
    // time per step is a twentieth of this.
    // Then again with conversations on and the list arranged by sender, where each Down worked
    // the conversations out again.
    for (conversations, by) in [(false, "date"), (true, "from")] {
        let _ = s.run("view.conversations", &json!({"on": conversations}));
        let _ = s.run("view.arrange", &json!({"by": by}));
        if let Some(k) = s.list.first().map(|m| m.key.clone()) {
            let _ = s.run("list.open", &json!({"key": k.to_id()}));
            step(&format!("Down x20 (conversations {conversations}, by {by})"), || (0..20).for_each(|_| drop(s.run("list.next", &json!({})))));
            // The same 20 messages read from the cache and parsed, nothing else: what's left of
            // a Down is the list's (and the read mark's) share.
            let passed: Vec<_> = s.list.iter().take(21).skip(1).map(|m| m.key.clone()).collect();
            step(&format!("read and parse those 20 (by {by})"), || {
                passed.iter().filter_map(|k| s.store.as_ref().and_then(|st| st.read_body(k))).for_each(|raw| drop(crate::mime::parse(&raw)))
            });
        }
    }
    let first = s.list.first().map(|m| m.key.clone());
    if let Some(k) = first {
        let _ = s.run("list.select", &json!({"key": k.to_id()}));
        step("mark one read", || drop(s.run("mail.markRead", &json!({}))));
    }
    if let Some(store) = &s.store {
        println!("cache before searching: {}", serde_json::to_string(&store.cache_use(&account)).unwrap());
    }
    // The account's search index, as the indexer brings it up (one thread here).
    step("indexing pass", || {
        let Some(store) = s.store.clone() else { return };
        let Ok(mut p) = crate::store::search::Pass::start(&store, &account, &[]) else { return };
        let adds = p.adds().clone();
        for (i, m) in adds.iter().enumerate() {
            let doc = store.read_body(&m.key).map(|raw| crate::mime::index_doc(&raw));
            let _ = p.add(&store, i, doc.as_ref());
        }
        let _ = p.finish(&store, true);
    });
    step("search this folder", || s.run_search(&here, crate::SearchScope::Current));
    println!("found {}", s.list.len());
    step("search everything", || s.run_search(&everywhere, crate::SearchScope::All));
    println!("found {}", s.list.len());
    memory::report(&s, "with a search of everything shown");
    step("search everything again", || s.run_search(&again, crate::SearchScope::All));
    step("is:unread", || s.run_search("is:unread", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("is:read", || s.run_search("is:read", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("phrase", || s.run_search(&phrase, crate::SearchScope::All));
    step("a common phrase", || s.run_search("\"thank you\"", crate::SearchScope::All));
    println!("found {}", s.list.len());
    // The domain most of the biggest folder's mail is from (not printed: the file's own).
    let domain = common_domain(&mut s, &account, &biggest);
    step("a common domain", || s.run_search(&domain, crate::SearchScope::All));
    println!("found {}", s.list.len());
    memory::report(&s, "with a large search shown");
    if let Some(k) = s.list.first().map(|m| m.key.clone()) {
        let _ = s.run("list.select", &json!({"key": k.to_id()}));
        step("search after mark-read", || drop(s.run("mail.markRead", &json!({}))));
    }
    step("address book", || s.address_book("all", "zo").len());
    step("rest", || std::thread::sleep(Duration::from_secs(2)));
    if let Some(store) = &s.store {
        println!("cache: {}", serde_json::to_string(&store.cache_use(&account)).unwrap());
    }
}
