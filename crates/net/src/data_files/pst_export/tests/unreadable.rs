//! Exporting an account one of whose folders can't be read.

use super::*;

/// A folder whose list can't be read (its file damaged, the disk failing) is said to be left out
/// of the export, and the rest goes in. It was left out with nothing said, and the export said
/// all was well.
#[test]
fn a_folder_that_cant_be_read_is_said_to_be_left_out() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-unreadable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::failing();
    let mut folders = Vec::new();
    for (path, role, subject) in [("INBOX", FolderRole::Inbox, "Kept"), ("Projects", FolderRole::Other, "Left out")] {
        let d =
            mime::Draft { to: vec![EmailAddress::new("Ana", "ana@example.com")], subject: subject.into(), body: "Hi.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("{path}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: path.into(), validity: 1, uid: 1 };
        store.write_body(&key, &raw).unwrap();
        let m = mime::summary_from_full(&raw, key, Flags::default(), 10, 0);
        store.save_folder("a1", path, &FolderState { validity: 1, uid_next: 2, messages: vec![m.into()], ..Default::default() }).unwrap();
        folders.push(mailcraft_model::Folder { account: "a1".into(), path: path.into(), role, selectable: true, total: 1, ..Default::default() });
    }
    store.save_folders("a1", &folders).unwrap();
    let file = dir.join("Out.pst");
    store.fail(Some(&format!("f{:016x}/state.json", mailcraft_store::fnv64("Projects"))), None);
    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {});
    store.fail(None, None);
    let done = done.unwrap();
    assert_eq!((done.messages, done.folders_skipped), (1, 1), "{done:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
