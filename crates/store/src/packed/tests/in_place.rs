//! Store tests: a MailCraft data file the user chose for an account, used where it is: never
//! copied, never removed, and refused when it can't be used there.

use mailcraft_model::{Folder, FolderRole};

use super::*;

/// A data file of account `o1` of the data folder in `t/old`, with a message in its Inbox, an
/// import's note, a change still to make on its server and a message still to send: left as a
/// removed account's (no one's) unless `kept`, when the old data folder still has it.
fn old_file(t: &std::path::Path, kept: bool) -> std::path::PathBuf {
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[account("o1", "zoe@studio.example")]).unwrap();
    old.save_folders("o1", &[Folder { account: "o1".into(), path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "o1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    old.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    let mut st = old.load_folder("o1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(Summary { key: k.clone(), subject: "hi".into(), ..Default::default() }.into());
    old.save_folder("o1", "INBOX", &st).unwrap();
    old.save_account_doc("o1", "import.json", &serde_json::json!({"version": 1, "state": "done"})).unwrap();
    old.save_ops("o1", &[mailcraft_model::Op::Expunge { keys: vec![k] }]).unwrap();
    old.outbox_add("o1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nx").unwrap();
    let file = old.data_file("o1").unwrap();
    if !kept {
        old.remove_account("o1").unwrap();
    }
    file
}

/// What the data folder at `root` lists as each account's data file.
fn listed(root: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root.join("datafiles.json")).unwrap()).unwrap()
}

#[test]
fn a_chosen_data_file_is_the_accounts_where_it_is() {
    let t = temp("in-place");
    let file = old_file(&t, false);
    let size = std::fs::metadata(&file).unwrap().len();
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("p7", "zoe@studio.example")]).unwrap();
    assert_eq!(s.use_in_place("p7", &file).unwrap(), file);
    assert_eq!(s.data_file("p7").as_deref(), Some(file.as_path()), "where it is");
    assert_eq!(listed(&t.join("app"))["p7"], file.to_string_lossy().as_ref(), "named by its whole path");
    assert_eq!(s.take_on_in_place("p7").unwrap(), 1);
    // Its folders and messages are this account's; the old account's work for its server and
    // the import's note go, its unsent mail stays.
    assert_eq!(s.load_folders("p7").unwrap()[0].account, "p7");
    let mine = MessageKey { account: "p7".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    assert_eq!(s.load_folder("p7", "INBOX").unwrap().messages[0].key, mine);
    assert_eq!(s.read_body(&mine).unwrap(), b"Subject: hi\r\n\r\nhello");
    assert!(s.load_ops("p7").unwrap().is_empty());
    assert_eq!(s.load_account_doc::<Option<serde_json::Value>>("p7", "import.json").unwrap(), None);
    assert_eq!(s.outbox("p7").len(), 1, "its unsent mail is this account's to send");
    // Nothing was copied: the data files' folder has no file of it.
    let files = t.join("MailCraft Files");
    assert!(std::fs::read_dir(&files).unwrap().flatten().all(|e| e.path().extension().is_none_or(|x| x != "mailcraft")), "nothing copied in");
    assert!(std::fs::metadata(&file).unwrap().len() >= size);

    // Deleted as an account's data file is (an import thrown away, an account started again):
    // let go of, and left where it is.
    s.delete_account("p7").unwrap();
    assert!(file.is_file(), "the user's own file is never removed");
    assert_eq!(s.data_file("p7"), None);
    // So it can be used again, by another account.
    s.save_accounts(&[account("p8", "zoe@studio.example")]).unwrap();
    assert_eq!(s.use_in_place("p8", &file).unwrap(), file);
    assert_eq!(s.take_on_in_place("p8").unwrap(), 1);
    // Not by two at once.
    let twice = s.use_in_place("p9", &file).unwrap_err();
    assert!(twice.contains("open as an account here"), "{twice}");
    assert_eq!(s.use_in_place("p8", &file).unwrap_err(), "the account has a data file already");
    // Removed, as any account's: it stays, no one's.
    assert_eq!(s.remove_account("p8").unwrap().as_deref(), Some(file.as_path()));

    // Copied in by another account (File › Open), MailCraft's copy is MailCraft's: removed with it.
    assert_eq!(s.import_data_file("c1", &file).unwrap(), 1);
    assert_eq!(s.load_account_doc::<Option<bool>>("c1", crate::IN_PLACE).unwrap(), None);
    let copy = s.data_file("c1").unwrap();
    assert!(copy.starts_with(&files));
    s.delete_account("c1").unwrap();
    assert!(!copy.exists() && file.is_file());
}

#[test]
fn a_data_file_that_cant_be_used_where_it_is_is_refused() {
    let t = temp("in-place-refused");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    let refused = |path: &std::path::Path| s.use_in_place("p1", path).unwrap_err();
    // Not one: another kind of file, junk, a folder, nothing there.
    let pst = t.join("Archive.pst");
    std::fs::write(&pst, b"!BDN").unwrap();
    assert!(refused(&pst).contains("isn't a MailCraft data file"));
    let junk = t.join("junk.mailcraft");
    std::fs::write(&junk, b"not a database at all").unwrap();
    assert!(refused(&junk).contains("wasn't closed properly"), "{}", refused(&junk));
    let dir = t.join("dir.mailcraft");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(refused(&dir).contains("isn't a file"));
    assert!(refused(&t.join("gone.mailcraft")).contains("can't be read"));
    // An account in another data folder that still has it; open there, refused before it's read.
    let file = old_file(&t, true);
    let held = refused(&file);
    assert!(held.contains("another MailCraft data folder") && held.contains(&t.join("old").display().to_string()), "{held}");
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.load_folders("o1").unwrap();
    assert!(refused(&file).contains("open in another program"), "{}", refused(&file));
    // No one's once its account is removed there, but read-only.
    old.remove_account("o1").unwrap();
    drop(old);
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms.clone()).unwrap();
    assert!(refused(&file).contains("can't be written"));
    #[expect(clippy::permissions_set_readonly_false, reason = "the test's own file, made read-only above")]
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();
    // Nothing was named for the account.
    assert_eq!(s.data_file("p1"), None);
    // Free, and in the data files' folder: named as theirs are, so it moves with them.
    let inside = t.join("MailCraft Files").join("zoe@studio.example (old).mailcraft");
    std::fs::copy(&file, &inside).unwrap();
    assert_eq!(s.use_in_place("p1", &inside).unwrap(), inside);
    assert_eq!(listed(&t.join("app"))["p1"], "zoe@studio.example (old).mailcraft");
}
