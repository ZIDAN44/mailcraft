//! An account removed while its worker is at work: nothing of it is written again.

use super::*;
use crate::fakes::proxy::{Cut, Hang, Proxy};

/// Found in review: an account removed while its worker was syncing a folder had the folder's
/// cache written back as the sync ended, and its files came back with the Inbox in them (an
/// account added again by the same name would have had it). The sync writes nothing of it.
#[test]
fn a_folder_synced_as_its_account_is_removed_writes_nothing() {
    let st = crate::fake::State::new();
    let (hold, holding) = crate::fake::Hold::on("SELECT \"INBOX\"");
    {
        let mut s = st.lock().unwrap();
        for n in 0..3 {
            s.add("INBOX", &format!("m{n}"), &[]);
        }
        s.hold_on = Some(hold);
    }
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::store("removed-mid-sync");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = imap_account("a1", port);
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    tx.send(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() }).unwrap();
    let t = std::thread::spawn(move || w.run(rx));
    // Removed while the Inbox is being read, as `Job::Remove` does (its channel dropped).
    holding.came();
    store.remove_account("a1").unwrap();
    drop(tx);
    holding.go(crate::fake::LetGo::Answer);
    let _ = t.join();
    assert!(st.lock().unwrap().log.iter().any(|c| c.starts_with("SELECT \"INBOX\"")), "the Inbox was read");
    assert_eq!(store.load_folder("a1", "INBOX").map(|f| f.messages.len()).unwrap_or(0), 0, "the Inbox cached again");
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "the folder list written again");
    let said_failed = pool.poll().into_iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. }));
    assert!(!said_failed, "a failure said of an account removed");
}

/// `base`, a server's address on this computer (`http://127.0.0.1:port/path`), through a
/// proxy holding the first request with `marker` in it (the Inbox's listing) until `hold` lets
/// it go.
fn held(base: &str, marker: &'static [u8], hold: &Arc<Hang>) -> String {
    let rest = base.strip_prefix("http://127.0.0.1:").unwrap();
    let (port, path) = rest.split_once('/').map_or((rest, String::new()), |(p, path)| (p, format!("/{path}")));
    let proxy = Proxy { client: Some((marker, Cut::Hold(hold.clone()))), ..Proxy::default() }.to(port.parse().unwrap());
    format!("http://127.0.0.1:{}{path}", proxy.port)
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
    let hold = Arc::new(Hang::default());
    let jmap = Some(held(&server.base, b"Email/query", &hold));
    let acc = AccountConfig { id: "j1".into(), email: USER.into(), username: USER.into(), jmap, ..Default::default() };
    removed_mid_sync("removed-mid-jmap-sync", acc, PASSWORD, &hold);
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
    let hold = Arc::new(Hang::default());
    let graph = Some(held(&server.base, b"/messages", &hold));
    let acc = AccountConfig { id: "j1".into(), email: "me@contoso.example".into(), graph, ..Default::default() };
    removed_mid_sync("removed-mid-graph-sync", acc, TOKEN, &hold);
}

/// An account (`j1`) removed while its worker syncs the Inbox, its listing held by `hold`: nothing
/// of it is on disk after.
fn removed_mid_sync(name: &str, acc: AccountConfig, secret: &str, hold: &Hang) {
    let store = crate::fake::store(name);
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let w = Worker::new(acc, Auth::shared(secret), store.clone(), pool.sink.clone(), SyncOptions::default());
    let (tx, rx) = mpsc::channel();
    let t = std::thread::spawn(move || w.run(rx));
    until(&pool, "the Inbox's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "INBOX"));
    // Removed while the Inbox is being read, as `Job::Remove` does (its channel dropped).
    eventually("the Inbox's listing held", || hold.hung.load(std::sync::atomic::Ordering::Relaxed));
    store.remove_account("j1").unwrap();
    drop(tx);
    hold.closed.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = t.join();
    assert_eq!(store.load_folder("j1", "INBOX").map(|f| f.messages.len()).unwrap_or(0), 0, "the Inbox cached again");
    assert!(!store.root().join("j1").exists(), "the account's files came back");
}

/// Found in the audit: the folder list was written once a look said the account was still
/// there, so a removal coming between the look and the write brought its `folders.json` back
/// (and the account's folder with it). Written while it's kept, it isn't.
#[test]
fn a_folder_list_published_as_its_account_is_removed_writes_nothing() {
    let store = crate::fake::store("removed-folder-list");
    let folder = |path: &str| Folder { account: "a1".into(), path: path.into(), selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder("INBOX")]).unwrap();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = Worker::new(imap_account("a1", 1), Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    w.folders = vec![folder("Archive")];
    // Removed after the worker decided to write its list, before it's written.
    let removing = store.clone();
    crate::worker::sync::BEFORE_FOLDERS_SAVED.with(|b| *b.borrow_mut() = Some(Box::new(move || drop(removing.remove_account("a1")))));
    w.publish_folders();
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "the folder list written again");
    assert!(!store.root().join("a1").exists(), "the account's files came back");
    assert!(!pool.poll().into_iter().any(|e| matches!(e, Event::Folders { .. })), "its folders said");
}
