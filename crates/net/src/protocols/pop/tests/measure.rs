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

/// A message of about 1 KB, sent `i` seconds into the day.
fn letter(i: usize) -> Vec<u8> {
    let (h, m, s) = (i / 3600, i / 60 % 60, i % 60);
    let body = "Some words of a letter, as many as an ordinary one has. ".repeat(16);
    format!(
        "From: Ana <ana@pop.example>\r\nTo: me@pop.example\r\nSubject: Message {i}\r\nDate: Mon, 1 Jan 2024 {h:02}:{m:02}:{s:02} +0000\r\nMessage-ID: <m{i}@pop.example>\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

/// A first download of a maildrop into an empty Inbox: the whole list was sorted and saved, and
/// the record of every message, once for each message downloaded. In memory, what the saving
/// itself costs; on disk, with each source written as a file of its own.
#[test]
#[ignore = "a measurement"]
fn measure_first_download() {
    for on_disk in [false, true] {
        for count in [5_000, 10_000] {
            let server = fake::serve((0..count).map(|i| (format!("id{i}"), letter(i))).collect());
            let dir = std::env::temp_dir().join(format!("mailcraft-pop-first-{}-{count}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let store = if on_disk { Store::open(&dir).unwrap() } else { Store::memory() };
            let cfg = ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None };
            let acc = AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() };
            folders(&store, "p1").unwrap();
            let t = Instant::now();
            let mut c = open(&acc, "pw").unwrap();
            let r = fetch_new(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000).unwrap();
            c.quit().unwrap();
            let place = if on_disk { "on disk" } else { "in memory" };
            println!("MEASURE POP first download of {count} messages, {place}: {:.0} ms", ms(t));
            assert_eq!((r.new, r.total as usize), (count, count));
            let inbox = store.load_folder("p1", "INBOX").unwrap();
            assert_eq!(inbox.messages.first().map(|m| m.subject.as_str()), Some(format!("Message {}", count - 1).as_str()), "newest first");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A check of a maildrop of `N` messages all downloaded before, then `GONE` of them moved to
/// Deleted Items and deleted there for good: each looked up the server's record of every message
/// in a list of the others.
#[test]
#[ignore = "a measurement"]
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
        state.insert(id.clone(), Kept { at: 0, key: Some(key.to_id()), ..Default::default() });
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
