//! Store tests: the copy of an Outlook data file kept beside it, found again, and read into
//! again once the file has changed.

use mailcraft_model::pim::{Pim, StoredNote};
use mailcraft_model::{Folder, FolderRole};

use super::*;

/// A message in `account`'s `folder` (made when needed), as an import writes one.
fn put(s: &Store, account: &str, folder: &str, uid: u32, id: &str, seen: bool) -> MessageKey {
    let mut folders = s.load_folders(account).unwrap();
    if !folders.iter().any(|f| f.path == folder) {
        let role = if folder == "INBOX" { FolderRole::Inbox } else { FolderRole::Other };
        folders.push(Folder { account: account.into(), path: folder.into(), role, selectable: true, delimiter: Some('/'), ..Default::default() });
        s.save_folders(account, &folders).unwrap();
    }
    let key = MessageKey { account: account.into(), folder: folder.into(), validity: 1, uid };
    s.write_body(&key, format!("Message-ID: <{id}>\r\nSubject: {id}\r\n\r\nbody of {id}").as_bytes()).unwrap();
    let mut st = s.load_folder(account, folder).unwrap();
    st.validity = 1;
    st.uid_next = st.uid_next.max(uid + 1);
    let flags = Flags { seen, ..Default::default() };
    st.messages.push(Summary { key: key.clone(), message_id: id.into(), subject: id.into(), flags, ..Default::default() }.into());
    s.save_folder(account, folder, &st).unwrap();
    key
}

/// The note an import of `source` leaves in the account it fills.
fn note(s: &Store, account: &str, source: &std::path::Path, state: &str) {
    let (size, modified) = crate::file_stamp(source).unwrap();
    let note = serde_json::json!({"version": 1, "state": state, "source": {"path": source.to_string_lossy(), "size": size, "modified": modified}});
    s.save_account_doc(account, "import.json", &note).unwrap();
}

fn ids(s: &Store, account: &str, folder: &str) -> Vec<String> {
    let mut v: Vec<String> = s.load_folder(account, folder).unwrap().messages.iter().map(|m| m.message_id.clone()).collect();
    v.sort();
    v
}

#[test]
fn an_opened_files_copy_is_kept_beside_it_and_found_again() {
    let t = temp("beside");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"!BDN not really a pst").unwrap();
    // Another file of that name beside it, not MailCraft's.
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst")]).unwrap();

    // Where it goes: beside the file, nothing made there yet.
    let b = s.copy_beside(&pst).unwrap();
    assert_eq!((b.path.clone(), b.found.clone(), b.elsewhere), (docs.join("Archive.mailcraft"), None, false));
    assert!(!docs.join("Archive.mailcraft").exists(), "only looked: the file made to try the folder is gone");
    // Taken as the account's: its mail goes there, nothing in MailCraft's own folder.
    assert_eq!(s.place_beside("a1", &pst).unwrap().path, docs.join("Archive.mailcraft"));
    assert_eq!(s.place_beside("a1", &pst), None, "it has one already");
    put(&s, "a1", "INBOX", 1, "one@x", false);
    note(&s, "a1", &pst, "done");
    assert_eq!(s.data_file("a1").unwrap(), docs.join("Archive.mailcraft"));
    assert!(docs.join("Archive.mailcraft").is_file());
    assert!(!t.join("MailCraft Files").join("Archive.mailcraft").exists());
    // While it's the account's, opening the file again gets a second copy beside the first.
    assert_eq!(s.copy_beside(&pst).unwrap().path, docs.join("Archive (2).mailcraft"));
    // The account removed: its copy stays, and opening the file again finds it.
    assert_eq!(s.remove_account("a1").unwrap().as_deref(), Some(docs.join("Archive.mailcraft").as_path()));
    let found = s.copy_beside(&pst).unwrap();
    assert_eq!(found.path, docs.join("Archive.mailcraft"));
    let f = found.found.unwrap();
    assert_eq!((f.state.as_str(), f.changed, f.account.as_str()), ("done", false, "a1"));
    // Opened by MailCraft started again, too (the account list kept apart from the files).
    drop(s);
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a3", "Archive.pst")]).unwrap();
    let placed = s.place_beside("a3", &pst).unwrap();
    assert!(placed.found.is_some());
    assert_eq!(s.claim_folders("a3").unwrap(), 1);
    assert_eq!(s.load_folders("a3").unwrap()[0].account, "a3");
    assert_eq!(s.load_folder("a3", "INBOX").unwrap().messages[0].key.account, "a3", "its messages are this account's now");
    // The file changed since: told so.
    s.remove_account("a3").unwrap();
    std::fs::write(&pst, b"!BDN not really a pst, and longer now").unwrap();
    assert!(s.copy_beside(&pst).unwrap().found.unwrap().changed);
    // A file of the copy's name that isn't a copy of this one: passed over.
    let other = docs.join("Notes.pst");
    std::fs::write(&other, b"x").unwrap();
    std::fs::write(docs.join("Notes.mailcraft"), b"someone else's file").unwrap();
    assert_eq!(s.copy_beside(&other).unwrap().path, docs.join("Notes (2).mailcraft"));
    assert_eq!(std::fs::read(docs.join("Notes.mailcraft")).unwrap(), b"someone else's file", "left as it was");
    // A file named by no folder, or one whose folder isn't there to write: with the others.
    let b = s.copy_beside(std::path::Path::new("Archive.pst")).unwrap();
    assert!(b.elsewhere && b.path == t.join("MailCraft Files"));
    let b = s.copy_beside(&t.join("gone").join("Archive.pst")).unwrap();
    assert!(b.elsewhere, "{b:?}");
    // A cache that doesn't keep data files has nowhere to put one.
    assert_eq!(Store::memory().copy_beside(&pst), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A file whose folder can't be made a file in (here, a disc taken out: the folder isn't there
/// any more): the copy goes with the other data files, named as any other, on every system.
#[test]
fn a_folder_gone_has_the_copy_go_with_the_others() {
    let t = temp("beside-gone");
    let pst = t.join("disc").join("Old.pst");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Old.pst")]).unwrap();
    assert!(s.copy_beside(&pst).is_some_and(|b| b.elsewhere && b.found.is_none()));
    let b = s.place_beside("a1", &pst).unwrap();
    assert!(b.elsewhere && b.found.is_none(), "{b:?}");
    assert_eq!(b.path, t.join("MailCraft Files"));
    put(&s, "a1", "INBOX", 1, "one@x", false);
    assert_eq!(s.data_file("a1").unwrap(), t.join("MailCraft Files").join("Old.mailcraft"));
    assert!(!t.join("disc").exists(), "nothing was made where the file was");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A folder that can't be written (read-only here): the copy goes with the other data files,
/// named as any other.
#[cfg(unix)]
#[test]
fn a_folder_that_cant_be_written_has_the_copy_go_with_the_others() {
    use std::os::unix::fs::PermissionsExt;
    let t = temp("beside-ro");
    let ro = t.join("cd");
    std::fs::create_dir_all(&ro).unwrap();
    let pst = ro.join("Old.pst");
    std::fs::write(&pst, b"x").unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Old.pst")]).unwrap();
    let b = s.place_beside("a1", &pst).unwrap();
    // Running as root, everything can be written.
    if b.elsewhere {
        assert_eq!(b.path, t.join("MailCraft Files"));
        put(&s, "a1", "INBOX", 1, "one@x", false);
        assert_eq!(s.data_file("a1").unwrap(), t.join("MailCraft Files").join("Old.mailcraft"));
    }
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn a_file_read_again_adds_what_it_has_new_and_keeps_what_was_done_here() {
    let t = temp("merge");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    // The first read: three messages and a note.
    let one = put(&s, "a1", "INBOX", 1, "one@x", false);
    let two = put(&s, "a1", "INBOX", 2, "two@x", false);
    let three = put(&s, "a1", "INBOX", 3, "three@x", false);
    let mut pim = Pim::default();
    pim.notes.push(StoredNote {
        note: mailcraft_model::pim::Note { uid: "pst-a1-0@mailcraft.invalid".into(), text: "old".into(), ..Default::default() },
        ..Default::default()
    });
    s.save_account_doc("a1", "pim.json", &pim).unwrap();
    s.note_imported("a1").unwrap();
    // Then, in MailCraft: one read, one moved, one deleted, and the note deleted.
    s.apply_op("a1", &mailcraft_model::Op::SetFlag { keys: vec![one.clone()], flag: mailcraft_model::Flag::Seen, on: true }).unwrap();
    let kept = s.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.key == two).cloned().unwrap();
    put(&s, "a1", "Kept", 1, "two@x", false);
    s.apply_op("a1", &mailcraft_model::Op::Move { keys: vec![kept.key.clone()], to: "Kept".into() }).unwrap();
    s.apply_op("a1", &mailcraft_model::Op::Expunge { keys: vec![three] }).unwrap();
    s.save_account_doc("a1", "pim.json", &Pim::default()).unwrap();

    // The file read again: it has all three still, a new one, a new folder, and a new note.
    let fresh = Store::open_packed(t.join("fresh"), t.join("fresh").join("files")).unwrap();
    fresh.save_accounts(&[account("r", "Archive.pst")]).unwrap();
    for (uid, id) in [(1, "one@x"), (2, "two@x"), (3, "three@x"), (4, "four@x")] {
        put(&fresh, "r", "INBOX", uid, id, false);
    }
    put(&fresh, "r", "Projects", 1, "five@x", true);
    let mut theirs = Pim::default();
    for (i, text) in ["old", "new"].iter().enumerate() {
        let note = mailcraft_model::pim::Note { uid: format!("pst-r-{i}@mailcraft.invalid"), text: (*text).into(), ..Default::default() };
        theirs.notes.push(StoredNote { note, ..Default::default() });
    }
    fresh.save_account_doc("r", "pim.json", &theirs).unwrap();

    let merged = s.merge_from("a1", &fresh, "r").unwrap();
    assert_eq!((merged.messages, merged.notes, merged.cards + merged.events + merged.tasks), (2, 1, 0), "{merged:?}");
    assert_eq!(merged.folders, ["INBOX", "Projects"]);
    // What was done here stays: read stays read, moved stays moved, deleted stays deleted.
    assert_eq!(ids(&s, "a1", "INBOX"), ["four@x", "one@x"]);
    assert_eq!(ids(&s, "a1", "Kept"), ["two@x"]);
    let inbox = s.load_folder("a1", "INBOX").unwrap();
    assert!(inbox.messages.iter().find(|m| m.message_id == "one@x").unwrap().flags.seen);
    // The new ones came with their sources, under numbers of this account's.
    let four = inbox.messages.iter().find(|m| m.message_id == "four@x").unwrap();
    assert_eq!((four.key.account.as_str(), four.key.uid), ("a1", 4));
    assert!(s.read_body(&four.key).unwrap().ends_with(b"body of four@x"));
    let folders = s.load_folders("a1").unwrap();
    let projects = folders.iter().find(|f| f.path == "Projects").unwrap();
    assert_eq!((projects.account.as_str(), projects.total, projects.unread), ("a1", 1, 0));
    assert_eq!(folders.iter().find(|f| f.path == "INBOX").map(|f| f.total), Some(2));
    let pim: Pim = s.load_account_doc("a1", "pim.json").unwrap();
    assert_eq!(pim.notes.iter().map(|n| n.note.text.as_str()).collect::<Vec<_>>(), ["new"], "the note deleted here stays deleted");
    assert!(pim.notes[0].note.uid.starts_with("pst-a1-"));
    // Read once more: nothing new.
    let again = s.merge_from("a1", &fresh, "r").unwrap();
    assert_eq!((again.messages, again.notes), (0, 0));
    drop((s, fresh));
    let _ = std::fs::remove_dir_all(&t);
}
