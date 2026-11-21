//! Engine tests: a long local move in a data file seals the file as it goes, so a crash part-way
//! reopens it at once; a short one doesn't. A long copy is written in batches too.

use mailcraft_model::{Flags, MessageKey, Summary};

use super::*;

/// A data file account (the sample's, kept in data files) whose Projects folder has `n` messages,
/// each with its source.
fn projects(root: &std::path::Path, n: u32) -> (Store, Session) {
    let store = Store::open_packed(root, root.join("files")).unwrap();
    let s = big_projects_with(&store, n, Flags { seen: true, ..Default::default() }, true);
    (store, s)
}

/// A move of 1,200 messages (more than a batch of sources) makes no commit that waits for the
/// disk unsealed: its batches don't wait, and what does (its last batch, the lists, the counts)
/// is sealed. Each batch was a commit that waited unsealed, so a crash part-way checked the whole
/// file as it opened again (105 s for a 37 GB one). Every message is moved with its source.
#[test]
fn a_long_move_leaves_its_data_file_sealed() {
    let root = scratch("long-move");
    let (store, mut s) = projects(&root, 1200);
    // Marks made before the move are written before it (unsealed: the file is sealed as the long
    // write begins); none come in during it.
    assert!(store.flush_marks());
    let paused = store.pause_marks();
    s.run("list.selectAll", &json!({})).unwrap();
    let before = store.commit_times();
    assert_eq!(s.run("mail.move", &json!({"folder": "INBOX"})).unwrap()["moved"], 1200);
    let after = store.commit_times();
    assert_eq!(after.plain.count, before.plain.count, "a commit that waited for the disk wasn't sealed");
    assert!(after.sealed.count > before.sealed.count, "nothing sealed");
    let inbox = store.load_folder("sample", "INBOX").unwrap();
    let moved: Vec<&Summary> = inbox.messages.iter().filter(|m| m.subject.starts_with("Report ")).map(|m| &**m).collect();
    assert_eq!(moved.len(), 1200);
    assert!(moved.iter().all(|m| store.read_body(&m.key).is_some_and(|b| b.starts_with(format!("Subject: {}", m.subject).as_bytes()))));
    assert!(store.load_folder("sample", "Projects").unwrap().messages.is_empty());
    drop(paused);

    // A short move stays as it was: its commits wait for the disk unsealed (the file is sealed
    // once quiet).
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = moved.first().map(|m| m.key.to_id()).unwrap();
    s.run("list.select", &json!({"key": k})).unwrap();
    let before = store.commit_times();
    s.run("mail.move", &json!({"folder": "Projects"})).unwrap();
    assert!(store.commit_times().plain.count > before.plain.count, "a short move sealed");
    drop(s);
    drop(store);
}

/// A copy of 1,200 messages within a data file is written in batches: a handful of commits that
/// wait for the disk, every one sealed, where each message was one (unsealed) of its own. Each
/// copy has its source, read once.
#[test]
fn a_long_copy_is_written_in_batches() {
    let root = scratch("long-copy");
    let (store, mut s) = projects(&root, 1200);
    assert!(store.flush_marks());
    let paused = store.pause_marks();
    s.run("list.selectAll", &json!({})).unwrap();
    let before = store.commit_times();
    assert_eq!(s.run("mail.copy", &json!({"folder": "INBOX"})).unwrap()["copied"], 1200);
    let after = store.commit_times();
    assert_eq!(after.plain.count, before.plain.count, "a commit that waited for the disk wasn't sealed");
    let waited = after.sealed.count.saturating_sub(before.sealed.count);
    assert!((1..10).contains(&waited), "{waited} commits waited for the disk");
    let inbox = store.load_folder("sample", "INBOX").unwrap();
    let copies: Vec<&Summary> = inbox.messages.iter().filter(|m| m.subject.starts_with("Report ")).map(|m| &**m).collect();
    assert_eq!(copies.len(), 1200);
    assert!(copies.iter().all(|m| store.read_body(&m.key).is_some_and(|b| b.starts_with(format!("Subject: {}", m.subject).as_bytes()))));
    assert_eq!(store.load_folder("sample", "Projects").unwrap().messages.len(), 1200, "still where they were");
    drop(paused);
    drop(s);
    drop(store);
}

/// Mail moved in an account kept only here (the sample, a data file imported) leaves nothing of
/// itself behind: its sources went with it, where a copy of each stayed in the data file under
/// its old place for good (2,453 messages moved to Deleted Items and emptied kept their 2 GB).
/// The same with the cache kept as files.
#[test]
fn mail_moved_here_leaves_no_copy_behind() {
    for packed in [true, false] {
        let root = scratch(if packed { "moved-sources-packed" } else { "moved-sources-files" });
        let store = if packed { Store::open_packed(&root, root.join("files")).unwrap() } else { Store::open(&root).unwrap() };
        let mut s = big_projects_with(&store, 3, Flags { seen: true, ..Default::default() }, true);
        let old: Vec<_> = store.load_folder("sample", "Projects").unwrap().messages.iter().map(|m| m.key.clone()).collect();
        assert_eq!(old.len(), 3);
        assert!(old.iter().all(|k| store.read_body(k).is_some()), "each with its source");
        s.run("list.selectAll", &json!({})).unwrap();
        assert_eq!(s.run("mail.move", &json!({"folder": "INBOX"})).unwrap()["moved"], 3);
        let inbox = store.load_folder("sample", "INBOX").unwrap();
        assert_eq!(inbox.messages.iter().filter(|m| m.subject.starts_with("Report ")).filter(|m| store.read_body(&m.key).is_some()).count(), 3);
        for k in &old {
            assert!(store.read_body(k).is_none(), "packed {packed}: the source of {} stayed where it was", k.to_id());
        }
        drop(s);
        drop(store);
    }
}

/// A move undone brings each message back with its source: the copy it went back from, in
/// Deleted Items, goes the same way (nothing of it left behind either).
#[test]
fn a_move_undone_brings_the_mail_back_with_its_source() {
    let mut s = sample_session("moved-sources-undo");
    let store = s.store.clone().expect("a store");
    let moved = key_of(&s, "Storyboard review on Thursday");
    let first = MessageKey::from_id(moved["key"].as_str().unwrap()).unwrap();
    let source = store.read_body(&first).expect("its source");
    s.run("list.select", &moved).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(store.read_body(&first).is_none(), "gone from where it was");
    let trash: Vec<MessageKey> = store.load_folder("sample", "Deleted Items").unwrap().messages.iter().map(|m| m.key.clone()).collect();
    s.run("edit.undo", &json!({})).unwrap();
    let back = store.load_folder("sample", "INBOX").unwrap();
    let again = back.messages.iter().find(|m| m.subject == "Storyboard review on Thursday").expect("back in the Inbox");
    assert_eq!(store.read_body(&again.key), Some(source), "with its source");
    let still: Vec<MessageKey> = store.load_folder("sample", "Deleted Items").unwrap().messages.iter().map(|m| m.key.clone()).collect();
    for k in trash.iter().filter(|k| !still.contains(k)) {
        assert!(store.read_body(k).is_none(), "the copy in Deleted Items of {} stayed", k.to_id());
    }
}
