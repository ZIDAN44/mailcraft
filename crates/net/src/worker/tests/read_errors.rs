//! What a worker keeps when the cache can't be read (the disk failing): a read error is never
//! taken for "nothing there", and nothing is cleared on it.

use super::*;

fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), selectable: true, ..Default::default() }
}

/// A folder `path` cached with one message.
fn cache_one(store: &Store, path: &str) {
    let key = MessageKey { account: "a1".into(), folder: path.into(), validity: 7, uid: 1 };
    let m = mailcraft_model::Summary { key, subject: "kept".into(), ..Default::default() };
    store
        .save_folder("a1", path, &mailcraft_store::FolderState { validity: 7, uid_next: 2, messages: vec![m.into()], ..Default::default() })
        .unwrap();
}

/// Found in review: a folder renamed here offline (Archive to Plans, not sent yet), not on the
/// server's list, lost its cache when the changes waiting, or the list on disk, couldn't be read:
/// taken for nothing waiting, and its mail was downloaded again. It keeps it till a listing can
/// tell.
#[test]
fn a_folder_kept_for_changes_waiting_keeps_its_cache_when_they_cant_be_read() {
    let st = crate::fake::State::new();
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    cache_one(&store, "Plans");
    store.add_op("a1", Op::RenameFolder { from: "Archive".into(), to: "Plans".into() }).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = imap_account("a1", port);
    let mut w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    w.connect().unwrap();
    let cached = || store.load_folder("a1", "Plans").map(|f| f.messages.len()).unwrap_or(0);
    // The list on disk can't be read, then the changes waiting can't.
    for unreadable in ["folders.json", "ops.json"] {
        store.fail(Some(unreadable), None);
        assert!(w.relist());
        store.fail(None, None);
        assert_eq!(cached(), 1, "{unreadable}: the cache cleared");
        assert_eq!(w.gone_meanwhile, ["Plans"], "{unreadable}: looked at again at the next listing");
    }
    // Read, the list on disk (the server's now) doesn't have it: cleared, as ever.
    assert!(w.relist());
    assert_eq!(cached(), 0);
    assert!(w.gone_meanwhile.is_empty());
}

/// Found in review: the folder list read again (a folder made here) that couldn't be read left
/// the worker with no folders, and a POP account's wrote that over the list on disk. Its own
/// stays as it was.
#[test]
fn a_folder_list_that_cant_be_read_again_leaves_the_workers_as_it_was() {
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    store.fail(Some("folders.json"), None);
    assert!(!w.reload_folders());
    store.fail(None, None);
    assert_eq!(w.folders, [folder("INBOX"), folder("Plans")]);
    assert!(w.reload_folders());
}

/// Found in review: a JMAP account's message numbers were pruned against changes (or a folder)
/// that couldn't be read: the number of a message a change waited on was forgotten, and the
/// change could never be sent. Nothing is pruned then.
#[test]
fn jmap_numbers_are_kept_when_the_changes_cant_be_read() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, account, serve};
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    let server = serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = account("a1", &server.base);
    let mut w = Worker::new(acc, Auth::shared(PASSWORD), store.clone(), pool.sink.clone(), SyncOptions::default());
    w.connect().unwrap();
    w.folders = vec![folder("INBOX")];
    cache_one(&store, "INBOX");
    // Message 2, moved here, is waiting to be moved on the server.
    let (cached, waiting) = (w.jstate.number("M-cached"), w.jstate.number("M-waiting"));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: waiting };
    store.add_op("a1", Op::SetFlag { keys: vec![key], flag: mailcraft_model::Flag::Seen, on: true }).unwrap();
    assert_eq!((cached, waiting), (1, 2));
    for unreadable in ["ops.json", "state.json"] {
        store.fail(Some(unreadable), None);
        w.prune_jmap_numbers();
        store.fail(None, None);
        assert_eq!(w.jstate.id(waiting), Some("M-waiting"), "{unreadable}: forgotten");
        assert_eq!(w.jstate.id(cached), Some("M-cached"), "{unreadable}: forgotten");
    }
    // Read, the numbers not used go, as ever.
    let gone = w.jstate.number("M-gone");
    w.prune_jmap_numbers();
    assert_eq!((w.jstate.id(waiting), w.jstate.id(cached), w.jstate.id(gone)), (Some("M-waiting"), Some("M-cached"), None));
}
