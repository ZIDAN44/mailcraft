//! Engine tests: a message's source in a window of its own (View message source, issue #27).

use mailcraft_mime as mime;

use super::*;
use crate::session::source_lines as lines_of;

/// The source as it's kept, its header apart, from the message read or one named; the same one
/// twice is one window.
#[test]
fn the_source_of_a_message_kept_here() {
    let mut s = sample_session("source");
    assert!(s.run("message.source", &json!({})).is_err(), "nothing chosen");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let key = s.reading.as_ref().unwrap().key.clone();
    let raw = s.store.as_ref().unwrap().read_body(&key).unwrap();
    let r = s.run("message.source", &json!({})).unwrap();
    assert_eq!(r["pending"], json!(false));
    assert_eq!(r["size"], json!(raw.len()));
    let text = r["text"].as_str().unwrap();
    let mut want = String::from_utf8_lossy(&raw).replace("\r\n", "\n");
    if !want.ends_with('\n') {
        want.push('\n');
    }
    assert_eq!(text, want);
    let h = s.run("message.source", &json!({"headers": true})).unwrap();
    let head = h["text"].as_str().unwrap();
    assert!(head.contains("Subject: Storyboard review on Thursday") && !head.contains("\n\n"), "{head}");
    assert!(text.starts_with(head) && text.len() > head.len());
    assert_eq!(s.sources.len(), 1, "the same message's source is one window");
    assert_eq!(s.sources[0].subject, "Storyboard review on Thursday");
    // Another, by its key: a second window; closed, it goes.
    let other = key_of(&s, "Character turnarounds");
    s.run("message.source", &other).unwrap();
    assert_eq!(s.sources.len(), 2);
    s.run("message.closeSource", &other).unwrap();
    assert_eq!(s.sources.len(), 1);
    assert!(s.run("message.source", &json!({"key": "nobody:1:1:INBOX"})).is_err());
    done(s);
}

/// Not downloaded yet: asked for (once, not again for the reading pane's request), shown when it
/// comes; Exchange's text first isn't the source and doesn't stand for it; a failure says why
/// and asking again tries again.
#[test]
fn a_source_not_here_yet() {
    let mut s = sample_session("source-later");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let key = s.selection[0].clone();
    let whole = s.store.as_ref().unwrap().read_body(&key).unwrap();
    s.store.as_ref().unwrap().remove_body(&key);
    let r = s.run("message.source", &k).unwrap();
    assert_eq!(r["pending"], json!(true));
    let fetches = |s: &mut Session| s.take_jobs().iter().filter(|j| matches!(j, Job::FetchBody { key: x } if *x == key)).count();
    assert_eq!(fetches(&mut s), 1);
    // Its text first, then the whole of it.
    let parsed = mime::parse(&whole).unwrap();
    let d = mime::Draft { subject: parsed.subject.clone(), body: "text".into(), ..Default::default() };
    let first = mime::text_first(&d, &parsed.from, NOW, &parsed.message_id, &[]).unwrap();
    s.apply(Event::Body { key: key.clone(), raw: first });
    assert!(s.sources[0].pending && s.sources[0].lines.is_empty(), "the text first isn't its source");
    s.apply(Event::Body { key: key.clone(), raw: whole.clone() });
    assert!(!s.sources[0].pending && s.sources[0].size == whole.len());
    // The reading pane asking for it already: not asked twice.
    s.run("message.closeSource", &k).unwrap();
    s.run("list.open", &k).unwrap();
    assert_eq!(fetches(&mut s), 1, "the reading pane asks");
    s.run("message.source", &k).unwrap();
    assert_eq!(fetches(&mut s), 0, "not asked again");
    s.apply(Event::BodyFailed { key: key.clone(), error: "the server went away".into() });
    assert_eq!(s.sources[0].error.as_deref(), Some("the server went away"));
    assert!(!s.sources[0].pending);
    // Asked again: tried again.
    let r = s.run("message.source", &k).unwrap();
    assert_eq!(r["pending"], json!(true));
    assert_eq!(fetches(&mut s), 1);
    done(s);
}

/// Hostile sources: control characters and direction overrides shown by name, bytes that aren't
/// UTF-8 as U+FFFD, a line of megabytes broken up, and a source of nothing but line ends capped.
#[test]
fn hostile_sources() {
    let (lines, header, cut) = lines_of(b"Subject: a\x00b\x1bc\x7f\r\nX: \xe2\x80\xaeevil\xe2\x80\xac\r\n\r\nbody\rold mac\nunix caf\xe9\r\n");
    assert_eq!(lines, vec!["Subject: a\u{2400}b\u{241b}c\u{2421}", "X: <U+202E>evil<U+202C>", "", "body", "old mac", "unix café"]);
    assert_eq!((header, cut), (2, false));
    // 8-bit text in the character set its part says; one that isn't known: U+FFFD.
    let (lines, _, _) = lines_of(b"Content-Type: text/plain; charset=\"koi8-r\"\r\n\r\n\xf0\xd2\xc9\xd7\xc5\xd4\r\n");
    assert_eq!(lines[2], "Привет");
    let (lines, _, _) = lines_of(b"Content-Type: text/plain; charset=x-nothing\r\n\r\n\xff\r\n");
    assert_eq!(lines[2], "\u{fffd}");
    // No blank line: all header. Nothing at all: one empty line, no panic.
    assert_eq!(lines_of(b"Subject: x"), (vec!["Subject: x".to_string()], 1, false));
    assert_eq!(lines_of(b""), (vec![String::new()], 0, false));
    // A cut through a character of more than one byte is made before it, not through it.
    let mut big = b"Subject: big\r\n\r\n".to_vec();
    big.extend(std::iter::repeat_n(b'A', crate::SOURCE_SHOWN - big.len() - 1));
    big.extend("é and more".as_bytes());
    let (lines, header, cut) = lines_of(&big);
    assert!(cut && header == 1);
    assert!(lines.iter().all(|l| l.chars().count() <= 2_000));
    let tail: String = lines.last().unwrap().chars().rev().take(3).collect();
    assert_eq!(tail, "AAA");
    // Nothing but line ends: capped in lines too.
    let (lines, _, cut) = lines_of(&vec![b'\n'; 1_000_000]);
    assert!(cut && lines.len() == 100_000);
    // Through the command: what it says is the size of the whole.
    let mut s = sample_session("source-hostile");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    let key = s.selection[0].clone();
    s.store.as_ref().unwrap().write_body(&key, &big).unwrap();
    let r = s.run("message.source", &k).unwrap();
    assert_eq!((r["size"].as_u64(), r["truncated"].as_bool()), (Some(big.len() as u64), Some(true)));
    done(s);
}

/// The Outbox's messages have a source too: what's waiting to go, Bcc and all.
#[test]
fn the_source_of_a_message_in_the_outbox() {
    let store = temp("source-outbox");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Waiting in the Outbox", "body": "See you"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let Some(Job::Send { id: send_id, raw, from, recipients, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else {
        panic!("no send")
    };
    let item = mailcraft_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list[0].key.clone();
    let r = s.run("message.source", &json!({"key": key.to_id()})).map_err(|e| e.to_string()).unwrap();
    assert!(r["text"].as_str().unwrap().contains("Subject: Waiting in the Outbox"), "{r}");
    assert!(s.take_jobs().is_empty(), "nothing to download");
    done(s);
}
