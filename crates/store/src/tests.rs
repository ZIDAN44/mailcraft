use mailcraft_model::{Flag, Flags};

use super::*;

mod changes;
mod counts;
mod flagging;
mod folder_uids;
mod lists;
mod memory;
mod merging;
mod pages;
pub(crate) mod process;
pub(crate) mod random;

/// The folders a test made ([`scratch`]), removed when its thread ends, whether the test passed
/// or failed (after the test's own values, its stores among them, have gone).
#[derive(Default)]
struct Scratch(Vec<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for folder in &self.0 {
            // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
            let _ = fs::remove_dir_all(folder);
        }
    }
}

thread_local!(static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::default());

/// A folder for the test, `mailcraft-<name>-<pid>` in the temporary folder (not made yet): what a
/// run before left there is removed now, and the folder with all in it once the test is over.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("mailcraft-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    SCRATCH.with(|s| s.borrow_mut().0.push(folder.clone()));
    folder
}

/// Files handed to other apps go in a folder of the user's own, made when missing; on Linux
/// and the BSDs not in the shared /tmp, and owner-only.
#[test]
fn scratch_folders_are_the_users_own() {
    let a = scratch_dir("test-scratch").unwrap();
    assert!(a.is_dir());
    assert_eq!(scratch_dir("test-scratch"), Some(a.clone()), "the same folder each time");
    if cfg!(all(unix, not(target_os = "macos"))) {
        assert!(!a.starts_with("/tmp"), "{}", a.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let parent = a.parent().unwrap();
            assert_eq!(fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
    let _ = fs::remove_dir(&a);
}

pub(crate) fn temp_store(name: &str) -> Store {
    let dir = scratch(&format!("store-{name}"));
    Store::open(dir).unwrap()
}

fn msg(uid: u32, seen: bool) -> Summary {
    Summary {
        key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
        subject: format!("m{uid}"),
        flags: Flags { seen, ..Default::default() },
        ..Default::default()
    }
}

/// An import adding folders as they finish while the window writes counts: every folder is
/// kept, with the counts last set for it.
#[test]
fn folder_upserts_and_counts_interleaved_lose_nothing() {
    let s = temp_store("upserts");
    let folder = |i: usize| Folder { account: "a1".into(), path: format!("F{i}"), delimiter: Some('/'), total: 1, ..Default::default() };
    s.save_folders("a1", &[folder(0)]).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for i in 1..120 {
                s.upsert_folder("a1", &folder(i)).unwrap();
            }
        });
        scope.spawn(|| {
            for n in 0..120u32 {
                s.update_folder_counts("a1", &[("F0".into(), n % 3, n + 1)]).unwrap();
            }
        });
    });
    let list = s.load_folders("a1").unwrap();
    assert_eq!(list.len(), 120, "{:?}", list.iter().map(|f| &f.path).collect::<Vec<_>>());
    assert_eq!((list[0].unread, list[0].total), (119 % 3, 120));
    // Upserted again (a folder the import finished twice): only its counts change.
    s.upsert_folder("a1", &Folder { total: 7, unread: 2, role: mailcraft_model::FolderRole::Trash, ..folder(5) }).unwrap();
    let f5 = s.load_folders("a1").unwrap().into_iter().find(|f| f.path == "F5").unwrap();
    assert_eq!((f5.total, f5.unread, f5.role), (7, 2, mailcraft_model::FolderRole::Other));
    // Counts for a folder the list doesn't have add nothing.
    s.update_folder_counts("a1", &[("Gone".into(), 1, 1)]).unwrap();
    assert_eq!(s.load_folders("a1").unwrap().len(), 120);
}

#[test]
fn accounts_folders_and_state_round_trip() {
    let s = temp_store("rt");
    assert!(s.load_accounts().unwrap().is_empty());
    let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
    s.save_accounts(std::slice::from_ref(&acc)).unwrap();
    assert_eq!(s.load_accounts().unwrap(), vec![acc]);

    let f = Folder { account: "a1".into(), path: "Projets/Caf&AOk-".into(), delimiter: Some('/'), ..Default::default() };
    s.save_folders("a1", std::slice::from_ref(&f)).unwrap();
    assert_eq!(s.load_folders("a1").unwrap(), vec![f]);

    let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 77, messages: vec![msg(9, false).into(), msg(4, true).into()], window: 0 };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!(back, st);
    assert_eq!(back.unread(), 1);

    let k = &st.messages[0].key;
    assert!(!s.has_body(k));
    s.write_body(k, b"Subject: x\r\n\r\nhi").unwrap();
    assert_eq!(s.read_body(k).unwrap(), b"Subject: x\r\n\r\nhi");
    assert_eq!(s.cached_bodies("a1", "INBOX", 3), vec![9]);
    s.clear_folder("a1", "INBOX").unwrap();
    assert!(!s.has_body(k));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), FolderState::default());
    s.remove_account("a1").unwrap();
}

#[test]
fn damaged_files_start_fresh() {
    let s = temp_store("bad");
    fs::write(s.root().join("accounts.json"), b"{not json").unwrap();
    assert!(s.load_accounts().unwrap().is_empty());
    assert!(s.root().join("accounts.json.bad").exists());
}

#[test]
fn ops_apply_locally_and_persist() {
    let s = temp_store("ops");
    let st = FolderState {
        validity: 3,
        uid_next: 10,
        highest_modseq: 0,
        messages: vec![msg(9, false).into(), msg(4, false).into(), msg(2, false).into()],
        window: 0,
    };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let k9 = st.messages[0].key.clone();
    let k4 = st.messages[1].key.clone();

    let changed = s.apply_op("a1", &Op::SetFlag { keys: vec![k9.clone()], flag: Flag::Seen, on: true }).unwrap();
    assert_eq!(changed, vec!["INBOX".to_string()]);
    assert!(s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);

    s.apply_op("a1", &Op::Move { keys: vec![k4.clone()], to: "Archive".into() }).unwrap();
    assert!(s.load_folder("a1", "INBOX").unwrap().find(4).is_none());

    s.write_body(&k9, b"x").unwrap();
    s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()] }).unwrap();
    assert!(!s.has_body(&k9));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

    // A stale key (old UIDVALIDITY) changes nothing.
    let mut stale = st.messages[2].key.clone();
    stale.validity = 1;
    s.apply_op("a1", &Op::Expunge { keys: vec![stale] }).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

    let ops = vec![Op::Move { keys: vec![k4], to: "Archive".into() }];
    s.save_ops("a1", &ops).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), ops);
    s.save_ops("a1", &[]).unwrap();
    assert!(s.load_ops("a1").unwrap().is_empty());
}

#[test]
fn folder_ops_move_caches_with_folders() {
    let s = temp_store("folders");
    let f = |p: &str| Folder {
        account: "a1".into(),
        path: p.into(),
        delimiter: Some('/'),
        role: mailcraft_model::FolderRole::from_name(p, Some('/')),
        selectable: true,
        ..Default::default()
    };
    s.save_folders("a1", &[f("INBOX"), f("Projects"), f("Projects/Film")]).unwrap();
    let film = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 9 };
    s.save_folder(
        "a1",
        "Projects/Film",
        &FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false).into()], window: 0 },
    )
    .unwrap();
    s.write_body(&film, b"Subject: x\r\n\r\nfilm").unwrap();

    assert_eq!(s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap(), vec!["Clients".to_string()]);
    // Creating twice keeps one.
    s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap();
    assert_eq!(s.load_folders("a1").unwrap().iter().filter(|f| f.path == "Clients").count(), 1);

    // Renaming moves the subfolder and its cached mail along.
    let changed = s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Clients/Work".into() }).unwrap();
    assert_eq!(changed, vec!["Clients/Work".to_string(), "Clients/Work/Film".to_string()]);
    let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert!(paths.contains(&"Clients/Work/Film".to_string()) && !paths.contains(&"Projects".to_string()), "{paths:?}");
    let moved = s.load_folder("a1", "Clients/Work/Film").unwrap();
    assert_eq!(moved.messages.len(), 1);
    assert_eq!(s.read_body(&moved.messages[0].key).unwrap(), b"Subject: x\r\n\r\nfilm");
    assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());

    // Deleting takes the subfolders and their caches.
    s.apply_op("a1", &Op::DeleteFolder { path: "Clients/Work".into() }).unwrap();
    let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(paths, vec!["INBOX".to_string(), "Clients".to_string()]);
    assert!(!s.has_body(&moved.messages[0].key));
}

#[test]
fn outbox_queues_and_forgets() {
    let s = temp_store("outbox");
    assert!(s.outbox("a1").is_empty());
    let item = |id, queued| OutboxItem { id, from: "z@x".into(), recipients: vec!["a@y".into()], queued, ..Default::default() };
    s.outbox_add("a1", &item(7, 20), b"Subject: later\r\n\r\nx").unwrap();
    s.outbox_add("a1", &item(3, 10), b"Subject: first\r\n\r\nx").unwrap();
    let ids: Vec<u64> = s.outbox("a1").iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![3, 7], "oldest first");
    let mut i = s.outbox("a1")[0].clone();
    i.attempts = 2;
    i.error = Some("offline".into());
    s.outbox_update("a1", &i).unwrap();
    assert_eq!(s.outbox("a1")[0], i);
    assert_eq!(s.outbox_raw("a1", 7).unwrap(), b"Subject: later\r\n\r\nx");
    s.outbox_remove("a1", 3).unwrap();
    assert_eq!(s.outbox("a1").len(), 1);
    // Damaged bookkeeping is skipped, not fatal.
    fs::write(s.root().join("a1").join("outbox").join("9.json"), b"{nope").unwrap();
    assert_eq!(s.outbox("a1").len(), 1);
}

#[test]
fn hostile_names_stay_inside_the_root() {
    let s = temp_store("names");
    let key = MessageKey { account: "../../etc".into(), folder: "..\\..\\x".into(), validity: 1, uid: 1 };
    let p = s.body_path(&key);
    assert!(p.starts_with(s.root()));
    assert!(!p.to_string_lossy().contains(".."));
}

/// Message sources read are counted, across the cache's clones (a search that needs none can be
/// shown to read none); lists, flags and listing the cached mail read none.
#[test]
fn message_sources_read_are_counted() {
    let s = Store::memory();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    s.write_body(&key, b"Subject: x\r\n\r\nx").unwrap();
    s.save_folder("a1", "INBOX", &FolderState { validity: 1, ..Default::default() }).unwrap();
    let _ = s.load_folder("a1", "INBOX").unwrap();
    assert!(s.has_body(&key));
    assert_eq!(s.cached_bodies("a1", "INBOX", 1), [1]);
    assert_eq!(s.reads(), 0);
    let other = s.clone();
    assert!(other.read_body(&key).is_some());
    assert!(s.read_body(&MessageKey { uid: 2, ..key.clone() }).is_none());
    assert_eq!((s.reads(), other.reads()), (2, 2));
    assert_eq!(Store::memory().reads(), 0, "a cache of its own counts its own");
}

/// A store that fails says so: a source that isn't kept is told from one that can't be read, and
/// the changes waiting for the server aren't written over when they can't be read.
#[test]
fn a_failing_store_says_so_and_writes_nothing_over() {
    let s = Store::failing();
    let st = FolderState { validity: 3, uid_next: 10, messages: vec![msg(9, false).into()], ..Default::default() };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let k9 = st.messages[0].key.clone();
    s.write_body(&k9, b"x").unwrap();
    let missing = MessageKey { uid: 4, ..k9.clone() };
    assert_eq!(s.load_body(&missing).unwrap(), None, "not kept: no error");
    let waiting = vec![Op::Move { keys: vec![k9.clone()], to: "Archive".into() }];
    s.save_ops("a1", &waiting).unwrap();

    s.fail(Some(".eml"), None);
    assert!(s.load_body(&k9).is_err() && s.load_body(&missing).is_err());
    assert!(s.load_folder("a1", "INBOX").is_ok(), "only what ends so fails");
    s.fail(Some("ops.json"), None);
    assert!(s.add_op("a1", Op::Expunge { keys: vec![k9.clone()] }).is_err());
    assert!(s.remove_op("a1", &waiting[0]).is_err());
    s.fail(None, Some("state.json"));
    assert!(s.save_folder("a1", "INBOX", &FolderState::default()).is_err());
    assert!(s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()] }).is_err());

    s.fail(None, None);
    assert_eq!(s.load_ops("a1").unwrap(), waiting);
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    assert_eq!(s.load_body(&k9).unwrap().as_deref(), Some(&b"x"[..]));
}

/// The sources of a folder past a number go, read as numbers (10 is past 9), in one change, and
/// only that folder's under that validity; a batch and the document naming it go in together.
#[test]
fn sources_past_a_number_are_dropped_by_number() {
    for packed in [false, true] {
        let dir = scratch(&format!("drop-from-{packed}"));
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::open(&dir).unwrap() };
        let key = |folder: &str, validity: u32, uid: u32| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
        let batch =
            [2, 9, 10, 11, 100].iter().map(|u| (key("INBOX", 1, *u), Store::prepare(s.packs("a1"), format!("Subject: {u}\r\n\r\nx").into_bytes())));
        let mut bodies: Vec<_> = batch.collect();
        bodies.push((key("INBOX", 2, 50), Store::prepare(s.packs("a1"), b"Subject: v2\r\n\r\nx".to_vec())));
        bodies.push((key("Sent", 1, 50), Store::prepare(s.packs("a1"), b"Subject: sent\r\n\r\nx".to_vec())));
        s.write_batch("a1", bodies, vec![("import.json", br#"{"version":1}"#.to_vec())], 0).unwrap();
        assert_eq!(s.load_account_doc::<serde_json::Value>("a1", "import.json").unwrap(), serde_json::json!({"version": 1}));
        assert_eq!(s.load_account_doc_bytes("a1", "import.json").unwrap().as_deref(), Some(&br#"{"version":1}"#[..]));
        assert_eq!(s.load_account_doc_bytes("a1", "gone.json").unwrap(), None);
        assert_eq!(s.drop_bodies_from("a1", "INBOX", 1, 10).unwrap(), 3, "{packed}");
        let mut left = s.cached_bodies("a1", "INBOX", 1);
        left.sort_unstable();
        assert_eq!(left, [2, 9], "{packed}");
        assert!(s.has_body(&key("INBOX", 2, 50)) && s.has_body(&key("Sent", 1, 50)), "{packed}: others stay");
        assert_eq!(s.drop_bodies_from("a1", "INBOX", 1, 10).unwrap(), 0);
        assert_eq!(s.drop_bodies_from("a1", "Nowhere", 1, 1).unwrap(), 0);
        drop(s);
    }
}

/// Turning online-only on with one of the account's files unreadable leaves it all where it was,
/// and says so. That file was left behind unread and its only copy removed with the rest (its
/// changes waiting for the server among what could go).
#[test]
fn online_only_with_a_file_that_cant_be_read_keeps_it() {
    let s = Store::failing();
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false).into()], ..Default::default() }).unwrap();
    s.save_ops("a1", &[Op::SetFlag { keys: vec![msg(1, false).key], flag: Flag::Seen, on: true }]).unwrap();
    s.fail(Some("ops.json"), None);
    assert!(s.set_online_only("a1", true).is_err());
    // Files are moved one at a time: the one read last failing, those moved before it go again.
    let dir = s.account_dir("a1");
    let listed = s.files.main.walk(&dir);
    let (Some(first), Some(last)) = (listed.first(), listed.last()) else { panic!("{listed:?}") };
    let last_name = last.file_name().unwrap().to_string_lossy().to_string();
    assert!(!first.ends_with(&last_name), "{listed:?}");
    s.fail(Some(&last_name), None);
    assert!(s.set_online_only("a1", true).is_err());
    assert!(s.files.held.walk(&dir).is_empty(), "nothing of it is left in memory");
    s.fail(None, None);
    assert!(!s.is_online_only("a1"));
    assert_eq!((s.load_ops("a1").unwrap().len(), s.load_folder("a1", "INBOX").unwrap().messages.len()), (1, 1), "still there");
    // Readable again, all of it goes over.
    s.set_online_only("a1", true).unwrap();
    assert_eq!((s.load_ops("a1").unwrap().len(), s.load_folder("a1", "INBOX").unwrap().messages.len()), (1, 1));
}
