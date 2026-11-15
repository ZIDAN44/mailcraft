//! Against a real POP3 server on this computer (GreenMail in Docker, any password; never a real
//! account), run by hand: what the in-memory server can't prove about a real one's answers.
//!
//! ```text
//! MAILCRAFT_REAL_POP="127.0.0.1:23110 127.0.0.1:23025 1200" \
//!   cargo test -p mailcraft-net real_pop -- --ignored --nocapture
//! ```
//!
//! The POP server, then the SMTP server that delivers to it, then how many messages to send (a
//! mailbox of the test's own, new each run).

use std::time::Instant;

use mailcraft_model::AccountConfig;
use mailcraft_store::Store;

use super::real_servers::at;
use crate::pop::{self, Keep};

/// Mail sent to a new mailbox comes down once, every message, and goes from the server when the
/// account says so.
#[test]
#[ignore = "needs real POP and SMTP servers in MAILCRAFT_REAL_POP"]
fn real_pop_server() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_POP") else {
        eprintln!("MAILCRAFT_REAL_POP isn't set: skipped");
        return;
    };
    let mut parts = v.split_whitespace();
    let mut next = || at(parts.next().expect("MAILCRAFT_REAL_POP: pop smtp [count]"));
    let (pop_cfg, smtp) = (next(), next());
    let count: usize = parts.next().map_or(30, |n| n.parse().unwrap());
    let run = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let me = format!("pop{run}@gm.test");
    let t = Instant::now();
    for i in 0..count {
        let raw = format!("From: {me}\r\nTo: {me}\r\nSubject: Message {i}\r\nMessage-ID: <m{i}.{run}@gm.test>\r\n\r\nbody {i}\r\n.dot line\r\n");
        crate::smtp::send(&smtp, &me, "pw", &me, std::slice::from_ref(&me), raw.as_bytes()).unwrap();
    }
    eprintln!("sent {count} in {:?}", t.elapsed());
    let dir = std::env::temp_dir().join(format!("mailcraft-real-pop-{run}"));
    let store = Store::open(&dir).unwrap();
    let acc = AccountConfig { id: "p1".into(), email: me.clone(), pop: Some(pop_cfg), ..Default::default() };
    pop::folders(&store, "p1").unwrap();
    let fetch = |keep: &Keep| {
        let mut c = pop::open(&acc, "pw").unwrap();
        let t = Instant::now();
        let r = pop::fetch_new(&mut c, &store, "p1", "INBOX", keep, 1_000).unwrap();
        c.quit().unwrap();
        eprintln!("{} new of {} in {:?}", r.new, r.total, t.elapsed());
        r
    };
    let r = fetch(&Keep::default());
    assert_eq!((r.new, r.total as usize), (count, count));
    let inbox = store.load_folder("p1", "INBOX").unwrap();
    let mut subjects: Vec<&str> = inbox.messages.iter().map(|m| m.subject.as_str()).collect();
    subjects.sort_unstable();
    subjects.dedup();
    assert_eq!(subjects.len(), count, "each once");
    let one = inbox.messages.iter().find(|m| m.subject == "Message 0").unwrap();
    let raw = String::from_utf8(store.read_body(&one.key).unwrap()).unwrap();
    assert!(raw.contains("\r\n.dot line\r\n"), "{raw}");
    assert_eq!(fetch(&Keep::default()).new, 0, "nothing twice");
    // Removed from the server: kept here, and nothing left there.
    assert_eq!(fetch(&Keep { remove_now: true, ..Keep::default() }).new, 0);
    let mut c = pop::open(&acc, "pw").unwrap();
    assert!(c.uidl().unwrap().is_empty(), "the server's copies went");
    c.quit().unwrap();
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), count);
    let _ = std::fs::remove_dir_all(dir);
}
