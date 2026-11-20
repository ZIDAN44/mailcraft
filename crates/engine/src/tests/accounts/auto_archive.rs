//! Engine tests: AutoArchive (issue #180), as Outlook's: its settings and a folder's own, when it
//! runs (asking first or not), and each folder's old mail archived or deleted by its settings.
//! Nothing archived or deleted mail by its age before.

use mailcraft_model::{Flags, Folder, MessageKey, Summary};

use super::*;

const DAY: i64 = 86_400;

/// A session with an account on a server: its Inbox holds an old message and a new one, its
/// Inbox/Projects an old one, its Sent an old one; all downloaded. Returns the session, the
/// account, and the keys of the Inbox's old one, Projects' and Sent's.
fn mailbox(name: &str) -> (Session, String, [MessageKey; 3]) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder =
        |path: &str, role| Folder { account: id.clone(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() };
    let folders = vec![folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other), folder("Sent", FolderRole::Sent)];
    s.apply(Event::Folders { account: id.clone(), folders });
    let key = |folder: &str, uid| MessageKey { account: id.clone(), folder: folder.into(), validity: 3, uid };
    let mut file = |k: &MessageKey, subject: &str, date: i64| {
        let raw = format!("Subject: {subject}\r\nMessage-ID: <{}.{}@x.example>\r\n\r\nText.\r\n", k.folder.len(), k.uid);
        let m = Summary { key: k.clone(), subject: subject.into(), date, flags: Flags::default(), ..Default::default() };
        file_in(&mut s, m, Some(raw.as_bytes()))
    };
    let keys = [
        file(&key("INBOX", 1), "Old", NOW - 400 * DAY),
        file(&key("INBOX/Projects", 1), "Plans", NOW - 40 * DAY),
        file(&key("Sent", 1), "Sent long ago", NOW - 400 * DAY),
    ];
    file(&key("INBOX", 2), "New", NOW - DAY);
    s.take_jobs();
    (s, id, keys)
}

/// What the archive's search asked for: answered with nothing more found, as all is here.
fn searched(s: &mut Session) -> Vec<String> {
    let jobs = s.take_jobs();
    let Some(Job::SearchServer { account, folders, query, .. }) = jobs.iter().find(|j| matches!(j, Job::SearchServer { .. })).cloned() else {
        panic!("no search: {jobs:?}")
    };
    s.apply(Event::ServerSearched { account, query, keys: Vec::new(), error: None, done: true });
    folders
}

/// Its settings and a folder's own, as the commands keep them.
#[test]
fn autoarchive_s_settings_are_kept() {
    let (mut s, id, _) = mailbox("auto-settings");
    assert_eq!((s.prefs.auto_archive.every_days, s.prefs.auto_archive.prompt), (0, true), "off at first, asking first");
    s.run("autoArchive.set", &json!({"everyDays": 14, "olderThan": 3, "unit": "months", "path": "C:/Mail/archive.pst"})).unwrap();
    let a = &s.prefs.auto_archive;
    assert_eq!((a.every_days, a.rule.older_than, a.rule.path.as_str(), a.last_run), (14, 3, "C:/Mail/archive.pst", NOW), "counted from now");
    assert!(s.run("autoArchive.set", &json!({"unit": "fortnights"})).is_err());
    assert!(s.run("autoArchive.set", &json!({"path": "C:/Mail/archive.txt"})).is_err());
    assert_eq!(s.run("autoArchive.set", &json!({"everyDays": 900})).unwrap()["everyDays"], 60, "60 days at most");

    // A folder never archived, one by its own settings, then back to the default ones.
    s.run("autoArchive.folder", &json!({"account": id, "folder": "Sent", "archive": "off"})).unwrap();
    s.run(
        "autoArchive.folder",
        &json!({"account": id, "folder": "INBOX/Projects", "archive": "own", "olderThan": 30, "unit": "days", "delete": true}),
    )
    .unwrap();
    let a = &s.prefs.auto_archive;
    assert_eq!(a.folder(&id, "Sent"), Some(None));
    let own = a.folder(&id, "INBOX/Projects").flatten().unwrap();
    assert_eq!((own.older_than, own.unit, own.delete), (30, crate::AgeUnit::Days, true));
    assert_eq!(a.folder(&id, "INBOX"), None);
    assert!(s.run("autoArchive.folder", &json!({"account": id, "folder": "Nope", "archive": "off"})).is_err());
    assert!(s.run("autoArchive.folder", &json!({"account": id, "folder": "Sent", "archive": "sometimes"})).is_err());
    s.run("autoArchive.folder", &json!({"account": id, "folder": "Sent", "archive": "default"})).unwrap();
    assert_eq!(s.prefs.auto_archive.folder(&id, "Sent"), None);
    // A folder's own setting goes with it when it's renamed, and with it when it's deleted.
    s.run("folder.rename", &json!({"account": id, "folder": "INBOX/Projects", "name": "Plans"})).unwrap();
    assert!(s.prefs.auto_archive.folder(&id, "INBOX/Plans").flatten().is_some_and(|r| r.delete), "{:?}", s.prefs.auto_archive.folders);
    assert_eq!(s.prefs.auto_archive.folder(&id, "INBOX/Projects"), None);
    s.run("autoArchive.folder", &json!({"account": id, "folder": "Sent", "archive": "off"})).unwrap();
    // Apply these settings to all folders now: each follows the default ones.
    assert_eq!(s.run("autoArchive.applyToAll", &json!({})).unwrap()["cleared"], 2);
    assert!(s.prefs.auto_archive.folders.is_empty());
}

/// Due, it asks first: once, and not again until the next period after "Not now". Run, each
/// folder goes by its settings, one group after another: the Inbox's old mail archived (that
/// folder alone, not Projects under it), Projects' deleted for good after its own 30 days, Sent
/// left alone.
#[test]
fn autoarchive_asks_then_goes_folder_by_folder() {
    let (mut s, id, [old, plans, _sent]) = mailbox("auto-run");
    s.run("autoArchive.set", &json!({"everyDays": 14, "path": "C:/Mail/archive.pst"})).unwrap();
    s.run("autoArchive.folder", &json!({"account": id, "folder": "Sent", "archive": "off"})).unwrap();
    s.run(
        "autoArchive.folder",
        &json!({"account": id, "folder": "INBOX/Projects", "archive": "own", "olderThan": 30, "unit": "days", "delete": true}),
    )
    .unwrap();
    s.tick();
    assert!(!s.take_auto_archive_ask(), "not yet due");
    s.clock.now += 15 * DAY;
    s.tick();
    assert!(s.take_auto_archive_ask(), "due: asks");
    s.tick();
    assert!(!s.take_auto_archive_ask(), "once");
    assert!(s.take_jobs().is_empty(), "nothing runs before the answer");
    // Not now: due again in 14 days.
    s.run("autoArchive.later", &json!({})).unwrap();
    s.clock.now += 13 * DAY;
    s.tick();
    assert!(!s.take_auto_archive_ask());
    s.clock.now += 2 * DAY;
    s.tick();
    assert!(s.take_auto_archive_ask());

    // Archive now: the Inbox's group first.
    assert_eq!(s.run("autoArchive.run", &json!({})).unwrap()["groups"], 2);
    assert!(s.auto_archive_running());
    assert_eq!(searched(&mut s), ["INBOX"], "the Inbox alone");
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::ArchivePst { folders, only: true, path, .. }] if folders.as_slice() == ["INBOX"] && path == "C:/Mail/archive.pst"),
        "{jobs:?}"
    );
    s.apply(Event::Archived {
        account: id.clone(),
        path: "C:/Mail/archive.pst".into(),
        messages: 1,
        archived: vec![old.clone()],
        skipped: 0,
        folders_skipped: 0,
        done: true,
        error: None,
    });
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::Expunge { keys }, .. } if keys == std::slice::from_ref(&old))), "{jobs:?}");
    assert!(jobs.iter().any(|j| matches!(j, Job::ImportPst { .. })), "shown in the folder list: {jobs:?}");
    // Then Projects': searched, and its old mail deleted for good, no file written.
    let search = jobs.into_iter().filter(|j| matches!(j, Job::SearchServer { .. })).collect::<Vec<_>>();
    s.jobs.extend(search);
    assert_eq!(searched(&mut s), ["INBOX/Projects"]);
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::Expunge { keys }, .. } if keys == std::slice::from_ref(&plans))), "{jobs:?}");
    assert!(!jobs.iter().any(|j| matches!(j, Job::ArchivePst { .. })));
    assert!(!s.auto_archive_running() && s.archiving_now().is_none());
    assert!(s.notices.iter().any(|n| n.text.contains("1 old message deleted")), "{:?}", s.notices);
    assert!(s.prefs.auto_archive.last_run == s.clock.now);
}

/// Set not to ask, it runs when due by itself; with no file chosen it says so, and archives
/// nothing; an Outlook data file opened here, whose mail is in a file already, isn't archived.
#[test]
fn autoarchive_runs_unasked_and_says_what_it_lacks() {
    let (mut s, id, _) = mailbox("auto-unasked");
    let file = s.run("account.importPst", &json!({"path": r"C:\Mail\Older.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported { account: file.clone(), messages: 0, skipped: 0, done: true, error: None, report: None });
    s.take_jobs();
    s.run("autoArchive.set", &json!({"everyDays": 1, "prompt": false})).unwrap();
    s.clock.now += 2 * DAY;
    s.tick();
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("choose the file to archive to")), "{:?}", s.notices);
    assert!(s.take_jobs().is_empty() && s.archiving_now().is_none());

    s.run("autoArchive.set", &json!({"path": "C:/Mail/archive.pst"})).unwrap();
    s.clock.now += 2 * DAY;
    s.tick();
    assert!(!s.take_auto_archive_ask(), "not asked");
    assert_eq!(s.archiving_now(), Some((id.as_str(), "C:/Mail/archive.pst")), "running");
    assert!(!s.auto_archives(&file), "the data file's account is left out");
}
