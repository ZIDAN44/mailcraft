//! Engine tests: the buttons a message in the list shows, as chosen (issue #274), and Snooze to a
//! time of the user's own.

use super::*;

/// Issue #274: Archive and Pin are a row's buttons too, acting on that message alone (what's
/// selected and open stays); the choice is kept, each once, as many as a row holds.
#[test]
fn a_row_archives_and_pins_its_message_alone() {
    let mut s = sample_session("row-archive");
    s.run("list.select", &key_of(&s, "Character turnarounds")).unwrap();
    let (selected, open) = (s.selection.clone(), s.reading.as_ref().map(|r| r.key.clone()));
    let target = key_of(&s, "Open call: winter group show");
    let id = target["key"].as_str().unwrap().to_string();
    s.run("list.rowAction", &json!({"key": id, "action": "pin"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Open call: winter group show" && m.flags.pinned()), "pinned");
    assert_eq!(s.selection, selected, "the selection stays");
    s.run("list.rowAction", &json!({"key": id, "action": "pin"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Open call: winter group show" && !m.flags.pinned()), "and unpinned");
    s.run("list.rowAction", &json!({"key": id, "action": "archive"})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Open call: winter group show"), "archived out of the Inbox");
    let archive = s.account("sample").unwrap().folder(FolderRole::Archive).map(|f| f.path.clone()).expect("an Archive folder");
    assert!(s.folder_state("sample", &archive).messages.iter().any(|m| m.subject == "Open call: winter group show"));
    assert_eq!(s.selection, selected, "the selection stays");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), open, "so does the message open");
    assert!(s.run("list.rowAction", &json!({"key": id, "action": "snooze"})).is_err());

    assert_eq!(s.prefs.row_actions, crate::RowAction::DEFAULT, "Delete, Flag and Read until chosen");
    let r = s.run("options.rowActions", &json!({"actions": ["pin", "archive", "delete", "archive"]})).unwrap();
    assert_eq!(r["actions"], json!(["delete", "archive", "pin"]), "each once, in a row's order");
    assert!(s.run("options.rowActions", &json!({"actions": ["delete", "archive", "flag", "read", "pin"]})).is_err(), "no room for five");
    assert!(s.run("options.rowActions", &json!({"actions": ["move"]})).is_err());
    assert!(s.run("options.rowActions", &json!({"actions": "archive"})).is_err());
    assert_eq!(s.run("options.rowActions", &json!({})).unwrap()["actions"], json!(["delete", "archive", "pin"]), "kept as it was");
    s.run("options.rowActions", &json!({"actions": []})).unwrap();
    assert!(s.prefs.row_actions.is_empty(), "none at all is a choice");
    // A settings file edited by hand: each once, no more than a row holds.
    let saved = json!({"rowActions": ["pin", "pin", "read", "flag", "archive", "delete"]});
    let (p, unread) = crate::Prefs::from_saved(&saved).unwrap();
    assert!(unread.is_empty());
    assert_eq!(p.row_actions.len(), crate::ROW_ACTIONS_MAX);
}

/// Issue #274: Snooze to a time of the user's own, as typed (local), a day alone at eight in its
/// morning as the presets; one past, or not a time, isn't taken.
#[test]
fn mail_is_snoozed_to_a_time_chosen() {
    let mut s = sample_session("snooze-at");
    // Two hours ahead of UTC: the time typed is the user's own.
    s.set_clock(NOW, 2 * 3600);
    s.run("list.select", &key_of(&s, "Character turnarounds")).unwrap();
    // NOW is 2026-10-14 12:00 UTC, 14:00 here.
    assert!(s.run("mail.snooze", &json!({"at": "2026-10-14T13:30"})).is_err(), "past");
    assert!(s.run("mail.snooze", &json!({"at": "next tuesday"})).is_err());
    assert!(s.run("mail.snooze", &json!({"at": "2026-10-14T25:00"})).is_err());
    s.run("mail.snooze", &json!({"at": "2026-10-20T17:45"})).unwrap();
    assert_eq!(s.snoozes.iter().map(|z| z.until).collect::<Vec<_>>(), [NOW + 6 * 86_400 + 3 * 3600 + 45 * 60]);
    s.run("list.select", &key_of(&s, "Open call: winter group show")).unwrap();
    s.run("mail.snooze", &json!({"at": "2026-10-16"})).unwrap();
    assert!(s.snoozes.iter().any(|z| z.until == NOW + 2 * 86_400 - 6 * 3600), "eight in the morning there");
}
