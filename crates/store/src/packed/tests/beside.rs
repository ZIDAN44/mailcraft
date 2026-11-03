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
    // A file named from where MailCraft runs is in that folder; one whose folder isn't there to
    // write goes with the others.
    let b = s.copy_beside(std::path::Path::new("Archive.pst")).unwrap();
    let here = std::env::current_dir().unwrap();
    assert!(!b.elsewhere && b.path == here.join("Archive.mailcraft"), "{b:?}");
    assert!(!here.join("Archive.mailcraft-probe").exists());
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

/// Trying the folder leaves nothing at the copy's name, even when a crash came between making
/// the file that tries it and removing it: that one is MailCraft's own, and removed next time.
#[test]
fn trying_the_folder_never_takes_the_copys_name() {
    let t = temp("beside-probe");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Fresh.pst");
    std::fs::write(&pst, b"x").unwrap();
    std::fs::write(docs.join("Fresh.mailcraft-probe"), b"").unwrap();
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    let b = s.copy_beside(&pst).unwrap();
    assert_eq!((b.path, b.elsewhere), (docs.join("Fresh.mailcraft"), false));
    assert!(!docs.join("Fresh.mailcraft").exists() && !docs.join("Fresh.mailcraft-probe").exists());
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A copy found beside the file is passed over when it can't be written (read-only here), and
/// for another copy of its own (`place_new_beside`).
#[test]
fn a_copy_found_is_taken_only_when_it_can_be_written_and_wanted() {
    let t = temp("beside-found");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"!BDN not really a pst").unwrap();
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst"), account("a2", "Archive.pst")]).unwrap();
    s.place_beside("a1", &pst).unwrap();
    put(&s, "a1", "INBOX", 1, "one@x", false);
    note(&s, "a1", &pst, "done");
    s.note_imported("a1").unwrap();
    s.remove_account("a1").unwrap();
    let copy = docs.join("Archive.mailcraft");
    let found = s.copy_beside(&pst).unwrap();
    assert_eq!(found.path, copy);
    assert!(found.found.is_some_and(|f| f.seen), "it notes what came in");
    // Another copy of its own: the next name, the one found left alone.
    let fresh = s.place_new_beside("a2", &pst).unwrap();
    assert_eq!((fresh.path, fresh.found), (docs.join("Archive (2).mailcraft"), None));
    s.remove_account("a2").unwrap();
    // Read-only: passed over (running as root on Unix, everything can be written).
    let mut perms = std::fs::metadata(&copy).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&copy, perms.clone()).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&copy).is_err() {
        let b = s.copy_beside(&pst).unwrap();
        assert_eq!((b.path, b.found, b.elsewhere), (docs.join("Archive (2).mailcraft"), None, false));
    }
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&copy, perms).unwrap();
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// An item with no Message-ID of its own has one made up by the import, naming the account and
/// the file's tag (both other the next time the file is read) and its node: it's known by its
/// node, so read again it's neither doubled nor, deleted here, brought back.
#[test]
fn a_message_with_no_message_id_of_its_own_isnt_doubled_or_brought_back() {
    let t = temp("merge-made-up");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    let gone = put(&s, "a1", "Drafts", 1, "pst.a1.0badcafe.21@mailcraft.invalid", false);
    put(&s, "a1", "Drafts", 2, "pst.a1.0badcafe.22@mailcraft.invalid", false);
    s.note_imported("a1").unwrap();
    s.apply_op("a1", &mailcraft_model::Op::Expunge { keys: vec![gone] }).unwrap();
    let fresh = Store::open_packed(t.join("fresh"), t.join("fresh").join("files")).unwrap();
    fresh.save_accounts(&[account("reread", "Archive.pst")]).unwrap();
    for (uid, node) in [(1, "21"), (2, "22"), (3, "23")] {
        put(&fresh, "reread", "Drafts", uid, &format!("pst.reread.12345678.{node}@mailcraft.invalid"), false);
    }
    let merged = s.merge_from("a1", &fresh, "reread").unwrap();
    assert_eq!(merged.messages, 1, "{merged:?}");
    assert_eq!(ids(&s, "a1", "Drafts"), ["pst.a1.0badcafe.22@mailcraft.invalid", "pst.reread.12345678.23@mailcraft.invalid"]);
    assert_eq!(s.merge_from("a1", &fresh, "reread").unwrap().messages, 0);
    drop((s, fresh));
    let _ = std::fs::remove_dir_all(&t);
}

/// What came in is noted as an import stops part-way too, beside what was noted before: a message
/// deleted here before it went on isn't brought back by a read of the file later.
#[test]
fn what_came_in_before_a_pause_stays_noted() {
    let t = temp("merge-paused");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    let one = put(&s, "a1", "INBOX", 1, "one@x", false);
    // Paused: noted; then deleted here; then it goes on, and ends.
    s.note_imported("a1").unwrap();
    s.apply_op("a1", &mailcraft_model::Op::Expunge { keys: vec![one] }).unwrap();
    put(&s, "a1", "INBOX", 2, "two@x", false);
    s.note_imported("a1").unwrap();
    let fresh = Store::open_packed(t.join("fresh"), t.join("fresh").join("files")).unwrap();
    fresh.save_accounts(&[account("r", "Archive.pst")]).unwrap();
    for (uid, id) in [(1, "one@x"), (2, "two@x"), (3, "three@x")] {
        put(&fresh, "r", "INBOX", uid, id, false);
    }
    assert_eq!(s.merge_from("a1", &fresh, "r").unwrap().messages, 1);
    assert_eq!(ids(&s, "a1", "INBOX"), ["three@x", "two@x"]);
    drop((s, fresh));
    let _ = std::fs::remove_dir_all(&t);
}

/// An account with no note of what came in from its file (opened before MailCraft kept one), or
/// one that can't be read, isn't read into: everything deleted here would come back.
#[test]
fn no_note_of_what_came_in_refuses_a_read_again() {
    let t = temp("merge-no-note");
    let s = Store::open_packed(t.join("app"), t.join("MailCraft Files")).unwrap();
    s.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    put(&s, "a1", "INBOX", 1, "one@x", false);
    let fresh = Store::open_packed(t.join("fresh"), t.join("fresh").join("files")).unwrap();
    fresh.save_accounts(&[account("r", "Archive.pst")]).unwrap();
    put(&fresh, "r", "INBOX", 1, "two@x", false);
    assert!(!s.has_seen("a1"));
    assert!(s.merge_from("a1", &fresh, "r").is_err());
    s.save_account_doc("a1", crate::SEEN_DOC, &"not a note").unwrap();
    assert!(s.merge_from("a1", &fresh, "r").is_err());
    assert!(s.has_seen("a1"), "a damaged note is left where it is");
    assert_eq!(ids(&s, "a1", "INBOX"), ["one@x"], "nothing added");
    drop((s, fresh));
    let _ = std::fs::remove_dir_all(&t);
}

/// A copy says which data folder it's an account of: another data folder finding it with the
/// first one closed is told it's held there, and takes it on only when asked (it's then its
/// own); held no longer once the first one is gone, no longer names it, or let it go.
#[test]
fn a_copy_is_its_data_folders_and_another_is_told() {
    let t = temp("beside-owner");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"!BDN not really a pst").unwrap();
    let copy = docs.join("Archive.mailcraft");
    let (one, two) = (t.join("one"), t.join("two"));
    let s1 = Store::open_packed(&one, t.join("Files 1")).unwrap();
    let id1 = s1.data_folder_id().unwrap();
    assert_eq!(s1.data_folder_id().as_deref(), Some(id1.as_str()), "made once");
    s1.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    s1.place_beside("a1", &pst).unwrap();
    put(&s1, "a1", "INBOX", 1, "one@x", false);
    note(&s1, "a1", &pst, "done");
    drop(s1);

    // The first one closed: the second finds the copy, held by the first.
    let s2 = Store::open_packed(&two, t.join("Files 2")).unwrap();
    let id2 = s2.data_folder_id().unwrap();
    assert_ne!(id1, id2);
    let f = s2.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner.as_str(), f.owner_path.as_str(), f.ours, f.held), (id1.as_str(), one.to_string_lossy().as_ref(), false, true));
    // Its own data folder (MailCraft started again on it) sees it as its own.
    let again = Store::open_packed(&one, t.join("Files 1")).unwrap();
    assert_eq!(again.data_folder_id(), Some(id1.clone()), "the same id, read back");
    // Named by the first as a1's: not one of its to place again.
    drop(again);

    // Taken over: the second's now, and held by it as the first sees it.
    s2.save_accounts(&[account("b1", "Archive.pst")]).unwrap();
    assert!(s2.place_beside("b1", &pst).unwrap().found.is_some_and(|f| f.held));
    s2.claim_folders("b1").unwrap();
    assert_eq!(s2.remove_account("b1").unwrap().as_deref(), Some(copy.as_path()));
    // Removed there: no folder's, anyone's to take on (the first one's list still names it, but
    // its record says no folder has it now).
    let f = s2.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner.as_str(), f.ours, f.held), ("", false, false));

    // Placed again by the second, and compacted by copying: still the second's.
    s2.save_accounts(&[account("b2", "Archive.pst")]).unwrap();
    s2.place_beside("b2", &pst).unwrap();
    s2.claim_folders("b2").unwrap();
    assert!(s2.copy_data_file("b2", &mut |_| true, &mut || true).unwrap().is_some());
    drop(s2);
    let s1 = Store::open_packed(&one, t.join("Files 1")).unwrap();
    let f = s1.copy_beside(&docs.join("Archive.pst")).unwrap();
    // The first still names it a1's: the next name for it, as for any copy an account has.
    assert_eq!(f.path, docs.join("Archive (2).mailcraft"));
    drop(s1);
    // Another data folder's view: held by the second, which names it b2's.
    let s3 = Store::open_packed(t.join("three"), t.join("Files 3")).unwrap();
    let f = s3.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner.as_str(), f.held), (id2.as_str(), true), "compacted, it's still the second's");
    // The second no longer names it: not held.
    std::fs::write(two.join("datafiles.json"), b"{}").unwrap();
    assert!(!s3.copy_beside(&pst).unwrap().found.unwrap().held);
    // A list that can't be read counts as naming it.
    std::fs::write(two.join("datafiles.json"), b"not json").unwrap();
    assert!(s3.copy_beside(&pst).unwrap().found.unwrap().held);
    // Another data folder there now: not held.
    std::fs::write(two.join("datafolder.json"), format!("{{\"id\":\"{}\"}}", "0".repeat(32))).unwrap();
    assert!(!s3.copy_beside(&pst).unwrap().found.unwrap().held);
    // The second gone: not held.
    std::fs::remove_dir_all(&two).unwrap();
    let f = s3.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner.as_str(), f.held), (id2.as_str(), false));
    drop(s3);
    let _ = std::fs::remove_dir_all(&t);
}

/// Taken over by another data folder, the copy is never opened by the one that had it, which
/// still names it: its account there says why, from the start, and nothing of the copy is
/// written. Removed there, the copy stays the other one's.
#[test]
fn a_copy_taken_over_isnt_opened_by_the_folder_that_had_it() {
    let t = temp("beside-taken-over");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"!BDN not really a pst").unwrap();
    let copy = docs.join("Archive.mailcraft");
    let (one, two) = (t.join("one"), t.join("two"));
    let s1 = Store::open_packed(&one, t.join("Files 1")).unwrap();
    s1.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    s1.place_beside("a1", &pst).unwrap();
    put(&s1, "a1", "INBOX", 1, "one@x", false);
    note(&s1, "a1", &pst, "done");
    drop(s1);
    let s2 = Store::open_packed(&two, t.join("Files 2")).unwrap();
    let id2 = s2.data_folder_id().unwrap();
    s2.save_accounts(&[account("b1", "Archive.pst")]).unwrap();
    assert!(s2.place_beside("b1", &pst).unwrap().found.is_some_and(|f| f.held));
    s2.claim_folders("b1").unwrap();
    drop(s2);

    let before = std::fs::read(&copy).unwrap();
    let s1 = Store::packed(&one, t.join("Files 1")).unwrap();
    s1.open_data_files().unwrap();
    let problems = s1.open_progress().problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    let two_path = two.to_string_lossy().into_owned();
    assert!(problems[0].1.contains("another MailCraft data folder") && problems[0].1.contains(&two_path), "{problems:?}");
    let used = s1.load_folders("a1").unwrap_err().to_string();
    assert!(used.contains("another MailCraft data folder") && used.contains(&two_path), "{used}");
    assert!(std::fs::read(&copy).unwrap() == before, "nothing of the copy written");
    // Its account removed here: the record still names the second, which holds it.
    assert_eq!(s1.remove_account("a1").unwrap().as_deref(), Some(copy.as_path()));
    drop(s1);
    let s3 = Store::open_packed(t.join("three"), t.join("Files 3")).unwrap();
    let f = s3.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner.as_str(), f.held), (id2.as_str(), true));
    drop(s3);
    // The second opens it as its own still.
    let s2 = Store::open_packed(&two, t.join("Files 2")).unwrap();
    assert_eq!(s2.load_folders("b1").unwrap().len(), 1);
    drop(s2);
    let _ = std::fs::remove_dir_all(&t);
}

/// A copy's record naming a data folder that isn't on this computer's own drives (a share, a
/// device): the folder isn't looked at (a read from a share could hand the user's sign-in to its
/// server), and the copy counts as held there.
#[test]
fn an_owner_on_a_share_is_never_looked_at() {
    use super::super::owner::{local_root, still_has};
    let copy = std::path::Path::new("C:/Mail/Archive.mailcraft");
    for path in [
        r"\\host.invalid\share\MailCraft",
        "//host.invalid/share/MailCraft",
        r"\/host.invalid\share",
        r"/\host.invalid\share",
        r"\\?\UNC\host.invalid\share",
        r"\\?\C:\MailCraft",
        r"\\.\pipe\MailCraft",
        r"\\.\C:\MailCraft",
        "C:MailCraft",
        "MailCraft",
        "",
        "C:/Mail\0/x",
    ] {
        assert!(local_root(path).is_none(), "{path}");
        assert!(still_has(path, &"0".repeat(32), copy), "{path}");
    }
    assert!(local_root(&format!("C:/{}", "a".repeat(5000))).is_none(), "over-long");
    let local = if cfg!(windows) { r"C:\Users\me\MailCraft" } else { "/home/me/MailCraft" };
    assert_eq!(local_root(local), Some(std::path::Path::new(local)));

    // Through a copy found beside a file: held, and not looked at.
    let t = temp("beside-owner-share");
    let docs = t.join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let pst = docs.join("Archive.pst");
    std::fs::write(&pst, b"!BDN not really a pst").unwrap();
    let s1 = Store::open_packed(t.join("one"), t.join("Files 1")).unwrap();
    s1.save_accounts(&[account("a1", "Archive.pst")]).unwrap();
    s1.place_beside("a1", &pst).unwrap();
    note(&s1, "a1", &pst, "done");
    drop(s1);
    {
        let db = redb::Database::create(docs.join("Archive.mailcraft")).unwrap();
        let tx = db.begin_write().unwrap();
        let record = serde_json::json!({"id": "1".repeat(32), "path": r"\\host.invalid\share\MailCraft"}).to_string();
        tx.open_table(super::super::health::META).unwrap().insert("owner", record.as_bytes()).unwrap();
        tx.commit().unwrap();
    }
    let s2 = Store::open_packed(t.join("two"), t.join("Files 2")).unwrap();
    let f = s2.copy_beside(&pst).unwrap().found.unwrap();
    assert_eq!((f.owner_path.as_str(), f.held), (r"\\host.invalid\share\MailCraft", true));
    drop(s2);
    let _ = std::fs::remove_dir_all(&t);
}

/// A data folder's id file that isn't one is set aside, and a new id made; never fatal.
#[test]
fn a_damaged_data_folder_id_is_set_aside() {
    let t = temp("beside-owner-junk");
    let root = t.join("app");
    std::fs::create_dir_all(&root).unwrap();
    for junk in [&b"not json"[..], br#"{"id":"../../x"}"#, br#"{"id":42}"#, &[0xff, 0xfe, 0x00][..]] {
        std::fs::write(root.join("datafolder.json"), junk).unwrap();
        let s = Store::open_packed(&root, t.join("Files")).unwrap();
        let id = s.data_folder_id().unwrap();
        assert!(id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
        assert_eq!(std::fs::read(root.join("datafolder.json.damaged")).unwrap(), junk, "set aside as it was");
        assert_eq!(super::super::owner::read_folder_id(&root).unwrap(), Some(id));
    }
    // A folder in its place is set aside as well, and its data files work all the same.
    let other = t.join("other");
    std::fs::create_dir_all(other.join("datafolder.json")).unwrap();
    let s = Store::open_packed(&other, t.join("Files 2")).unwrap();
    s.save_accounts(&[account("a1", "x@y")]).unwrap();
    put(&s, "a1", "INBOX", 1, "one@x", false);
    assert!(s.load_folders("a1").unwrap().len() == 1);
    assert!(s.data_folder_id().is_some() && other.join("datafolder.json.damaged").is_dir());
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// The fuzz target's input: its seeds, and junk of every kind, read as a data folder's files.
#[test]
fn a_data_folders_files_read_as_hostile_input() {
    let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/data_folder");
    for e in std::fs::read_dir(&seeds).unwrap() {
        assert!(crate::fuzz_data_folder(&std::fs::read(e.unwrap().path()).unwrap()));
    }
    let id = "0123456789abcdef0123456789abcdef";
    let deep = format!("{{\"a\":{}}}", "[".repeat(200));
    let junk = [
        format!("{{\"id\":\"{}\"}}", id.to_uppercase()),
        format!("{{\"id\":\"{id}0\",\"path\":\"x\"}}"),
        format!("{{\"id\":[\"{id}\"]}}"),
        "{\"path\":\"C:/x\"}".to_string(),
        "[1,2,3]".to_string(),
        deep,
    ];
    for j in &junk {
        assert!(crate::fuzz_data_folder(j.as_bytes()), "{j}");
    }
    assert!(crate::fuzz_data_folder(&[0, 0xff, 0xfe]));
}
