//! An account's search index found damaged: made again at once, over every folder.

use std::collections::HashSet;

use super::*;

/// Account `a1` of [`fill`]'s two Inbox messages, and one in its Archive (`filed`).
fn two_folders(s: &Store) {
    fill(s, 2);
    let mut folders = s.load_folders("a1").unwrap();
    folders.push(Folder { account: "a1".into(), path: "Archive".into(), selectable: true, ..Default::default() });
    s.save_folders("a1", &folders).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Archive".into(), validity: 9, uid: 1 };
    let m = mailcraft_model::Summary { key: key.clone(), message_id: "old@x".into(), size: 40, ..Default::default() };
    s.save_folder("a1", "Archive", &mailcraft_store::FolderState { validity: 9, uid_next: 2, messages: vec![m.into()], ..Default::default() })
        .unwrap();
    s.write_body(&key, b"Subject: old\r\nMessage-ID: <old@x>\r\n\r\nfiled away\r\n").unwrap();
}

/// The pool's next finished pass of `a1`: how many messages it planned.
fn next_pass(pool: &Pool) -> u64 {
    let seen = until(pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    let Some(Event::Indexed { work, .. }) = seen.last() else { panic!("{seen:?}") };
    work.total
}

/// Found in any order (the messages are read on several threads).
fn sorted(mut uids: Vec<u32>) -> Vec<u32> {
    uids.sort_unstable();
    uids
}

/// Found in review: an index whose meta didn't read back was set aside as a pass began, and made
/// again over the folders that pass was asked for only: asked for the Inbox, the Archive was left
/// out of search till the next ask for every folder. It's made again over every folder.
#[test]
fn an_index_found_damaged_as_a_pass_begins_is_made_again_over_every_folder() {
    let s = crate::fake::store("index-damaged-begins");
    two_folders(&s);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    assert_eq!(next_pass(&pool), 3);
    let meta = s.root().join("a1").join("search").join("v1").join("meta.bin");
    assert!(meta.exists(), "{}", meta.display());
    std::fs::write(&meta, b"not an index").unwrap();
    pool.submit(Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] });
    assert_eq!(next_pass(&pool), 3, "every folder");
    assert_eq!(found(&s, "filed"), [1], "the Archive searched again");
    assert_eq!(sorted(found(&s, "common")), [1, 2]);
    pool.submit(Job::Remove { account: "a1".into() });
}

/// Found in review: a pass that found the index damaged part-way (a page of its texts) set it
/// aside and asked for nothing more: search stayed empty till the next ask, which may name one
/// folder. It's made again at once, over every folder.
#[test]
fn an_index_found_damaged_part_way_is_made_again_at_once() {
    let s = Store::failing();
    two_folders(&s);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    assert_eq!(next_pass(&pool), 3);
    // A third message in the Inbox: its text goes in the page the others' are in, which is read.
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 3 };
    let mut inbox = s.load_folder("a1", "INBOX").unwrap();
    inbox.messages.push(mailcraft_model::Summary { key: key.clone(), message_id: "m3@x".into(), size: 40, date: 3, ..Default::default() }.into());
    inbox.uid_next = 4;
    s.save_folder("a1", "INBOX", &inbox).unwrap();
    s.write_body(&key, b"Subject: m\r\nMessage-ID: <m3@x>\r\n\r\nword3 common\r\n").unwrap();
    s.fail(Some("!a1/search/v1/t/1-0.bin"), None);
    pool.submit(Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] });
    // The pass that found it ends (its one message; its end may be said over by the next), then
    // one over every folder comes.
    until(&pool, "all four indexed", |e| matches!(e, Event::Indexed { work, finished: true, .. } if work.total == 4));
    s.fail(None, None);
    assert_eq!(found(&s, "filed"), [1]);
    assert_eq!(sorted(found(&s, "common")), [1, 2, 3]);
}

/// An index damaged again as it's made again is made again once, not again and again, till a
/// pass ends well; nor when its pass was stopped (the account removed).
#[test]
fn an_index_damaged_again_isnt_made_again_and_again() {
    let s = Store::memory();
    let (mut pending, mut remade, mut unmade) = (indexing::Pending::new(), HashSet::new(), HashSet::new());
    indexing::remake(&s, "a1".into(), true, &mut pending, &mut remade, &mut unmade);
    assert!(pending.is_empty() && remade.is_empty(), "stopped: {pending:?}");
    indexing::remake(&s, "a1".into(), false, &mut pending, &mut remade, &mut unmade);
    assert!(pending.get("a1").is_some_and(|(folders, _)| folders.is_none()), "every folder: {pending:?}");
    pending.clear();
    indexing::remake(&s, "a1".into(), false, &mut pending, &mut remade, &mut unmade);
    assert!(pending.is_empty(), "{pending:?}");
    remade.remove("a1");
    indexing::remake(&s, "a1".into(), false, &mut pending, &mut remade, &mut unmade);
    assert!(pending.contains_key("a1"));
}

/// An index set aside with no pass asked for (its pass was stopped; or it was damaged again as
/// it was made again) has none of the account's folders: the next ask, for one folder, made an
/// index of that folder alone. Till a pass of it ends well, each is over every folder.
#[test]
fn an_index_set_aside_and_not_made_again_is_next_made_over_every_folder() {
    let s = Store::memory();
    let inbox = || Some(std::collections::BTreeSet::from(["INBOX".to_string()]));
    for stopped in [true, false] {
        let (mut pending, mut remade, mut unmade) = (indexing::Pending::new(), HashSet::from(["a1".to_string()]), HashSet::new());
        assert_eq!(indexing::asked(&unmade, "a1", inbox()), ["INBOX"]);
        indexing::remake(&s, "a1".into(), stopped, &mut pending, &mut remade, &mut unmade);
        assert!(pending.is_empty(), "no pass asked for: {pending:?}");
        assert!(indexing::asked(&unmade, "a1", inbox()).is_empty(), "every folder (stopped: {stopped})");
        assert_eq!(indexing::asked(&unmade, "a2", inbox()), ["INBOX"], "another account's as asked");
    }
}
