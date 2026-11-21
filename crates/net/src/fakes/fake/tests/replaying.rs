//! Changes made offline, replayed on the server once the account is back: each sent once, in
//! order, and none lost to another queued meanwhile or to a stop part-way.

use super::*;

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
    pool.submit(Job::Start { account: imap_account(port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    (st, store, pool)
}

/// Wait until nothing waits for the server: the changes read, and none. Not the file looked for:
/// on Windows a look while the worker replaces or removes it can be refused for a moment, and
/// that read as gone (found under load: the last changes never sent by the time it was).
fn all_sent(store: &Store, what: &str) {
    // Longer than the usual deadline: the measurement replays a thousand changes.
    let sent = || store.load_ops("a1").is_ok_and(|ops| ops.is_empty());
    crate::waiting::eventually_within(Duration::from_secs(600), &format!("every change sent ({what})"), sent);
}

/// MEASURE: `n` changes made offline, each naming `per` messages, replayed once back online.
#[test]
#[ignore = "a measurement"]
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
    }
}

/// A move into a folder the server hasn't (deleted elsewhere, say) is refused there, as a real
/// server refuses it (`NO [TRYCREATE]`): the refusal is said, and the mail is back where it was,
/// on the server and here. Found in review: the in-memory server took such a move and lost the
/// mail, so nothing proved this.
#[test]
fn a_move_into_a_folder_not_there_is_said_and_the_mail_stays() {
    let st = State::new();
    for i in 0..2 {
        st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
    }
    let port = serve_tcp(&st);
    let store = store("replay-move-nowhere");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: imap_account(port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let cached = || store.load_folder("a1", "INBOX").map(|f| f.messages.len()).unwrap_or(0);
    assert_eq!(cached(), 2);
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: keys(2), to: "Gone".into() } });
    until(&pool, "the move refused", |e| matches!(e, Event::OpFailed { op: Op::Move { .. }, .. }));
    assert_eq!(st.lock().unwrap().mailbox("INBOX").map(|b| b.msgs.len()), Some(2), "on the server");
    eventually("the mail back here", || cached() == 2);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Mail moved on a server leaves nothing of itself where it was: its sources kept here went
/// with it, where they stayed in the cache for good, in no folder (the destination gets the
/// mail from the server under its new UID).
#[test]
fn mail_moved_on_the_server_leaves_no_source_behind() {
    let st = State::new();
    for i in 0..2 {
        st.lock().unwrap().add("INBOX", &format!("m{i}"), &[]);
    }
    let port = serve_tcp(&st);
    let store = store("replay-move-sources");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: imap_account(port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    for k in keys(2) {
        store.write_body(&k, b"Subject: m\r\n\r\nkept here").unwrap();
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::Move { keys: keys(2), to: "Archive".into() } });
    eventually("moved on the server", || st.lock().unwrap().mailbox("Archive").map(|b| b.msgs.len()) == Some(2));
    for k in keys(2) {
        assert!(store.read_body(&k).is_none(), "the source of {} stayed where it was", k.to_id());
    }
    pool.submit(Job::Remove { account: "a1".into() });
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

/// A change queued while those made offline replay (the window's, as one of them is on its
/// way to the server) is kept and sent after them; each is sent once, in order.
#[test]
fn a_change_queued_while_others_replay_is_kept_and_sent_once() {
    let (st, store, mut pool) = offline(1, "replay-queued-meanwhile");
    let mut ops = keyworded(10);
    store.save_ops("a1", &ops).unwrap();
    let (hold, holding) = Hold::on("$k3");
    st.lock().unwrap().hold_on = Some(hold);
    pool.submit(Job::WorkOffline { on: false });
    holding.came();
    let late = Op::SetKeyword { keys: keys(1), keyword: "$late".into(), on: true };
    store.add_op("a1", late.clone()).unwrap();
    assert!(!st.lock().unwrap().log.iter().any(|l| l.contains("$k3")), "queued while $k3 was on its way");
    holding.go(LetGo::Answer);
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
}

/// The connection lost part-way through the replay: the changes made are off the queue and
/// the rest still on it, so back online each is sent once, none lost and none twice.
#[test]
fn a_replay_stopped_part_way_sends_the_rest_once() {
    let (st, store, mut pool) = offline(1, "replay-stopped");
    let ops = keyworded(10);
    store.save_ops("a1", &ops).unwrap();
    let (hold, holding) = Hold::on("$k3");
    st.lock().unwrap().hold_on = Some(hold);
    pool.submit(Job::WorkOffline { on: false });
    holding.came();
    // $k3 is answered and the next command, $k4's, dropped unanswered. Signing in again is
    // refused until the queue is looked at, so the account can't come back and go on first.
    st.lock().unwrap().refuse_passwords = Some("not now".into());
    holding.go(LetGo::AnswerThenHangUp);
    until(&pool, "connection lost", |e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    assert_eq!(store.load_ops("a1").unwrap(), ops[4..], "just the ones not made");
    st.lock().unwrap().refuse_passwords = None;
    pool.submit(Job::SyncAll { account: "a1".into() });
    all_sent(&store, "after the stop");
    let words: Vec<String> = (0..10).map(|i| format!("$k{i}")).collect();
    assert_eq!(stored(&st, &words), vec![1; 10], "each sent once");
    pool.submit(Job::Remove { account: "a1".into() });
}
