//! Issue #232 against a real IMAP server that offers NOTIFY (RFC 5465; Dovecot does), run by
//! hand: mail another program files into a folder other than the Inbox while the account waits
//! on the Inbox is read as it comes, not at the next full look.
//!
//! ```text
//! MAILCRAFT_REAL_NOTIFY="127.0.0.1:31143 mover@local.test pw" \
//!   cargo test -p mailcraft-net real_imap_notify -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{Event, Job, SyncStatus};

use super::real_servers::{at, imap_account, until};
use crate::worker::{MemoryCredentials, Pool};

#[test]
#[ignore = "needs a real IMAP server with NOTIFY in MAILCRAFT_REAL_NOTIFY"]
fn real_imap_notify() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_NOTIFY") else {
        eprintln!("MAILCRAFT_REAL_NOTIFY isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [addr, user, pass] = p.as_slice() else { panic!("MAILCRAFT_REAL_NOTIFY: host:port user password") };
    let cfg = at(addr);
    // Another program: makes the folder, later files mail into it.
    let mut other = crate::imap::Client::connect(&cfg).unwrap();
    other.login(user, pass).unwrap();
    other.capability().unwrap();
    assert!(other.has("NOTIFY"), "the server offers NOTIFY");
    let folder = format!("Filed{}", std::process::id());
    other.create(&folder).unwrap();

    let store = crate::fake::store("real-notify");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account: imap_account("n1", user, &cfg), password: Some(Secret(pass.to_string())) });
    until(&pool, 60, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Waiting on the Inbox by now.
    std::thread::sleep(Duration::from_secs(2));
    let subject = format!("Filed by a rule {}", std::process::id());
    let raw = format!("From: a@local.test\r\nTo: {user}\r\nSubject: {subject}\r\nDate: Sat, 21 Nov 2026 10:00:00 +0000\r\n\r\nbody\r\n");
    let filed_at = Instant::now();
    other.append(&folder, "", raw.as_bytes()).unwrap();
    let has = || store.load_folder("n1", &folder).is_ok_and(|f| f.messages.iter().any(|m| m.subject == subject));
    until(&pool, 30, "the filed mail", |_| has());
    eprintln!("read {:?} after it was filed", filed_at.elapsed());
    assert!(filed_at.elapsed() < Duration::from_secs(20), "as it came, not at the next look");
    pool.submit(Job::Remove { account: "n1".into() });
    let _ = other.delete(&folder);
}
