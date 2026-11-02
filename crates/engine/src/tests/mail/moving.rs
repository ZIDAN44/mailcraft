//! Engine tests: a long local move in a data file seals the file as it goes, so a crash part-way
//! reopens it at once; a short one doesn't.

use mailcraft_model::{Flags, MessageKey, Summary};

use super::*;

/// A data file account (the sample's, kept in data files) whose Projects folder has `n` messages,
/// each with its source.
fn projects(root: &std::path::Path, n: u32) -> (Store, Session) {
    let store = Store::open_packed(root, root.join("files")).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut st = store.load_folder("sample", "Projects").unwrap();
    st.validity = 7;
    st.messages = (1..=n)
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Projects".into(), validity: 7, uid },
            subject: format!("Report {uid}"),
            date: NOW - i64::from(uid) * 60,
            flags: Flags { seen: true, ..Default::default() },
            ..Default::default()
        })
        .map(Into::into)
        .collect();
    let bodies: Vec<(MessageKey, Vec<u8>)> =
        st.messages.iter().map(|m| (m.key.clone(), format!("Subject: Report {}\r\n\r\nThe figures.\r\n", m.key.uid).into_bytes())).collect();
    for chunk in bodies.chunks(500) {
        store.write_bodies(chunk).unwrap();
    }
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    (store, s)
}

/// A move of 1,200 messages (more than a batch of sources) makes no commit that waits for the
/// disk unsealed: its batches don't wait, and what does (its last batch, the lists, the counts)
/// is sealed. Each batch was a commit that waited unsealed, so a crash part-way checked the whole
/// file as it opened again (105 s for a 37 GB one). Every message is moved with its source.
#[test]
fn a_long_move_leaves_its_data_file_sealed() {
    let root = std::env::temp_dir().join(format!("mailcraft-engine-long-move-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    let _ = std::fs::remove_dir_all(&root);
}
