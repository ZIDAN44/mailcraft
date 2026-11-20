//! Engine tests: old mail archived to a data file on this computer (issue #179), as Outlook's
//! Clean Up Old Items does. It could only be moved there by hand, folder by folder.

use mailcraft_model::{Flags, Folder, MessageKey, Summary};

use super::*;

/// When the old mail is from: before a year ago.
const BEFORE: i64 = NOW - 365 * 86_400;

/// A session with an account on a server whose Inbox holds two old messages (one not
/// downloaded) and a new one, and whose Inbox/Projects holds an old one; the Inbox open. Returns
/// the session, the account and the keys: Old, Older (not here), Plans, New.
fn mailbox(name: &str) -> (Session, String, [MessageKey; 4]) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder =
        |path: &str, role| Folder { account: id.clone(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() };
    let folders = vec![folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other), folder("Sent", FolderRole::Sent)];
    s.apply(Event::Folders { account: id.clone(), folders });
    let key = |folder: &str, uid| MessageKey { account: id.clone(), folder: folder.into(), validity: 3, uid };
    let mut file = |k: &MessageKey, subject: &str, date: i64, here: bool| {
        let raw = format!("Subject: {subject}\r\nMessage-ID: <{}.{}@x.example>\r\n\r\nText.\r\n", k.folder.len(), k.uid);
        let m = Summary { key: k.clone(), subject: subject.into(), date, flags: Flags::default(), ..Default::default() };
        file_in(&mut s, m, here.then_some(raw.as_bytes()))
    };
    let keys = [
        file(&key("INBOX", 1), "Old", NOW - 400 * 86_400, true),
        file(&key("INBOX", 2), "Older", NOW - 500 * 86_400, false),
        file(&key("INBOX/Projects", 1), "Plans", NOW - 400 * 86_400, true),
        file(&key("INBOX", 3), "New", NOW - 86_400, true),
    ];
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    (s, id, keys)
}

/// The query the archive's search sent; the session told its folders found `found` and that
/// it's done.
fn searched(s: &mut Session, id: &str, found: &[MessageKey]) -> Vec<String> {
    let jobs = s.take_jobs();
    let Some(Job::SearchServer { account, folders, query, .. }) = jobs.iter().find(|j| matches!(j, Job::SearchServer { .. })).cloned() else {
        panic!("no search: {jobs:?}")
    };
    assert_eq!(account, id);
    s.apply(Event::ServerSearched { account: id.into(), query: query.clone(), keys: found.to_vec(), error: None, done: false });
    s.apply(Event::ServerSearched { account: id.into(), query, keys: Vec::new(), error: None, done: true });
    folders
}

/// The file written: the job's account, file and folder asked for.
fn written(s: &mut Session) -> (String, String, Vec<String>) {
    let jobs = s.take_jobs();
    match jobs.iter().find(|j| matches!(j, Job::ArchivePst { .. })) {
        Some(Job::ArchivePst { account, path, folders, before, .. }) => {
            assert_eq!(*before, BEFORE);
            (account.clone(), path.clone(), folders.clone())
        }
        _ => panic!("nothing written: {jobs:?}"),
    }
}

fn done(s: &mut Session, id: &str, messages: usize, archived: Vec<MessageKey>, skipped: usize) {
    s.apply(Event::Archived {
        account: id.into(),
        path: "C:/Mail/archive.pst".into(),
        messages,
        archived,
        skipped,
        folders_skipped: 0,
        done: true,
        error: None,
    });
}

/// The keys each delete asked of the server named.
fn expunged(jobs: &[Job], id: &str) -> Vec<MessageKey> {
    jobs.iter()
        .filter_map(|j| match j {
            Job::Apply { account, op: Op::Expunge { keys } } if account == id => Some(keys.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// Asked for: the server is searched for the old mail of the folder and those under it, what of
/// it isn't here is downloaded, then the file is written; what the file then has leaves the
/// mailbox, and the file opens in the folder list.
#[test]
fn old_mail_leaves_the_mailbox_once_the_archive_has_it() {
    let (mut s, id, [old, older, plans, _new]) = mailbox("archive-old");
    // What it needs, and what it won't do.
    assert!(s.run("dataFile.archive", &json!({"path": "", "before": BEFORE})).is_err());
    assert!(s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.txt", "before": BEFORE})).is_err_and(|e| e.to_string().contains(".pst")));
    assert!(s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst"})).is_err(), "no time");
    assert!(s.run("dataFile.archive", &json!({"path": "archive.pst", "before": BEFORE})).is_err_and(|e| e.to_string().contains("whole path")));
    assert!(s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE, "folder": "Nope"})).is_err());
    assert!(s.take_jobs().is_empty());

    s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE, "account": id, "folder": "INBOX"})).unwrap();
    // One at a time: a second would write the same file.
    assert!(s.run("dataFile.archive", &json!({"path": "C:/Mail/other.pst", "before": BEFORE})).is_err());
    let folders = searched(&mut s, &id, &[old.clone(), older.clone()]);
    assert_eq!(folders, ["INBOX", "INBOX/Projects"], "the folder and those under it, not Sent");
    // Older isn't here: downloaded first, the file written once it's come.
    let jobs = s.take_jobs();
    assert!(matches!(jobs.as_slice(), [Job::FetchBody { key }] if *key == older), "{jobs:?}");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("downloading 1 old message")));
    s.apply(Event::Body { key: older.clone(), raw: b"Subject: Older\r\n\r\nText.\r\n".to_vec() });
    assert_eq!(written(&mut s), (id.clone(), "C:/Mail/archive.pst".to_string(), vec!["INBOX".to_string()]));

    // Written so far: said, and nothing leaves yet.
    s.apply(Event::Archived {
        account: id.clone(),
        path: "C:/Mail/archive.pst".into(),
        messages: 2,
        archived: Vec::new(),
        skipped: 0,
        folders_skipped: 0,
        done: false,
        error: None,
    });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("2 messages so far")));
    assert_eq!(s.list.len(), 3);

    // Done, the file holding the three: they leave the mailbox, the new one stays, and the file
    // opens as a data file of its own.
    done(&mut s, &id, 3, vec![old.clone(), older.clone(), plans.clone()], 0);
    let jobs = s.take_jobs();
    assert_eq!(expunged(&jobs, &id), [old, older, plans], "{jobs:?}");
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["New"]);
    assert!(jobs.iter().any(|j| matches!(j, Job::ImportPst { path, .. } if path == "C:/Mail/archive.pst")), "{jobs:?}");
    assert!(s.accounts.iter().any(|a| a.config.name == "archive" && a.is_local()));
    assert!(s.notices.iter().any(|n| !n.error && n.text.contains("3 messages archived to archive.pst")), "{:?}", s.notices);
    assert!(s.archiving.is_none(), "another can start");
}

/// A folder that found as many as a server finds at a time may have more: another round
/// searches again once the first's mail is gone, until a round finds fewer. Downloads that
/// stop coming aren't waited for for ever.
#[test]
fn a_big_folder_is_archived_in_rounds() {
    let (mut s, id, [old, older, ..]) = mailbox("archive-rounds");
    s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE, "folder": "INBOX"})).unwrap();
    // As many as a folder finds at a time (the same two named again stand for them).
    let full: Vec<MessageKey> = (0..500).map(|i| if i % 2 == 0 { old.clone() } else { older.clone() }).collect();
    searched(&mut s, &id, &full);
    s.take_jobs();
    // Older's download never comes: after a while, what's here is written all the same.
    s.clock.now += 11 * 60;
    s.tick();
    written(&mut s);
    done(&mut s, &id, 1, vec![old.clone()], 1);
    let jobs = s.take_jobs();
    assert_eq!(expunged(&jobs, &id), std::slice::from_ref(&old));
    assert!(jobs.iter().any(|j| matches!(j, Job::SearchServer { .. })), "another round: {jobs:?}");
    assert!(s.archiving.is_some() && !jobs.iter().any(|j| matches!(j, Job::ImportPst { .. })));
    // The second round finds fewer: the last; Older's still not here, so it stays, and is said.
    s.jobs.extend(jobs.into_iter().filter(|j| matches!(j, Job::SearchServer { .. })));
    searched(&mut s, &id, std::slice::from_ref(&older));
    s.apply(Event::BodyFailed { key: older.clone(), error: "gone".into() });
    written(&mut s);
    done(&mut s, &id, 0, vec![old], 1);
    let jobs = s.take_jobs();
    assert!(expunged(&jobs, &id).is_empty(), "what left before isn't asked for again");
    assert!(s.archiving.is_none());
    assert!(s.notices.iter().any(|n| !n.error && n.text.contains("1 message archived")), "{:?}", s.notices);
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("1 old message stayed in the mailbox")), "{:?}", s.notices);
}

/// A search of the user's stops the archive's: what was found is archived, and the user is told
/// to archive again for the rest, with no other round.
#[test]
fn a_search_cut_short_is_said() {
    let (mut s, id, [old, ..]) = mailbox("archive-cut");
    s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE})).unwrap();
    let jobs = s.take_jobs();
    s.run("search.run", &json!({"query": "budget"})).unwrap();
    s.run("search.server", &json!({})).unwrap();
    s.take_jobs();
    s.jobs.extend(jobs);
    let full: Vec<MessageKey> = vec![old.clone(); 500];
    searched(&mut s, &id, &full);
    s.take_jobs();
    s.apply(Event::BodyFailed { key: MessageKey { uid: 2, ..old.clone() }, error: "gone".into() });
    written(&mut s);
    done(&mut s, &id, 1, vec![old], 0);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::SearchServer { .. })), "no other round");
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("archive again to take the rest")), "{:?}", s.notices);
}

/// The file couldn't be written: nothing leaves the mailbox, and another try can start. Nor does
/// an answer take out mail of another account than the one archived.
#[test]
fn mail_stays_when_the_archive_fails() {
    let (mut s, id, [old, ..]) = mailbox("archive-fails");
    s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE, "folder": "INBOX/Projects"})).unwrap();
    searched(&mut s, &id, &[]);
    written(&mut s);
    s.apply(Event::Archived {
        account: id.clone(),
        path: "C:/Mail/archive.pst".into(),
        messages: 0,
        archived: Vec::new(),
        skipped: 0,
        folders_skipped: 0,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.take_jobs().is_empty());
    assert_eq!(s.list.len(), 3);
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full") && n.text.contains("stays where it was")));
    assert!(s.archiving.is_none());

    s.run("dataFile.archive", &json!({"path": "C:/Mail/archive.pst", "before": BEFORE, "folder": "INBOX/Projects"})).unwrap();
    searched(&mut s, &id, &[]);
    written(&mut s);
    let elsewhere = MessageKey { account: "another".into(), ..old };
    done(&mut s, &id, 1, vec![elsewhere], 0);
    assert!(expunged(&s.take_jobs(), &id).is_empty());
    assert_eq!(s.list.len(), 3);
}

/// A data file's own mail isn't archived into that file (its copy would be taken as the mail
/// already archived, and deleted); an account kept online only has none here to archive; and an
/// account removed meanwhile takes its archive with it.
#[test]
fn what_cant_be_archived_says_so() {
    let mut s = Session::open(Some(temp("archive-refused")), mailcraft_model::dates::Clock::new(NOW, 0));
    let file = s.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported { account: file.clone(), messages: 0, skipped: 0, done: true, error: None, report: None });
    s.take_jobs();
    let r = s.run("dataFile.archive", &json!({"path": r"C:\Mail\Archive.pst", "before": NOW, "account": file}));
    assert!(r.is_err_and(|e| e.to_string().contains("the data file the mail is in")));
    // Another file: written at once, all of its mail being here.
    s.run("dataFile.archive", &json!({"path": r"C:\Mail\Older.pst", "before": NOW, "account": file})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ArchivePst { .. }]));
    s.run("account.remove", &json!({"account": file})).unwrap();
    assert!(s.archiving.is_none(), "gone with its account");

    let online = s.run("account.add", &json!({"email": "kim@example.org", "password": "pw", "onlineOnly": true})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let r = s.run("dataFile.archive", &json!({"path": r"C:\Mail\Kim.pst", "before": NOW, "account": online}));
    assert!(r.is_err_and(|e| e.to_string().contains("none of its mail on this computer")));
}
