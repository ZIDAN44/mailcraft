//! Opening a message over IMAP (issue #24): its text first, and never behind a sync.

use super::*;

/// A message with a big file: text, an HTML version with a picture, and a 1 MB attachment.
fn with_a_big_file() -> Vec<u8> {
    let file = "QUJD".repeat(300_000);
    format!(
        "From: Sam <sam@studio.example>\r\nTo: me@studio.example\r\nSubject: Rushes\r\nMessage-ID: <r1@studio.example>\r\n\
         Importance: high\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
         --B\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n\
         --A\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nThe notes, to read n=C3=B6w.\r\n\
         --A\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>The notes, to read <b>now</b>.</p><img src=\"cid:still@x\">\r\n--A--\r\n\
         --B\r\nContent-Type: image/png\r\nContent-ID: <still@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n\
         --B\r\nContent-Type: video/quicktime; name=\"reel.mov\"\r\nContent-Disposition: attachment; filename=\"reel.mov\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{file}\r\n--B--\r\n"
    )
    .into_bytes()
}

fn account(id: &str, port: u16) -> AccountConfig {
    AccountConfig {
        id: id.into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    }
}

fn body_of(seen: &[Event], key: &MessageKey) -> Vec<Vec<u8>> {
    seen.iter()
        .filter_map(|e| match e {
            Event::Body { key: k, raw } if k == key => Some(raw.clone()),
            _ => None,
        })
        .collect()
}

/// A big message's header and text parts come first, its files listed with their sizes but no
/// bytes; then the whole of it, kept.
#[test]
fn a_big_message_comes_as_its_text_first() {
    let st = State::new();
    let raw = with_a_big_file();
    st.lock().unwrap().add_raw("INBOX", raw.clone());
    let store = store("imap-text-first");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: account("a1", serve_tcp(&st)), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    assert!(!store.has_body(&key), "not downloaded ahead");
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the whole of it", |e| matches!(e, Event::Body { key: k, raw } if *k == key && !mailcraft_mime::is_text_first(raw)));
    let bodies = body_of(&seen, &key);
    assert_eq!(bodies.len(), 2, "its text, then all of it");
    let first = &bodies[0];
    assert!(mailcraft_mime::is_text_first(first));
    assert!(first.len() < 4096, "no bytes of the files: {}", first.len());
    let p = mailcraft_mime::parse(first).unwrap();
    assert_eq!((p.subject.as_str(), p.from.email.as_str(), p.message_id.as_str()), ("Rushes", "sam@studio.example", "r1@studio.example"));
    assert!(p.text.contains("The notes, to read **now**."), "the HTML version: {}", p.text);
    let files: Vec<(&str, usize, bool)> = p.attachments.iter().map(|a| (a.name.as_str(), a.size, a.inline)).collect();
    assert!(files.contains(&("reel.mov", 900_000, false)), "{files:?}");
    assert!(files.iter().any(|f| f.2), "the picture, inline: {files:?}");
    assert_eq!(bodies[1], raw, "then the whole of it, as it is");
    assert!(store.has_body(&key));
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.contains("BODYSTRUCTURE")) && log.iter().any(|l| l.contains("BODY.PEEK[1.2]")), "{log:?}");

    // A small one comes whole at once.
    let uid = st.lock().unwrap().add("INBOX", "Small", &[]);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "the new one", |e| matches!(e, Event::FolderChanged { .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.key.uid == uid).unwrap().key.clone();
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the small one", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(body_of(&seen, &key).iter().all(|b| !mailcraft_mime::is_text_first(b)));
}

/// A message opened while the main connection is in the middle of a slow folder comes at once,
/// on the second connection, not when that folder is done.
#[test]
fn a_message_opened_during_a_sync_doesnt_wait_for_it() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "Opened during the sync", &[]);
    st.lock().unwrap().add("Archive", "Old", &[]);
    let store = store("imap-busy-open");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: account("a1", serve_tcp(&st)), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    assert!(!store.has_body(&key));
    // The next full sync spends 8 seconds in the Archive.
    st.lock().unwrap().stall_on = Some(("SELECT \"Archive\"".into(), Duration::from_secs(8)));
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the Archive's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "Archive"));
    let began = Instant::now();
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(began.elapsed() < Duration::from_secs(5), "it waited for the Archive: {:?}", began.elapsed());
    st.lock().unwrap().stall_on = None;
}
