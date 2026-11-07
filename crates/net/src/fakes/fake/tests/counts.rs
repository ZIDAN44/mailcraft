//! Folder counts said as a full sync goes, once a second at most, and kept as folders are made
//! or renamed here.

use super::*;

/// The Inbox's counts said last, as a full sync goes (`make`: a folder made while it's between
/// two folders, within a second of the Inbox's counts changing).
fn inbox_counts_said(make: bool) -> Option<(u32, u32)> {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 0..3 {
            s.add("INBOX", &format!("m{n}"), &[]);
        }
        s.stall_on = Some(("SELECT INBOX".into(), Duration::from_millis(600)));
    }
    let port = serve_tcp(&st);
    let store = store(if make { "counts-folder-made" } else { "counts-nothing-made" });
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "syncing the Inbox", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "INBOX"));
    if make {
        pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Plans".into() } });
    }
    let mut seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    std::thread::sleep(Duration::from_millis(300));
    seen.extend(pool.poll());
    let said = seen
        .iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => folders.iter().find(|f| f.path == "INBOX").map(|f| (f.unread, f.total)),
            _ => None,
        })
        .next_back();
    let kept = store.load_folders("a1").unwrap_or_default().into_iter().find(|f| f.path == "INBOX").map(|f| (f.unread, f.total));
    assert_eq!(said, kept, "said as kept");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
    said
}

/// Found in review: a folder made while a full sync was between two folders read the account's
/// folder list again from disk, which hadn't the counts not said yet: they were lost, and the
/// Inbox stayed empty. They're written first.
#[test]
fn a_folder_made_mid_sync_keeps_the_counts_not_said_yet() {
    assert_eq!(inbox_counts_said(false), Some((3, 3)));
    assert_eq!(inbox_counts_said(true), Some((3, 3)));
}

/// Found in review: a folder renamed (and another made) while a full sync was between two
/// folders was shown undone. The counts not said yet were written with the worker's folder list
/// from before, over the renamed one already on disk, and said: the old name came back, and a
/// view of the renamed folder fell back to the Inbox. The counts go into the list as it is on
/// disk, and no list said after the renamed one has the old name.
#[test]
fn a_folder_renamed_mid_sync_is_never_shown_undone() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 0..3 {
            s.add("INBOX", &format!("m{n}"), &[]);
        }
        s.stall_on = Some(("SELECT \"INBOX\"".into(), Duration::from_millis(600)));
    }
    let port = serve_tcp(&st);
    let store = store("counts-folder-renamed");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let mut seen =
        until(&pool, "syncing the Inbox", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "INBOX"));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Archive".into(), to: "Kept".into() } });
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Plans".into() } });
    seen.extend(until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    std::thread::sleep(Duration::from_millis(300));
    seen.extend(pool.poll());
    let lists: Vec<Vec<String>> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => Some(folders.iter().map(|f| f.path.clone()).collect()),
            _ => None,
        })
        .collect();
    let renamed = lists.iter().position(|l| l.iter().any(|p| p == "Kept")).expect("the renamed folder said");
    for later in &lists[renamed..] {
        assert!(!later.iter().any(|p| p == "Archive"), "the old name back after the new one: {lists:?}");
        assert!(later.iter().any(|p| p == "Kept"), "the new name gone after it was said: {lists:?}");
    }
    let made = lists.iter().position(|l| l.iter().any(|p| p == "Plans")).expect("the folder made said");
    assert!(lists[made..].iter().all(|l| l.iter().any(|p| p == "Plans")), "the folder made gone after it was said: {lists:?}");
    let kept = store.load_folders("a1").unwrap_or_default();
    assert!(kept.iter().any(|f| f.path == "Kept") && kept.iter().any(|f| f.path == "Plans"), "{kept:?}");
    assert!(!kept.iter().any(|f| f.path == "Archive"), "{kept:?}");
    let inbox = |l: &[mailcraft_model::Folder]| l.iter().find(|f| f.path == "INBOX").map(|f| (f.unread, f.total));
    assert_eq!(inbox(&kept), Some((3, 3)), "the Inbox's counts kept");
    let said = seen.iter().rev().find_map(|e| match e {
        Event::Folders { folders, .. } => inbox(folders),
        _ => None,
    });
    assert_eq!(said, Some((3, 3)), "the Inbox's counts said");
    // The Inbox read here (the window writes its count), then another folder's mail changes: the
    // pool says the Inbox's count as it counted it (the session takes it as no news), and the
    // window's count on disk stays.
    store.update_folder_counts("a1", &[("INBOX".into(), 0, 3)]).unwrap();
    st.lock().unwrap().add("Sent", "copy", &["\\Seen"]);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "Sent".into() });
    let seen = until(&pool, "Sent said", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Sent" && f.total == 1)));
    let said = seen.iter().rev().find_map(|e| match e {
        Event::Folders { folders, .. } => inbox(folders),
        _ => None,
    });
    assert_eq!(said, Some((3, 3)), "the pool's count said");
    assert_eq!(inbox(&store.load_folders("a1").unwrap()), Some((0, 3)), "the window's count kept");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: a folder renamed here while a full sync was on another folder, its own counts
/// synced and not said yet, lost them: they were written under the old name, which the list on
/// disk had no more, and the renamed folder took the list's counts from before its sync (none).
/// It showed no mail, unread or not, until the rename was made on the server, after the
/// calendars and contacts. The counts follow the folder to its new name, and the change made
/// here during the last folder is sent before the calendars.
#[test]
fn a_folder_renamed_mid_sync_keeps_its_counts() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("Archive", "kept", &["\\Seen"]);
        s.add("Archive", "kept", &[]);
        s.stall_on = Some(("SELECT \"Projects".into(), Duration::from_millis(1500)));
    }
    let port = serve_tcp(&st);
    let store = store("counts-renamed-own");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let mut seen = until(
        &pool,
        "syncing Projects",
        |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder.starts_with("Projects")),
    );
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Archive".into(), to: "Kept".into() } });
    seen.extend(until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    let end = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < end {
        seen.extend(pool.wait(Duration::from_millis(50)));
    }
    let kept = |l: &[mailcraft_model::Folder]| l.iter().find(|f| f.path == "Kept").map(|f| (f.unread, f.total));
    let said: Vec<(u32, u32)> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => kept(folders),
            _ => None,
        })
        .collect();
    assert_eq!(store.load_folder("a1", "Kept").map(|f| f.messages.len()).unwrap_or(0), 2, "the renamed folder's mail");
    // Said first as the folder was renamed on disk, before its counts got there; then by the
    // worker, each time with them.
    assert!(said.len() >= 2 && said[1..].iter().all(|c| *c == (1, 2)), "said: {said:?}");
    assert_eq!(kept(&store.load_folders("a1").unwrap_or_default()), Some((1, 2)), "kept");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: a folder renamed here while it was being read: the read, ending after its
/// cache had gone with it to the new name, wrote it again under the old one, left there for good
/// beside the renamed folder. Nothing stays under the old name, and the renamed folder has its
/// mail.
#[test]
fn a_folder_renamed_as_it_is_read_leaves_no_cache_behind() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("Archive", "kept", &["\\Seen"]);
        s.add("Archive", "kept", &[]);
    }
    let port = serve_tcp(&st);
    let store = store("renamed-as-read");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let cached = |path: &str| store.load_folder("a1", path).map(|f| f.messages.len()).unwrap_or(0);
    assert_eq!(cached("Archive"), 2);
    // Read again, slowly, with new mail in it: renamed as it's read.
    {
        let mut s = st.lock().unwrap();
        s.add("Archive", "new", &[]);
        s.stall_on = Some(("SELECT \"Archive\"".into(), Duration::from_millis(1000)));
    }
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "Archive".into() });
    until(&pool, "reading Archive", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "Archive"));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameFolder { from: "Archive".into(), to: "Kept".into() } });
    let end = Instant::now() + Duration::from_secs(10);
    while st.lock().unwrap().mailbox("Kept").is_none() || cached("Kept") < 3 {
        assert!(Instant::now() < end, "never renamed on the server and read: {} in Kept", cached("Kept"));
        let _ = pool.wait(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = pool.poll();
    assert_eq!(cached("Archive"), 0, "a cache left under the old name");
    assert_eq!(cached("Kept"), 3, "the renamed folder's mail");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: a folder made here had the pool read the account's folder list again with its
/// own counts, and write that list over the one on disk: the Inbox read here (the window writes
/// its count) came back unread at the next start, until the Inbox was synced again, and offline
/// for good. Said after the list on disk (the pool says that one as it makes the change there),
/// the count from before was news, and the window took it too. The list on disk keeps the
/// window's counts, and the pool's lists say them from then on.
#[test]
fn a_folder_made_here_keeps_the_counts_of_mail_read_here() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 0..3 {
            s.add("INBOX", &format!("m{n}"), &[]);
        }
    }
    let port = serve_tcp(&st);
    let store = store("counts-read-then-made");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let inbox = |l: &[mailcraft_model::Folder]| l.iter().find(|f| f.path == "INBOX").map(|f| (f.unread, f.total));
    assert_eq!(inbox(&store.load_folders("a1").unwrap()), Some((3, 3)));
    std::thread::sleep(Duration::from_millis(300));
    let _ = pool.poll();
    // One read here (the window writes the Inbox's count), then a folder made.
    store.update_folder_counts("a1", &[("INBOX".into(), 2, 3)]).unwrap();
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Plans".into() } });
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(10);
    while st.lock().unwrap().mailbox("Plans").is_none() || !store.load_ops("a1").unwrap_or_default().is_empty() {
        assert!(Instant::now() < end, "Plans never made on the server");
        seen.extend(pool.wait(Duration::from_millis(50)));
    }
    std::thread::sleep(Duration::from_millis(300));
    seen.extend(pool.poll());
    let kept = store.load_folders("a1").unwrap();
    assert!(kept.iter().any(|f| f.path == "Plans"), "{kept:?}");
    assert_eq!(inbox(&kept), Some((2, 3)), "the window's count kept");
    let said: Vec<(u32, u32)> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => inbox(folders),
            _ => None,
        })
        .collect();
    assert!(said.len() >= 2 && said.iter().all(|c| *c == (2, 3)), "said: {said:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
