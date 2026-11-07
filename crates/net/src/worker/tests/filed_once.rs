//! A draft filed once on the server however its filing is cut short: an account's connection
//! started again while it's on its way, or the server's answer lost (#53).

use super::drafts::{kept_texts, on_the_server, wait_for, with_drafts};
use super::*;

/// Every event the pool says until `done`, thirty seconds at most (a guard against hanging).
fn events_until(pool: &Pool, done: impl Fn() -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < end, "timed out; saw {seen:?}");
        seen.extend(pool.wait(Duration::from_millis(20)));
    }
    seen
}

/// The account's connection started again (a password or setting changed, Work Offline turned
/// off and on) while the server takes 1.5 s over each draft filed: the drafts in Drafts, by their
/// text, and what the pool said.
fn restarted_while_filing(name: &str, kept: Option<&str>, saved: Option<&str>) -> (Vec<String>, Vec<Event>, Vec<String>) {
    let (st, store, account) = with_drafts(name);
    if let Some(k) = kept {
        keep_draft(&store, "a1", "Drafts", None, k.into(), false, &Order::default()).unwrap();
    }
    st.lock().unwrap().stall_on = Some(("APPEND".into(), Duration::from_millis(1500)));
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    if let Some(s) = saved {
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: s.into(), folder: "Drafts".into(), replaces: None });
    }
    // The draft on its way to the server as the connection starts again.
    wait_for("the draft on its way", || st.lock().unwrap().stalled > 0);
    pool.submit(Job::Start { account, password: None });
    let mut seen = events_until(&pool, || !on_the_server(&st).is_empty() && kept_texts(&store).is_empty());
    // Long enough for a second copy, filed by the new connection, to get there.
    let end = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < end {
        seen.extend(pool.wait(Duration::from_millis(50)));
    }
    let (filed, kept) = (on_the_server(&st), kept_texts(&store));
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    (filed, seen, kept)
}

/// Found in review: a draft kept on this computer stays in the kept drafts' file until it's
/// filed, so an account's connection started again while the old one was filing it read it there
/// and filed it again, and Drafts had it twice. The new connection sets it aside until the old
/// one's filing of it ends, and finds it filed.
#[test]
fn a_kept_draft_on_its_way_as_the_connection_starts_again_is_filed_once() {
    let text = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <once@studio.example>\r\n\r\nto finish\r\n";
    let (filed, _, kept) = restarted_while_filing("restart-kept-once", Some(text), None);
    assert_eq!(filed, [text]);
    assert!(kept.is_empty(), "{kept:?}");
}

/// The same for a draft saved, on its way as the connection starts again: the pool keeps it on
/// this computer for the new connection, which sets it aside until the old one's filing of it
/// ends. Its save is said done once.
#[test]
fn a_save_on_its_way_as_the_connection_starts_again_is_filed_once() {
    let text = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <saved-once@studio.example>\r\n\r\nto finish\r\n";
    let (filed, seen, kept) = restarted_while_filing("restart-save-once", None, Some(text));
    assert_eq!(filed, [text]);
    assert!(kept.is_empty(), "{kept:?}");
    let said = seen.iter().filter(|e| matches!(e, Event::DraftSaved { message_id, .. } if message_id == "saved-once@studio.example")).count();
    assert_eq!(said, 1, "{seen:?}");
}

/// A proxy to the server at `port` that drops the connection, both ways, the first time the
/// server's answer has `marker` in it (the server has done what it answers).
fn answer_dropping_proxy(port: u16, marker: &'static [u8]) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().port();
    let dropped = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", port)) else { continue };
            let (Ok(mut from_server), Ok(mut to_server)) = (server.try_clone(), server.try_clone()) else { continue };
            let (Ok(mut from_client), mut to_client) = (client.try_clone(), client) else { continue };
            let dropped = dropped.clone();
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut from_client, &mut to_server);
                let _ = to_server.shutdown(std::net::Shutdown::Both);
            });
            std::thread::spawn(move || {
                let (mut tail, mut buf) = (Vec::new(), [0u8; 8192]);
                while let Ok(n @ 1..) = from_server.read(&mut buf) {
                    tail.extend_from_slice(&buf[..n]);
                    let hit = tail.windows(marker.len()).any(|w| w == marker);
                    if (hit && !dropped.swap(true, Ordering::Relaxed)) || to_client.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    tail.drain(..tail.len().saturating_sub(marker.len()));
                }
                let _ = to_client.shutdown(std::net::Shutdown::Both);
                let _ = from_server.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    at
}

/// Found in review: a draft was remembered as MailCraft's own copy only once the server answered
/// its APPEND, so a copy the server kept without its answer getting here (the connection lost, or
/// MailCraft closed meanwhile, the draft kept to be filed again) wasn't known as its own, and the
/// draft filed again was in Drafts twice. It's remembered before it goes, and its copy there is
/// replaced.
#[test]
fn a_draft_whose_answer_was_lost_is_filed_once() {
    let (st, store, mut account) = with_drafts("answer-lost-once");
    account.imap.port = answer_dropping_proxy(account.imap.port, b"APPENDUID");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let text = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <lost@studio.example>\r\n\r\nto finish\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: text.into(), folder: "Drafts".into(), replaces: None });
    events_until(&pool, || kept_texts(&store) == [text]);
    assert_eq!(on_the_server(&st), [text], "the server kept it");
    // Send / Receive: the draft kept here is filed again.
    pool.submit(Job::SyncAll { account: "a1".into() });
    events_until(&pool, || kept_texts(&store).is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(on_the_server(&st), [text]);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A link that hangs ([`hanging_proxy`]): whether it has, and, set by the test, that it's closed.
#[derive(Default)]
struct Hang {
    hung: AtomicBool,
    closed: AtomicBool,
}

/// The longest a hung link waits to be closed: longer than [`events_until`] waits, so a
/// connection that waited for it is caught.
const HANG_MOST: Duration = Duration::from_secs(60);

/// A proxy to the server at `port` whose first connection goes silent, both ways, once the client
/// sends `marker` (the link hangs), and is closed once the test says ([`Hang::closed`]); the others
/// pass everything.
fn hanging_proxy(port: u16, marker: &'static [u8]) -> (u16, Arc<Hang>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().port();
    let hang = Arc::new(Hang::default());
    let first = hang.clone();
    std::thread::spawn(move || {
        for (n, client) in listener.incoming().flatten().enumerate() {
            let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", port)) else { continue };
            let (Ok(mut from_client), Ok(mut to_server)) = (client.try_clone(), server.try_clone()) else { continue };
            let (mut from_server, mut to_client) = (server, client);
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut from_server, &mut to_client);
                let _ = to_client.shutdown(std::net::Shutdown::Both);
            });
            let first = (n == 0).then(|| first.clone());
            std::thread::spawn(move || {
                let (mut tail, mut buf) = (Vec::new(), [0u8; 8192]);
                while let Ok(n @ 1..) = from_client.read(&mut buf) {
                    tail.extend_from_slice(&buf[..n]);
                    if let Some(hang) = first.as_ref().filter(|_| tail.windows(marker.len()).any(|w| w == marker)) {
                        hang.hung.store(true, Ordering::Relaxed);
                        let end = Instant::now() + HANG_MOST;
                        while !hang.closed.load(Ordering::Relaxed) && Instant::now() < end {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        break;
                    }
                    if to_server.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    tail.drain(..tail.len().saturating_sub(marker.len()));
                }
                let _ = from_client.shutdown(std::net::Shutdown::Both);
                let _ = to_server.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    (at, hang)
}

/// A draft whose filing hangs, and another.
const HANGING: &str = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <hung@studio.example>\r\n\r\nto finish\r\n";
const NEXT: &str = "From: me@studio.example\r\nSubject: Notes\r\nMessage-ID: <next@studio.example>\r\n\r\nfor later\r\n";

/// The account's connection started again (Work Offline off and on, as a user does when it hangs)
/// while the link has hung on [`HANGING`]'s filing; `idle`, the server pushes new mail. Then
/// `newer`, a newer version of it, saved, [`NEXT`] saved and the Inbox read, all while the link
/// still hangs (a new connection that waited for the old one's filing would get none of it done);
/// then the link closed: the drafts in Drafts once the old connection's filing has failed.
fn restarted_while_a_filing_hangs(name: &str, idle: bool, newer: Option<&str>) -> Vec<String> {
    let (st, store, mut account) = with_drafts(name);
    if idle {
        st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
    }
    let (port, hang) = hanging_proxy(account.imap.port, b"APPEND");
    account.imap.port = port;
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account: account.clone(), password: Some(Secret("pw".into())) });
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: HANGING.into(), folder: "Drafts".into(), replaces: None });
    wait_for("the filing to hang", || hang.hung.load(Ordering::Relaxed));
    pool.submit(Job::Start { account, password: None });
    if let Some(n) = newer {
        pool.submit(Job::SaveDraft { account: "a1".into(), raw: n.into(), folder: "Drafts".into(), replaces: None });
    }
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: NEXT.into(), folder: "Drafts".into(), replaces: None });
    st.lock().unwrap().add("INBOX", "News", &[]);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    let read = || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.len() == 1);
    let seen = events_until(&pool, || on_the_server(&st).iter().any(|m| m == NEXT) && read());
    if newer.is_some() {
        let said = seen.iter().any(|e| matches!(e, Event::DraftSaved { message_id, error: None, .. } if message_id == "hung@studio.example"));
        assert!(said, "the newer version said kept: {seen:?}");
    }
    // The old connection's filing fails as the link closes: the new one files the draft.
    hang.closed.store(true, Ordering::Relaxed);
    events_until(&pool, || on_the_server(&st).len() == 2 && kept_texts(&store).is_empty());
    std::thread::sleep(Duration::from_millis(300));
    let filed = on_the_server(&st);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    filed
}

/// Found in review: the account's connection started again while the link hung on a draft's
/// filing waited for the old connection's filing of it to end (a minute, as its read gave up),
/// and the whole account with it: other drafts, the Inbox, sending. The new connection leaves
/// that draft kept on this computer meanwhile, as it does a newer version of it saved then, and
/// files it once the old filing has ended, with the server pushing new mail (IDLE) or not.
#[test]
fn a_connection_started_again_while_a_filing_hangs_waits_for_nothing() {
    let newer = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <hung@studio.example>\r\n\r\nto finish, and more\r\n";
    assert_eq!(restarted_while_a_filing_hangs("restart-hung", false, Some(newer)), [NEXT, newer]);
    assert_eq!(restarted_while_a_filing_hangs("restart-hung-idle", true, None), [NEXT, HANGING], "IDLE");
}

/// A draft saved and filed, the pool stopping the worker (Work Offline turned on) before the
/// window has heard: what the pool said of it, how many copies of it it kept, and the drafts in
/// Drafts once the account is back online. `uidplus`: the server can remove exactly MailCraft's own
/// copies.
fn filed_then_offline_before_heard(name: &str, uidplus: bool) -> (usize, usize, Vec<String>) {
    let (st, store, account) = with_drafts(name);
    if !uidplus {
        st.lock().unwrap().caps = "IMAP4rev1 MOVE CONDSTORE ENABLE QUOTA";
    }
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let mut seen = super::until(&pool, |e| matches!(e, Event::DavSynced { .. }));
    let text = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <heard@studio.example>\r\n\r\nto finish\r\n";
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: text.into(), folder: "Drafts".into(), replaces: None });
    // Filed, and Drafts read again, with nothing heard of it here yet.
    wait_for("the draft filed", || on_the_server(&st) == [text]);
    std::thread::sleep(Duration::from_millis(500));
    pool.submit(Job::WorkOffline { on: true });
    seen.extend(pool.poll());
    let kept = kept_texts(&store).len();
    pool.submit(Job::WorkOffline { on: false });
    seen.extend(events_until(&pool, || kept_texts(&store).is_empty()));
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        seen.extend(pool.wait(Duration::from_millis(50)));
    }
    let said = seen.iter().filter(|e| matches!(e, Event::DraftSaved { message_id, .. } if message_id == "heard@studio.example")).count();
    let filed = on_the_server(&st);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    (said, kept, filed)
}

/// Found in review: the pool took a save off its list of saves on their way only as the window
/// heard it said done, so a save filed and said done, the worker stopped (Work Offline, a restart,
/// MailCraft closing) before the window heard, was kept on this computer again and said done
/// twice; filed again at the next connection, it was in Drafts twice on a server without UIDPLUS.
/// The worker takes it off that list as it says it done.
#[test]
fn a_save_filed_before_its_worker_stops_is_said_once_and_kept_nowhere() {
    let text = "From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <heard@studio.example>\r\n\r\nto finish\r\n";
    for uidplus in [true, false] {
        let (said, kept, filed) = filed_then_offline_before_heard(&format!("filed-then-offline-{uidplus}"), uidplus);
        assert_eq!(said, 1, "said done (UIDPLUS: {uidplus})");
        assert_eq!(kept, 0, "kept again (UIDPLUS: {uidplus})");
        assert_eq!(filed, [text], "in Drafts (UIDPLUS: {uidplus})");
    }
}
