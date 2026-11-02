//! Mail with files, counted three ways on a big mailbox: the paperclip (what each summary says),
//! the Has files filter folder by folder, and `has:attachment` over everything. An ignored test,
//! run by hand on a cache a data file was imported into, or a copy of a data folder (`MC_DATA`;
//! a pass of the index may put summaries right):
//!
//! ```text
//! MC_CACHE=C:\mc-bench\cache cargo test -p mailcraft-engine --release --lib bench::files -- --ignored --nocapture
//! ```
//!
//! It prints the three counts (numbers only) and fails when they differ.

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::json;

use crate::Session;
use crate::model::dates::Clock;
use crate::store::Store;

#[test]
#[ignore]
fn files_agree() {
    let (store, account) = if let Ok(data) = std::env::var("MC_DATA") {
        let data = PathBuf::from(data);
        let store = Store::open_packed(data.clone(), data.join("MailCraft Files")).unwrap();
        store.open_data_files().unwrap();
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
        println!("MC_CACHE or MC_DATA: the mailbox to count");
        return;
    };
    // What's left to index, as the indexer does it: summaries found wrong are put right.
    let mut p = crate::store::search::Pass::start(&store, &account, &[]).unwrap();
    let adds = p.adds().clone();
    for (i, m) in adds.iter().enumerate() {
        let doc = store.read_body(&m.key).map(|raw| crate::mime::index_doc(&raw));
        p.add(&store, i, doc.as_ref()).unwrap();
    }
    let mut corrected = 0;
    for (folder, found) in p.take_files() {
        corrected += store.set_has_files(&account, &folder, &found).unwrap();
    }
    p.finish(&store, true).unwrap();
    println!("indexed {} left over, {corrected} summaries put right", adds.len());
    drop(p);
    let mut s = Session::open(Some(store), Clock::new(1_800_000_000, 0));
    let folders: Vec<String> = s.account(&account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default();
    let (mut messages, mut clipped, mut filtered) = (0usize, HashSet::new(), HashSet::new());
    for folder in &folders {
        s.run("folder.open", &json!({"account": account, "folder": folder})).unwrap();
        s.run("list.filter", &json!({"value": "all"})).unwrap();
        messages += s.list.len();
        clipped.extend(s.list.iter().filter(|m| m.has_attachments).map(|m| m.key.clone()));
        s.run("list.filter", &json!({"value": "attachments"})).unwrap();
        filtered.extend(s.list.iter().map(|m| m.key.clone()));
        s.run("list.filter", &json!({"value": "all"})).unwrap();
    }
    s.run_search("has:attachment", crate::SearchScope::All);
    let searched: HashSet<_> = s.list.iter().map(|m| m.key.clone()).collect();
    s.run_search("-has:attachment", crate::SearchScope::All);
    let without = s.list.len();
    println!(
        "files: {messages} messages in {} folders; paperclip {}, Has files {}, has:attachment {} (-has:attachment {without}); \
         paperclip not found by has:attachment {}, found but no paperclip {}",
        folders.len(),
        clipped.len(),
        filtered.len(),
        searched.len(),
        clipped.difference(&searched).count(),
        searched.difference(&clipped).count()
    );
    assert_eq!(clipped, filtered, "the paperclip and Has files");
    assert_eq!(clipped, searched, "the paperclip and has:attachment");
}
