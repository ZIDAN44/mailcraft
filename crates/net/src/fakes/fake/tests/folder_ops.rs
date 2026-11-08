//! Folders renamed or deleted here while the account syncs: the sync goes on without a word.

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
