//! Changes made offline, replayed on the server once the account is back: each sent once, in
//! order, and none lost to another queued meanwhile or to a stop part-way.

use super::*;

fn account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    }
}

/// The Inbox's first `n` messages, as the cache keys them.
fn keys(n: u32) -> Vec<MessageKey> {
    (1..=n).map(|uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid }).collect()
}

/// A server whose Inbox has `n` messages, and an account on it working offline.
fn offline(n: u32, name: &str) -> (Arc<Mutex<State>>, Store, crate::worker::Pool) {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for i in 0..n {
            s.add("INBOX", &format!("m{i}"), &[]);
        }
    }
    let port = serve_tcp(&st);
    let store = store(name);
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::WorkOffline { on: true });
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    (st, store, pool)
}

/// Wait until nothing waits for the server.
fn all_sent(store: &Store, what: &str) {
    let ops = store.root().join("a1").join("ops.json");
    let end = Instant::now() + Duration::from_secs(600);
    while ops.exists() {
        assert!(Instant::now() < end, "{what}: never all sent");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// MEASURE: `n` changes made offline, each naming `per` messages, replayed once back online.
#[test]
#[ignore]
fn measure_replaying_queued_changes() {
    for (n, per) in [(250, 50), (500, 50), (1000, 50)] {
        let (_st, store, mut pool) = offline(per, &format!("replay-measure-{n}"));
        let ops: Vec<Op> = (0..n).map(|i| Op::SetFlag { keys: keys(per), flag: Flag::Seen, on: i % 2 == 0 }).collect();
        store.save_ops("a1", &ops).unwrap();
        let bytes = std::fs::metadata(store.root().join("a1").join("ops.json")).map(|m| m.len()).unwrap_or(0);
        let began = Instant::now();
        pool.submit(Job::WorkOffline { on: false });
        all_sent(&store, "measure");
        let took = began.elapsed();
        eprintln!("MEASURE replay: {n} changes of {per} messages ({} KB queued) sent in {took:?}", bytes / 1024);
        pool.submit(Job::Remove { account: "a1".into() });
        pool.shutdown();
        let _ = std::fs::remove_dir_all(store.root());
    }
}

/// `n` changes made offline: keyword `$k0`, `$k1`… put on the Inbox's first message.
fn keyworded(n: usize) -> Vec<Op> {
    (0..n).map(|i| Op::SetKeyword { keys: keys(1), keyword: format!("$k{i}"), on: true }).collect()
}

/// How many times the server was asked to put each of these keywords on.
fn stored(st: &Arc<Mutex<State>>, words: &[String]) -> Vec<usize> {
    let log = st.lock().unwrap().log.clone();
    words.iter().map(|w| log.iter().filter(|l| l.contains("STORE") && l.split([' ', '(', ')']).any(|t| t == w)).count()).collect()
}

/// Wait until the server is on its way through the command `stall_on` holds.
fn stalled(st: &Arc<Mutex<State>>) {
    let end = Instant::now() + Duration::from_secs(20);
    while st.lock().unwrap().stalled == 0 {
        assert!(Instant::now() < end, "never reached the stalled change");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A change queued while those made offline replay (the window's, as one of them is on its
/// way to the server) is kept and sent after them; each is sent once, in order.
#[test]
fn a_change_queued_while_others_replay_is_kept_and_sent_once() {
    let (st, store, mut pool) = offline(1, "replay-queued-meanwhile");
    let mut ops = keyworded(10);
    store.save_ops("a1", &ops).unwrap();
    st.lock().unwrap().stall_on = Some(("$k3".into(), Duration::from_millis(1500)));
    pool.submit(Job::WorkOffline { on: false });
    stalled(&st);
    let late = Op::SetKeyword { keys: keys(1), keyword: "$late".into(), on: true };
    store.add_op("a1", late.clone()).unwrap();
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("$k3")), "queued while $k3 was on its way");
    ops.push(late);
    all_sent(&store, "queued meanwhile");
    let words: Vec<String> = ops.iter().map(|o| if let Op::SetKeyword { keyword, .. } = o { keyword.clone() } else { String::new() }).collect();
    assert_eq!(stored(&st, &words), vec![1; words.len()], "each sent once: {words:?}");
    let log = st.lock().unwrap().log.clone();
    let order: Vec<usize> = words.iter().filter_map(|w| log.iter().position(|l| l.contains("STORE") && l.contains(w.as_str()))).collect();
    assert!(order.windows(2).all(|p| p[0] < p[1]), "in order: {order:?}");
    let flags = st.lock().unwrap().mailbox("INBOX").unwrap().msgs[0].flags.clone();
    assert!(words.iter().all(|w| flags.contains(w)), "{flags:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// The connection lost part-way through the replay: the changes made are off the queue and
/// the rest still on it, so back online each is sent once, none lost and none twice.
#[test]
fn a_replay_stopped_part_way_sends_the_rest_once() {
    let (st, store, mut pool) = offline(1, "replay-stopped");
    let ops = keyworded(10);
    store.save_ops("a1", &ops).unwrap();
    st.lock().unwrap().stall_on = Some(("$k3".into(), Duration::from_millis(800)));
    pool.submit(Job::WorkOffline { on: false });
    stalled(&st);
    // The command after $k3 ($k4's) is dropped unanswered.
    {
        let mut s = st.lock().unwrap();
        s.stall_on = None;
        s.hang_up = true;
    }
    until(&pool, "connection lost", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    assert_eq!(store.load_ops("a1").unwrap(), ops[4..], "just the ones not made");
    pool.submit(Job::SyncAll { account: "a1".into() });
    all_sent(&store, "after the stop");
    let words: Vec<String> = (0..10).map(|i| format!("$k{i}")).collect();
    assert_eq!(stored(&st, &words), vec![1; 10], "each sent once");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
