//! A long local move, on demand: what moving a data file account's biggest folder into its Inbox
//! costs, and what sealing the file as it goes adds. An ignored test, run by hand on a copy of a
//! data folder as the app's `--data` keeps it (the move changes it):
//!
//! ```text
//! MC_DATA=C:\mc-bench\data-copy cargo test -p mailcraft-engine --release --lib bench::moving -- --ignored --nocapture
//! ```
//!
//! It prints the move's time, the commits it made that waited for the disk, sealed and not
//! (how many, how long in all, the longest), and what a seal costs by itself after.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::store::Store;

#[test]
#[ignore]
fn big_move() {
    let Ok(data) = std::env::var("MC_DATA") else {
        println!("MC_DATA: a copy of a data folder, to move its biggest folder in");
        return;
    };
    let data = PathBuf::from(data);
    let store = Store::open_packed(data.clone(), data.join("MailCraft Files")).unwrap();
    store.open_data_files().unwrap();
    let account = store.load_accounts().unwrap().first().map(|a| a.id.clone()).expect("an account in MC_DATA");
    let mut s = Session::open(Some(store), Clock::new(1_800_000_000, 0));
    let folders: Vec<(String, u32)> = s.account(&account).map(|a| a.folders.iter().map(|f| (f.path.clone(), f.total)).collect()).unwrap_or_default();
    let Some(biggest) = folders.iter().filter(|(p, _)| p != "INBOX").max_by_key(|(_, n)| *n).map(|(p, _)| p.clone()) else {
        println!("no folder to move");
        return;
    };
    s.run("folder.open", &json!({"account": account, "folder": biggest})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let n = s.list.len();
    let before = s.store.as_ref().map(|st| st.commit_times()).unwrap_or_default();
    let t = Instant::now();
    s.run("mail.move", &json!({"folder": "INBOX"})).unwrap();
    let took = t.elapsed();
    let after = s.store.as_ref().map(|st| st.commit_times()).unwrap_or_default();
    println!("moved {n} messages in {:.2} s", took.as_secs_f64());
    for (what, t) in [("sealed", after.sealed.since(&before.sealed)), ("other durable", after.plain.since(&before.plain))] {
        println!(
            "{what} commits: {} ({:.1} ms in all, the longest {:.1} ms)",
            t.count,
            t.total.as_secs_f64() * 1000.0,
            t.longest.as_secs_f64() * 1000.0
        );
    }
    // What a seal costs by itself on this file: a one-source write that waits for the disk, then
    // an empty sealed commit (the seal a long write's first commit, or a quiet file, adds).
    let Some(store) = s.store.clone() else { return };
    let (mut plain, mut sealed) = (Vec::new(), Vec::new());
    for i in 0..9 {
        let key = crate::model::MessageKey { account: account.clone(), folder: "INBOX".into(), validity: 1, uid: u32::MAX - i };
        let t = Instant::now();
        store
            .write_body(
                &key,
                b"Subject: x

x",
            )
            .unwrap();
        plain.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        store.seal(&account).unwrap();
        sealed.push(t.elapsed().as_secs_f64() * 1000.0);
        store.remove_body(&key);
    }
    for (what, mut v) in [("a one-source write", plain), ("a seal by itself", sealed)] {
        v.sort_by(f64::total_cmp);
        println!("{what}: median {:.1} ms, max {:.1} ms", v.get(v.len() / 2).copied().unwrap_or(0.0), v.last().copied().unwrap_or(0.0));
    }
}
