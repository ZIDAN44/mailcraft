//! A copy found beside an Outlook file that can't be opened: on a cache whose files can be made
//! to fail (the copies beside real files are in `tests/accounts/beside.rs`).

use mailcraft_store::Store;

use super::*;

const NOW: i64 = 1_792_000_000;
const FILE: &str = "/mail/Archive.pst";

/// A session with the account made for the file's copy, as `import_pst` makes it before the
/// copy is opened, and a mailbox that was there before.
fn with_account() -> (Session, Store, String) {
    let store = Store::failing();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &serde_json::json!({"email": "me@home.example", "local": true})).unwrap();
    let id = s.new_account_id();
    s.data_file_account(&id, FILE);
    assert!(s.account(&id).is_some());
    (s, store, id)
}

/// A copy whose folder list can't be read isn't opened with no folders, saying nothing: the
/// account made for it goes again, nothing is kept of it, and the command says why. The same
/// when its folders can't be made this account's (a copy that was another account's).
#[test]
fn a_copy_whose_folders_cant_be_read_isnt_opened() {
    for was in ["", "a9"] {
        let (mut s, store, id) = with_account();
        let before = s.accounts.len();
        let found = Found { state: "done".into(), account: if was.is_empty() { id.clone() } else { was.into() }, ..Default::default() };
        store.fail(Some("folders.json"), None);
        let r = s.open_copy(&id, FILE, &found, None);
        store.fail(None, None);
        assert!(matches!(&r, Err(CmdError::Failed(e)) if e.contains("copy of this file couldn't be opened")), "{was:?}: {r:?}");
        assert!(s.account(&id).is_none(), "{was:?}: the account made for it is gone again");
        assert_eq!(s.accounts.len(), before - 1, "{was:?}: the others stay");
        assert!(!s.importing.sources.contains_key(&id) && !s.importing.running.contains_key(&id), "{was:?}");
        assert!(!matches!(&s.view, View::Folder { account, .. } if *account == id), "{was:?}");
        // Readable again: opened, with its folders.
        s.data_file_account(&id, FILE);
        let inbox = Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
        store.save_folders(&id, std::slice::from_ref(&inbox)).unwrap();
        let opened = s.open_copy(&id, FILE, &Found { state: "done".into(), account: id.clone(), ..Default::default() }, None).unwrap();
        assert_eq!(opened["reopened"].as_bool(), Some(true), "{opened}");
        assert_eq!(s.account(&id).map(|a| a.folders.len()), Some(1));
    }
}
