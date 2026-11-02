//! Engine tests: a data file's import followed from start to end, stopped, gone on with, kept,
//! looked at twice, and removed while it runs.

use std::collections::BTreeMap;

use mailcraft_model::job::{
    Blocked, DataFileFacts, DataSource, ImportPhase, ImportProgress, ImportReport, ImportStage, PlannedFolder, StopHow, Work,
};

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

/// Why a command was refused: a failure's reason as given (the window shows it; a disabled
/// command's reason it doesn't), anything else marked as what it is.
fn refusal(e: CmdError) -> String {
    match e {
        CmdError::Failed(why) => why,
        e => format!("{e:?}"),
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

    // The folders it will bring in, as planned: kept for the window's outline.
    let planned = vec![PlannedFolder { path: "INBOX".into(), name: "Inbox".into(), items: 100 }];
    s.apply(Event::ImportPlanned { account: id.clone(), folders: planned.clone() });
    assert_eq!(s.import_state(&id).unwrap().planned, planned);

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

/// An import paused as MailCraft closed (or cut short) goes on by itself the next time it opens,
/// a POP account's connection still waiting for it; one the user stopped, or one blocked, waits
/// on them: its account's folders aren't changed until it's gone on with or kept.
#[test]
fn an_import_interrupted_goes_on_and_one_stopped_waits_after_a_restart() {
    let mut s = sample_session("import-restart");
    let facts = DataFileFacts { path: "C:/Mail/Paused.pst".into(), size: 4096, record_key: "00bb".into(), ..Default::default() };
    s.apply(Event::DataFileInspected { path: "C:/Mail/Paused.pst".into(), facts: Box::new(facts) });
    let paused = import(&mut s, "C:/Mail/Paused.pst");
    let stopped = import(&mut s, "C:/Mail/Stopped.pst");
    let blocked = import(&mut s, "C:/Mail/Blocked.pst");
    let pop = json!({"email": "ana@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/old.pst"});
    let pop = s.run("account.add", &pop).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    for a in [&paused, &stopped, &blocked] {
        s.apply(Event::Folders { account: a.clone(), folders: vec![inbox(a)] });
    }
    s.apply(Event::ImportProgress { account: paused.clone(), progress: progress(ImportPhase::Paused, 5, 10, 9_000) });
    assert!(!s.import_state(&paused).unwrap().live);
    assert_eq!(sources(&s)[&paused].stage, ImportStage::Running, "to go on the next time");
    s.apply(ended(&stopped, 3, Some(StopHow::Cancel)));
    let mut stuck = progress(ImportPhase::Blocked, 1, 10, 1_000);
    stuck.blocked = Some(Blocked::SourceMissing);
    s.apply(Event::ImportProgress { account: blocked.clone(), progress: stuck });
    assert_eq!(sources(&s)[&blocked].stage, ImportStage::Stopped, "waits on the user");
    let store = s.store.clone().unwrap();
    drop(s);

    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let mut going_on: Vec<(String, String)> = s
        .take_jobs()
        .into_iter()
        .map(|j| match j {
            Job::ImportPst { account, path, resume: true } => (account, path),
            j => panic!("only the imports interrupted go on: {j:?}"),
        })
        .collect();
    going_on.sort();
    let mut want = vec![(paused.clone(), "C:/Mail/Paused.pst".to_string()), (pop.clone(), "C:/Mail/old.pst".to_string())];
    want.sort();
    assert_eq!(going_on, want, "the POP account's connection waits for its file");
    assert!(s.import_state(&paused).unwrap().live);
    let row = &sources(&s)[&paused];
    assert_eq!((row.stage, row.size, row.record_key.as_str()), (ImportStage::Running, 4096, "00bb"), "what was known of the file stays");
    assert_eq!(
        s.run("folder.create", &json!({"account": paused, "name": "New"})).map_err(refusal),
        Err("wait until the data file is imported".into())
    );
    for a in [&stopped, &blocked] {
        let st = s.import_state(a).unwrap();
        assert!(!st.live && st.progress.phase == ImportPhase::Paused, "{a}");
        assert_eq!(
            s.run("folder.create", &json!({"account": a, "name": "New"})).map_err(refusal),
            Err("continue importing the data file, or keep what's imported, first".into())
        );
    }
    // Kept as it is: the account is any other from then on, and the network side lists the
    // folder a pause left part-way through with what it has.
    s.run("account.keepImported", &json!({"account": stopped})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::KeepImport { account }] if *account == stopped));
    assert!(s.import_state(&stopped).is_none());
    assert_eq!(
        s.run("folder.create", &json!({"account": stopped, "name": "New"})).map_err(refusal),
        Err("wait until what was imported is kept".into())
    );
    s.apply(Event::ImportKept { account: stopped.clone(), error: None });
    assert_eq!(sources(&s)[&stopped].stage, ImportStage::Done);
    s.run("folder.create", &json!({"account": stopped, "name": "New"})).unwrap();
    s.run("account.resumeImport", &json!({"account": blocked})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, resume: true, .. }] if *account == blocked));
    // The POP account connects once its file is in.
    s.apply(ended(&pop, 3, None));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.id == pop));
    done(s);
}

/// A POP account's import stopped and kept as it is: its connection (which writes the Inbox)
/// waits until the network side says the part-way folder is in, not just until Keep is asked;
/// a keep that fails leaves it waiting, to be kept again, and the account can't go meanwhile.
#[test]
fn a_pop_account_connects_only_once_its_stopped_import_is_kept() {
    let mut s = sample_session("import-keep-pop");
    let pop = json!({"email": "ana@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/old.pst"});
    let pop = s.run("account.add", &pop).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.apply(Event::Folders { account: pop.clone(), folders: vec![inbox(&pop)] });
    s.apply(ended(&pop, 3, Some(StopHow::Cancel)));
    assert!(s.take_jobs().is_empty(), "stopped: still held");

    s.run("account.keepImported", &json!({"account": pop})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::KeepImport { account }] if *account == pop), "no connection yet");
    assert!(s.run("account.keepImported", &json!({"account": pop})).is_err(), "asked once");
    assert!(s.run("account.remove", &json!({"account": pop})).is_err(), "not while it's kept");
    // Not all of it kept: it waits again, still unconnected.
    s.apply(Event::ImportKept { account: pop.clone(), error: Some("disk full".into()) });
    assert!(s.take_jobs().is_empty());
    assert!(!s.import_state(&pop).unwrap().live);
    assert_eq!(sources(&s)[&pop].stage, ImportStage::Stopped);
    // Kept: the import is over, and the account connects now.
    s.run("account.keepImported", &json!({"account": pop})).unwrap();
    s.take_jobs();
    s.apply(Event::ImportKept { account: pop.clone(), error: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.id == pop));
    assert_eq!(sources(&s)[&pop].stage, ImportStage::Done);
    // A late answer changes nothing.
    s.apply(Event::ImportKept { account: pop.clone(), error: None });
    assert!(s.take_jobs().is_empty());
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

    // Waiting while other events come: gone from the list after the last one counted. The
    // file is still the import's (Windows wouldn't let it go): it stays, and so does its
    // record, keeping the id.
    let id = import(&mut s, "C:/Mail/Stuck.pst");
    store.save_folders(&id, &[inbox(&id)]).unwrap();
    s.run("account.remove", &json!({"account": id})).unwrap();
    let quota = || Event::Quota { account: "sample".into(), used: 1, limit: 2 };
    for _ in 1..REMOVE_BACKSTOP {
        s.apply(quota());
    }
    assert!(s.account(&id).is_some(), "still waiting");
    s.apply(quota());
    assert!(s.account(&id).is_none() && s.import_state(&id).is_none());
    assert_eq!(sources(&s)[&id].stage, ImportStage::Removing);
    assert!(store.account_path(&id).exists(), "the import still holds it");
    assert!(s.take_jobs().contains(&Job::Remove { account: id.clone() }));
    assert_ne!(import(&mut s, "C:/Mail/Next.pst"), id, "its id isn't given again yet");
    drop(s);
    // The next start removes it, and only then forgets it.
    let s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(!store.account_path(&id).exists());
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
            ("mail.delete", json!({"permanent": true})),
            ("mail.snooze", json!({"until": NOW + 3600})),
            ("mail.unsnooze", json!({})),
        ];
        for (id, v) in tries {
            assert_eq!(s.run(id, &v).map_err(refusal), Err(why.to_string()), "{id} {v}");
        }
        assert_eq!(s.selected().len(), 7, "nothing went");
    };
    refused(&mut s, "wait until the data file is imported");
    s.run("account.stopImport", &json!({"account": "sample"})).unwrap();
    s.apply(ended("sample", 3, Some(StopHow::Cancel)));
    s.run("list.selectAll", &json!({})).unwrap();
    refused(&mut s, "continue importing the data file, or keep what's imported, first");
    s.run("account.keepImported", &json!({"account": "sample"})).unwrap();
    // Until the part-way folder is in, the import still writes the account's folder list.
    s.run("list.selectAll", &json!({})).unwrap();
    refused(&mut s, "wait until what was imported is kept");
    s.apply(Event::ImportKept { account: "sample".into(), error: None });
    s.run("folder.create", &json!({"account": "sample", "name": "New"})).unwrap();
    s.run("mail.move", &json!({"folder": "Archive"})).unwrap();
    done(s);
}

/// What was asked before an import began doesn't move its account's mail while it runs: Undo of
/// a move waits (and stays to be done), and snoozed mail comes back only once the import is over.
#[test]
fn undo_and_snoozes_wait_for_an_import() {
    let mut s = sample_session("import-holds-later");
    let moved = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &moved).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    let snoozed = key_of(&s, "Character turnarounds");
    s.run("list.select", &snoozed).unwrap();
    s.run("mail.snooze", &json!({"until": NOW + 3600})).unwrap();
    s.attach_data_file("sample", "C:/Mail/old.pst").unwrap();
    s.take_jobs();
    let in_inbox = |s: &mut Session, subject: &str| s.folder_state("sample", "INBOX").messages.iter().any(|m| m.subject == subject);

    assert_eq!(s.run("edit.undo", &json!({})).map_err(refusal), Err("wait until the data file is imported".into()));
    s.set_clock(NOW + 3601, 0);
    s.tick();
    assert!(!in_inbox(&mut s, "Character turnarounds"), "still snoozed");
    s.apply(ended("sample", 3, None));
    s.tick();
    assert!(in_inbox(&mut s, "Character turnarounds"), "back once the import is over");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(in_inbox(&mut s, "Storyboard review on Thursday"), "the move undone");
    done(s);
}

/// A data file's mail coming into the Inbox isn't new mail: stopped part-way and gone on with,
/// the rest of the Inbox isn't announced or filed by rules. Mail after the import is.
#[test]
fn no_notices_or_rules_after_a_cancel_and_continue() {
    let mut s = sample_session("import-quiet");
    s.run(
        "rules.add",
        &json!({"rule": {"name": "Zoe to Archive", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "moveToRole", "value": "archive"}]}}),
    )
    .unwrap();
    let id = import(&mut s, "C:/Mail/Zoe.pst");
    let archive =
        mailcraft_model::Folder { account: id.clone(), path: "Archive".into(), role: FolderRole::Archive, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox(&id), archive] });
    let store = s.store.clone().unwrap();
    let m = |uid: u32| mailcraft_model::Summary {
        key: mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid },
        subject: format!("m{uid}"),
        from: mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example"),
        date: NOW,
        ..Default::default()
    };
    let save = |uids: std::ops::RangeInclusive<u32>| {
        let messages = uids.rev().map(m).collect();
        store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 1, uid_next: 99, highest_modseq: 0, messages, window: 0 }).unwrap();
    };
    let came_in = |s: &mut Session| s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    // The first part of the Inbox, then stopped and gone on with: the rest comes in.
    save(1..=2);
    came_in(&mut s);
    s.run("account.stopImport", &json!({"account": id})).unwrap();
    s.apply(ended(&id, 2, Some(StopHow::Cancel)));
    s.run("account.resumeImport", &json!({"account": id})).unwrap();
    save(1..=4);
    came_in(&mut s);
    assert!(s.take_new_mail().is_empty(), "the file's mail isn't news");
    assert_eq!(store.load_folder(&id, "INBOX").unwrap().messages.len(), 4, "no rule ran");
    s.apply(ended(&id, 4, None));
    // Mail that comes after the import is new, and rules file it.
    save(1..=5);
    came_in(&mut s);
    assert_eq!(s.take_new_mail().into_iter().map(|m| m.subject).collect::<Vec<_>>(), ["m5"]);
    assert_eq!(store.load_folder(&id, "INBOX").unwrap().messages.len(), 4, "filed by the rule");
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

/// The note an import keeps in its data file decides over `imports.json`: one recorded as running
/// that its note says was cancelled waits on the user rather than going on by itself; a data
/// file's account with no record whose note says its import is going on goes on, from the file
/// the note names, and one cancelled waits; one done, or with a note that can't be read, is left.
#[test]
fn an_import_s_own_note_decides_over_the_record() {
    let store = temp("import-notes");
    let pst = |id: &str, file: &str| mailcraft_model::AccountConfig { id: id.into(), name: file.into(), email: file.into(), ..Default::default() };
    let ids = ["a1", "a2", "a3", "a4", "a5"];
    store.save_accounts(&ids.map(|a| pst(a, &format!("{a}.pst")))).unwrap();
    for a in ids {
        store.save_folders(a, &[inbox(a)]).unwrap();
    }
    let note = |state: &str, path: &str| json!({"version": 1, "state": state, "source": {"path": path}});
    store.save_account_doc("a1", "import.json", &note("cancelled", "C:/Mail/a1.pst")).unwrap();
    store.save_account_doc("a2", "import.json", &note("running", "C:/Mail/a2.pst")).unwrap();
    store.save_account_doc("a3", "import.json", &note("cancelled", "C:/Mail/a3.pst")).unwrap();
    store.save_account_doc("a4", "import.json", &note("done", "C:/Mail/a4.pst")).unwrap();
    store.save_account_doc("a5", "import.json", &json!({"state": 7})).unwrap();
    let row = |path: &str| DataSource { path: path.into(), stage: ImportStage::Running, ..Default::default() };
    store.save_doc("imports.json", &std::collections::BTreeMap::from([("a1".to_string(), row("C:/Mail/a1.pst"))])).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let going_on: Vec<(String, String)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::ImportPst { account, path, resume: true } => Some((account, path)),
            _ => None,
        })
        .collect();
    assert_eq!(going_on, [("a2".to_string(), "C:/Mail/a2.pst".to_string())]);
    assert_eq!(sources(&s)["a1"].stage, ImportStage::Stopped, "cancelled, not gone on with");
    assert_eq!(sources(&s)["a3"].stage, ImportStage::Stopped);
    for a in ["a1", "a3"] {
        assert!(!s.import_state(a).unwrap().live, "{a} waits on the user");
    }
    assert!(s.import_state("a4").is_none() && s.import_state("a5").is_none());
    assert!(!sources(&s).contains_key("a4") && !sources(&s).contains_key("a5"));
    done(s);
}

/// What the window reads: the imports it shows, the time left by the session's clock, what a
/// data file looked at holds and the account it's open as, and each import finished once (not
/// one stopped, or one that failed), for the window to show what it came to.
#[test]
fn the_window_is_handed_what_it_shows() {
    let mut s = sample_session("import-window");
    let id = import(&mut s, "C:/Mail/Archive.pst");
    assert_eq!(s.imports().map(|(a, st)| (a.to_string(), st.live)).collect::<Vec<_>>(), [(id.clone(), true)]);
    assert_eq!(s.import_eta_s(&id), None, "nothing to go by yet");
    for (i, ms) in [(1u64, 0u64), (2, 25_000), (3, 50_000)] {
        s.set_clock(NOW + i64::try_from(ms / 1000).unwrap(), 0);
        s.apply(Event::ImportProgress { account: id.clone(), progress: progress(ImportPhase::Importing, i * 10, 100, ms) });
    }
    assert!(s.import_eta_s(&id).is_some_and(|left| left > 0));
    assert_eq!(s.import_eta_s("sample"), None);

    s.run("account.inspectDataFile", &json!({"path": "c:/mail/archive.pst"})).unwrap();
    assert!(s.inspecting("c:/mail/archive.pst") && s.data_file_facts("c:/mail/archive.pst").is_none());
    let facts = DataFileFacts { path: "c:/mail/archive.pst".into(), messages: 7, ..Default::default() };
    s.apply(Event::DataFileInspected { path: "c:/mail/archive.pst".into(), facts: Box::new(facts) });
    assert!(!s.inspecting("c:/mail/archive.pst"));
    assert_eq!(s.data_file_facts("c:/mail/archive.pst").map(|f| f.messages), Some(7));
    assert_eq!(s.data_file_open_as("c:/mail/archive.pst"), Some(id.clone()), "the same file");
    assert_eq!(s.data_file_open_as("D:/Other.pst"), None);

    // Stopped: nothing to show yet. Done: shown once.
    s.run("account.stopImport", &json!({"account": id})).unwrap();
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox(&id)] });
    s.apply(ended(&id, 30, Some(StopHow::Cancel)));
    assert_eq!(s.take_finished_import(), None);
    s.run("account.resumeImport", &json!({"account": id})).unwrap();
    s.apply(ended(&id, 100, None));
    assert_eq!(s.take_finished_import(), Some(id.clone()));
    assert_eq!(s.take_finished_import(), None, "taken once");
    assert!(s.imports().next().is_none());

    // One that failed with nothing in it says so in a notice instead.
    let failed = import(&mut s, "C:/Mail/Broken.pst");
    s.apply(Event::Imported { account: failed, messages: 0, skipped: 0, done: true, error: Some("damaged".into()), report: None });
    assert_eq!(s.take_finished_import(), None);
    done(s);
}
