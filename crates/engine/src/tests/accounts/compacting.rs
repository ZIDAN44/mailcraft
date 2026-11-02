//! Engine tests: an account's data file measured and made smaller, by hand and when MailCraft is
//! left alone, with the account waiting while a compaction has its file to itself.

use mailcraft_model::job::{CompactMode, CompactPhase, CompactPlan, CompactReport, DataFileHealth, Work};

use super::*;
use crate::IDLE_AFTER_S;

/// A session over a store of data files, with one account kept in one.
fn with_data_file(name: &str) -> (Session, String) {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-compact-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("app"), dir.join("MailCraft Files")).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox = mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), selectable: true, ..Default::default() };
    s.store.as_ref().unwrap().save_folders(&id, &[inbox]).unwrap();
    assert!(s.store.as_ref().unwrap().data_file(&id).is_some());
    s.take_jobs();
    (s, id)
}

fn finish(s: Session) {
    let root = s.store.as_ref().unwrap().root().parent().unwrap().to_path_buf();
    drop(s);
    let _ = std::fs::remove_dir_all(root);
}

/// A health that needs compacting at idle: 2 GiB of a 4 GiB file to give back.
fn bloated() -> DataFileHealth {
    DataFileHealth { file_bytes: 4 << 30, live_bytes: 2 << 30, reclaimable: 2 << 30, recompressed: true, ..DataFileHealth::default() }
}

fn idle(s: &mut Session, seconds: u64) -> Vec<String> {
    let r = s.run("dataFile.idle", &json!({ "seconds": seconds })).unwrap();
    serde_json::from_value(r["measuring"].clone()).unwrap()
}

#[test]
fn left_alone_data_files_are_measured_once_a_day_and_compacted_when_they_need_it() {
    let (mut s, id) = with_data_file("idle");
    // Not long enough, a message being written, the account syncing: nothing.
    assert!(idle(&mut s, IDLE_AFTER_S - 1).is_empty());
    s.run("mail.new", &json!({})).unwrap();
    assert!(idle(&mut s, IDLE_AFTER_S).is_empty(), "a message is being written");
    s.run("compose.discard", &json!({})).unwrap();
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
    assert!(idle(&mut s, IDLE_AFTER_S).is_empty(), "syncing");
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    s.take_jobs();

    // Left alone: measured, once.
    assert_eq!(idle(&mut s, IDLE_AFTER_S), vec![id.clone()]);
    assert!(matches!(s.take_jobs().as_slice(), [Job::DataFileHealth { account }] if *account == id));
    assert!(idle(&mut s, 2 * IDLE_AFTER_S).is_empty(), "not again the same day");
    // It needs it: compacted at idle.
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(bloated())), error: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::CompactDataFile { account, mode: CompactMode::Idle }] if *account == id));
    assert!(s.compaction.running.contains_key(&id));
    s.apply(Event::Compacted { account: id.clone(), report: Box::new(CompactReport { too_long_s: Some(80), ..CompactReport::default() }) });
    assert!(s.compaction.running.is_empty() && s.compaction.last[&id].too_long_s == Some(80));
    assert!(s.take_jobs().is_empty(), "nothing done: nothing to measure again");
    assert!(s.notices.iter().all(|n| !n.text.contains("compact")), "at idle, said nowhere but the File page");

    // Measured by hand (not for the idle policy): not compacted by itself.
    s.run("dataFile.health", &json!({ "account": id })).unwrap();
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(bloated())), error: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::DataFileHealth { .. }]), "only measured");

    // A recompress stopped part way (the user came back) goes on at the next idle, without the
    // day's wait.
    let stopped = DataFileHealth { recompress_stopped: true, recompressed: false, ..DataFileHealth::default() };
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(stopped)), error: None });
    s.set_clock(NOW + IDLE_AFTER_S as i64, 0);
    assert_eq!(idle(&mut s, IDLE_AFTER_S), vec![id.clone()]);
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(DataFileHealth::default())), error: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::DataFileHealth { .. }]), "measured, nothing to do");
    // A day on: measured again.
    s.set_clock(NOW + 25 * 3600, 0);
    assert_eq!(idle(&mut s, IDLE_AFTER_S), vec![id.clone()]);
    finish(s);
}

#[test]
fn the_account_waits_while_its_file_is_had_to_itself() {
    let (mut s, id) = with_data_file("exclusive");
    s.run("dataFile.compact", &json!({ "account": id })).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::CompactDataFile { mode: CompactMode::Manual, .. }]));
    assert!(matches!(s.run("dataFile.compact", &json!({ "account": id })), Err(CmdError::Failed(_))), "once at a time");
    assert!(s.run("account.list", &json!({ "account": id })).is_ok(), "not had to itself yet");

    let exclusive = Event::Compacting { account: id.clone(), phase: CompactPhase::Exclusive, work: Work::default(), exclusive_s: Some(9) };
    s.apply(exclusive);
    assert!(s.compaction.exclusive(&id));
    // Commands on the account wait; the data file's own, and the view, don't.
    assert!(matches!(s.run("account.list", &json!({ "account": id })), Err(CmdError::Disabled(_))));
    assert!(s.run("dataFile.health", &json!({ "account": id })).is_ok());
    // What the account sends meanwhile is held, its status aside, and applied after, in order.
    let folders = |n: &str| Event::Folders {
        account: id.clone(),
        folders: vec![mailcraft_model::Folder { account: id.clone(), path: n.into(), selectable: true, ..Default::default() }],
    };
    s.apply(folders("Archive"));
    s.apply(folders("Projects"));
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    assert!(s.account(&id).unwrap().folders.iter().all(|f| f.path != "Projects"), "held");
    assert!(matches!(s.account(&id).unwrap().status, SyncStatus::Online { .. }), "its status isn't");
    s.run("dataFile.cancelCompact", &json!({ "account": id })).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::CancelCompact { .. })));

    let report = CompactReport { mode: CompactMode::Manual, before: 900 << 20, after: 300 << 20, compacted: true, ..CompactReport::default() };
    s.apply(Event::Compacted { account: id.clone(), report: Box::new(report) });
    assert!(!s.compaction.exclusive(&id) && s.compaction.running.is_empty());
    let paths: Vec<&str> = s.account(&id).unwrap().folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Projects"], "applied, in the order they came");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("compacted from 900 MB to 300 MB")), "asked for: said");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::DataFileHealth { account } if *account == id)), "measured again");
    assert!(s.run("account.list", &json!({ "account": id })).is_ok());
    assert!(matches!(s.run("dataFile.cancelCompact", &json!({ "account": id })), Err(CmdError::Disabled(_))), "nothing to stop");
    finish(s);
}

#[test]
fn an_account_kept_in_memory_has_no_data_file_to_compact() {
    let mut s = sample_session("compact-memory");
    let id = s.accounts[0].config.id.clone();
    assert!(matches!(s.run("dataFile.compact", &json!({ "account": id })), Err(CmdError::Failed(_))));
    assert!(idle(&mut s, 10 * IDLE_AFTER_S).is_empty());
    assert!(s.run("dataFile.idle", &json!({})).is_err(), "how long it's been is needed");
    done(s);
}

/// While a compaction has an account's file to itself, the window's thread reads nothing of it:
/// a search over every mailbox waits (and goes on once it's given back), a folder's list that
/// isn't in memory isn't read (nor taken for empty to work out new mail from), and the account
/// can't take an import.
#[test]
fn a_search_and_the_lists_wait_while_a_file_is_had_to_itself() {
    let (mut s, id) = with_data_file("search");
    let key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let summary = mailcraft_model::Summary { key, subject: "Quarterly budget".into(), date: NOW - 60, ..Default::default() };
    let st = mailcraft_store::FolderState { validity: 1, uid_next: 2, messages: vec![summary], ..Default::default() };
    s.store.as_ref().unwrap().save_folder(&id, "INBOX", &st).unwrap();
    let inbox =
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    s.run("dataFile.compact", &json!({ "account": id })).unwrap();
    s.take_jobs();
    s.apply(Event::Compacting { account: id.clone(), phase: CompactPhase::Recompress, work: Work::default(), exclusive_s: None });
    let resumed = s.run("account.resumeImport", &json!({ "account": id }));
    assert!(resumed.is_err_and(|e| e.to_string().contains("being compacted")), "no import beside a compaction");
    s.apply(Event::Compacting { account: id.clone(), phase: CompactPhase::Exclusive, work: Work::default(), exclusive_s: Some(9) });
    s.cache.clear();

    s.start_search("budget", SearchScope::All, false);
    let p = s.search_step(1000);
    assert!(!p.done && p.found == 0, "it waits: {p:?}");
    let inbox_key = (id.clone(), "INBOX".to_string());
    assert!(!s.cache.contains_key(&inbox_key), "its list isn't read");
    assert!(s.folder_state(&id, "INBOX").messages.is_empty() && !s.cache.contains_key(&inbox_key), "nor here");
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    assert!(!s.seen_uid.contains_key(&inbox_key), "new mail isn't worked out from a list not read");

    s.apply(Event::Compacted { account: id.clone(), report: Box::new(CompactReport { mode: CompactMode::Manual, ..CompactReport::default() }) });
    let mut p = s.search_step(1000);
    for _ in 0..100 {
        if p.done {
            break;
        }
        p = s.search_step(1000);
    }
    assert!(p.done && p.found == 1, "given back, it goes on: {p:?}");
    finish(s);
}

/// An idle compaction stopped part way is measured again: where its recompress stopped is in the
/// file, and the idle policy goes on from there at the next idle rather than a day later.
#[test]
fn a_compaction_stopped_part_way_is_measured_again() {
    let (mut s, id) = with_data_file("stopped");
    s.run("dataFile.compact", &json!({ "account": id })).unwrap();
    s.take_jobs();
    let report = CompactReport { mode: CompactMode::Idle, cancelled: true, ..CompactReport::default() };
    s.apply(Event::Compacted { account: id.clone(), report: Box::new(report) });
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::DataFileHealth { account } if *account == id)));
    finish(s);
}

/// Closing compacts only what the user chose to (off unless chosen), by what was last measured:
/// a quick one in place as MailCraft closes, a long one offered with its estimate instead.
#[test]
fn closing_compacts_only_when_chosen_and_short() {
    let (mut s, id) = with_data_file("close");
    // Not measured, or not chosen: nothing.
    assert!(s.close_compactions().is_empty());
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(bloated())), error: None });
    assert!(s.close_compactions().is_empty(), "off unless chosen");
    assert_eq!(s.run("dataFile.compactOnClose", &json!({})).unwrap()["on"], json!(true));
    assert_eq!(s.close_compactions(), vec![(id.clone(), CompactPlan::InPlace { recompress: false })]);

    // Big: longer than closing waits, so it's offered with how long it takes.
    let big = DataFileHealth { file_bytes: 40 << 30, live_bytes: 30 << 30, reclaimable: 10 << 30, recompressed: true, ..DataFileHealth::default() };
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(big)), error: None });
    assert!(matches!(s.close_compactions().as_slice(), [(a, CompactPlan::TooLong { seconds })] if *a == id && *seconds > 30));
    // Nothing worth it: nothing.
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(DataFileHealth::default())), error: None });
    assert!(s.close_compactions().is_empty());

    // Compacted as it closes; one running already isn't asked again.
    s.apply(Event::DataFileHealth { account: id.clone(), health: Some(Box::new(bloated())), error: None });
    s.take_jobs();
    s.run("dataFile.compact", &json!({ "account": id, "mode": "close" })).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::CompactDataFile { mode: CompactMode::Close, .. }]));
    assert!(s.close_compactions().is_empty(), "running already");
    assert!(matches!(s.run("dataFile.compact", &json!({ "account": id, "mode": "sideways" })), Err(CmdError::Params(_))));
    // Done on close: not said (MailCraft is going), but kept.
    let report = CompactReport { mode: CompactMode::Close, before: 4 << 30, after: 2 << 30, compacted: true, ..CompactReport::default() };
    let said = s.notices.len();
    s.apply(Event::Compacted { account: id.clone(), report: Box::new(report) });
    assert_eq!(s.notices.len(), said);
    assert!(s.compaction.last[&id].compacted);
    assert_eq!(s.run("dataFile.compactOnClose", &json!({ "on": false })).unwrap()["on"], json!(false));
    finish(s);
}
