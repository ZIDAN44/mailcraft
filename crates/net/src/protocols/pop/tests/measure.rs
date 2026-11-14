//! POP over a large maildrop, measured (run by hand, in release):
//! `cargo test --release -p mailcraft-net --lib pop::tests::measure -- --ignored --nocapture`

use std::time::Instant;

use mailcraft_model::Summary;

use super::*;

/// Messages left on the server.
const N: usize = 50_000;
/// Messages moved to Deleted Items, then deleted there for good.
const GONE: usize = 5_000;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// A check of a maildrop of `N` messages all downloaded before, then `GONE` of them moved to
/// Deleted Items and deleted there for good: each looked up the server's record of every message
/// in a list of the others.
#[test]
#[ignore]
fn measure_bookkeeping() {
    let ids: Vec<String> = (0..N).map(|i| format!("{i:0>12}{:0>12x}", i * 7919)).collect();
    let server = fake::serve(ids.iter().map(|id| (id.clone(), Vec::new())).collect());
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-measure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let cfg = ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None };
    let acc = AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() };
    folders(&store, "p1").unwrap();
    let mut inbox = FolderState { validity: 1, uid_next: N as u32 + 1, ..Default::default() };
    let mut state = State::new();
    for (i, id) in ids.iter().enumerate() {
        let key = MessageKey { account: "p1".into(), folder: "INBOX".into(), validity: 1, uid: i as u32 + 1 };
        state.insert(id.clone(), Kept { at: 0, key: Some(key.to_id()), gone: false });
        inbox.messages.push(Summary { key, subject: format!("Message {i}"), date: i as i64, ..Default::default() }.into());
    }
    sort_newest_first(&mut inbox.messages);
    store.save_folder("p1", "INBOX", &inbox).unwrap();
    store.save_account_doc("p1", STATE_DOC, &state).unwrap();

    let t = Instant::now();
    let mut c = open(&acc, "pw").unwrap();
    let r = fetch_new(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000).unwrap();
    c.quit().unwrap();
    println!("MEASURE POP check, {N} messages on the server, none new: {:.0} ms", ms(t));
    assert_eq!((r.new, r.total as usize), (0, N));

    let going: Vec<MessageKey> = inbox.messages.iter().rev().take(GONE).map(|m| m.key.clone()).collect();
    let t = Instant::now();
    move_locally(&store, "p1", &going, "Deleted Items").unwrap();
    println!("MEASURE POP move of {GONE} of {N} to Deleted Items: {:.0} ms", ms(t));
    let trash: Vec<MessageKey> = store.load_folder("p1", "Deleted Items").unwrap().messages.iter().map(|m| m.key.clone()).collect();
    assert_eq!((trash.len(), store.load_folder("p1", "INBOX").unwrap().messages.len()), (GONE, N - GONE));

    let t = Instant::now();
    deleted_for_good(&store, "p1", &trash).unwrap();
    println!("MEASURE POP {GONE} deleted for good of {N}: {:.0} ms", ms(t));
    // The records followed: the first GONE ids are in Deleted Items, deleted for good.
    let state: State = store.load_account_doc("p1", STATE_DOC).unwrap();
    let trash: std::collections::HashSet<String> = trash.iter().map(MessageKey::to_id).collect();
    for (i, id) in ids.iter().enumerate() {
        let k = &state[id];
        let there = k.key.as_ref().is_some_and(|key| trash.contains(key));
        assert_eq!((there, k.gone), (i < GONE, i < GONE), "{i}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
