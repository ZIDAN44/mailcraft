//! Engine tests: a data file's import followed from start to end, stopped, gone on with, kept,
//! looked at twice, and removed while it runs.

use std::collections::BTreeMap;

use mailcraft_model::job::{Blocked, DataFileFacts, DataSource, ImportPhase, ImportProgress, ImportReport, ImportStage, StopHow, Work};

use super::*;
use crate::session::REMOVE_BACKSTOP;

fn import(s: &mut Session, path: &str) -> String {
    let id = s.run("account.importPst", &json!({"path": path})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    id
}

fn progress(phase: ImportPhase, done: u64, total: u64, run_ms: u64) -> ImportProgress {
    ImportProgress {
        phase,
        work: Work { done, total, ..Default::default() },
        folder: "INBOX".into(),
        imported: done,
        run_ms,
        elapsed_ms: run_ms,
        ..Default::default()
    }
}

fn ended(account: &str, messages: u64, stopped: Option<StopHow>) -> Event {
    let report = ImportReport { messages, stopped, runs: 1, ..Default::default() };
    Event::Imported {
        account: account.to_string(),
        messages: usize::try_from(messages).unwrap(),
        skipped: 0,
        done: true,
        error: None,
        report: Some(Box::new(report)),
    }
}

fn sources(s: &Session) -> BTreeMap<String, DataSource> {
    s.store.as_ref().unwrap().load_doc("imports.json").unwrap()
}

/// Why a command was refused: a disabled one's reason as given.
fn refusal(e: CmdError) -> String {
    match e {
        CmdError::Disabled(why) => why,
        e => e.to_string(),
    }
}

fn inbox(account: &str) -> mailcraft_model::Folder {
    mailcraft_model::Folder { account: account.into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() }
}

/// An import from its start: followed with its progress and the time left (no notices), stopped
/// as asked and kept as stopped, gone on with, and done with its report kept.
#[test]
fn an_import_is_followed_from_start_to_end() {
    let mut s = sample_session("import-states");
    assert_eq!(s.run("account.stopImport", &json!({"all": true})), Err(CmdError::Disabled("nothing is being imported".into())));
    let id = s.run("account.importPst", &json!({"path": "C:/Mail/Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, resume: false, .. }] if *account == id));
    let st = s.import_state(&id).unwrap();
    assert!(st.live && st.file == "Archive.pst" && st.progress.phase == ImportPhase::Planning);
    // Where it came from is kept as soon as it's asked for.
    assert_eq!(sources(&s)[&id].stage, ImportStage::Running);
    assert_eq!(sources(&s)[&id].path, "C:/Mail/Archive.pst");

    // Progress: shown, with the time left once there's a rate to go by; never a notice.
    let notices = s.notices.len();
    for (i, ms) in [(1u64, 0u64), (2, 25_000), (3, 50_000)] {
        s.set_clock(NOW + i64::try_from(ms / 1000).unwrap(), 0);
        s.apply(Event::ImportProgress { account: id.clone(), progress: progress(ImportPhase::Importing, i * 10, 100, ms) });
    }
    assert_eq!(s.notices.len(), notices);
    let status = s.run("account.importStatus", &json!({"account": id})).unwrap();
    assert_eq!((status["live"].as_bool(), status["permille"].as_u64(), status["phase"].as_str()), (Some(true), Some(300), Some("importing")));
    assert!(status["etaSeconds"].as_u64().is_some_and(|eta| eta > 0), "{status}");
    assert_eq!(s.account(&id).unwrap().status, SyncStatus::Syncing { folder: "INBOX".into() });

    // Stopped: asked, then kept as it is when the import says it has stopped.
    assert!(s.run("account.stopImport", &json!({"account": id, "how": "discard"})).is_err(), "only removing throws an import away");
    s.run("account.stopImport", &json!({"account": id})).unwrap();
    assert_eq!(s.take_jobs(), [Job::StopImport { account: id.clone(), how: StopHow::Cancel }]);
    assert_eq!(s.import_state(&id).unwrap().progress.phase, ImportPhase::Stopping);
    s.apply(Event::ImportProgress { account: id.clone(), progress: progress(ImportPhase::Importing, 31, 100, 51_000) });
    assert_eq!(s.import_state(&id).unwrap().progress.phase, ImportPhase::Stopping, "still stopping");
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox(&id)] });
    s.apply(ended(&id, 31, Some(StopHow::Cancel)));
    let st = s.import_state(&id).unwrap();
    assert!(!st.live && st.progress.phase == ImportPhase::Paused);
    assert_eq!(s.account(&id).unwrap().status, SyncStatus::Offline);
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("stopped") && n.text.contains("31")));
    let row = &sources(&s)[&id];
    assert_eq!((row.stage, row.report.as_ref().map(|r| r.messages)), (ImportStage::Stopped, Some(31)));
    assert!(s.run("account.stopImport", &json!({"account": id})).is_err(), "nothing is being imported");
    assert!(s.run("account.keepImported", &json!({"account": "nope"})).is_err());

    // Gone on with: from where it stopped.
    s.run("account.resumeImport", &json!({"account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, path, resume: true }] if *account == id && path == "C:/Mail/Archive.pst"));
    assert!(s.run("account.resumeImport", &json!({"account": id})).is_err(), "it's being read");
    assert!(s.import_state(&id).unwrap().live);
    assert_eq!(sources(&s)[&id].stage, ImportStage::Running);
    s.apply(Event::ImportProgress { account: id.clone(), progress: progress(ImportPhase::Planning, 0, 0, 0) });
    s.apply(ended(&id, 100, None));
    assert!(s.import_state(&id).is_none(), "nothing more to follow");
    let row = &sources(&s)[&id];
    assert_eq!((row.stage, row.report.as_ref().map(|r| r.messages)), (ImportStage::Done, Some(100)));
    assert_eq!(s.run("account.importStatus", &json!({"account": id})).unwrap()["stage"], "done");
    assert!(s.run("account.importStatus", &json!({"account": "sample"})).is_err());
    assert!(s.run("account.resumeImport", &json!({"account": id})).is_err(), "done: nothing to go on with");
    done(s);
}

/// An import paused as MailCraft closed, and one stopped, wait on the user the next time it
/// opens: their accounts' folders aren't changed until they're gone on with or kept.
#[test]
fn imports_paused_or_stopped_wait_on_the_user_after_a_restart() {
    let mut s = sample_session("import-restart");
    let paused = import(&mut s, "C:/Mail/Paused.pst");
    let stopped = import(&mut s, "C:/Mail/Stopped.pst");
    for a in [&paused, &stopped] {
        s.apply(Event::Folders { account: a.clone(), folders: vec![inbox(a)] });
    }
    s.apply(Event::ImportProgress { account: paused.clone(), progress: progress(ImportPhase::Paused, 5, 10, 9_000) });
    assert!(!s.import_state(&paused).unwrap().live);
    assert_eq!(sources(&s)[&paused].stage, ImportStage::Running, "to go on the next time");
    s.apply(ended(&stopped, 3, Some(StopHow::Cancel)));
    let store = s.store.clone().unwrap();
    drop(s);

    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty(), "nothing goes on by itself yet");
    for a in [&paused, &stopped] {
        let st = s.import_state(a).unwrap();
        assert!(!st.live && st.progress.phase == ImportPhase::Paused, "{a}");
        assert_eq!(
            s.run("folder.create", &json!({"account": a, "name": "New"})).map_err(refusal),
            Err("continue importing the data file, or keep what's imported, first".into())
        );
    }
    // Kept as it is: the account is any other from then on.
    s.run("account.keepImported", &json!({"account": paused})).unwrap();
    assert!(s.import_state(&paused).is_none());
    assert_eq!(sources(&s)[&paused].stage, ImportStage::Done);
    s.run("folder.create", &json!({"account": paused, "name": "New"})).unwrap();
    s.run("account.resumeImport", &json!({"account": stopped})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, resume: true, .. }] if *account == stopped));
    done(s);
}

/// Removing an account while its data file is read: the import is told to throw it away, and
/// the account (with what came in) goes only once it has; meanwhile its id isn't given again
/// and its folders don't change.
#[test]
fn removing_an_importing_account_waits_for_its_import() {
    let mut s = sample_session("import-remove");
    let id = import(&mut s, "C:/Mail/Big.pst");
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox(&id)] });
    let store = s.store.clone().unwrap();
    store.save_folders(&id, &[inbox(&id)]).unwrap();
    assert!(store.account_path(&id).exists());

    let r = s.run("account.remove", &json!({"account": id})).unwrap();
    assert_eq!(r["removing"].as_str(), Some(id.as_str()));
    assert_eq!(s.take_jobs(), [Job::StopImport { account: id.clone(), how: StopHow::Discard }]);
    assert!(s.account(&id).is_some() && s.import_state(&id).unwrap().removing);
    assert_eq!(sources(&s)[&id].stage, ImportStage::Removing);
    // Asked again: still being removed, and asked only once.
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert!(s.take_jobs().is_empty());
    assert_eq!(s.run("folder.create", &json!({"account": id, "name": "New"})).map_err(refusal), Err("this account is being removed".into()));
    assert!(s.run("account.resumeImport", &json!({"account": id})).is_err());
    // Its progress no longer counts, and another file opened meanwhile gets an id of its own.
    s.apply(Event::ImportProgress { account: id.clone(), progress: progress(ImportPhase::Importing, 1, 2, 2_000) });
    assert!(s.import_state(&id).unwrap().removing);
    let other = import(&mut s, "C:/Mail/Other.pst");
    assert_ne!(other, id);

    // The import says it's thrown away: the account goes, its data with it.
    s.apply(ended(&id, 0, Some(StopHow::Discard)));
    assert!(s.account(&id).is_none() && s.import_state(&id).is_none());
    assert!(!sources(&s).contains_key(&id));
    assert!(!store.account_path(&id).exists(), "what came in goes");
    assert_eq!(s.take_jobs(), [Job::Remove { account: id.clone() }]);
    // The answer to that is the same word again: nothing more happens.
    s.apply(ended(&id, 0, Some(StopHow::Discard)));
    assert!(s.import_state(&other).is_some_and(|st| st.live));
    done(s);
}

/// A removal whose import never said it stopped: finished the next time MailCraft opens, or
/// after enough other events.
#[test]
fn a_removal_left_waiting_is_finished() {
    let mut s = sample_session("import-remove-later");
    let id = import(&mut s, "C:/Mail/Big.pst");
    let store = s.store.clone().unwrap();
    store.save_folders(&id, &[inbox(&id)]).unwrap();
    s.run("account.remove", &json!({"account": id})).unwrap();
    drop(s);
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.account(&id).is_none());
    assert!(!store.account_path(&id).exists());
    assert!(!sources(&s).contains_key(&id));
    assert_eq!(s.take_jobs(), [Job::Remove { account: id.clone() }]);

    // Waiting while other events come: gone after the last one counted.
    let id = import(&mut s, "C:/Mail/Stuck.pst");
    s.run("account.remove", &json!({"account": id})).unwrap();
    let quota = || Event::Quota { account: "sample".into(), used: 1, limit: 2 };
    for _ in 1..REMOVE_BACKSTOP {
        s.apply(quota());
    }
    assert!(s.account(&id).is_some(), "still waiting");
    s.apply(quota());
    assert!(s.account(&id).is_none());
    assert!(!sources(&s).contains_key(&id));
    done(s);
}

/// While a data file is imported into an account, or its import waits to go on, the account's
/// folders aren't made, renamed, moved, deleted, emptied or cleaned up, and its mail isn't moved:
/// the import writes them (and, gone on with, writes them again).
#[test]
fn an_importing_accounts_folders_dont_change() {
    let mut s = sample_session("import-holds");
    s.attach_data_file("sample", "C:/Mail/old.pst").unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let refused = |s: &mut Session, why: &str| {
        let tries = [
            ("folder.create", json!({"account": "sample", "name": "New"})),
            ("folder.rename", json!({"account": "sample", "folder": "Projects", "name": "Work"})),
            ("folder.move", json!({"account": "sample", "folder": "Projects/Short film"})),
            ("folder.delete", json!({"account": "sample", "folder": "Projects"})),
            ("folder.empty", json!({"account": "sample", "role": "trash"})),
            ("folder.cleanUp", json!({"account": "sample", "folder": "INBOX"})),
            ("mail.move", json!({"folder": "Archive"})),
            ("mail.delete", json!({})),
        ];
        for (id, v) in tries {
            assert_eq!(s.run(id, &v).map_err(refusal), Err(why.to_string()), "{id}");
        }
    };
    refused(&mut s, "wait until the data file is imported");
    s.run("account.stopImport", &json!({"account": "sample"})).unwrap();
    s.apply(ended("sample", 3, Some(StopHow::Cancel)));
    s.run("list.selectAll", &json!({})).unwrap();
    refused(&mut s, "continue importing the data file, or keep what's imported, first");
    s.run("account.keepImported", &json!({"account": "sample"})).unwrap();
    s.run("folder.create", &json!({"account": "sample", "name": "New"})).unwrap();
    s.run("mail.move", &json!({"folder": "Archive"})).unwrap();
    done(s);
}

/// A data file looked at before it's imported, and one already open as an account: caught by
/// its path (however it's written) or its record key, unless asked to import another copy.
#[test]
fn a_data_file_already_open_is_caught() {
    let mut s = sample_session("import-twice");
    let first = import(&mut s, r"C:\Mail\Archive.pst");
    // Looked at: asked once, answered later, with the account it's open as.
    let asked = s.run("account.inspectDataFile", &json!({"path": "c:/mail/ARCHIVE.pst"})).unwrap();
    assert_eq!(asked["pending"], true);
    assert_eq!(s.take_jobs(), [Job::InspectDataFile { path: "c:/mail/ARCHIVE.pst".into() }]);
    s.run("account.inspectDataFile", &json!({"path": "c:/mail/ARCHIVE.pst"})).unwrap();
    assert!(s.take_jobs().is_empty(), "asked once");
    let facts = DataFileFacts { path: "c:/mail/ARCHIVE.pst".into(), messages: 12, record_key: "00aa".into(), ..Default::default() };
    s.apply(Event::DataFileInspected { path: "c:/mail/ARCHIVE.pst".into(), facts: Box::new(facts) });
    let seen = s.run("account.inspectDataFile", &json!({"path": "c:/mail/ARCHIVE.pst"})).unwrap();
    assert_eq!((seen["messages"].as_u64(), seen["alreadyOpenAs"]["account"].as_str()), (Some(12), Some(first.as_str())));
    let refused = s.run("account.importPst", &json!({"path": "C:/MAIL/archive.pst"})).unwrap_err().to_string();
    assert!(refused.contains(&first) && refused.contains("again"), "{refused}");
    // Asked for: another copy. Its record key comes from the look.
    let second = s.run("account.importPst", &json!({"path": "c:/mail/ARCHIVE.pst", "again": true})).unwrap()["account"].as_str().unwrap().to_string();
    assert_eq!(sources(&s)[&second].record_key, "00aa");
    s.take_jobs();

    // The same file under another name: its record key gives it away.
    let facts = DataFileFacts { path: "D:/Copy.pst".into(), record_key: "00aa".into(), ..Default::default() };
    s.apply(Event::DataFileInspected { path: "D:/Copy.pst".into(), facts: Box::new(facts) });
    assert!(s.run("account.importPst", &json!({"path": "D:/Copy.pst"})).is_err());
    // A file being removed isn't open any more.
    s.run("account.remove", &json!({"account": first})).unwrap();
    s.run("account.remove", &json!({"account": second})).unwrap();
    import(&mut s, "D:/Copy.pst");
    // At most a few files are kept looked at.
    for n in 0..10 {
        s.run("account.inspectDataFile", &json!({"path": format!("E:/{n}.pst")})).unwrap();
    }
    assert_eq!(s.take_jobs().len(), 10);
    s.run("account.inspectDataFile", &json!({"path": "E:/0.pst"})).unwrap();
    assert_eq!(s.take_jobs(), [Job::InspectDataFile { path: "E:/0.pst".into() }], "forgotten: asked again");
    done(s);
}

/// A data file's account from before imports were followed, left with no folders (its import
/// never ended), is one to import again or remove; one whose data file notes how far its import
/// got isn't.
#[test]
fn a_data_file_account_with_nothing_in_it_is_found() {
    let store = temp("import-orphan");
    let pst = |id: &str, file: &str| mailcraft_model::AccountConfig { id: id.into(), name: file.into(), email: file.into(), ..Default::default() };
    store.save_accounts(&[pst("a1", "Lost.pst"), pst("a2", "Noted.pst"), pst("a3", "Fine.pst")]).unwrap();
    store.save_account_doc("a2", "import.json", &json!({"version": 1})).unwrap();
    store.save_folders("a3", &[inbox("a3")]).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let lost = s.import_state("a1").unwrap();
    assert_eq!((lost.live, lost.progress.phase, lost.progress.blocked.clone()), (false, ImportPhase::Blocked, Some(Blocked::SourceMissing)));
    assert!(s.import_state("a2").is_none() && s.import_state("a3").is_none());
    assert!(s.run("account.resumeImport", &json!({"account": "a1"})).is_err(), "where it came from isn't known");
    assert!(s.run("account.restartImport", &json!({"account": "a1"})).is_err(), "a file is needed");
    s.run("account.restartImport", &json!({"account": "a1", "path": "C:/Mail/Lost.pst"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, resume: false, .. }] if account == "a1"));
    assert!(s.import_state("a1").unwrap().live);
    assert_eq!(sources(&s)["a1"].path, "C:/Mail/Lost.pst");
    done(s);
}
