//! Exporting an account one of whose folders can't be read.

use super::*;
use crate::data_files::scratch::scratch;

/// A folder whose list can't be read (its file damaged, the disk failing) is said to be left out
/// of the export, and the rest goes in. It was left out with nothing said, and the export said
/// all was well.
#[test]
fn a_folder_that_cant_be_read_is_said_to_be_left_out() {
    let dir = scratch("pst-export-unreadable");
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
}

/// A file added to whose folders can't be read back (after its mail went in) isn't taken for one
/// with no calendar items or no folders: its contacts, calendar, notes and tasks went in again
/// whole, beside the copies it had, and a folder's name could clash with one of its own.
#[test]
fn a_file_that_cant_be_read_back_has_no_items_said_for_it() {
    use super::super::pim::{already, top_folders};
    let dir = scratch("pst-export-read-back");
    let junk = dir.join("junk.pst");
    std::fs::write(&junk, b"!BDN not really a data file").unwrap();
    assert!(already(&junk, &["Calendar"]).is_err());
    assert!(top_folders(&junk).is_err());
    let empty = dir.join("empty.pst");
    write::create_empty(&empty, mailcraft_pst::Format::Unicode).unwrap();
    assert_eq!(already(&empty, &["Calendar"]).unwrap(), []);
    assert!(top_folders(&empty).is_ok());
}

/// What of a file with a note exported into it can't be read, as the step that asks which of
/// the account's notes it has finds it: `None` when it all reads, or the file doesn't open.
fn unread_in(path: &Path) -> Option<&'static str> {
    let any = PstStore::read(Rc::new(PstFile::open(path).ok()?)).ok()?;
    let exported = crate::pst_pim::Named::of(any.as_ref()).exported()?;
    let top = any.open_folder(any.ipm_subtree().ok()?).ok()?;
    let rows: Vec<u32> = top.read_hierarchy_table().ok()??.rows_matrix().map(|row| row.id()).collect();
    let mut notes = None;
    for row in rows {
        match any.open_folder(row) {
            Ok(f) if f.properties().display_name().is_ok_and(|n| n == "Notes") => notes = Some(f),
            Ok(_) => {}
            Err(_) => return Some("a folder"),
        }
    }
    let Ok(table) = notes?.read_contents_table() else { return Some("the list of items") };
    let unread = |row: u32| any.message_properties(row, &[exported]).is_err();
    table?.rows_matrix().any(|row| unread(row.id())).then_some("an item")
}

/// A file added to that reads, but for one folder, one folder's list of items, one item or its
/// record of which folder is its Notes: each is an error of the export, which leaves the file as
/// it was. Each was taken for nothing there (no such folder, no items, no mark on the item, no
/// folder recorded), and the account's notes went in again beside the copies the file had.
#[test]
fn a_file_part_of_which_cant_be_read_back_isnt_added_to() {
    use super::super::pim::{already, folder_for, top_folders};
    use crate::data_files::pst_bytes::block_list;
    use mailcraft_model::pim::{Note, Pim, StoredNote};
    let dir = scratch("pst-export-part-unread");
    let store = Store::open(dir.join("cache")).unwrap();
    let note = Note { uid: "n1".into(), text: "Call back".into(), color: 3, modified: 1, files: Vec::new() };
    let pim = Pim { notes: vec![StoredNote { note, ..Default::default() }], ..Default::default() };
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Out.pst");
    let done = export(&store, "a1", "Mine", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!(done.notes, 1, "{done:?}");
    assert_eq!(already(&file, &["Notes"]).unwrap().len(), 1);
    let whole = std::fs::read(&file).unwrap();
    assert_eq!(unread_in(&file), None);

    // Each block in turn damaged (a byte changed, so its CRC no longer fits).
    let damaged = dir.join("Damaged.pst");
    let mut seen = std::collections::BTreeSet::new();
    for (_, (start, _)) in block_list(&whole) {
        let mut bytes = whole.clone();
        bytes[start] ^= 0xFF;
        std::fs::write(&damaged, &bytes).unwrap();
        // Its record of the Notes folder, with its folders still listed.
        let record = write::special_folder(&damaged, Special::Notes).is_err() && top_folders(&damaged).is_ok();
        if record && seen.insert("the record") {
            assert!(folder_for(&damaged, Special::Notes, true, &[]).is_err(), "a second Notes folder for a record that can't be read");
        }
        let Some(what) = unread_in(&damaged) else { continue };
        if !seen.insert(what) {
            continue;
        }
        assert!(already(&damaged, &["Notes"]).is_err(), "{what} that can't be read taken for none");
        let again = export(&store, "a1", "Mine", &damaged, NONE, &mut |_| {});
        assert!(again.is_err(), "{what}: {again:?}");
        assert!(std::fs::read(&damaged).unwrap() == bytes, "{what}: the file added to was changed");
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["a folder", "an item", "the list of items", "the record"]);
}
