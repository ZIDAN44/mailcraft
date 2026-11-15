//! Changes to a server account's mail, made in the cache on the pool's own thread: off the
//! caller's, in the order they were made, and dropped for an account removed.

use super::*;

/// A read mark (or any change to a server account's mail) is made in the cache on the pool's
/// own thread, not the caller's (the window's: on a folder of ten thousand messages it read and
/// wrote the whole list there, every Down that opened one); said changed once it's on disk.
#[test]
fn a_change_is_made_off_the_callers_thread() {
    let s = indexed_mail(3);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    // The thread each event is said on: the change's own are said where it's made.
    let said_on: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let on = said_on.clone();
    pool.set_waker(Arc::new(move || on.lock().unwrap().push(std::thread::current().name().map(str::to_string))));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 2 };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    until(&pool, "the Inbox changed", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2 && m.flags.seen));
    assert_eq!(s.load_ops("a1").unwrap().len(), 1, "queued for the server");
    let me = std::thread::current().name().map(str::to_string);
    // The waker runs just after the event is sent: it may come a moment after it.
    let made_there = || said_on.lock().unwrap().iter().any(|t| t.as_deref() == Some("mail-edits"));
    eventually("the change's thread said", made_there);
    let on = said_on.lock().unwrap().clone();
    assert!(on.iter().any(|t| t.as_deref() == Some("mail-edits")), "{on:?}");
    assert!(!on.contains(&me), "made on the caller's thread: {on:?}");
}

/// Changes made in a row are made in that order, and are all on disk once the pool has shut
/// down (the app closing right after a read mark keeps it).
#[test]
fn changes_keep_their_order_and_outlive_the_pool() {
    let s = indexed_mail(3);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 1 };
    for i in 0..41 {
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: i % 2 == 0 } });
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetKeyword { keys: vec![key.clone()], keyword: "$label1".into(), on: i % 2 == 1 } });
    }
    // As the app closes: the changes on disk before the data files are sealed.
    pool.shutdown();
    let st = s.load_folder("a1", "INBOX").unwrap();
    let m = st.messages.iter().find(|m| m.key.uid == 1).unwrap();
    assert!(m.flags.seen, "the last of them, read, wins");
    assert!(!m.flags.keywords.iter().any(|k| k == "$label1"));
    assert_eq!(s.load_ops("a1").unwrap().len(), 82);
}

/// An account removed while changes to its mail wait on the pool's thread: its files go at
/// once, on the window's thread, and the changes queued before aren't made after, which wrote
/// its folder and its queue for the server back.
#[test]
fn changes_waiting_for_a_removed_account_are_dropped() {
    let s = indexed_mail(2000);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    for uid in 1..=2000 {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    }
    s.delete_account("a1").unwrap();
    pool.submit(Job::Remove { account: "a1".into() });
    pool.shutdown();
    assert_eq!(s.account_size("a1"), 0, "nothing of it written back");
    assert!(s.load_ops("a1").unwrap_or_default().is_empty());
    // Added again under the same id: its changes are made.
    fill(&s, 3);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 2 };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    pool.shutdown();
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2 && m.flags.seen));
}

/// A change the cache can't take is said, and its folder said changed all the same: the window
/// shows it already, and reads the folder again to show what's really kept.
#[test]
fn a_change_the_cache_cant_take_is_said_and_read_again() {
    let s = Store::failing();
    fill(&s, 2);
    s.fail(Some("state.json"), None);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 1 };
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true } });
    let seen = until(&pool, "the Inbox changed", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    assert!(matches!(seen.as_slice(), [.., Event::Notice { .. }, Event::FolderChanged { .. }]), "{seen:?}");
    s.fail(None, None);
    assert!(s.load_ops("a1").unwrap().is_empty(), "nothing queued that the cache doesn't have");
}
