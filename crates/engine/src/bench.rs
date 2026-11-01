//! A big mailbox, on demand: what opening it, its biggest folder, a search and the address book
//! cost. An ignored test, run by hand on a cache a data file was imported into (see
//! `mailcraft-net`'s `pst_bench`):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache-many cargo test -p mailcraft-engine --release --lib bench::big_mailbox -- --ignored --nocapture
//! ```
//!
//! Or `MC_DATA`, a data folder as the app's `--data` keeps it (a copy: Down marks mail read),
//! whose first account is the big one.
//!
//! Each step prints how long it took; memory is watched from outside. Steps are a few seconds
//! apart so the watcher sees each. `MC_DATA_CACHE_MB` opens the account's data file with that
//! cache instead of the usual one; built with `-F mailcraft-store/cache-metrics`, `cache` lines
//! say how it was used before the searches and at the end.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::store::Store;

fn step<T>(what: &str, f: impl FnOnce() -> T) -> T {
    std::thread::sleep(Duration::from_secs(3));
    let t = Instant::now();
    let r = f();
    println!("{what}: {:.2} s", t.elapsed().as_secs_f64());
    r
}

#[test]
#[ignore]
fn big_mailbox() {
    let (store, account) = if let Ok(data) = std::env::var("MC_DATA") {
        let data = PathBuf::from(data);
        let store = Store::open_packed(data.clone(), data.join("MailCraft Files")).unwrap();
        let account = store.load_accounts().unwrap().first().map(|a| a.id.clone()).expect("an account in MC_DATA");
        (store, account)
    } else {
        let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE: a cache a data file was imported into"));
        let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
        store
            .save_accounts(&[crate::model::AccountConfig { id: "d1".into(), name: "Big".into(), email: "big@local".into(), ..Default::default() }])
            .unwrap();
        (store, "d1".to_string())
    };
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
    step("search this folder", || s.run_search("budget review", crate::SearchScope::Current));
    println!("found {}", s.list.len());
    step("search everything", || s.run_search("storyboard deadline", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("search everything again", || s.run_search("festival", crate::SearchScope::All));
    step("is:unread", || s.run_search("is:unread", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("phrase", || s.run_search("\"budget review\"", crate::SearchScope::All));
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
