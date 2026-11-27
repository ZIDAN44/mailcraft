//! Issue #203: a folder whose mail the reading pane doesn't mark read (a shared mailbox, where
//! unread mail is what nobody has dealt with yet), with its subfolders; mail elsewhere is marked
//! as before.

use mailcraft_model::MessageKey;

use super::*;

fn seen(s: &Session, k: &MessageKey) -> bool {
    s.summary(k).is_some_and(|m| m.flags.seen)
}

/// The folder's mail, opened unread in the reading pane.
fn shown_unread(s: &mut Session, folder: &str) -> MessageKey {
    s.run("folder.open", &json!({"account": "sample", "folder": folder})).unwrap();
    let k = s.list.first().map(|m| m.key.clone()).unwrap_or_else(|| panic!("mail in {folder}"));
    s.selection = vec![k.clone()];
    s.run("mail.markUnread", &json!({})).unwrap();
    s.open_message(&k);
    k
}

#[test]
fn a_folder_can_keep_its_mail_unread_in_the_reading_pane() {
    let mut s = sample_session("kept-unread");
    let off = json!({"account": "sample", "folder": "Projects", "on": false});
    assert_eq!(s.run("folder.markReadWhenShown", &off).unwrap()["on"], json!(false));

    // In it and in a folder within it: shown, and left unread, whatever the wait.
    let k = shown_unread(&mut s, "Projects/Short film");
    assert!(!seen(&s, &k), "a subfolder's mail stays unread");
    s.run("view.readingOptions", &json!({"when": "shown", "after": 2})).unwrap();
    let k = shown_unread(&mut s, "Projects/Short film");
    s.clock.now += 5;
    s.tick();
    assert!(!seen(&s, &k), "nor after the wait");
    s.run("view.readingOptions", &json!({"when": "left"})).unwrap();
    let k = shown_unread(&mut s, "Projects/Short film");
    shown_unread(&mut s, "INBOX");
    assert!(!seen(&s, &k), "nor once another is chosen");
    // Elsewhere, as before; and opened in its own window, or marked by hand, it's read.
    s.run("view.readingOptions", &json!({"when": "shown", "after": 0})).unwrap();
    let inbox = shown_unread(&mut s, "INBOX");
    assert!(seen(&s, &inbox), "the Inbox's mail is marked read");
    let k = shown_unread(&mut s, "Projects/Short film");
    s.run("message.popOut", &json!({"key": k.to_id()})).unwrap();
    assert!(seen(&s, &k), "opened in its own window");

    // Set on the folder above, a folder within it can't be turned back on alone.
    let sub_on = json!({"account": "sample", "folder": "Projects/Short film", "on": true});
    assert!(s.run("folder.markReadWhenShown", &sub_on).is_err());
    assert_eq!(s.unread_folder_of("sample", "Projects/Short film").map(|f| f.folder.as_str()), Some("Projects"));
    assert!(s.unread_folder_of("sample", "INBOX").is_none());
    assert!(s.unread_folder_of("sample", "Projects2").is_none(), "a folder whose name only starts the same isn't within it");

    // It follows the folder renamed, and goes with it deleted.
    s.run("folder.rename", &json!({"account": "sample", "folder": "Projects", "name": "Work"})).unwrap();
    assert_eq!(s.unread_folder_of("sample", "Work/Short film").map(|f| f.folder.as_str()), Some("Work"));
    let k = shown_unread(&mut s, "Work/Short film");
    assert!(!seen(&s, &k), "the renamed folder's mail stays unread");
    // Back on (the switch's other way): marked again.
    assert_eq!(s.run("folder.markReadWhenShown", &json!({"account": "sample", "folder": "Work"})).unwrap()["on"], json!(true));
    let k = shown_unread(&mut s, "Work/Short film");
    assert!(seen(&s, &k), "marked read once it's on again");
    s.run("folder.markReadWhenShown", &json!({"account": "sample", "folder": "Work", "on": false})).unwrap();
    s.run("folder.delete", &json!({"account": "sample", "folder": "Work", "permanent": true})).unwrap();
    assert!(s.prefs.unread_folders.is_empty(), "gone with the folder: {:?}", s.prefs.unread_folders);

    // Kept with the settings; older settings read as before.
    s.prefs.unread_folders = vec![crate::FolderRef { account: "sample".into(), folder: "INBOX".into() }];
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.unread_folders, s.prefs.unread_folders);
    let old: crate::Prefs = serde_json::from_value(json!({"markReadOnOpen": true})).unwrap();
    assert!(old.unread_folders.is_empty());
    assert!(s.run("folder.markReadWhenShown", &json!({"account": "sample", "folder": "No such folder", "on": false})).is_err());
}
