//! A data file copied in whole whose folder list can't be read back.

use super::*;

/// Found in review: a data file copied in whole whose folder list couldn't be read back (the disk
/// failing) was said to have no folders, and the import said all was well. No list is said, and
/// the import says why.
#[test]
fn a_copy_whose_folder_list_cant_be_read_back_says_so() {
    let store = Store::failing();
    let inbox = Folder { account: "m1".into(), path: "INBOX".into(), role: mailcraft_model::FolderRole::Inbox, ..Folder::default() };
    store.save_folders("m1", std::slice::from_ref(&inbox)).unwrap();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    store.fail(Some("folders.json"), None);
    said_copied(&store, &p.sink, "m1", "Old.mailcraft", 4, Instant::now());
    store.fail(None, None);
    let seen = p.poll();
    assert!(!seen.iter().any(|e| matches!(e, Event::Folders { .. })), "no list said: {seen:?}");
    let Some(Event::Imported { messages: 4, error: Some(error), done: true, .. }) = seen.last() else { panic!("{seen:?}") };
    assert!(error.contains("folder list"), "{error}");
    // Read back, it's said with the import's end.
    said_copied(&store, &p.sink, "m1", "Old.mailcraft", 4, Instant::now());
    let seen = p.poll();
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if *folders == [inbox.clone()])), "{seen:?}");
    assert!(matches!(seen.last(), Some(Event::Imported { error: None, .. })), "{seen:?}");
}
