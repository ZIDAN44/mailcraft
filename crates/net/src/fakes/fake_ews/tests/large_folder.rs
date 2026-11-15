//! A folder of many messages: a sync that finds nothing changed keeps them all and asks the
//! server for none (and takes time in step with the folder, not its square).

use std::time::Instant;

use super::*;

/// A fake server whose Inbox holds `n` messages, each received a second after the one before.
fn inbox_of(n: u32) -> Server {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    for i in 0..n {
        let at = format!("2024-01-{:02}T{:02}:{:02}:{:02}Z", 1 + i / 86_400, i / 3600 % 24, i / 60 % 60, i % 60);
        d.deliver(&inbox, &mail(&format!("Note {i}"), &format!("n{i}")), true, &at);
    }
    serve(d)
}

/// Sync the Inbox, then `quiet` times more with nothing changed (the window takes in every
/// message): how long the first took and the quickest of the others, and the last one's report
/// and the server operations it asked for.
fn sync_then_again(server: &Server, n: u32, name: &str, quiet: usize) -> (Duration, Duration, crate::sync::SyncReport, Vec<String>, Store) {
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let store = crate::fake::store(name);
    let mut st = crate::jmap::State::default();
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let opts = crate::sync::SyncOptions { window: n as usize, prefetch: 0, ..Default::default() };
    let at = Instant::now();
    let first = crate::ews::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let first_took = at.elapsed();
    assert_eq!(first.new, n as usize, "the first sync brings in every message");
    let mut again_took = Duration::MAX;
    let mut again = crate::sync::SyncReport::default();
    for _ in 0..quiet.max(1) {
        server.data.lock().unwrap().calls.clear();
        let at = Instant::now();
        again = crate::ews::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
        again_took = again_took.min(at.elapsed());
    }
    let calls = server.data.lock().unwrap().calls.clone();
    (first_took, again_took, again, calls, store)
}

/// A sync of a large folder in which nothing changed keeps every message and fetches none.
#[test]
fn a_quiet_sync_of_a_large_folder_keeps_every_message_and_asks_for_none() {
    let n = 3000;
    let server = inbox_of(n);
    let (_, _, again, calls, store) = sync_then_again(&server, n, "ews-large-quiet", 1);
    assert_eq!((again.new, again.removed, again.changed), (0, 0, 0));
    assert!(!calls.iter().any(|c| c == "GetItem"), "no details asked for: {calls:?}");
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.len(), n as usize);
    let uids: std::collections::HashSet<u32> = fs.messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(uids.len(), n as usize, "each message once");
    assert_eq!(fs.messages.first().map(|m| m.subject.as_str()), Some("Note 2999"), "newest first");
}

/// How long syncing a 50,000-message folder takes, first and again with nothing changed (the
/// quickest of five). Run:
/// `cargo test --release -p mailcraft-net --lib measure_ews_large_folder_sync -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_ews_large_folder_sync() {
    let n = 50_000;
    let server = inbox_of(n);
    let (first, again, report, calls, store) = sync_then_again(&server, n, "ews-large-measure", 5);
    assert_eq!((report.new, report.removed), (0, 0));
    assert!(!calls.iter().any(|c| c == "GetItem"));
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize);
    println!("MEASURE ews sync of {n} messages: first {first:?}, again with nothing changed {again:?} (quickest of 5)");
}
