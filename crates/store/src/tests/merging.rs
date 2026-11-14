//! Store tests: a data file read again into its copy when a message of it can't be read.

use mailcraft_model::Folder;

use super::*;

/// A message in `account`'s INBOX of `s` (made when needed), with its source.
fn put(s: &Store, account: &str, uid: u32, id: &str) {
    let mut folders = s.load_folders(account).unwrap();
    if folders.is_empty() {
        folders.push(Folder { account: account.into(), path: "INBOX".into(), selectable: true, ..Default::default() });
        s.save_folders(account, &folders).unwrap();
    }
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: 1, uid };
    s.write_body(&key, format!("Message-ID: <{id}>\r\n\r\nbody of {id}").as_bytes()).unwrap();
    let mut st = s.load_folder(account, "INBOX").unwrap();
    st.validity = 1;
    st.uid_next = st.uid_next.max(uid + 1);
    st.messages.push(Summary { key, message_id: id.into(), subject: id.into(), ..Default::default() }.into());
    sort_newest_first(&mut st.messages);
    s.save_folder(account, "INBOX", &st).unwrap();
}

fn ids(s: &Store) -> Vec<String> {
    let mut ids: Vec<String> = s.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.message_id.clone()).collect();
    ids.sort_unstable();
    ids
}

/// A message of the file read again whose source can't be read there isn't noted as come in:
/// it's said (counted as unreadable), and the next read brings it.
#[test]
fn a_message_whose_source_cant_be_read_comes_in_next_time() {
    let s = Store::memory();
    put(&s, "a1", 1, "one@x");
    s.note_imported("a1").unwrap();
    let fresh = Store::failing();
    for (uid, id) in [(1, "one@x"), (2, "two@x"), (3, "three@x")] {
        put(&fresh, "r", uid, id);
    }
    // Two's source damaged in the file read again.
    fresh.fail(Some("!/1/2.eml"), None);
    let merged = s.merge_from("a1", &fresh, "r").unwrap();
    assert_eq!((merged.messages, merged.unreadable), (1, 1), "{merged:?}");
    assert_eq!(ids(&s), ["one@x", "three@x"]);
    // Read again, it can be: it comes in, and once only.
    fresh.fail(None, None);
    let merged = s.merge_from("a1", &fresh, "r").unwrap();
    assert_eq!((merged.messages, merged.unreadable), (1, 0), "{merged:?}");
    assert_eq!(ids(&s), ["one@x", "three@x", "two@x"]);
    let two = s.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.message_id == "two@x").map(|m| m.key.clone()).unwrap();
    assert!(s.read_body(&two).unwrap().ends_with(b"body of two@x"));
    assert_eq!(s.merge_from("a1", &fresh, "r").unwrap().messages, 0);
}
