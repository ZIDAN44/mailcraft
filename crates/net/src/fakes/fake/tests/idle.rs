//! Mail that arrives between the Inbox's sync and IDLE shows at once, not after IDLE's ten
//! minutes; and, where the server offers NOTIFY, mail filed into another folder while IDLE waits
//! (issue #232).

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
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: imap_account(port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(st.lock().unwrap().deliver_on.is_none(), "delivered during the sync");
    let has_late = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Late");
    events_until(&pool, "the late mail", |_| has_late());
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Issue #232: with NOTIFY on, the server's `STATUS` for another folder ends IDLE and names it;
/// the selected folder isn't said to have changed. A server without it isn't asked; one that
/// refuses it is asked once.
#[test]
fn notify_names_the_other_folders_that_changed() {
    let st = State::new();
    st.lock().unwrap().caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE NOTIFY";
    // Over a socket, as IDLE waits on one.
    let cfg = mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port: serve_tcp(&st), security: mailcraft_model::Security::None };
    let mut c = crate::imap::Client::connect(&cfg).unwrap();
    c.login("riley", "pw").unwrap();
    assert!(c.notify().unwrap() && c.notify().unwrap(), "on, asked once");
    c.select("INBOX", false).unwrap();
    st.lock().unwrap().add("Archive", "Filed by a rule", &[]);
    let started = std::time::Instant::now();
    let changed = c.idle(std::time::Duration::from_secs(20), || false).unwrap();
    assert!(!changed && started.elapsed() < std::time::Duration::from_secs(10), "ended by the other folder, not the Inbox");
    assert_eq!(c.take_notified(), ["Archive"]);
    assert!(c.take_notified().is_empty(), "said once");

    // Without NOTIFY: not asked, nothing told.
    let plain = State::new();
    let mut c = login(&plain);
    assert!(!c.notify().unwrap());
    assert!(!plain.lock().unwrap().log.iter().any(|l| l.contains("NOTIFY")));
    // Said to have it but refusing it (BAD): off, not a failure, and not asked again.
    let refusing = State::new();
    {
        let mut r = refusing.lock().unwrap();
        r.caps = "IMAP4rev1 IDLE NOTIFY";
        r.refuse_notify = true;
    }
    let mut c = login(&refusing);
    c.capability().unwrap();
    let asked = |st: &std::sync::Arc<std::sync::Mutex<State>>| st.lock().unwrap().log.iter().filter(|l| l.starts_with("NOTIFY")).count();
    assert!(!c.notify().unwrap() && !c.notify().unwrap());
    assert_eq!(asked(&refusing), 1);
}

/// Through the worker: mail a server's rule files into another folder while the account waits on
/// the Inbox is read as it comes, not at the next full look.
#[test]
fn mail_filed_into_another_folder_shows_at_once() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA IDLE NOTIFY";
        s.add("INBOX", "First", &[]);
    }
    let port = serve_tcp(&st);
    let store = store("notify-filed");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: imap_account(port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    crate::waiting::eventually("IDLE with NOTIFY on", || {
        let log = st.lock().unwrap().log.clone();
        log.iter().any(|l| l.starts_with("NOTIFY SET")) && log.last().is_some_and(|l| l == "IDLE")
    });
    st.lock().unwrap().add("Archive", "Filed by a rule", &[]);
    let filed = || store.load_folder("a1", "Archive").unwrap().messages.iter().any(|m| m.subject == "Filed by a rule");
    events_until(&pool, "the filed mail", |_| filed());
    pool.submit(Job::Remove { account: "a1".into() });
}
