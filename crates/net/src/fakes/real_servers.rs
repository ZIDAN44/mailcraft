//! Against real servers on this computer (Dovecot, GreenMail in Docker; never a real account),
//! run by hand: what the in-memory servers can't prove about a real one's answers.
//!
//! ```text
//! MAILCRAFT_REAL_IMAP="127.0.0.1:1143 user password" \
//! MAILCRAFT_REAL_SMTP="127.0.0.1:3025 user password 127.0.0.1:3143" \
//!   cargo test -p mailcraft-net real_ -- --ignored --nocapture
//! ```
//!
//! The IMAP server's Inbox holds `Message 1`…`Message 3000` (every hundredth's body says
//! "needle"), and one message with a 30 MB attachment, newest.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Job, Security, ServerConfig, SyncStatus};

use crate::worker::{MemoryCredentials, Pool};

/// `host:port user password [more…]` from the environment, or `None` (the test is skipped).
fn server(var: &str) -> Option<(ServerConfig, String, String, Vec<String>)> {
    let v = std::env::var(var).ok()?;
    let mut parts = v.split_whitespace().map(str::to_string);
    let (addr, user, pass) = (parts.next()?, parts.next()?, parts.next()?);
    let (host, port) = addr.rsplit_once(':')?;
    let cfg = ServerConfig { host: host.into(), port: port.parse().ok()?, security: Security::None };
    Some((cfg, user, pass, parts.collect()))
}

/// Events until `f` says yes (or `secs` pass): all of them, the last the one wanted.
fn until(pool: &Pool, secs: u64, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; last events: {:?}", seen.iter().rev().take(5).collect::<Vec<_>>());
}

/// A real IMAP server through the worker: a window of 25, Show more over QRESYNC, a search of
/// the whole Inbox a range at a time (its finds as they come), a search stopped, a 30 MB message
/// read whole past the per-command caps, and mail put in by another client noticed at once.
#[test]
#[ignore]
fn real_imap_server() {
    let Some((imap, user, pass, _)) = server("MAILCRAFT_REAL_IMAP") else {
        eprintln!("MAILCRAFT_REAL_IMAP isn't set: skipped");
        return;
    };
    let store = crate::fake::store("real-imap");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 25, offline: 0 });
    let account = AccountConfig {
        id: "r1".into(),
        email: user.clone(),
        username: user.clone(),
        imap: imap.clone(),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    let began = Instant::now();
    pool.submit(Job::Start { account, password: Some(Secret(pass.clone())) });
    until(&pool, 120, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let kept = || store.load_folder("r1", "INBOX").map(|f| f.messages.len()).unwrap_or(0);
    eprintln!("online in {:?}, {} kept", began.elapsed(), kept());
    assert_eq!(kept(), 25);

    // Show more: 25 more, through the QRESYNC sync (Dovecot has it).
    pool.submit(Job::MoreMail { account: "r1".into(), folder: "INBOX".into(), more: 25 });
    let end = Instant::now() + Duration::from_secs(60);
    while kept() < 50 {
        assert!(Instant::now() < end, "Show more brought {} of 50", kept());
        let _ = pool.wait(Duration::from_millis(100));
    }

    // Search the server: newest first, its finds as they come.
    let began = Instant::now();
    pool.submit(Job::SearchServer { account: "r1".into(), folders: vec!["INBOX".into()], query: "needle".into(), search: 1 });
    let seen = until(&pool, 120, "the search's end", |e| matches!(e, Event::ServerSearched { done: true, .. }));
    let batches: Vec<usize> =
        seen.iter().filter_map(|e| if let Event::ServerSearched { keys, done: false, .. } = e { Some(keys.len()) } else { None }).collect();
    let found: usize = batches.iter().sum();
    eprintln!("search: {found} found in {:?}, in batches {batches:?}", began.elapsed());
    assert_eq!(found, 30);
    assert!(batches.len() > 1, "shown as found, not all at the end");

    // A search stopped before it starts its next range answers that it's done, with no error.
    pool.submit(Job::SearchServer { account: "r1".into(), folders: vec!["INBOX".into()], query: "nowhere-at-all".into(), search: 2 });
    pool.submit(Job::StopSearch);
    until(&pool, 60, "the stopped search's end", |e| matches!(e, Event::ServerSearched { done: true, error: None, .. }));

    // The 30 MB message, whole: the per-command caps leave real mail alone.
    let big =
        store.load_folder("r1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == "Big attachment").expect("the big message is kept");
    let began = Instant::now();
    pool.submit(Job::FetchBody { key: big.key.clone() });
    let seen = until(&pool, 180, "the big message", |e| matches!(e, Event::Body { key, .. } | Event::BodyFailed { key, .. } if *key == big.key));
    match seen.last() {
        Some(Event::Body { raw, .. }) => {
            eprintln!("big message: {} bytes in {:?}", raw.len(), began.elapsed());
            assert!(raw.len() > 40_000_000, "{}", raw.len());
        }
        other => panic!("the big message didn't come: {other:?}"),
    }

    // Another client puts a message in the Inbox: noticed without waiting for the next sync.
    let mut other = crate::imap::Client::connect(&imap).unwrap();
    other.login(&user, &pass).unwrap();
    let subject = format!("Arrived {}", std::process::id());
    let raw =
        format!("From: x@studio.test\r\nTo: {user}\r\nSubject: {subject}\r\nMessage-ID: <late{}@studio.test>\r\n\r\nhi\r\n", std::process::id());
    let began = Instant::now();
    other.append("INBOX", "", raw.as_bytes()).unwrap();
    let end = Instant::now() + Duration::from_secs(45);
    while !store.load_folder("r1", "INBOX").unwrap().messages.iter().any(|m| m.subject == subject) {
        assert!(Instant::now() < end, "the new message wasn't noticed in 45 s");
        let _ = pool.wait(Duration::from_millis(100));
    }
    eprintln!("new mail noticed in {:?}", began.elapsed());
    pool.submit(Job::Remove { account: "r1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// A real SMTP server: a message sent, then found in the recipient's mailbox over IMAP; a
/// refused sign-in, where the server refuses one, is a sign-in failure, not a refusal of the
/// message.
#[test]
#[ignore]
fn real_smtp_server() {
    let Some((smtp, user, pass, more)) = server("MAILCRAFT_REAL_SMTP") else {
        eprintln!("MAILCRAFT_REAL_SMTP isn't set: skipped");
        return;
    };
    let to = if user.contains('@') { user.clone() } else { format!("{user}@gm.test") };
    let subject = format!("Sent {}", std::process::id());
    let raw = format!("From: {to}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <sent{}@studio.test>\r\n\r\nhello\r\n", std::process::id());
    crate::smtp::send(&smtp, &user, &pass, &to, std::slice::from_ref(&to), raw.as_bytes()).unwrap();
    // A refused sign-in is a sign-in failure, not a refusal of the message (some test servers,
    // GreenMail among them, take any password: then there's nothing to see).
    let copies = match crate::smtp::send(&smtp, &user, "wrong-password", &to, std::slice::from_ref(&to), raw.as_bytes()) {
        Ok(()) => {
            eprintln!("this server takes any password: a refused sign-in isn't tried");
            2
        }
        Err(e) => {
            eprintln!("wrong password: {e}");
            assert!(matches!(e, crate::NetError::Auth(_)), "{e:?}");
            1
        }
    };
    // Read back over the same server's IMAP, when given.
    if let Some((host, port)) = more.first().and_then(|a| a.rsplit_once(':')) {
        let cfg = ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None };
        let mut c = crate::imap::Client::connect(&cfg).unwrap();
        c.login(&user, &pass).unwrap();
        let info = c.select("INBOX", true).unwrap();
        let found = c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap();
        eprintln!("inbox has {}, the sent one: {found:?}", info.exists);
        assert_eq!(found.len(), copies);
    }
}
