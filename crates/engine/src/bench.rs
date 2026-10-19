//! A big mailbox, on demand: what opening it, its biggest folder, a search and the address book
//! cost. An ignored test, run by hand on a cache a data file was imported into (see
//! `mailcraft-net`'s `pst_bench`):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache-many cargo test -p mailcraft-engine --release --lib bench::big_mailbox -- --ignored --nocapture
//! ```
//!
//! Each step prints how long it took; memory is watched from outside. Steps are a few seconds
//! apart so the watcher sees each.

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
    let cache = PathBuf::from(std::env::var("MC_CACHE").expect("MC_CACHE: a cache a data file was imported into"));
    let store = Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    store
        .save_accounts(&[crate::model::AccountConfig { id: "d1".into(), name: "Big".into(), email: "big@local".into(), ..Default::default() }])
        .unwrap();
    let mut s = step("open", || Session::open(Some(store), Clock::new(1_800_000_000, 0)));
    let folders: Vec<(String, u32)> = s.account("d1").map(|a| a.folders.iter().map(|f| (f.path.clone(), f.total)).collect()).unwrap_or_default();
    println!("folders: {folders:?}");
    let biggest = folders.iter().max_by_key(|(_, n)| *n).map(|(p, _)| p.clone()).unwrap_or_else(|| "INBOX".into());
    step(&format!("open {biggest}"), || s.run("folder.open", &json!({"account": "d1", "folder": biggest})).unwrap());
    println!("list: {} messages", s.list.len());
    step("refresh the list", || s.refresh_list());
    let first = s.list.first().map(|m| m.key.clone());
    if let Some(k) = first {
        let _ = s.run("list.select", &json!({"key": k.to_id()}));
        step("mark one read", || drop(s.run("mail.markRead", &json!({}))));
    }
    step("search this folder", || s.run_search("budget review", crate::SearchScope::Current));
    println!("found {}", s.list.len());
    step("search everything", || s.run_search("storyboard deadline", crate::SearchScope::All));
    println!("found {}", s.list.len());
    step("search everything again", || s.run_search("festival", crate::SearchScope::All));
    step("address book", || s.address_book("all", "zo").len());
    step("rest", || std::thread::sleep(Duration::from_secs(2)));
}
