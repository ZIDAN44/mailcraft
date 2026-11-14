//! Issue #58: an account that looks for new mail at a set time looks then, and only then. In
//! tests a minute of it is a tenth of a second.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Security, ServerConfig, SyncStatus};

use super::*;
use crate::fake::{State, serve_tcp};

fn until(pool: &Pool, what: &str, f: impl Fn(&Event) -> bool) -> Duration {
    let began = Instant::now();
    let mut seen = Vec::new();
    while began.elapsed() < Duration::from_secs(20) {
        if let Some(e) = pool.wait(Duration::from_millis(20)) {
            if f(&e) {
                return began.elapsed();
            }
            seen.push(e);
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

fn inbox_changed(e: &Event) -> bool {
    matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")
}

fn online(e: &Event) -> bool {
    matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })
}

fn start(name: &str, minutes: Option<u32>) -> (std::sync::Arc<std::sync::Mutex<State>>, Pool, mailcraft_store::Store) {
    let st = State::new();
    let store = crate::fake::store(name);
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.test".into(),
        username: "me@studio.test".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: serve_tcp(&st), security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        check_every: minutes,
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", online);
    (st, pool, store)
}

/// The server can say at once (it has IDLE), but a set time was asked for: no IDLE, and the mail
/// comes when the time does.
#[test]
fn a_set_time_is_kept_instead_of_waiting_on_the_server() {
    let (st, pool, _store) = start("check-every-idle", Some(3));
    st.lock().unwrap().add("INBOX", "New one", &[]);
    let took = until(&pool, "the mail", inbox_changed);
    assert!(took >= Duration::from_millis(150), "came early, after {took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(!st.lock().unwrap().log.iter().any(|c| c.eq_ignore_ascii_case("IDLE")), "{:?}", st.lock().unwrap().log);
    pool_done(pool);
}

/// One in use still looks: what it's asked to do in between doesn't put the time off.
#[test]
fn using_the_account_does_not_put_the_time_off() {
    let (st, pool, _store) = start("check-every-busy", Some(2));
    st.lock().unwrap().add("INBOX", "New one", &[]);
    let began = Instant::now();
    let mut pool = pool;
    let mut came = false;
    // Sooner than every folder is read (1.5 s), which would find it whatever the Inbox's time did.
    while began.elapsed() < Duration::from_millis(1300) && !came {
        pool.submit(Job::SyncDav { account: "a1".into() });
        std::thread::sleep(Duration::from_millis(40));
        came = pool.poll().iter().any(inbox_changed);
    }
    assert!(came, "set for every 0.2 s, and not looked in {:?}", began.elapsed());
    pool_done(pool);
}

/// Long between looks: nothing comes meanwhile, Send / Receive looks at once, and the connection
/// is kept from going quiet for so long that the server drops it: every ten minutes (a second
/// here), as the server measures it, so a busy computer slow to get there changes nothing.
#[test]
fn a_long_time_is_quiet_but_send_receive_still_looks() {
    let (st, mut pool, _store) = start("check-every-long", Some(60));
    let before = st.lock().unwrap().quiet.len();
    st.lock().unwrap().add("INBOX", "New one", &[]);
    let began = Instant::now();
    let mut said = Vec::new();
    let kept_alive = loop {
        let noop = st.lock().unwrap().quiet[before..].iter().find(|(c, _)| c.eq_ignore_ascii_case("NOOP")).map(|(_, q)| *q);
        if let Some(quiet) = noop {
            break quiet;
        }
        assert!(began.elapsed() < Duration::from_secs(60), "never kept alive: {:?}", &st.lock().unwrap().quiet[before..]);
        said.extend(pool.wait(Duration::from_millis(20)));
    };
    said.extend(pool.poll());
    assert!(!said.iter().any(inbox_changed), "looked early");
    let ahead = st.lock().unwrap().quiet[before..].to_vec();
    assert!(ahead.first().is_some_and(|(c, _)| c.eq_ignore_ascii_case("NOOP")), "{ahead:?}");
    assert!(kept_alive >= Duration::from_secs(1), "kept alive after {kept_alive:?}, not ten minutes (a second here)");
    assert!(kept_alive < Duration::from_secs(6), "kept alive after {kept_alive:?}, the set time");
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the mail, when asked for", inbox_changed);
    pool_done(pool);
}

/// Found on a real server (GreenMail lets a quiet connection go within a minute): the look at the
/// set time failed, said "the server closed the connection", and came five seconds late. It's no
/// news to anyone: a new connection, and the mail. Counted, not timed: a busy computer is slow
/// to get there either way.
#[test]
fn a_connection_the_server_dropped_meanwhile_is_no_error() {
    let (st, pool, store) = start("check-every-dropped", Some(3));
    st.lock().unwrap().add("INBOX", "New one", &[]);
    st.lock().unwrap().hang_up = true;
    let began = Instant::now();
    let mut said = Vec::new();
    while !said.last().is_some_and(inbox_changed) {
        assert!(began.elapsed() < Duration::from_secs(60), "the mail never came: {said:?}");
        said.extend(pool.wait(Duration::from_millis(20)));
    }
    assert_eq!(connect::waits::begun(&store, "a1"), 0, "waited to try again: {said:?}");
    assert!(!said.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. } | Event::Notice { .. })), "{said:?}");
    pool_done(pool);
}

/// A POP account has no way to be told: it downloads at its set time.
#[test]
fn a_pop_account_downloads_at_its_set_time() {
    let server = crate::pop::fake::serve(Vec::new());
    let store = crate::fake::store("check-every-pop");
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "p1".into(),
        email: "me@pop.example".into(),
        username: "me@pop.example".into(),
        pop: Some(ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None }),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        check_every: Some(2),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", online);
    let mail = b"From: a@x.example\r\nTo: me@pop.example\r\nSubject: Hi\r\nMessage-ID: <n@x>\r\n\r\nhi\r\n".to_vec();
    server.drop.lock().unwrap().push(("u1".into(), mail));
    until(&pool, "the download", inbox_changed);
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 1);
    pool.submit(Job::Remove { account: "p1".into() });
}

fn pool_done(mut pool: Pool) {
    pool.submit(Job::Remove { account: "a1".into() });
}
