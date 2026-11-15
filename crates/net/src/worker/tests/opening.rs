//! Opening a message on each kind of server, each way the worker has: on the main connection
//! (asked to open it, or the whole of one whose text went first) and on the second one (opened
//! while the main one was busy, or the whole of one handed to it). Each says the same.

use super::*;

/// What a worker of `acc` on `store` says (through `pool`) of `key`, its source not here, as it
/// opens it each way: [`Worker::fetch`], [`Worker::fetch_whole`], [`Worker::fetch_opened`] and
/// [`Worker::fetch_for_main`], each on a worker of its own.
fn opened_each_way(pool: &mut Pool, store: &Store, acc: &AccountConfig, secret: &str, key: &MessageKey) -> Vec<Vec<Event>> {
    let ways: [fn(&mut Worker, MessageKey); 4] = [
        |w, k| w.fetch(k),
        |w, k| w.fetch_whole(k),
        |w, k| {
            w.fetch_opened(k);
        },
        |w, k| w.fetch_for_main(k),
    ];
    let mut said = Vec::new();
    for way in ways {
        store.remove_body(key);
        let _ = pool.poll();
        let mut w = Worker::new(acc.clone(), Auth::shared(secret), store.clone(), pool.sink.clone(), SyncOptions::default());
        way(&mut w, key.clone());
        said.push(
            pool.poll()
                .into_iter()
                .filter(|e| match e {
                    Event::Body { key: k, .. } | Event::BodyFailed { key: k, .. } => k == key,
                    Event::Filled { summary } => summary.key == *key,
                    _ => false,
                })
                .collect(),
        );
    }
    said
}

/// `acc` started on `pool` and synced; the key of the Inbox's newest message.
fn synced(pool: &mut Pool, store: &Store, acc: &AccountConfig, secret: &str) -> MessageKey {
    pool.submit(Job::Start { account: acc.clone(), password: Some(Secret(secret.into())) });
    until(pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    store.load_folder(&acc.id, "INBOX").unwrap().messages[0].key.clone()
}

/// Each way says the message's source, as the server has it, once, and the same as the others.
fn each_way_says(pool: &mut Pool, store: &Store, acc: &AccountConfig, secret: &str, raw: &[u8]) {
    let key = synced(pool, store, acc, secret);
    let said = opened_each_way(pool, store, acc, secret, &key);
    for (way, events) in said.iter().enumerate() {
        assert!(matches!(events.first(), Some(Event::Body { raw: r, .. }) if r == raw), "{}: way {way}: {events:?}", acc.id);
        assert_eq!(events, &said[0], "{}: way {way}", acc.id);
    }
    assert!(store.has_body(&key), "{}: kept", acc.id);
    pool.submit(Job::Remove { account: acc.id.clone() });
}

fn mail(subject: &str) -> Vec<u8> {
    format!("From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@x>\r\nDate: Mon, 6 May 2024 09:00:00 +0000\r\n\r\nHello.\r\n")
        .into_bytes()
}

#[test]
fn a_message_opened_on_an_imap_server_comes_each_way() {
    let st = crate::fake::State::new();
    st.lock().unwrap().add("INBOX", "hello", &[]);
    let raw = st.lock().unwrap().boxes[0].msgs[0].raw.clone();
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::store("opening-imap");
    crate::fakes::fake_dav::looked_for_today(&store, "i1");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let acc = imap_account("i1", port);
    each_way_says(&mut pool, &store, &acc, "pw", &raw);
}

#[test]
fn a_message_opened_on_a_jmap_server_comes_each_way() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, account, serve};
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    let raw = mail("jmap");
    d.deliver(&inbox, &raw, &[], "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::store("opening-jmap");
    crate::fakes::fake_dav::looked_for_today(&store, "j1");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let acc = account("j1", &server.base);
    each_way_says(&mut pool, &store, &acc, PASSWORD, &raw);
}

#[test]
fn a_message_opened_on_microsoft_graph_comes_each_way() {
    use crate::fakes::fake_graph::{Data, TOKEN, account, serve};
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let raw = mail("graph");
    d.deliver(&inbox, &raw, false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::store("opening-graph");
    crate::fakes::fake_dav::looked_for_today(&store, "g1");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let acc = account("g1", &server.base);
    each_way_says(&mut pool, &store, &acc, TOKEN, &raw);
}

#[test]
fn a_message_opened_on_exchange_comes_each_way() {
    use crate::fakes::fake_ews::{Data, PASSWORD, account, serve};
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let raw = mail("ews");
    d.deliver(&inbox, &raw, false, "2024-05-06T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::store("opening-ews");
    crate::fakes::fake_dav::looked_for_today(&store, "e1");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let acc = account("e1", &server.url);
    each_way_says(&mut pool, &store, &acc, PASSWORD, &raw);
}

/// POP mail is downloaded whole: a message not here isn't anywhere. On the main connection the
/// window is told so; on the second, nothing is said (it's left to the main one, here none).
#[test]
fn a_pop_message_not_here_is_said_to_be_nowhere() {
    let server = crate::pop::fake::serve(Vec::new());
    let store = crate::fake::store("opening-pop");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = pop_account(server.port);
    let key = MessageKey { account: "p1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let said = opened_each_way(&mut pool, &store, &acc, "pw", &key);
    let nowhere = Event::BodyFailed { key, error: "the server refused: this message isn't on this computer".into() };
    assert_eq!(said, [vec![nowhere.clone()], vec![nowhere], Vec::new(), Vec::new()]);
}
