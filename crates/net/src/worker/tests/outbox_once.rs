//! A message sent from the Outbox goes out once, even when its files can't be taken out of the
//! Outbox (held by another program, a failing disk); and one refused for good isn't tried again
//! when the mark saying so can't be written.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex as StdMutex;

use mailcraft_model::{Security, ServerConfig};
use mailcraft_store::OutboxItem;

use super::*;

/// What the SMTP server below took: each message's subject, and each recipient it refused.
#[derive(Default)]
struct Took {
    messages: Vec<String>,
    refused: Vec<String>,
}

/// An SMTP server on localhost taking any number of messages, refusing for good (550) the
/// recipients whose address starts with `refused@`.
fn smtp_server() -> (u16, Arc<StdMutex<Took>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let took = Arc::new(StdMutex::new(Took::default()));
    let t = took.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(s) = s else { return };
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            let _ = w.write_all(b"220 test ESMTP\r\n");
            let (mut line, mut data, mut subject) = (String::new(), false, String::new());
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                let cmd = line.trim_end().to_string();
                let upper = cmd.to_ascii_uppercase();
                let answer: &[u8] = if data {
                    if let Some(s) = cmd.strip_prefix("Subject: ") {
                        subject = s.to_string();
                    }
                    if cmd == "." {
                        data = false;
                        t.lock().unwrap().messages.push(std::mem::take(&mut subject));
                        b"250 queued\r\n"
                    } else {
                        b""
                    }
                } else if upper.starts_with("EHLO") {
                    b"250-test\r\n250 AUTH PLAIN\r\n"
                } else if upper.starts_with("AUTH") {
                    b"235 ok\r\n"
                } else if upper.starts_with("RCPT TO:<REFUSED@") {
                    t.lock().unwrap().refused.push(cmd.clone());
                    b"550 no such user\r\n"
                } else if upper == "DATA" {
                    data = true;
                    b"354 go\r\n"
                } else if upper == "QUIT" {
                    let _ = w.write_all(b"221 bye\r\n");
                    break;
                } else {
                    b"250 ok\r\n"
                };
                let _ = w.write_all(answer);
                line.clear();
            }
        }
    });
    (port, took)
}

fn queue(store: &Store, id: u64, to: &str, subject: &str) {
    let raw = format!("From: me@example.com\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}@example.com>\r\n\r\nhi\r\n");
    let item = OutboxItem {
        id,
        from: "me@example.com".into(),
        recipients: vec![to.into()],
        sent_folder: Some("Sent".into()),
        queued: i64::try_from(id).unwrap(),
        ..Default::default()
    };
    store.outbox_add("a1", &item, raw.as_bytes()).unwrap();
}

/// A worker for the account on these servers, as a new start of MailCraft makes one.
fn worker(pool: &Pool, store: &Store, imap: u16, smtp: u16) -> Worker {
    let acc = AccountConfig {
        id: "a1".into(),
        email: "me@example.com".into(),
        username: "me@example.com".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: imap, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp, security: Security::None },
        ..Default::default()
    };
    Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default())
}

fn subjects(took: &StdMutex<Took>) -> Vec<String> {
    took.lock().unwrap().messages.clone()
}

fn in_sent(st: &Arc<std::sync::Mutex<crate::fake::State>>) -> usize {
    st.lock().unwrap().mailbox("Sent").unwrap().msgs.len()
}

/// Sent, but its two files can't be removed: it leaves the Outbox all the same, and neither
/// the next flush nor the next start sends it again. Its files go once they can.
#[test]
fn a_sent_message_whose_files_cant_be_removed_isnt_sent_again() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "ana@example.com", "Held");
    store.fail(None, Some("outbox/7.json|outbox/7.eml"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    assert_eq!(subjects(&took), ["Held"]);
    assert_eq!(in_sent(&st), 1, "its copy filed");
    w.flush_outbox();
    assert_eq!(subjects(&took), ["Held"], "sent once");
    assert!(store.outbox("a1").is_empty(), "out of the Outbox: {:?}", store.outbox("a1"));
    worker(&pool, &store, imap, smtp).flush_outbox();
    assert_eq!(subjects(&took), ["Held"], "sent once, after a new start too");
    assert_eq!(in_sent(&st), 1);
    // The files can go now: the next flush takes them.
    store.fail(None, None);
    queue(&store, 8, "ana@example.com", "Next");
    worker(&pool, &store, imap, smtp).flush_outbox();
    assert_eq!(subjects(&took), ["Held", "Next"]);
    assert!(store.outbox("a1").is_empty());
    assert!(store.outbox_raw("a1", 7).is_none(), "its source removed");
    assert_eq!(in_sent(&st), 2);
}

/// Nothing in the Outbox can be changed at all: this run of MailCraft doesn't send it again,
/// and says the message is still there.
#[test]
fn a_sent_message_nothing_can_take_out_isnt_sent_again_while_running() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "ana@example.com", "Stuck");
    store.fail(None, Some("outbox/7.json|outbox/7.eml|outbox/7.gone"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    assert_eq!(subjects(&took), ["Stuck"], "sent once");
    let mut said = Vec::new();
    while let Some(e) = pool.wait(Duration::from_millis(100)) {
        said.push(e);
    }
    assert!(said.iter().any(|e| matches!(e, Event::Notice { message, .. } if message.contains("Outbox"))), "{said:?}");
    assert_eq!(said.iter().filter(|e| matches!(e, Event::Sent { id: 7, .. })).count(), 1, "{said:?}");
}

/// Refused for good, and the mark saying so can't be written: not tried again by this run.
#[test]
fn a_refused_message_whose_mark_cant_be_written_isnt_tried_again() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "refused@example.com", "Refused");
    store.fail(None, Some("outbox/7.json"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    assert_eq!(took.lock().unwrap().refused.len(), 1, "asked once");
    assert!(subjects(&took).is_empty());
    assert_eq!(store.outbox("a1").len(), 1, "it waits in the Outbox to be changed");
}

/// Refused for good is a mark of its own, not words in its error: the error says only what the
/// server said, and one marked so (or kept so by an earlier MailCraft, in its error's words)
/// isn't tried again, whatever its error says.
#[test]
fn a_refused_message_is_marked_so_and_left() {
    let st = crate::fake::State::new();
    let imap = crate::fake::serve_tcp(&st);
    let (smtp, took) = smtp_server();
    let store = Store::memory();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, "refused@example.com", "Refused");
    worker(&pool, &store, imap, smtp).flush_outbox();
    let item = store.outbox("a1").remove(0);
    assert!(item.refused, "{item:?}");
    assert!(item.error.as_deref().is_some_and(|e| !e.starts_with("refused") && e.contains("no such user")), "{item:?}");
    // Marked, whatever its error says: left.
    for (id, error) in [(8, None), (9, Some("refused: sending failed: 550 no such user"))] {
        queue(&store, id, "ana@example.com", "Marked");
        let mut item = store.outbox("a1").into_iter().find(|i| i.id == id).unwrap();
        item.error = error.map(str::to_string);
        item.refused = error.is_none();
        store.outbox_update("a1", &item).unwrap();
    }
    worker(&pool, &store, imap, smtp).flush_outbox();
    assert_eq!(took.lock().unwrap().refused.len(), 1, "asked once");
    assert!(subjects(&took).is_empty(), "{:?}", subjects(&took));
    assert_eq!(store.outbox("a1").len(), 3);
}

/// The first test through a real server on this computer, by hand: GreenMail's SMTP and IMAP
/// (any password), `MAILCRAFT_REAL_OUTBOX="23025 23143"`. Delivered once to the recipient, one
/// copy in Sent.
#[test]
#[ignore = "needs real SMTP and IMAP servers in MAILCRAFT_REAL_OUTBOX"]
fn real_outbox_sends_once() {
    let Some((smtp, imap)) = std::env::var("MAILCRAFT_REAL_OUTBOX").ok().and_then(|v| {
        let mut p = v.split_whitespace().map(|p| p.parse::<u16>().ok());
        Some((p.next()??, p.next()??))
    }) else {
        eprintln!("MAILCRAFT_REAL_OUTBOX isn't set: skipped");
        return;
    };
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let subject = format!("Outbox once {}", std::process::id());
    let to = format!("ana{}@example.com", std::process::id());
    queue(&store, 7, &to, &subject);
    store.fail(None, Some("outbox/7.json|outbox/7.eml"));
    let mut w = worker(&pool, &store, imap, smtp);
    w.flush_outbox();
    w.flush_outbox();
    worker(&pool, &store, imap, smtp).flush_outbox();
    let found = |user: &str, folder: &str| {
        let cfg = ServerConfig { host: "127.0.0.1".into(), port: imap, security: Security::None };
        let mut c = crate::imap::Client::connect(&cfg).unwrap();
        c.login(user, "pw").unwrap();
        c.select(folder, true).unwrap();
        c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap().len()
    };
    // GreenMail delivers at once.
    assert_eq!(found(&to, "INBOX"), 1, "delivered once");
    assert_eq!(found("me@example.com", "Sent"), 1, "one copy in Sent");
    assert!(store.outbox("a1").is_empty());
}
