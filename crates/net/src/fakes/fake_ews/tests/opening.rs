//! Opening a big message over EWS (issues #24 and #191): its text first, and the whole of it
//! whatever the server says of that.

use super::*;

/// A message with a 1 MB file: its text, and the file.
fn with_a_big_file(subject: &str) -> Vec<u8> {
    let file = "QUJD".repeat(300_000);
    format!(
        "From: Sam <sam@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@contoso.example>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
         --B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe notes for {subject}.\r\n\
         --B\r\nContent-Type: video/quicktime; name=\"reel.mov\"\r\nContent-Disposition: attachment; filename=\"reel.mov\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{file}\r\n--B--\r\n"
    )
    .into_bytes()
}

/// What came of opening the message `key`: its sources as they came, and any failure said.
fn opened(pool: &crate::worker::Pool, key: &mailcraft_model::MessageKey) -> (Vec<Vec<u8>>, Vec<String>) {
    let seen = until(pool, "the whole of it", |e| {
        matches!(e, Event::Body { key: k, raw } if k == key && !mailcraft_mime::is_text_first(raw))
            || matches!(e, Event::BodyFailed { key: k, .. } if k == key)
    });
    let mut bodies = Vec::new();
    let mut failed = Vec::new();
    for e in seen {
        match e {
            Event::Body { key: k, raw } if k == *key => bodies.push(raw),
            Event::BodyFailed { key: k, error } if k == *key => failed.push(error),
            _ => {}
        }
    }
    (bodies, failed)
}

fn start(d: Data, name: &str) -> (Server, Store, crate::worker::Pool) {
    let server = serve(d);
    let store = crate::fake::store(name);
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    (server, store, pool)
}

/// Issue #191: every field the text first and the list ask for is one Exchange's schema knows
/// (`message:InReplyTo` isn't: on-premises Exchange refused the request whole, and the message
/// showed "The request is invalid." in place of its text). Its text comes first, then all of it.
#[test]
fn a_big_message_over_ews_comes_as_its_text_first() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let raw = with_a_big_file("Rushes");
    d.deliver(&inbox, &raw, false, "2024-05-06T09:00:00Z");
    let (server, store, mut pool) = start(d, "ews-text-first");
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.has_attachments && m.size > 1 << 20, "a big message with a file: {} bytes", m.size);
    pool.submit(Job::FetchBody { key: m.key.clone() });
    let (bodies, failed) = opened(&pool, &m.key);
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(bodies.len(), 2, "its text, then all of it");
    assert!(mailcraft_mime::is_text_first(&bodies[0]));
    let p = mailcraft_mime::parse(&bodies[0]).unwrap();
    assert!(p.text.contains("The notes for Rushes."), "{}", p.text);
    assert!(p.attachments.iter().any(|a| a.name == "reel.mov"), "its file listed");
    assert_eq!(bodies[1], raw);
    assert_eq!(server.data.lock().unwrap().field_refusals, Vec::<String>::new(), "nothing asked the schema doesn't know");
}

/// Issue #191: a server that refuses the text first's request (a fault, the request as a whole):
/// each message comes whole all the same, the refusal never said in its place, and a connection
/// it was refused on doesn't ask for a text first again (the account has two: three messages
/// opened are refused twice at most).
#[test]
fn a_text_first_refused_still_opens_the_message() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.refused_fields = vec!["item:Body".into()];
    let raws = [with_a_big_file("Rushes"), with_a_big_file("Stills"), with_a_big_file("Grade")];
    for (n, raw) in raws.iter().enumerate() {
        d.deliver(&inbox, raw, false, &format!("2024-05-0{}T09:00:00Z", n + 6));
    }
    let (server, store, mut pool) = start(d, "ews-text-first-refused");
    let messages = store.load_folder("a1", "INBOX").unwrap().messages;
    for m in &messages {
        pool.submit(Job::FetchBody { key: m.key.clone() });
        let (bodies, failed) = opened(&pool, &m.key);
        assert!(failed.is_empty(), "never the message's failure: {failed:?}");
        let whole = bodies.last().unwrap();
        assert!(raws.contains(whole), "the whole of it");
        assert!(store.has_body(&m.key));
    }
    let refused = server.data.lock().unwrap().field_refusals.clone();
    assert!((1..=2).contains(&refused.len()) && refused.iter().all(|f| f == "item:Body"), "once on each connection at most: {refused:?}");
}
