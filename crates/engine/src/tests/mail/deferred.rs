//! Engine tests: the open message's body got ready on another thread (the window's reading
//! lane), shown the same as when it's read and parsed at once.

use super::*;

use mailcraft_model::MessageKey;

const HTML: &[u8] = b"From: Ana <ana@posteo.example>\r\nTo: Riley <riley@studio.example>\r\nSubject: Colour notes\r\n\
Date: Fri, 16 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p>The <b>grade</b> is warmer now.</p>\r\n";

const NO_DATE: &[u8] = b"From: Ana <ana@posteo.example>\r\nSubject: Undated\r\n\r\nWhen was this?\r\n";

fn smime_testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name)).unwrap()
}

/// Put `raw` in the sample Inbox (no body kept when `None`); its key.
fn put(s: &mut Session, uid: u32, raw: &[u8], keep: bool) -> MessageKey {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW - 3600));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    if keep {
        store.write_body(&key, raw).unwrap();
    }
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    key
}

/// The sample with a message of each kind: HTML, without a date, its text first, signed,
/// encrypted (no key here) and one whose body isn't kept.
fn with_kinds(name: &str) -> (Session, Vec<MessageKey>) {
    let mut s = sample_session(name);
    let text_first = {
        let d = mailcraft_mime::Draft { subject: "Files coming".into(), body: "The text first.".into(), ..Default::default() };
        let files = [mailcraft_mime::PendingFile { name: "cut.mov".into(), mime: "video/quicktime".into(), size: 9_000_000, ..Default::default() }];
        mailcraft_mime::text_first(&d, &mailcraft_model::EmailAddress::new("Ana", "ana@posteo.example"), NOW, "<tf@posteo.example>", &files).unwrap()
    };
    let mut keys = vec![
        put(&mut s, 9401, HTML, true),
        put(&mut s, 9402, NO_DATE, true),
        put(&mut s, 9403, &text_first, true),
        put(&mut s, 9404, &smime_testdata("signed-clear.eml"), true),
        put(&mut s, 9405, &smime_testdata("encrypted-gcm.eml"), true),
        put(&mut s, 9406, HTML, false),
    ];
    keys.extend(s.list.iter().filter(|m| m.key.uid < 9400).map(|m| m.key.clone()));
    s.take_jobs();
    (s, keys)
}

/// Got ready on another thread or at once, every kind of message is shown the same: the same
/// reading (its text, date, warning, protection), the same mail marked read, the same download
/// asked for one not kept; and none of them parsed on the session's thread but those that are
/// signed or encrypted.
#[test]
fn deferred_open_then_finish_equals_sync_open() {
    let (mut now, keys) = with_kinds("deferred-sync");
    let (mut later, _) = with_kinds("deferred-later");
    later.defer_bodies(true);
    let before = later.bodies_parsed_here();
    for key in &keys {
        now.run("list.open", &json!({"key": key.to_id()})).unwrap();
        later.run("list.open", &json!({"key": key.to_id()})).unwrap();
        assert!(later.reading.as_ref().unwrap().parsed.is_none() && later.body_waiting(key), "{key:?} waits");
        let work = later.take_body_work().unwrap();
        assert!(later.take_body_work().is_none(), "handed out once");
        later.finish_body(work.run());
        assert!(!later.body_waiting(key));
        assert_eq!(later.reading, now.reading, "{key:?}");
        assert_eq!(later.take_jobs(), now.take_jobs(), "{key:?}");
        assert_eq!(later.summary(key).map(|m| m.flags.seen), now.summary(key).map(|m| m.flags.seen));
        if key.uid == 9404 {
            assert!(now.reading.as_ref().unwrap().security.is_some(), "the signed one says so");
        }
    }
    // The undated one shows when it came in.
    later.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    let work = later.take_body_work().unwrap();
    later.finish_body(work.run());
    assert_eq!(later.reading.as_ref().unwrap().parsed.as_ref().unwrap().date, NOW - 3600);
    assert_eq!(later.bodies_parsed_here() - before, 2, "only the signed and the encrypted message were parsed here");
    done(now);
    done(later);
}

/// A body got ready for a message the pane moved on from is let go: the pane waits for the
/// one it shows now.
#[test]
fn stale_prepared_body_is_ignored() {
    let mut s = sample_session("deferred-stale");
    s.defer_bodies(true);
    let (a, b) = (s.list[0].key.clone(), s.list[1].key.clone());
    s.run("list.open", &json!({"key": a.to_id()})).unwrap();
    let first = s.take_body_work().unwrap();
    s.run("list.next", &json!({})).unwrap();
    assert_eq!(s.reading.as_ref().unwrap().key, b);
    s.finish_body(first.run());
    let r = s.reading.as_ref().unwrap();
    assert!(r.parsed.is_none() && r.error.is_none() && s.body_waiting(&b), "still waiting for the one shown");
    let second = s.take_body_work().unwrap();
    assert_eq!(second.key(), &b);
    s.finish_body(second.run());
    assert_eq!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().subject, s.summary(&b).unwrap().subject);
    // Left for another folder: what was waiting is let go too.
    s.run("list.open", &json!({"key": a.to_id()})).unwrap();
    let w = s.take_body_work().unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Trash"})).unwrap();
    // A command run now doesn't read it either.
    let parsed = s.bodies_parsed_here();
    s.finish_pending_body();
    assert!(!s.body_waiting(&a) && s.bodies_parsed_here() == parsed);
    s.finish_body(w.run());
    assert!(s.reading.is_none() && s.take_jobs().is_empty());
    done(s);
}

/// A command run while the body is still being got ready acts on it as shown: it's got ready
/// first, whether or not the window had handed it to its thread already. Moving about the list
/// doesn't wait for it.
#[test]
fn a_command_finishes_the_body_first() {
    let mut s = sample_session("deferred-reply");
    s.defer_bodies(true);
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    s.run("list.next", &json!({})).unwrap();
    s.run("list.prev", &json!({})).unwrap();
    assert!(s.reading.as_ref().unwrap().parsed.is_none(), "moving about waits for nothing");
    s.run("mail.reply", &json!({})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.subject.contains("Storyboard review")));
    assert!(s.reading.as_ref().unwrap().parsed.is_some() && !s.body_waiting(&s.reading.as_ref().unwrap().key.clone()));
    s.run("compose.discard", &json!({})).unwrap();
    // Handed out already (on the window's thread now): done again here, and what comes back
    // later is let go.
    s.run("list.next", &json!({})).unwrap();
    let handed = s.take_body_work().unwrap();
    let read = s.run("message.read", &json!({})).unwrap();
    assert!(read["text"].as_str().is_some_and(|t| !t.is_empty()), "{read}");
    let shown = s.reading.clone();
    s.finish_body(handed.run());
    assert_eq!(s.reading, shown);
    done(s);
}

/// A message the parser trips on (a panic) shows that it couldn't be read; the session goes on.
#[test]
fn a_panicking_parse_shows_an_error() {
    let caught = crate::PANICS_CAUGHT.load(std::sync::atomic::Ordering::Relaxed);
    let mut s = sample_session("deferred-panic");
    s.defer_bodies(true);
    let k = s.list[0].key.clone();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let mut work = s.take_body_work().unwrap();
    work.breaks = true;
    s.finish_body(work.run());
    let r = s.reading.as_ref().unwrap();
    assert!(r.parsed.is_none() && r.error.as_deref() == Some("this message could not be read"));
    assert!(!s.body_waiting(&k));
    s.run("list.next", &json!({})).unwrap();
    let w = s.take_body_work().unwrap();
    s.finish_body(w.run());
    assert!(s.reading.as_ref().unwrap().parsed.is_some(), "the next one shows");
    assert_eq!(crate::PANICS_CAUGHT.load(std::sync::atomic::Ordering::Relaxed), caught, "no command panicked");
    done(s);
}

/// A body not kept is downloaded once the work finds it missing, and parsed on the other
/// thread when it comes; one waited for in a window of its own too is shown at once (both from
/// one parse).
#[test]
fn a_body_downloaded_is_parsed_on_the_other_thread() {
    let mut s = sample_session("deferred-download");
    s.defer_bodies(true);
    let k = s.list[0].key.clone();
    s.store.as_ref().unwrap().remove_body(&k);
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    assert!(s.take_jobs().is_empty(), "nothing asked until the work looked");
    let w = s.take_body_work().unwrap();
    s.finish_body(w.run());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FetchBody { key } if *key == k)));
    let before = s.bodies_parsed_here();
    s.apply(Event::Body { key: k.clone(), raw: NO_DATE.to_vec() });
    assert!(s.body_waiting(&k) && s.reading.as_ref().unwrap().parsed.is_none());
    let w = s.take_body_work().unwrap();
    s.finish_body(w.run());
    assert_eq!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().subject, "Undated");
    assert_eq!(s.bodies_parsed_here(), before);
    // In a window of its own as well: shown in both at once.
    let other = s.list[1].key.clone();
    s.store.as_ref().unwrap().remove_body(&other);
    s.run("list.open", &json!({"key": other.to_id()})).unwrap();
    let w = s.take_body_work().unwrap();
    s.finish_body(w.run());
    s.run("message.popOut", &json!({"key": other.to_id()})).unwrap();
    s.apply(Event::Body { key: other.clone(), raw: HTML.to_vec() });
    assert!(!s.body_waiting(&other));
    let pane = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(std::sync::Arc::ptr_eq(&pane, s.windows[0].parsed.as_ref().unwrap()));
    done(s);
}

/// A body that arrives while the cache is still being read for it wins: the cache read's
/// result, handed out before, is let go, so the arrived body isn't downloaded again (cache
/// empty) or hidden behind the text-first copy (cache holding that).
#[test]
fn a_body_arriving_during_the_cache_read_wins() {
    let mut s = sample_session("deferred-race");
    s.defer_bodies(true);
    let k = s.list[0].key.clone();
    s.store.as_ref().unwrap().remove_body(&k);
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let read = s.take_body_work().unwrap().run();
    s.apply(Event::Body { key: k.clone(), raw: NO_DATE.to_vec() });
    let arrived = s.take_body_work().unwrap();
    s.finish_body(read);
    assert!(s.take_jobs().is_empty(), "not downloaded again");
    assert!(s.body_waiting(&k), "still waits for the arrived body");
    s.finish_body(arrived.run());
    assert_eq!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().subject, "Undated");
    done(s);

    // The cache holds the text-first copy; the full message arrives while it's read.
    let (mut s, keys) = with_kinds("deferred-race-text-first");
    s.defer_bodies(true);
    let k = keys[2].clone();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let read = s.take_body_work().unwrap().run();
    s.apply(Event::Body { key: k.clone(), raw: HTML.to_vec() });
    let arrived = s.take_body_work().unwrap();
    s.finish_body(read);
    assert!(s.body_waiting(&k) && s.reading.as_ref().unwrap().parsed.is_none());
    s.finish_body(arrived.run());
    let r = s.reading.as_ref().unwrap();
    assert!(!r.text_first && r.parsed.as_ref().unwrap().subject == "Colour notes", "the full body is shown");
    done(s);
}

/// Commands that only change the list or the view (typing in the search box, the Focused tab,
/// a filter, the sort) leave the open message's body being got ready on the other thread.
#[test]
fn list_and_view_commands_leave_the_body_waiting() {
    let mut s = sample_session("deferred-list-view");
    s.defer_bodies(true);
    let k = s.list[0].key.clone();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let parsed = s.bodies_parsed_here();
    for id in ["list.filter", "list.focusTab", "search.run", "search.clear", "view.reverseSort", "view.arrange", "view.folderPane", "dataFile.idle"] {
        assert!(crate::cmd::leaves_body_waiting(id), "{id}");
    }
    let _ = s.run("view.reverseSort", &json!({}));
    let _ = s.run("search.run", &json!({"query": "zzzz"}));
    let _ = s.run("search.clear", &json!({}));
    let _ = s.run("dataFile.idle", &json!({"seconds": 30}));
    assert_eq!(s.bodies_parsed_here(), parsed, "nothing read on the session's thread");
    assert!(!crate::cmd::leaves_body_waiting("mail.reply") && !crate::cmd::leaves_body_waiting("view.html"));
    done(s);
}
