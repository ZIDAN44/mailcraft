//! The counts a worker has synced and not said yet, as folders are renamed here, and the
//! window's, as one is made.

use super::*;

/// Found in review: a folder renamed here as a full sync went had its counts, synced and not
/// said yet, said under its old name, which the list on disk had no more: they were lost, and
/// the folder showed empty. Said before the worker hears of the rename, they wait, and are said
/// under the new name, the folders in it with it.
#[test]
fn counts_said_before_a_rename_reaches_the_worker_follow_it() {
    let store = crate::fake::tests_store("counts-follow-rename");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let folder = |path: &str, unread, total| Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        selectable: true,
        unread,
        total,
        ..Default::default()
    };
    store.save_folders("a1", &[folder("INBOX", 0, 0), folder("Archive", 0, 0), folder("Archive/2025", 0, 0)]).unwrap();
    let acc = AccountConfig { id: "a1".into(), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    // Synced, not said yet.
    w.folders = vec![folder("INBOX", 0, 0), folder("Archive", 1, 2), folder("Archive/2025", 3, 4)];
    w.counts_unsaid = vec!["Archive".into(), "Archive/2025".into()];
    // Renamed here: on disk first, then the worker hears of it.
    store.apply_op("a1", &Op::RenameFolder { from: "Archive".into(), to: "Kept".into() }).unwrap();
    w.say_counts();
    w.renamed("Archive", "Kept");
    w.reload_folders();
    let counts = |path: &str| store.load_folders("a1").unwrap().into_iter().find(|f| f.path == path).map(|f| (f.unread, f.total));
    assert_eq!(counts("Kept"), Some((1, 2)));
    assert_eq!(counts("Kept/2025"), Some((3, 4)));
    assert!(w.counts_unsaid.is_empty(), "{:?}", w.counts_unsaid);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: a POP account's folder made here had its worker write the folder list with
/// its own counts from before over the one on disk, and say it: the Inbox read here (the window
/// writes its count) came back unread. The list keeps the window's counts, and says them.
#[test]
fn a_pop_folder_made_here_keeps_the_counts_of_mail_read_here() {
    let store = crate::fake::tests_store("pop-counts-read-then-made");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let inbox = Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, unread: 3, total: 3, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let acc = AccountConfig { id: "a1".into(), pop: Some(mailcraft_model::ServerConfig::default()), ..Default::default() };
    let mut w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    // One read here, then a folder made.
    store.update_folder_counts("a1", &[("INBOX".into(), 2, 3)]).unwrap();
    w.handle(Job::Apply { account: "a1".into(), op: Op::CreateFolder { path: "Plans".into() } });
    let counts = |l: &[Folder]| l.iter().find(|f| f.path == "INBOX").map(|f| (f.unread, f.total));
    let kept = store.load_folders("a1").unwrap();
    assert!(kept.iter().any(|f| f.path == "Plans"), "{kept:?}");
    assert_eq!(counts(&kept), Some((2, 3)), "the window's count kept");
    let said: Vec<(u32, u32)> = pool
        .poll()
        .into_iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => counts(&folders),
            _ => None,
        })
        .collect();
    assert_eq!(said, [(2, 3)]);
    let _ = std::fs::remove_dir_all(store.root());
}
