//! Folders renamed or deleted here while the account syncs, or while it's offline: the sync
//! goes on without a word, and the mail cached in them stays.

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

/// What a folder change made here while a full sync is on the Inbox, before the folder's turn,
/// leaves said: the notices, and the server's folders after.
fn mid_sync(op: Op, name: &str) -> (Vec<String>, Vec<String>) {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("Archive", "kept", &[]);
        s.stall_on = Some(("SELECT \"INBOX\"".into(), Duration::from_millis(800)));
    }
    let port = serve_tcp(&st);
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    let mut seen =
        until(&pool, "syncing the Inbox", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "INBOX"));
    pool.submit(Job::Apply { account: "a1".into(), op });
    seen.extend(until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    std::thread::sleep(Duration::from_millis(500));
    seen.extend(pool.poll());
    let notices = seen
        .iter()
        .filter_map(|e| match e {
            Event::Notice { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    let folders = st.lock().unwrap().boxes.iter().map(|b| b.name.clone()).collect();
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    (notices, folders)
}

/// Found in review: a folder renamed or deleted here while a full sync was still on an earlier
/// folder said "the server refused: no such mailbox". The sync went on to the folder by the name
/// it had when the sync began, after the change had been made on the server. A folder no longer
/// in the account's list is passed over.
#[test]
fn a_folder_renamed_or_deleted_mid_sync_says_nothing() {
    let (notices, folders) = mid_sync(Op::RenameFolder { from: "Archive".into(), to: "Kept".into() }, "mid-sync-renamed");
    assert!(notices.is_empty(), "{notices:?}");
    assert!(folders.iter().any(|f| f == "Kept") && !folders.iter().any(|f| f == "Archive"), "{folders:?}");
    let (notices, folders) = mid_sync(Op::DeleteFolder { path: "Archive".into() }, "mid-sync-deleted");
    assert!(notices.is_empty(), "{notices:?}");
    assert!(!folders.iter().any(|f| f == "Archive"), "{folders:?}");
}

/// Found in review: a folder renamed twice while offline (Archive to Plans, Plans to Done) had
/// its mail downloaded again once the renames were sent, and said "no such mailbox". Listed
/// after the first, the server had no Done yet, so its cache was cleared as a folder gone from
/// the server, and the folder in between was synced though it was gone. A folder the list on
/// disk still has keeps its cache while folder changes are still to be sent, and only folders
/// the server has are synced after them. Then a folder another program deleted on the server
/// meanwhile kept its cache for good: it's cleared once the changes are sent.
#[test]
fn a_folder_renamed_twice_offline_keeps_its_mail() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("Archive", "kept", &[]);
        s.boxes.push(Mailbox { name: "Other".into(), attr: "", validity: 400, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
        s.add("Other", "elsewhere", &[]);
    }
    let port = serve_tcp(&st);
    let store = store("renamed-twice-offline");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(port), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let cached = |path: &str| store.load_folder("a1", path).map(|f| f.messages.len()).unwrap_or(0);
    assert_eq!((cached("Archive"), cached("Other")), (1, 1));
    pool.submit(Job::WorkOffline { on: true });
    until(&pool, "offline", |e| matches!(e, Event::Status { status: SyncStatus::Offline, .. }));
    st.lock().unwrap().boxes.retain(|b| b.name != "Other");
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Archive".into(), to: "Plans".into() } });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Plans".into(), to: "Done".into() } });
    // Both in the cache and waiting for the server (the second is queued just after the cache
    // has it).
    let end = Instant::now() + Duration::from_secs(20);
    while cached("Done") == 0 || store.load_ops("a1").unwrap_or_default().len() < 2 {
        assert!(Instant::now() < end, "the renames never reached the cache");
        std::thread::sleep(Duration::from_millis(50));
    }
    st.lock().unwrap().log.clear();
    pool.submit(Job::WorkOffline { on: false });
    let mut seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    std::thread::sleep(Duration::from_millis(500));
    seen.extend(pool.poll());
    let notices: Vec<&String> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Notice { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert!(notices.is_empty(), "{notices:?}");
    let names: Vec<String> = st.lock().unwrap().boxes.iter().map(|b| b.name.clone()).collect();
    assert!(names.iter().any(|n| n == "Done") && !names.iter().any(|n| n == "Archive" || n == "Plans"), "{names:?}");
    let log = st.lock().unwrap().log.clone();
    assert!(!log.iter().any(|l| l.contains("BODY")), "mail downloaded again: {log:?}");
    assert_eq!(cached("Done"), 1);
    assert_eq!(cached("Other"), 0, "the folder deleted on the server keeps its cache");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
