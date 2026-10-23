//! Mail that arrives between the Inbox's sync and IDLE shows at once, not after IDLE's ten
//! minutes.

use super::*;

/// An `EXISTS` with a new count in any command's answer is news; the next `SELECT` starts over.
#[test]
fn a_command_saying_the_folder_grew_is_news() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "one", &[]);
    let mut c = login(&st);
    c.select("INBOX", false).unwrap();
    c.noop().unwrap();
    assert!(!c.take_news(), "the same count is no news");
    st.lock().unwrap().add("INBOX", "two", &[]);
    c.noop().unwrap();
    assert!(c.take_news());
    assert!(!c.take_news(), "said once");
    st.lock().unwrap().add("INBOX", "three", &[]);
    c.noop().unwrap();
    let info = c.select("INBOX", false).unwrap();
    assert!(!c.take_news(), "a SELECT looks at everything");
    assert_eq!(c.seen("INBOX"), Some(&info));
}

#[test]
fn mail_arriving_after_the_inbox_sync_shows_before_idle() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE";
        s.add("INBOX", "First", &[]);
        // Delivered once the Inbox is synced, while the sync is on another folder.
        s.deliver_on = Some(("SELECT \"Archive\"".into(), "Late".into()));
    }
    let port = serve_tcp(&st);
    let store = store("late-mail");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { port, security: Security::None, host: "127.0.0.1".into() },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(st.lock().unwrap().deliver_on.is_none(), "delivered during the sync");
    let has_late = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Late");
    let end = Instant::now() + Duration::from_secs(10);
    while !has_late() {
        assert!(Instant::now() < end, "the late mail waited for IDLE to end");
        let _ = pool.wait(Duration::from_millis(100));
    }
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
