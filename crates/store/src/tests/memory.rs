//! Store tests: a cache kept in memory, as the browser's is, working as one on disk does.

use super::*;

#[test]
fn a_memory_cache_works_like_the_disk_and_keeps_a_snapshot() {
    let s = Store::memory();
    assert!(s.is_memory());
    let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
    s.save_accounts(std::slice::from_ref(&acc)).unwrap();
    let folders = vec![
        Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() },
        Folder { account: "a1".into(), path: "Projects/Film".into(), delimiter: Some('/'), ..Default::default() },
    ];
    s.save_folders("a1", &folders).unwrap();
    let st = FolderState { validity: 3, uid_next: 3, messages: vec![msg(2, false).into(), msg(1, true).into()], ..Default::default() };
    s.save_folder("a1", "Projects/Film", &st).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 2 };
    s.write_body(
        &key,
        b"Subject: m2

hi",
    )
    .unwrap();
    assert!(s.has_body(&key));
    assert_eq!(s.cached_bodies("a1", "Projects/Film", 3), vec![2]);
    assert!(s.account_size("a1") > 0);

    // A renamed folder takes its cache with it, children too.
    s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
    assert_eq!(s.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
    let moved = MessageKey { folder: "Work/Film".into(), ..key.clone() };
    assert_eq!(
        s.read_body(&moved).unwrap(),
        b"Subject: m2

hi"
    );
    assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());
    s.apply_op("a1", &Op::SetFlag { keys: vec![MessageKey { uid: 2, ..moved.clone() }], flag: Flag::Seen, on: true }).unwrap();
    assert!(s.load_folder("a1", "Work/Film").unwrap().messages.iter().all(|m| m.flags.seen));

    let item = OutboxItem { id: 5, from: "z@x".into(), recipients: vec!["a@y".into()], ..Default::default() };
    s.outbox_add("a1", &item, b"x").unwrap();
    assert_eq!(s.outbox("a1"), vec![item]);
    s.outbox_remove("a1", 5).unwrap();
    assert!(s.outbox("a1").is_empty());

    // What outlasts the page: here everything but message sources.
    let snap = s.snapshot(|p| !p.ends_with(".eml"));
    assert!(snap.iter().any(|(p, _)| p == "accounts.json"));
    assert!(snap.iter().all(|(p, _)| !p.ends_with(".eml")));
    let back = Store::memory_from(snap);
    assert_eq!(back.load_accounts().unwrap(), vec![acc]);
    assert_eq!(back.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
    assert!(!back.has_body(&moved));
    // Paths from a snapshot stay inside the cache.
    let odd = Store::memory_from(vec![("../accounts.json".into(), b"[]".to_vec()), ("a//b".into(), b"x".to_vec())]);
    assert!(odd.snapshot(|_| true).is_empty());

    s.remove_account("a1").unwrap();
    assert!(s.load_folders("a1").unwrap().is_empty() && s.account_size("a1") == 0);
    // Clones share the cache; a damaged document starts fresh.
    let other = s.clone();
    other.save_doc("rules.json", &vec![1, 2]).unwrap();
    assert_eq!(s.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![1, 2]);
    let bad = Store::memory_from(vec![("rules.json".into(), b"{oops".to_vec())]);
    assert!(bad.load_doc::<Vec<i32>>("rules.json").unwrap().is_empty());
}

/// A memory cache lists a folder from where its files start: only the files directly in it, not
/// a sibling whose name starts the same, nor what's in a folder inside; and a folder with files
/// under it exists.
#[test]
fn a_memory_folder_lists_only_its_own_files() {
    let s = Store::memory();
    let key = |folder: &str, validity: u32, uid: u32| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
    let bodies: Vec<(MessageKey, Vec<u8>)> = [key("INBOX", 7, 1), key("INBOX", 7, 2), key("INBOX", 70, 3), key("INBOX2", 7, 4), key("INB", 7, 5)]
        .into_iter()
        .map(|k| (k, b"Subject: x\r\n\r\nx".to_vec()))
        .collect();
    s.write_bodies(bodies).unwrap();
    let mut cached = s.cached_bodies("a1", "INBOX", 7);
    cached.sort_unstable();
    assert_eq!(cached, [1, 2]);
    assert_eq!(s.cached_bodies("a1", "INBOX", 70), [3]);
    assert_eq!(s.cached_bodies("a1", "INBOX2", 7), [4]);
    assert!(s.cached_bodies("a1", "INBOX", 9).is_empty() && s.cached_bodies("a1", "Other", 7).is_empty());
    let dir = s.folder_dir("a1", "INBOX");
    assert!(s.files.exists(&dir) && s.files.exists(&dir.join("70")));
    assert!(!s.files.exists(&dir.join("8")) && !s.files.exists(&s.folder_dir("a1", "Other")));
    assert_eq!(s.files.main.walk(&dir).len(), 3);
}
