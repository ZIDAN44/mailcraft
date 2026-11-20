//! Archiving into a data file (issue #179): a folder's old mail and its folders', into folders
//! of the same names; nothing else of the account; and only what the file has named as archived.

use super::*;
use crate::data_files::scratch::scratch;

const OLD: i64 = 1_600_000_000;
const NEW: i64 = 1_800_000_000;
const BEFORE: i64 = 1_700_000_000;

/// The account: "Old one" and "New one" in its Inbox, "Old plan" in Inbox/Projects, and in Notes
/// "Old note", whose Message-ID is Old one's; and a contact.
fn account(dir: &Path) -> (Store, [MessageKey; 4]) {
    let store = Store::open(dir.join("cache")).unwrap();
    let mail = |folder: &str, uid: u32, subject: &str, id: &str, date: i64| {
        let d = mime::Draft { subject: subject.into(), body: "Hello.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), date, id).unwrap();
        let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        let mut m = mime::summary_from_full(&raw, key, Flags::default(), 10, 0);
        m.date = date;
        m
    };
    let one = mail("INBOX", 1, "Old one", "m1@example.com", OLD);
    let new = mail("INBOX", 2, "New one", "m2@example.com", NEW);
    let plan = mail("INBOX/Projects", 1, "Old plan", "m3@example.com", OLD);
    let note = mail("Notes", 1, "Old note", "m1@example.com", OLD);
    let keys = [one.key.clone(), new.key.clone(), plan.key.clone(), note.key.clone()];
    let state = |messages: Vec<mailcraft_model::Summary>| FolderState {
        validity: 1,
        uid_next: 3,
        messages: messages.into_iter().map(Into::into).collect(),
        ..Default::default()
    };
    store.save_folder("a1", "INBOX", &state(vec![new, one])).unwrap();
    store.save_folder("a1", "INBOX/Projects", &state(vec![plan])).unwrap();
    store.save_folder("a1", "Notes", &state(vec![note])).unwrap();
    let folder = |path: &str, role| mailcraft_model::Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    store
        .save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other), folder("Notes", FolderRole::Other)])
        .unwrap();
    let mut pim = mailcraft_model::pim::Pim::default();
    pim.cards.push(mailcraft_model::pim::StoredCard {
        card: mailcraft_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    (store, keys)
}

/// The subjects in the folder at `names` of the data file at `path`.
fn subjects(path: &Path, names: &[&str]) -> Vec<String> {
    let any = read_file(path);
    let folder = folder_at(&any, names);
    messages_in(&any, &folder).iter().map(|m| subject_of(m)).collect()
}

/// The names of the folders at the top of the data file at `path`.
fn top_folders(path: &Path) -> Vec<String> {
    let any = read_file(path);
    let top = any.open_folder(any.ipm_subtree().unwrap()).unwrap();
    let table = top.hierarchy_table().unwrap().clone();
    table.rows_matrix().filter_map(|row| any.open_folder(row.id()).ok()).filter_map(|f| f.properties().display_name().ok()).collect()
}

#[test]
fn a_folder_s_old_mail_is_archived_into_folders_of_the_same_names() {
    let dir = scratch("pst-archive");
    let (store, [one, _new, plan, note]) = account(&dir);
    let file = dir.join("archive.pst");
    let inbox = Pick { folders: vec!["INBOX".into()], only: false, before: BEFORE };

    // Nothing old enough: no file is made.
    let none = archive(&store, "a1", "Archive", &file, &Pick { before: OLD, ..inbox.clone() }, NONE, &mut |_| {}).unwrap();
    assert_eq!((none.messages, none.archived.len()), (0, 0));
    assert!(!file.exists());

    let mut ticks = 0;
    let done = archive(&store, "a1", "Archive", &file, &inbox, NONE, &mut |_| ticks += 1).unwrap();
    assert_eq!((done.messages, done.already, done.skipped, done.folders_skipped), (2, 0, 0, 0));
    assert_eq!(done.archived, [one.clone(), plan.clone()], "what the file has, the Inbox's and the folder under it");
    assert!(ticks > 0);
    assert_eq!(subjects(&file, &["Inbox"]), ["Old one"], "the new one stays out");
    assert_eq!(subjects(&file, &["Inbox", "Projects"]), ["Old plan"]);
    let top = top_folders(&file);
    assert!(!top.iter().any(|n| n == "Notes"), "a folder outside the one asked for: {top:?}");
    assert!(!top.iter().any(|n| n == "Contacts" || n == "Calendar"), "nothing but the mail: {top:?}");

    // Again, as when its mail couldn't be deleted from the mailbox: nothing is written twice,
    // and the file's copies are named as archived all the same.
    let again = archive(&store, "a1", "Archive", &file, &inbox, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.already), (0, 2));
    assert_eq!(again.archived, [one, plan]);
    assert_eq!(subjects(&file, &["Inbox"]), ["Old one"]);

    // A message whose Message-ID the file has in another folder isn't taken as archived: it's
    // written where it goes.
    let notes =
        archive(&store, "a1", "Archive", &file, &Pick { folders: vec!["Notes".into()], only: false, before: BEFORE }, NONE, &mut |_| {}).unwrap();
    assert_eq!((notes.messages, notes.already), (1, 0));
    assert_eq!(notes.archived, [note]);
    assert_eq!(subjects(&file, &["Notes"]), ["Old note"]);
}

/// Which folders and messages a pick takes.
#[test]
fn a_pick_takes_a_folder_and_those_under_it_and_old_mail() {
    let folder = |path: &str| mailcraft_model::Folder { path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() };
    let pick = Pick { folders: vec!["INBOX".into()], only: false, before: BEFORE };
    assert!(pick.takes(&folder("INBOX")) && pick.takes(&folder("INBOX/Projects")) && pick.takes(&folder("INBOX/Projects/2020")));
    assert!(!pick.takes(&folder("INBOX2")) && !pick.takes(&folder("Sent")), "a name that only starts the same isn't under it");
    let all = Pick { folders: Vec::new(), only: false, before: BEFORE };
    assert!(all.takes(&folder("Sent")) && !all.takes(&folder("\u{0}outbox")), "every folder, never the Outbox");
    // AutoArchive's (issue #180): the folders named alone, each by its own settings.
    let only = Pick { only: true, ..pick.clone() };
    assert!(only.takes(&folder("INBOX")) && !only.takes(&folder("INBOX/Projects")));
    let at = |date, received| mailcraft_model::Summary { date, received, ..Default::default() };
    assert!(pick.old(&at(OLD, 0)) && !pick.old(&at(NEW, 0)));
    assert!(!pick.old(&at(OLD, NEW)), "by when it was received, when that's known");
    assert!(!pick.old(&at(0, 0)), "no date: never taken as old");
}
