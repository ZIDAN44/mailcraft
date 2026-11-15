//! The worker's drafts: kept on this computer when it stops before saving them (#53).

use super::*;
use crate::fakes::proxy::{Cut, Proxy};

mod filed_once;
mod kept_bench;
mod kept_files;

/// A worker stopped (working offline, its settings or password changed, MailCraft closing) with
/// a draft still to save keeps it on this computer and says so, so the message held for it goes
/// (found in review: the queue was dropped, and the message stayed held, unseen, all session).
#[test]
fn a_worker_stopped_keeps_the_drafts_it_had_still_to_save() {
    let store = crate::fake::store("stopped-drafts");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    let draft = b"Subject: Plans\r\nMessage-ID: <plans@studio.example>\r\n\r\nto finish\r\n".to_vec();
    tx.send(Job::SaveDraft { account: "a1".into(), raw: draft.clone(), folder: "Drafts".into(), replaces: None }).unwrap();
    drop(tx);
    w.run(rx);
    let saved =
        pool.poll().into_iter().any(|e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "plans@studio.example"));
    assert!(saved, "said kept");
    assert_eq!(kept_raws(&store, "a1").unwrap(), std::slice::from_ref(&draft));
    // Kept again (MailCraft closing kept it too): once.
    keep_draft(&store, "a1", "Drafts", None, draft, true, &Order::default()).unwrap();
    assert_eq!(kept_raws(&store, "a1").unwrap().len(), 1);
    assert_eq!(store.kept_draft_ids("a1").len(), 1, "the one it replaced has no source left");
}

/// Found in review: an account removed while its worker still had a draft to save had the
/// draft written into its files again as the worker stopped, and an account added again by the
/// same name later had it. A draft of an account removed since its worker started isn't kept.
#[test]
fn a_removed_accounts_drafts_are_not_kept_again() {
    let store = crate::fake::store("removed-drafts");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    let draft = b"Subject: Plans\r\nMessage-ID: <gone@studio.example>\r\n\r\nto finish\r\n".to_vec();
    tx.send(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "Drafts".into(), replaces: None }).unwrap();
    drop(tx);
    store.remove_account("a1").unwrap();
    w.run(rx);
    assert!(kept_raws(&store, "a1").unwrap_or_default().is_empty(), "kept again");
    assert!(store.kept_draft_ids("a1").is_empty(), "its source kept again");
    assert!(!pool.poll().into_iter().any(|e| matches!(e, Event::DraftSaved { .. })), "nothing said of it");
}

/// A server that takes the connections and never answers, until what it gives back with its
/// port is dropped (the test's end): then it lets them go, so a worker waiting on it ends, and
/// nothing writes in the test's folder after it went. (It held them a minute, past the test.)
fn silent_server() -> (u16, std::sync::mpsc::Sender<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut held: Vec<std::net::TcpStream> = Vec::new();
        while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(Duration::from_millis(20)) {
            held.extend(std::iter::from_fn(|| listener.accept().ok().map(|(s, _)| s)));
        }
    });
    (port, tx)
}

/// A worker restarted (a password or setting changed) while busy, with a draft handed to it and
/// not saved yet: the pool keeps that draft on this computer before the new worker starts and
/// reads what's kept, and says so, so the message held for it goes (found in review: the old
/// worker kept it only once it noticed it had stopped, after the new one had read the file, and
/// the draft waited for the next connection unseen).
#[test]
fn a_restart_keeps_the_drafts_on_their_way_first() {
    // The worker waits on a server that never answers.
    let (port, _server) = silent_server();
    let store = crate::fake::store("restart-drafts");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = imap_account("a1", port);
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    let draft = b"Subject: Plans\r\nMessage-ID: <restart@studio.example>\r\n\r\nto finish\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft.clone(), folder: "Drafts".into(), replaces: None });
    pool.submit(Job::Start { account, password: None });
    assert_eq!(kept_raws(&store, "a1").unwrap(), [draft], "kept before the new worker reads it");
    let saved =
        pool.poll().into_iter().any(|e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "restart@studio.example"));
    assert!(saved, "said kept");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Found in review: saved, written on, and MailCraft closed while the save was still on its way:
/// the save, kept as the worker stopped, replaced the draft kept as MailCraft closed (the same
/// Message-ID), and what was written after the save was lost. The one kept on close stays, and so
/// it does when the save fails on its way after it.
#[test]
fn what_was_written_after_a_save_is_kept_on_close() {
    // The save stays on its way to a server that never answers.
    let (port, _server) = silent_server();
    let store = crate::fake::store("written-after-save");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = imap_account("a1", port);
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let saved = b"Subject: Plans\r\nMessage-ID: <later@studio.example>\r\n\r\nto finish\r\n".to_vec();
    let written = b"Subject: Plans\r\nMessage-ID: <later@studio.example>\r\n\r\nto finish, and the rest of it\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: saved.clone(), folder: "Drafts".into(), replaces: None });
    pool.submit(Job::KeepDraft { account: "a1".into(), raw: written.clone(), folder: "Drafts".into(), replaces: None });
    pool.shutdown();
    assert_eq!(kept_raws(&store, "a1").unwrap(), std::slice::from_ref(&written), "the draft as it was written last");
    // The save failing on its way after that (the worker's own end) changes nothing.
    keep_draft(&store, "a1", "Drafts", None, saved, false, &pool.order).unwrap();
    assert_eq!(kept_raws(&store, "a1").unwrap(), [written]);
    drop(pool);
}

/// A fake IMAP server with an empty Drafts, and an account on it whose calendars and contacts
/// were looked for today: the saves queued after its first sync don't wait for them to be looked
/// for on the network.
pub(super) fn with_drafts(name: &str) -> (Arc<Mutex<crate::fake::State>>, Store, AccountConfig) {
    let st = crate::fake::State::new();
    st.lock().unwrap().boxes.push(crate::fake::Mailbox::new("Drafts", r"\Drafts", 600));
    let port = crate::fake::serve_tcp(&st);
    let account = imap_account("a1", port);
    let store = crate::fake::store(name);
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    (st, store, account)
}

/// The drafts in Drafts on the fake server, by their text as written.
pub(super) fn on_the_server(st: &Arc<Mutex<crate::fake::State>>) -> Vec<String> {
    let mut s = st.lock().unwrap();
    s.mailbox("Drafts")
        .map(|b| b.msgs.iter().map(|m| String::from_utf8_lossy(&crate::fake::as_written(&m.raw)).into_owned()).collect())
        .unwrap_or_default()
}

/// Found in review: the drafts kept on this computer were taken out of their file as the account
/// connected, and held in the worker's memory only until each was filed: MailCraft closing
/// meanwhile (the pool doesn't wait for its workers) lost every one still to file, and the one
/// on its way. Each stays in the file until it's filed.
#[test]
fn kept_drafts_stay_kept_until_each_is_filed() {
    let (st, store, account) = with_drafts("kept-until-filed");
    let texts: Vec<String> =
        (1..=3).map(|n| format!("From: me@studio.example\r\nSubject: Plans {n}\r\nMessage-ID: <k{n}@studio.example>\r\n\r\nto finish\r\n")).collect();
    for t in &texts {
        keep_draft(&store, "a1", "Drafts", None, t.clone().into_bytes(), false, &Order::default()).unwrap();
    }
    // The first draft's APPEND held by the server, until the test lets it go: on its way as
    // MailCraft closes, however long a busy computer takes to close.
    let (hold, holding) = crate::fake::Hold::on("APPEND");
    st.lock().unwrap().hold_on = Some(hold);
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    holding.came();
    pool.shutdown();
    assert_eq!(kept_texts(&store), texts, "every draft not filed yet kept as MailCraft closes");
    // The one on its way gets there, and is kept no more; the others wait for the next run.
    holding.go(crate::fake::LetGo::Answer);
    eventually("the first filed", || on_the_server(&st).len() == 1 && kept_texts(&store).len() == 2);
    // The stopped thread done: nothing more of it.
    threads_ended(&store, 1);
    assert_eq!(on_the_server(&st), texts[..1]);
    assert_eq!(kept_texts(&store), texts[1..]);
    drop(pool);
}

/// The drafts kept on this computer, by their text.
pub(super) fn kept_texts(store: &Store) -> Vec<String> {
    kept_raws(store, "a1").unwrap_or_default().iter().map(|raw| String::from_utf8_lossy(raw).into_owned()).collect()
}

/// Wait until `n` of the account's threads over `store` have ended (stopped ones, done with
/// what they were doing): what they did is all done.
pub(super) fn threads_ended(store: &Store, n: usize) {
    eventually(&format!("{n} of the account's threads to end"), || connect::ended::count(store, "a1") >= n);
}

/// Every event until the account's thread waits (for a job, or out a wait of its own): what it
/// was doing is all done, and nothing more is until something wakes it.
pub(super) fn until_waiting(pool: &Pool) -> Vec<Event> {
    events_until(pool, "the account's thread to wait", |_| pool.idle("a1"))
}

/// A draft kept as MailCraft closed, in a pool's files that the next run reads.
fn closed_with(store: &Store, raw: &[u8]) {
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::KeepDraft { account: "a1".into(), raw: raw.to_vec(), folder: "Drafts".into(), replaces: None });
    pool.shutdown();
}

/// Found in review: the draft kept as MailCraft closed was marked so in the kept drafts' file,
/// and the next run's save of the same draft, reopened from Drafts and saved offline, was said
/// kept and dropped: what was written that run was lost. A later run's save is newer.
#[test]
fn the_next_runs_save_offline_replaces_the_draft_kept_on_close() {
    let store = crate::fake::store("next-run-offline");
    closed_with(&store, b"Subject: Plans\r\nMessage-ID: <x@studio.example>\r\n\r\nkept on close\r\n");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let newer = b"Subject: Plans\r\nMessage-ID: <x@studio.example>\r\n\r\nwritten the next day\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: newer, folder: "Drafts".into(), replaces: None });
    assert!(pool.poll().iter().any(|e| matches!(e, Event::DraftSaved { error: None, .. })), "said kept");
    assert_eq!(kept_texts(&store), ["Subject: Plans\r\nMessage-ID: <x@studio.example>\r\n\r\nwritten the next day\r\n"]);
}

/// The same with the account online and its server not answering: the save fails, and is kept
/// over the draft kept as MailCraft closed the run before.
#[test]
fn the_next_runs_failed_save_replaces_the_draft_kept_on_close() {
    let store = crate::fake::store("next-run-failed");
    closed_with(&store, b"Subject: Plans\r\nMessage-ID: <y@studio.example>\r\n\r\nkept on close\r\n");
    // Nothing listening there.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = imap_account("a1", port);
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let newer = b"Subject: Plans\r\nMessage-ID: <y@studio.example>\r\n\r\nwritten the next day\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: newer, folder: "Drafts".into(), replaces: None });
    let seen = until(&pool, "the draft saved", |e| matches!(e, Event::DraftSaved { .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::DraftSaved { error: None, .. })), "said kept");
    assert_eq!(kept_texts(&store), ["Subject: Plans\r\nMessage-ID: <y@studio.example>\r\n\r\nwritten the next day\r\n"]);
    pool.shutdown();
}

/// Found in review: a save on its way as Work Offline was turned on, and a newer version of the
/// draft saved offline: the stopped worker's save, filed, took the newer one off this computer as
/// older, and what was written offline never got to the server. It stays kept, and replaces the
/// older one at the next connection.
#[test]
fn a_draft_saved_offline_outlives_the_older_save_filed_after() {
    let (st, store, account) = with_drafts("offline-after-save");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "calendars and contacts synced", |e| matches!(e, Event::DavSynced { .. }));
    let (hold, holding) = crate::fake::Hold::on("APPEND");
    st.lock().unwrap().hold_on = Some(hold);
    let first = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <off@studio.example>\r\n\r\nfirst\r\n";
    let later = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <off@studio.example>\r\n\r\nfirst, and what was written offline after\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: first.into(), folder: "Drafts".into(), replaces: None });
    // On its way to the server, held there, as Work Offline is turned on.
    holding.came();
    pool.submit(Job::WorkOffline { on: true });
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: later.into(), folder: "Drafts".into(), replaces: None });
    assert_eq!(kept_texts(&store), [later]);
    holding.go(crate::fake::LetGo::Answer);
    eventually("the first filed", || on_the_server(&st) == [first]);
    // The stopped thread done with it.
    threads_ended(&store, 1);
    assert_eq!(kept_texts(&store), [later], "what was written offline still kept");
    // Online again: it replaces the older one there.
    pool.submit(Job::WorkOffline { on: false });
    eventually("the later filed", || on_the_server(&st) == [later] && kept_texts(&store).is_empty());
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Found in review: a save on its way as Work Offline was turned on, filed after: the stopped
/// worker read Drafts again, and the status bar said the account was online. It reads nothing
/// more, and says nothing of the account.
#[test]
fn a_save_filed_after_work_offline_says_nothing_of_the_account() {
    let (st, store, account) = with_drafts("offline-says-nothing");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "calendars and contacts synced", |e| matches!(e, Event::DavSynced { .. }));
    let (hold, holding) = crate::fake::Hold::on("APPEND");
    st.lock().unwrap().hold_on = Some(hold);
    let draft = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <quiet@studio.example>\r\n\r\nfirst\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft.into(), folder: "Drafts".into(), replaces: None });
    holding.came();
    let _ = pool.poll();
    pool.submit(Job::WorkOffline { on: true });
    holding.go(crate::fake::LetGo::Answer);
    eventually("the draft filed", || on_the_server(&st) == [draft]);
    // The stopped thread done: all it said is said.
    threads_ended(&store, 1);
    let said: Vec<SyncStatus> = pool
        .poll()
        .into_iter()
        .filter_map(|e| match e {
            Event::Status { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(said, [SyncStatus::Offline], "said of the account after Work Offline");
    let log = st.lock().unwrap().log.clone();
    let appended = log.iter().position(|c| c.starts_with("APPEND")).unwrap();
    assert!(!log[appended..].iter().any(|c| c.starts_with("SELECT") || c.starts_with("EXAMINE")), "read again: {log:?}");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Kept as the newest version (Keep all, or a held message let go) while MailCraft runs on: a
/// save made after it is newer, and replaces it.
#[test]
fn a_save_made_after_a_draft_is_kept_replaces_it() {
    let store = crate::fake::store("saved-after-kept");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let kept = b"Subject: Plans\r\nMessage-ID: <z@studio.example>\r\n\r\nkept\r\n".to_vec();
    pool.submit(Job::KeepDraft { account: "a1".into(), raw: kept, folder: "Drafts".into(), replaces: None });
    let newer = b"Subject: Plans\r\nMessage-ID: <z@studio.example>\r\n\r\nwritten after\r\n".to_vec();
    DraftOrder::handed(&pool.order, &newer);
    keep_draft(&store, "a1", "Drafts", None, newer, false, &pool.order).unwrap();
    assert_eq!(kept_texts(&store), ["Subject: Plans\r\nMessage-ID: <z@studio.example>\r\n\r\nwritten after\r\n"]);
}

/// Found in review: an account removed while its draft was on the way to the server, saved
/// there: the worker read Drafts again after, and wrote the account's folder list and the draft
/// into its files, which came back. Nothing of it is written, nor said.
#[test]
fn a_removed_accounts_saved_draft_writes_nothing() {
    let (st, store, acc) = with_drafts("removed-saved");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let (hold, holding) = crate::fake::Hold::on("APPEND");
    st.lock().unwrap().hold_on = Some(hold);
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    let draft = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <gone2@studio.example>\r\n\r\nto finish\r\n".to_vec();
    tx.send(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "Drafts".into(), replaces: None }).unwrap();
    let t = std::thread::spawn(move || w.run(rx));
    // Removed while the APPEND is on its way, as `Job::Remove` does (its channel dropped).
    holding.came();
    store.remove_account("a1").unwrap();
    drop(tx);
    holding.go(crate::fake::LetGo::Answer);
    let _ = t.join();
    assert_eq!(st.lock().unwrap().mailbox("Drafts").map(|b| b.msgs.len()), Some(1), "saved on the server");
    assert!(!pool.poll().iter().any(|e| matches!(e, Event::DraftSaved { .. })), "nothing said of it");
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "the folder list written again");
    assert!(store.load_folder("a1", "Drafts").map(|f| f.messages.len()).unwrap_or(0) == 0, "the draft cached again");
}

/// A proxy to the server at `port` that drops the connection, both ways, when the client sends
/// `marker` (as a proxy resetting on a message it won't pass), and how many connections it took.
fn dropping_proxy(port: u16, marker: &'static [u8]) -> (u16, Arc<AtomicUsize>) {
    let proxy = Proxy { client: Some((marker, Cut::Drop)), ..Proxy::default() }.to(port);
    (proxy.port, proxy.taken)
}

/// Found in review: a draft kept on this computer whose filing dropped the connection (too big
/// for a proxy, say) was read again ahead of everything at each connection, and each save made
/// meanwhile connected again for itself, ignoring the wait after a failed connection: 605
/// sign-ins in 6 s, the save never got there, and the kept draft's failure was said each time.
/// The save waits for the next connection like any other, the kept draft that failed is tried
/// again only after a wait of its own, and its failure is said once. A newer version of it saved
/// meanwhile replaces it, and the older one is kept no more.
#[test]
fn a_kept_draft_that_drops_the_connection_holds_nothing_up() {
    let (st, store, mut account) = with_drafts("kept-drops");
    let (port, connections) = dropping_proxy(account.imap.port, b"NOT-PASSED");
    account.imap.port = port;
    let big = "From: me@studio.example\r\nSubject: Big\r\nMessage-ID: <big@studio.example>\r\n\r\nNOT-PASSED\r\n";
    keep_draft(&store, "a1", "Drafts", None, big.into(), false, &Order::default()).unwrap();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let plans = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <plans@studio.example>\r\n\r\nto finish\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: plans.into(), folder: "Drafts".into(), replaces: None });
    let mut seen = events_until(&pool, "the save on the server", |_| on_the_server(&st).iter().any(|m| m == plans));
    // The kept draft's newer version, saved now: it goes, and the older one goes from this computer.
    let newer = "From: me@studio.example\r\nSubject: Big\r\nMessage-ID: <big@studio.example>\r\n\r\nsmaller now\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: newer.into(), folder: "Drafts".into(), replaces: None });
    // On the server before its answer gets here: the older version goes once the worker has it.
    seen.extend(events_until(&pool, "the newer version on the server, the older one kept no more", |_| {
        on_the_server(&st).iter().any(|m| m == newer) && kept_texts(&store).is_empty()
    }));
    seen.extend(until_waiting(&pool));
    assert!(connections.load(Ordering::Relaxed) <= 3, "{} connections", connections.load(Ordering::Relaxed));
    assert_eq!(on_the_server(&st), [plans, newer]);
    assert!(kept_texts(&store).is_empty(), "the older version kept again: {:?}", kept_texts(&store));
    let said_of_big =
        seen.iter().filter(|e| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer couldn't"))).count();
    assert_eq!(said_of_big, 1, "{seen:?}");
    let saved: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            Event::DraftSaved { message_id, error: None, .. } => Some(message_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(saved, ["plans@studio.example", "big@studio.example"], "each save said done once, and the kept draft not at all");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A kept draft that fails each time it's tried (Send / Receive tries it again at once) is said
/// once in a run, and stays kept on this computer. The connection it drops is made again at once,
/// and it's tried again only after a wait of its own, or with Send / Receive.
#[test]
fn a_kept_draft_failing_again_is_said_once() {
    let (_st, store, mut account) = with_drafts("kept-fails-again");
    let (port, connections) = dropping_proxy(account.imap.port, b"NOT-PASSED");
    account.imap.port = port;
    let big = "From: me@studio.example\r\nSubject: Big\r\nMessage-ID: <big@studio.example>\r\n\r\nNOT-PASSED\r\n";
    keep_draft(&store, "a1", "Drafts", None, big.into(), false, &Order::default()).unwrap();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let failed = |e: &Event| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer couldn't"));
    // A busy machine's time to connect three times: what's checked is waited for, never timed.
    let mut seen = until(&pool, "the kept draft tried", failed);
    seen.extend(events_until(&pool, "connected again", |_| connections.load(Ordering::Relaxed) >= 2));
    seen.extend(until_waiting(&pool));
    assert_eq!(connections.load(Ordering::Relaxed), 2, "tried again before its wait");
    pool.submit(Job::SyncAll { account: "a1".into() });
    seen.extend(events_until(&pool, "tried again with Send / Receive", |_| connections.load(Ordering::Relaxed) >= 3));
    seen.extend(until_waiting(&pool));
    assert_eq!(seen.iter().filter(|e| failed(e)).count(), 1, "{seen:?}");
    assert_eq!(connections.load(Ordering::Relaxed), 3);
    assert_eq!(kept_texts(&store), [big]);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Three drafts kept on this computer, each dropping the connection as it's filed, and a save
/// (`reconnecting`: made once the first has dropped it, with Send / Receive, so it waits in the
/// queue as the account connects again; else as the account starts): how many connections there
/// were once it was made, and once it got there.
fn save_behind_kept_drafts_that_drop(name: &str, reconnecting: bool) -> (usize, usize) {
    let (st, store, mut account) = with_drafts(name);
    let (port, connections) = dropping_proxy(account.imap.port, b"NOT-PASSED");
    account.imap.port = port;
    for n in 1..=3 {
        let big = format!("From: me@studio.example\r\nSubject: Big {n}\r\nMessage-ID: <big{n}@studio.example>\r\n\r\nNOT-PASSED\r\n");
        keep_draft(&store, "a1", "Drafts", None, big.into_bytes(), false, &Order::default()).unwrap();
    }
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    if reconnecting {
        until(&pool, "a kept draft failed", |e| matches!(e, Event::Notice { message, .. } if message.contains("kept on this computer couldn't")));
        pool.submit(Job::SyncAll { account: "a1".into() });
    }
    let plans = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <plans@studio.example>\r\n\r\nto finish\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: plans.into(), folder: "Drafts".into(), replaces: None });
    let before = connections.load(Ordering::Relaxed);
    events_until(&pool, "the save on the server", |_| on_the_server(&st).iter().any(|m| m == plans));
    let taken = connections.load(Ordering::Relaxed);
    pool.submit(Job::Remove { account: "a1".into() });
    (before, taken)
}

/// Found in review: each draft kept on this computer whose filing dropped the connection held a
/// save made meanwhile back by one more connection, five seconds each: with three, the save got
/// there after 15 s and four connections. Once a kept draft has failed in this run, a save goes
/// ahead of the kept drafts of other drafts at the next connection, whether it waits for it kept
/// on this computer or in the queue.
#[test]
fn a_save_waits_for_no_more_than_one_kept_draft_that_drops_the_connection() {
    let (_, taken) = save_behind_kept_drafts_that_drop("kept-drops-many", false);
    assert_eq!(taken, 2, "filed at the connection after the first one dropped");
    // At most the connection after the one there as it was made: Send / Receive's, or the one made
    // again at once as the next kept draft dropped it.
    let (before, taken) = save_behind_kept_drafts_that_drop("kept-drops-many-queued", true);
    assert!(taken <= before + 1, "{before} connections as it was made, {taken} as it got there");
}

/// Found in review: saves made as the account was online, each but the last dropping the
/// connection as it was filed (too big for a proxy, say): each kept the saves after it waiting
/// one more wait to connect again (five seconds, then ten), and the last got there after 19 s and
/// four connections. A connection a draft's own filing dropped is made again at once (one that
/// can't be made waits as ever), and that draft waits to be tried again on its own.
#[test]
fn saves_that_drop_the_connection_hold_the_next_back_by_no_wait() {
    let (st, store, mut pool, connections) = online_saving_drafts_that_drop("saves-drop", 3);
    let plans = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <plans@studio.example>\r\n\r\nto finish\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: plans.into(), folder: "Drafts".into(), replaces: None });
    // Whether it waited is counted, not timed.
    events_until(&pool, "the save on the server", |_| on_the_server(&st).iter().any(|m| m == plans));
    // Each connection a filing dropped made again at once: no wait to connect again on its way.
    assert_eq!(pool.waits_begun("a1"), 0, "waited to connect again");
    until_waiting(&pool);
    // One connection more for each that dropped it, and none after: each waits on its own.
    assert_eq!(connections.load(Ordering::Relaxed), 4);
    assert_eq!(kept_texts(&store).len(), 3, "those that dropped it kept here");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// An account online, its first sync over, through a proxy that drops the connection on a draft
/// too big for it, and `drops` such drafts saved: the server, the files, the pool and how many
/// connections the proxy took.
fn online_saving_drafts_that_drop(name: &str, drops: usize) -> (Arc<Mutex<crate::fake::State>>, Store, Pool, Arc<AtomicUsize>) {
    let (st, store, mut account) = with_drafts(name);
    let (port, connections) = dropping_proxy(account.imap.port, b"NOT-PASSED");
    account.imap.port = port;
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    // Calendars and contacts are the first sync's last.
    until(&pool, "calendars and contacts synced", |e| matches!(e, Event::DavSynced { .. }));
    for n in 1..=drops {
        let big = format!("From: me@studio.example\r\nSubject: Big {n}\r\nMessage-ID: <big{n}@studio.example>\r\n\r\nNOT-PASSED\r\n");
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: big.into_bytes(), folder: "Drafts".into(), replaces: None });
    }
    (st, store, pool, connections)
}

/// Drafts that each drop the connection as they're filed, saved one after another (an autosave
/// of a draft too big for a proxy): the connection is made again at once a few times at most in
/// a while, and then waits as a connection that failed does, rather than signing in again and
/// again.
#[test]
fn drafts_that_drop_each_connection_sign_in_again_at_once_a_few_times_at_most() {
    let (_st, store, mut pool, connections) = online_saving_drafts_that_drop("saves-drop-many", 8);
    events_until(&pool, "the worker to wait to connect again", |_| pool.waits_begun("a1") > 0);
    // The first connection's, and four made again at once; the fifth's drop waits as a connection
    // that failed does, and the saves left, made meanwhile, are each kept here with no sign-in.
    assert_eq!(connections.load(Ordering::Relaxed), 5);
    assert_eq!(kept_texts(&store).len(), 8, "each kept here");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A pending-drafts list that can't be read isn't saved over with just the newest draft: the
/// drafts waiting in it stay.
#[test]
fn a_draft_kept_offline_doesnt_write_over_a_list_that_cant_be_read() {
    let store = Store::failing();
    keep_draft(&store, "a1", "Drafts", None, b"Subject: first\r\n\r\nOne\r\n".to_vec(), false, &Order::default()).unwrap();
    store.fail(Some(KEPT_DRAFTS), None);
    assert!(keep_draft(&store, "a1", "Drafts", None, b"Subject: second\r\n\r\nTwo\r\n".to_vec(), false, &Order::default()).is_err());
    store.fail(None, None);
    assert_eq!(kept_raws(&store, "a1").unwrap(), [b"Subject: first\r\n\r\nOne\r\n".to_vec()]);
}
