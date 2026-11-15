//! Issue #58: an account that looks for new mail at a set time looks then, and only then. In
//! tests a minute of it is a tenth of a second.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, SyncStatus};

use super::*;
use crate::fake::{State, serve_tcp};

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
    let account = AccountConfig { smtp: no_smtp(), check_every: minutes, ..imap_account("a1", serve_tcp(&st)) };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", online);
    (st, pool, store)
}

/// Whether the server's log from `from` on has the Inbox looked at before any folder was listed:
/// a look at the Inbox at its time, not every folder's turn, which lists them first.
fn inbox_looked_at_alone(st: &Arc<Mutex<State>>, from: usize) -> bool {
    let st = st.lock().unwrap();
    let ahead = st.log.get(from..).unwrap_or_default();
    let listed = ahead.iter().position(|c| c.to_ascii_uppercase().starts_with("LIST"));
    let inbox = ahead.iter().position(|c| c.contains("INBOX"));
    inbox.is_some_and(|i| listed.is_none_or(|l| i < l))
}

/// The server can say at once (it has IDLE), but a set time was asked for: no IDLE, and the mail
/// comes when the time does. Timed as the server sees it: the connection left quiet that long
/// first, however long a busy computer then took.
#[test]
fn a_set_time_is_kept_instead_of_waiting_on_the_server() {
    let (st, pool, _store) = start("check-every-idle", Some(3));
    let (logged, before) = {
        let mut st = st.lock().unwrap();
        st.add("INBOX", "New one", &[]);
        (st.log.len(), st.quiet.len())
    };
    until(&pool, "the mail", inbox_changed);
    let ahead = st.lock().unwrap().quiet[before..].to_vec();
    assert!(ahead.first().is_some_and(|(_, quiet)| *quiet >= Duration::from_millis(300)), "came early: {ahead:?}");
    assert!(inbox_looked_at_alone(&st, logged), "came with every folder's turn: {ahead:?}");
    assert!(!st.lock().unwrap().log.iter().any(|c| c.eq_ignore_ascii_case("IDLE")), "{:?}", st.lock().unwrap().log);
    pool_done(pool);
}

/// One in use still looks: what it's asked to do in between doesn't put the time off. Set for
/// every 0.2 s, it looks at the Inbox before every folder's turn (1.5 s), which would find the
/// mail whatever the Inbox's time did.
#[test]
fn using_the_account_does_not_put_the_time_off() {
    let (st, mut pool, _store) = start("check-every-busy", Some(2));
    let before = st.lock().unwrap().log.len();
    st.lock().unwrap().add("INBOX", "New one", &[]);
    let began = Instant::now();
    let mut came = false;
    while !came {
        assert!(began.elapsed() < Duration::from_secs(60), "the mail never came");
        pool.submit(Job::SyncDav { account: "a1".into() });
        std::thread::sleep(Duration::from_millis(40));
        came = pool.poll().iter().any(inbox_changed);
    }
    assert!(inbox_looked_at_alone(&st, before), "not looked at before every folder's turn: {:?}", &st.lock().unwrap().log[before..]);
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
    let noop = || st.lock().unwrap().quiet[before..].iter().find(|(c, _)| c.eq_ignore_ascii_case("NOOP")).map(|(_, q)| *q);
    let mut said = events_until(&pool, "the connection kept alive", |_| noop().is_some());
    let kept_alive = noop().unwrap();
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
    let (st, pool, _store) = start("check-every-dropped", Some(3));
    st.lock().unwrap().add("INBOX", "New one", &[]);
    st.lock().unwrap().hang_up = true;
    let said = until(&pool, "the mail", inbox_changed);
    assert_eq!(pool.waits_begun("a1"), 0, "waited to try again: {said:?}");
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
    let account = AccountConfig { smtp: no_smtp(), check_every: Some(2), ..pop_account(server.port) };
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

/// Issue #104: an account that looks only when asked isn't signed in as MailCraft starts, nor
/// looks by itself; Send / Receive on it looks, and after that it doesn't look again by itself
/// (here a minute is a tenth of a second: two "hours" go by).
#[test]
fn an_account_looking_only_when_asked() {
    let st = State::new();
    let store = crate::fake::store("check-by-hand");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig { smtp: no_smtp(), check_by_hand: true, ..imap_account("a1", serve_tcp(&st)) };
    st.lock().unwrap().add("INBOX", "Waiting", &[]);
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let quiet = |pool: &mut Pool, how_long: Duration| {
        let end = Instant::now() + how_long;
        let mut said = Vec::new();
        while Instant::now() < end {
            said.extend(pool.poll());
            std::thread::sleep(Duration::from_millis(20));
        }
        said
    };
    let said = quiet(&mut pool, Duration::from_secs(2));
    assert!(st.lock().unwrap().log.is_empty(), "signed in by itself: {:?}", st.lock().unwrap().log);
    assert!(!said.iter().any(inbox_changed), "{said:?}");

    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the mail, when asked for", inbox_changed);
    // Its look done, and quiet a while: signed out, not kept awake.
    let end = Instant::now() + Duration::from_secs(30);
    while !st.lock().unwrap().log.iter().any(|c| c.eq_ignore_ascii_case("LOGOUT")) {
        assert!(Instant::now() < end, "never let go: {:?}", st.lock().unwrap().log);
        pool.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    let looked = st.lock().unwrap().log.len();
    st.lock().unwrap().add("INBOX", "Later", &[]);
    let said = quiet(&mut pool, Duration::from_secs(12));
    assert!(!said.iter().any(inbox_changed), "looked by itself: {said:?}");
    let after: Vec<String> = st.lock().unwrap().log[looked..].to_vec();
    assert!(after.is_empty(), "not signed in again by itself: {after:?}");
    pool_done(pool);
}
