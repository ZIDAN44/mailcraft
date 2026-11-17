//! Send / Receive as the window shows it: a POP account's download said as it goes and how it
//! ended, its Outbox's too, and Cancel stopping a download between two messages, what came kept.

use mailcraft_model::job::{Secret, Transfer, TransferKind, TransferState};

use super::*;

fn mail(i: usize) -> Vec<u8> {
    format!("From: ana@pop.example\r\nTo: me@pop.example\r\nSubject: M{i}\r\nMessage-ID: <m{i}@x>\r\n\r\nhello {i}\r\n").into_bytes()
}

/// The transfer of `kind` an event says, when it says one.
fn transfer(e: &Event, kind: TransferKind) -> Option<&Transfer> {
    match e {
        Event::Transfer { transfer, .. } if transfer.kind == kind => Some(transfer),
        _ => None,
    }
}

fn ended(e: &Event, kind: TransferKind) -> bool {
    transfer(e, kind).is_some_and(|t| !t.state.is_open())
}

#[test]
fn a_pop_accounts_send_receive_says_how_it_goes_and_stops_when_cancelled() {
    let drop: Vec<(String, Vec<u8>)> = (0..4).map(|i| (format!("u{i}"), mail(i))).collect();
    let server = crate::pop::fake::serve(drop);
    let store = crate::fake::store("transfers");
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { smtp: no_smtp(), ..pop_account(server.port) };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let seen = until(&pool, "the download's end", |e| ended(e, TransferKind::Receiving));
    let receiving: Vec<&Transfer> = seen.iter().filter_map(|e| transfer(e, TransferKind::Receiving)).collect();
    assert!(receiving.iter().any(|t| t.state == TransferState::Working && t.total == 4 && t.bytes_total > 0), "{receiving:?}");
    let last = receiving.last().copied().cloned().unwrap_or_default();
    assert_eq!((last.state, last.done, last.total), (TransferState::Done, 4, 4));
    assert_eq!(last.bytes_done, last.bytes_total);
    // Its Outbox, empty: done, with nothing to send.
    let seen = until(&pool, "the Outbox's end", |e| ended(e, TransferKind::Sending));
    let sent = seen.iter().rev().find_map(|e| transfer(e, TransferKind::Sending)).cloned().unwrap_or_default();
    assert_eq!((sent.state, sent.total), (TransferState::Done, 0));

    // More mail, each message slow to come: cancelled after the first, the rest left on the
    // server for next time.
    {
        let mut d = server.drop.lock().unwrap();
        d.extend((4..10).map(|i| (format!("u{i}"), mail(i))));
    }
    *server.on_retr.lock().unwrap() = Some(Box::new(|_| {
        std::thread::sleep(std::time::Duration::from_millis(150));
        true
    }));
    pool.submit(Job::SyncAll { account: "p1".into() });
    until(&pool, "the second download under way", |e| transfer(e, TransferKind::Receiving).is_some_and(|t| t.done >= 1 && t.total == 6));
    pool.submit(Job::CancelTransfers { account: String::new() });
    let seen = until(&pool, "the download's end", |e| ended(e, TransferKind::Receiving));
    let last = seen.iter().rev().find_map(|e| transfer(e, TransferKind::Receiving)).cloned().unwrap_or_default();
    assert_eq!(last.state, TransferState::Cancelled, "{last:?}");
    assert!(last.done < 6, "stopped before the end: {last:?}");
    let kept = store.load_folder("p1", "INBOX").unwrap().messages.len();
    assert_eq!(kept, 4 + last.done as usize, "what came before the stop is kept");

    // Asked again: it goes on from there, not stopped by the Cancel before.
    *server.on_retr.lock().unwrap() = None;
    pool.submit(Job::SyncAll { account: "p1".into() });
    let seen = until(&pool, "the rest", |e| ended(e, TransferKind::Receiving));
    let last = seen.iter().rev().find_map(|e| transfer(e, TransferKind::Receiving)).cloned().unwrap_or_default();
    assert_eq!(last.state, TransferState::Done, "{last:?}");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 10);
    pool.submit(Job::Remove { account: "p1".into() });
}

/// An IMAP account's Send / Receive: its folders, one by one, then done.
#[test]
fn an_imap_accounts_send_receive_says_its_folders() {
    let st = crate::fake::State::new();
    st.lock().unwrap().add("INBOX", "hello", &[]);
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::store("transfers-imap");
    crate::fakes::fake_dav::looked_for_today(&store, "i1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig { smtp: no_smtp(), ..imap_account("i1", port) };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let seen = until(&pool, "the folders' end", |e| ended(e, TransferKind::Receiving));
    let receiving: Vec<&Transfer> = seen.iter().filter_map(|e| transfer(e, TransferKind::Receiving)).collect();
    assert!(receiving.iter().any(|t| t.state == TransferState::Working && t.folder == "INBOX" && t.folders_total > 0), "{receiving:?}");
    let last = receiving.last().copied().cloned().unwrap_or_default();
    assert_eq!(last.state, TransferState::Done, "{last:?}");
    assert_eq!(last.folders_done, last.folders_total);
    assert_eq!(last.total, 0, "counted in folders, not messages");
    pool.submit(Job::Remove { account: "i1".into() });
}

/// A big download's Inbox is said to have changed as each batch is saved, not only at its end:
/// the window lists its mail as it comes.
#[test]
fn a_big_pop_download_shows_its_mail_as_it_comes() {
    let big = "y".repeat((16 << 20) / 2 + 1);
    let drop: Vec<(String, Vec<u8>)> =
        (0..3).map(|i| (format!("u{i}"), format!("Subject: B{i}\r\nMessage-ID: <b{i}@x>\r\n\r\n{big}\r\n").into_bytes())).collect();
    let server = crate::pop::fake::serve(drop);
    let store = crate::fake::store("transfers-batches");
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { smtp: no_smtp(), ..pop_account(server.port) };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let seen = until(&pool, "the download's end", |e| ended(e, TransferKind::Receiving));
    let inbox_changed = seen.iter().position(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    let all_in = seen.iter().position(|e| transfer(e, TransferKind::Receiving).is_some_and(|t| t.done == 3));
    assert!(inbox_changed.is_some() && inbox_changed < all_in, "the first two listed before the third came: {inbox_changed:?} {all_in:?}");
    pool.submit(Job::Remove { account: "p1".into() });
}
