//! An account removed while its worker is at work: nothing of it is written again.

use super::*;

/// Found in review: an account removed while its worker was syncing a folder had the folder's
/// cache written back as the sync ended, and its files came back with the Inbox in them (an
/// account added again by the same name would have had it). The sync writes nothing of it.
#[test]
fn a_folder_synced_as_its_account_is_removed_writes_nothing() {
    let st = crate::fake::State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 0..3 {
            s.add("INBOX", &format!("m{n}"), &[]);
        }
        s.stall_on = Some(("SELECT \"INBOX\"".into(), Duration::from_millis(1000)));
    }
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::tests_store("removed-mid-sync");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None },
        ..Default::default()
    };
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    tx.send(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() }).unwrap();
    let t = std::thread::spawn(move || w.run(rx));
    // Removed while the Inbox is being read, as `Job::Remove` does (its channel dropped).
    let end = Instant::now() + Duration::from_secs(10);
    while !st.lock().unwrap().log.iter().any(|c| c.starts_with("LOGIN")) {
        assert!(Instant::now() < end, "never signed in");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(400));
    store.remove_account("a1").unwrap();
    drop(tx);
    let _ = t.join();
    assert!(st.lock().unwrap().log.iter().any(|c| c.starts_with("SELECT \"INBOX\"")), "the Inbox was read");
    assert_eq!(store.load_folder("a1", "INBOX").map(|f| f.messages.len()).unwrap_or(0), 0, "the Inbox cached again");
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "the folder list written again");
    let said_failed = pool.poll().into_iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    assert!(!said_failed, "a failure said of an account removed");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A proxy to the server at `port` that holds each answer `hold` before passing it on (a slow
/// server: each HTTP request its own connection).
fn slow_proxy(port: u16, hold: Duration) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let at = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", port)) else { continue };
            let (Ok(mut from_client), Ok(mut to_server)) = (client.try_clone(), server.try_clone()) else { continue };
            let (mut from_server, mut to_client) = (server, client);
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut from_client, &mut to_server);
            });
            std::thread::spawn(move || {
                let mut answer = Vec::new();
                let _ = std::io::Read::read_to_end(&mut from_server, &mut answer);
                std::thread::sleep(hold);
                let _ = std::io::Write::write_all(&mut to_client, &answer);
                let _ = to_client.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    at
}

/// `base`, a server's address on this computer (`http://127.0.0.1:port/path`), through a
/// [`slow_proxy`] holding each answer 400 ms.
fn slowed(base: &str) -> String {
    let rest = base.strip_prefix("http://127.0.0.1:").unwrap();
    let (port, path) = rest.split_once('/').map_or((rest, String::new()), |(p, path)| (p, format!("/{path}")));
    format!("http://127.0.0.1:{}{path}", slow_proxy(port.parse().unwrap(), Duration::from_millis(400)))
}

/// Found in review: the same for a JMAP account (and from the code, Microsoft Graph, Exchange
/// and POP ones): removed while its worker synced a folder, the folder's state and the messages
/// it brought in were written again, and its files came back. Its sync writes nothing of it.
#[test]
fn a_jmap_folder_synced_as_its_account_is_removed_writes_nothing() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    let inbox = d.mailbox("Inbox", None, Some("inbox"));
    for n in 1..=3 {
        let raw = format!("From: Zoe <zoe@studio.example>\r\nTo: {USER}\r\nSubject: m{n}\r\nMessage-ID: <j{n}@x>\r\n\r\nHello {n}.\r\n");
        d.deliver(&inbox, raw.as_bytes(), &[], &format!("2026-05-0{n}T09:00:00Z"));
    }
    let server = serve(d);
    let acc = AccountConfig { id: "j1".into(), email: USER.into(), username: USER.into(), jmap: Some(slowed(&server.base)), ..Default::default() };
    removed_mid_sync("removed-mid-jmap-sync", acc, PASSWORD);
}

/// The same for a Microsoft Graph account.
#[test]
fn a_graph_folder_synced_as_its_account_is_removed_writes_nothing() {
    use crate::fakes::fake_graph::{Data, TOKEN, serve};
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    for n in 1..=3 {
        let raw = format!("From: Zoe <zoe@x.example>\r\nTo: me@contoso.example\r\nSubject: m{n}\r\nMessage-ID: <g{n}@x>\r\n\r\nHello {n}.\r\n");
        d.deliver(&inbox, raw.as_bytes(), false, &format!("2026-05-0{n}T09:00:00Z"));
    }
    let server = serve(d);
    let acc = AccountConfig { id: "j1".into(), email: "me@contoso.example".into(), graph: Some(slowed(&server.base)), ..Default::default() };
    removed_mid_sync("removed-mid-graph-sync", acc, TOKEN);
}

/// An account (`j1`) removed while its worker syncs the Inbox: nothing of it is on disk after.
fn removed_mid_sync(name: &str, acc: AccountConfig, secret: &str) {
    let store = crate::fake::tests_store(name);
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let w = Worker::new(acc, Auth::shared(secret), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    let t = std::thread::spawn(move || w.run(rx));
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < end, "the Inbox never synced");
        let e = pool.wait(Duration::from_millis(50));
        if matches!(e, Some(Event::Status { status: SyncStatus::Syncing { folder }, .. }) if folder == "INBOX") {
            break;
        }
    }
    // Removed while the Inbox is being read, as `Job::Remove` does (its channel dropped).
    std::thread::sleep(Duration::from_millis(200));
    store.remove_account("j1").unwrap();
    drop(tx);
    let _ = t.join();
    assert_eq!(store.load_folder("j1", "INBOX").map(|f| f.messages.len()).unwrap_or(0), 0, "the Inbox cached again");
    assert!(!store.root().join("j1").exists(), "the account's files came back");
    let _ = std::fs::remove_dir_all(store.root());
}
