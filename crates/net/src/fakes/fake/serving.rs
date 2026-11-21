//! The fake served over TCP for end-to-end tests, the stores those tests use, and a scenario run
//! the same against every kind of server.

use super::*;

/// Serve the fake over TCP on localhost (one thread per connection), for end-to-end tests of
/// the worker pool. Returns the port.
pub fn serve_tcp(st: &Arc<Mutex<State>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    serve_tcp_on(listener, st);
    port
}

/// Serve the fake on a listener of the caller's (one thread per connection).
pub(crate) fn serve_tcp_on(listener: std::net::TcpListener, st: &Arc<Mutex<State>>) {
    let st = st.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let st = st.clone();
            std::thread::spawn(move || {
                let mut fake = Fake::connect(&st);
                let mut sock = sock;
                // Woken now and then while the client says nothing, for what the server tells
                // unasked (NOTIFY during IDLE).
                let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(100)));
                let mut buf = [0u8; 8192];
                loop {
                    fake.tell_notified();
                    let out: Vec<u8> = fake.out.drain(..).collect();
                    if !out.is_empty() && sock.write_all(&out).is_err() {
                        return;
                    }
                    match sock.read(&mut buf) {
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if fake.hang_up_next || std::mem::take(&mut st.lock().unwrap().hang_up) {
                                return;
                            }
                            let _ = fake.write(&buf[..n]);
                        }
                    }
                }
            });
        }
    });
}

/// A store in a temporary folder of its own (emptied first, removed when the test's thread ends:
/// [`crate::data_files::scratch::scratch`]), for the tests of every fake and of the worker.
pub(crate) fn store(name: &str) -> Store {
    Store::open(crate::data_files::scratch::scratch(&format!("fake-{name}"))).unwrap()
}

/// Gap #7 through the worker, the same for JMAP, Graph and EWS accounts: a server whose Inbox
/// has `Message 1`…`Message 25` (bodies `tag1`…, the first five unread) and a window of ten. A
/// search of the server brings in what it finds past the window and keeps it across syncs while
/// the server has it, its flags followed; words, fields and the read state are asked; Show more
/// keeps more. `server(n, true)` marks `Message n` read on the server, `server(n, false)` deletes it.
pub(crate) fn search_the_server_past_the_window(name: &str, account: mailcraft_model::AccountConfig, secret: &str, server: &dyn Fn(u32, bool)) {
    use mailcraft_model::job::Secret;
    use mailcraft_model::{Event, Job, SyncStatus};

    use crate::waiting::{events_until, until};
    let store = store(name);
    crate::fakes::fake_dav::looked_for_today(&store, &account.id);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let online = |e: &Event| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
    let subjects = || {
        let mut v: Vec<String> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
        v.sort();
        v
    };
    let has = |s: &str| subjects().iter().any(|x| x == s);
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: q.into(), search: 1 });
        // What's found comes as it's found, then the end.
        let seen = until(pool, "searched", |e| matches!(e, Event::ServerSearched { done: true, .. }));
        match seen.last() {
            Some(Event::ServerSearched { error: None, .. }) => {
                seen.iter().map(|e| if let Event::ServerSearched { keys, .. } = e { keys.len() } else { 0 }).sum::<usize>()
            }
            other => panic!("{q}: {other:?}"),
        }
    };
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
    pool.submit(Job::Start { account, password: Some(Secret(secret.into())) });
    until(&pool, "online", online);
    events_until(&pool, "ten in the Inbox", |_| subjects().len() >= 10);
    assert!(has("Message 16") && !has("Message 15") && !has("Message 3"), "{:?}", subjects());

    // Words: found past the window, and kept by the next sync.
    assert_eq!(search(&mut pool, "tag3"), 1);
    assert!(has("Message 3"), "{:?}", subjects());
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", online);
    assert!(has("Message 3"), "kept past the window: {:?}", subjects());
    // A field and a word.
    assert_eq!(search(&mut pool, "subject:message tag7"), 1);
    assert!(has("Message 7"));
    // A date alone (issue #179): before the 4th, a day wider, is the first four days' mail.
    assert_eq!(search(&mut pool, "before:2024-05-04"), 4);
    assert!(has("Message 1") && has("Message 4"), "{:?}", subjects());

    // Show more: five more than kept (ten and the one found).
    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
    events_until(&pool, "more shown", |_| has("Message 10"));

    // The read state alone; finds add up, as IMAP keeps what it fetches (another search doesn't
    // take Message 3 and 7 away).
    assert_eq!(search(&mut pool, "is:unread"), 5);
    assert_eq!(search(&mut pool, "nothing-like-this"), 0);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", online);
    assert!(has("Message 1") && has("Message 5") && has("Message 3") && has("Message 7"), "{:?}", subjects());

    // On the server, Message 3 is deleted and Message 2 read: the next sync follows both.
    server(3, false);
    server(2, true);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    // (A push stream may say "online" before this sync ends: wait for what it does.)
    let seen = |s: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == s).map(|m| m.flags.seen);
    events_until(&pool, "the server's deletion and flag followed", |_| !has("Message 3") && seen("Message 2") == Some(true));
    assert!(has("Message 7"), "{:?}", subjects());
    assert_eq!(seen("Message 1"), Some(false));
    pool.submit(Job::Remove { account: "a1".into() });
}

pub(super) fn login(st: &Arc<Mutex<State>>) -> Client<Fake> {
    let mut c = Client::from_stream(Fake::connect(st)).unwrap();
    c.login("me@studio.example", "pw").unwrap();
    c
}
