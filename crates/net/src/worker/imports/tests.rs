//! Imports as the pool runs them: one at a time into an account, stopped as asked, and never
//! holding MailCraft up as it closes.

use std::path::PathBuf;
use std::sync::OnceLock;

use mailcraft_model::Summary;
use mailcraft_model::job::{ImportPhase, StopHow};
use pst_rw::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

use super::*;
use crate::pst::Fault;
use crate::space::{Filling, LOW_SPACE};

/// Accounts whose imports a test puts a fault in: slowed down (each item, in milliseconds), or
/// failing to plan a folder.
fn faults() -> &'static Mutex<HashMap<String, Fault>> {
    static FAULTS: OnceLock<Mutex<HashMap<String, Fault>>> = OnceLock::new();
    FAULTS.get_or_init(Mutex::default)
}

fn slow(account: &str, ms: u64) {
    faults().lock().unwrap().insert(account.to_string(), Fault::Slow(ms));
}

/// The fault a test asked for in `account`'s import.
pub(super) fn fault_for(account: &str) -> Option<Fault> {
    faults().lock().ok()?.get(account).copied()
}

/// Drives a test gave an account's import (by the account) or a look at a data file (by its path).
fn spaces() -> &'static Mutex<HashMap<String, Arc<Filling>>> {
    static SPACES: OnceLock<Mutex<HashMap<String, Arc<Filling>>>> = OnceLock::new();
    SPACES.get_or_init(Mutex::default)
}

fn give_drive(key: &str, drive: Filling) -> Arc<Filling> {
    let drive = Arc::new(drive);
    spaces().lock().unwrap().insert(key.to_string(), drive.clone());
    drive
}

/// The drive a test gave `key`'s import or look.
pub(super) fn space_for(key: &str) -> Option<Arc<dyn crate::space::Space>> {
    let drive = spaces().lock().ok()?.get(key).cloned()?;
    Some(drive)
}

/// A data file with `inbox` messages in its Inbox and `projects` in a folder Projects, in a folder
/// of its own; and a pool over a store of data files beside it, whose imports into `account`
/// take `ms` longer an item.
fn setup(name: &str, inbox: usize, projects: usize, account: &str, ms: u64) -> (PathBuf, String, Store, Pool) {
    let dir = std::env::temp_dir().join(format!("mailcraft-imports-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("in.pst");
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    for (f, (folder, n)) in [("Inbox", inbox), ("Projects", projects)].into_iter().enumerate() {
        if n == 0 {
            continue;
        }
        let owned: Vec<(String, String)> = (0..n).map(|k| (format!("Report {f}-{k}"), format!("<r{f}-{k}@run.example>"))).collect();
        let batch: Vec<UnicodePstBatchMessage<'_>> = owned
            .iter()
            .enumerate()
            .map(|(k, o)| UnicodePstBatchMessage {
                message: UnicodePstMessage {
                    subject: &o.0,
                    sender_name: "Ana",
                    sender_email: "ana@soundlab.example",
                    recipients: &recipients,
                    body: "The week's plan, as agreed.",
                    html_body: None,
                    message_id: &o.1,
                    delivery_time: (1_700_000_000 + k as i64 * 60 + 11_644_473_600) * 10_000_000,
                },
                attachments: &[],
            })
            .collect();
        if f == 0 {
            Writer::create_in_folder_with_attachments(&pst, &[folder], &batch[0].message, batch[0].attachments).unwrap();
            Writer::append_many_in_folder_with_attachments(&pst, &[folder], &batch[1..]).unwrap();
        } else {
            Writer::append_many_in_folder_with_attachments(&pst, &[folder], &batch).unwrap();
        }
    }
    slow(account, ms);
    let store = Store::open_packed(dir.join("app"), dir.join("files")).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    (dir, pst.to_string_lossy().into_owned(), store, pool)
}

/// Events until `until` is one of them (or `wait` passes): all of them, that one last.
fn events_until(p: &Pool, wait: Duration, until: impl Fn(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + wait;
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = p.wait(Duration::from_millis(5)) {
            let last = until(&e);
            seen.push(e);
            if last {
                break;
            }
        }
    }
    seen
}

fn ended(e: &Event) -> bool {
    matches!(e, Event::Imported { done: true, .. })
}

fn stopped_as(e: &Event) -> Option<StopHow> {
    match e {
        Event::Imported { report: Some(r), .. } => r.stopped,
        _ => None,
    }
}

/// Files in the folder the data files go in.
fn data_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir.join("files")).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// Cancelled once its first folder is in, an import keeps that folder, and the folder it was
/// part-way through too: listed, and counted as far as it got.
#[test]
fn a_cancel_keeps_finished_and_partial_folders() {
    let (dir, pst, store, mut p) = setup("cancel", 20, 1000, "c1", 30);
    p.submit(Job::ImportPst { account: "c1".into(), path: pst, resume: false });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "INBOX" && f.total == 20))));
    assert!(
        seen.iter().all(|e| !matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "Projects"))),
        "a folder part-way isn't shown"
    );
    assert!(
        seen.iter().any(|e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Planning)),
        "said from the start"
    );
    p.submit(Job::StopImport { account: "c1".into(), how: StopHow::Cancel });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    let end = seen.last().unwrap();
    assert_eq!(stopped_as(end), Some(StopHow::Cancel), "{end:?}");
    assert!(matches!(end, Event::Imported { report: Some(r), error: None, .. } if r.stopped_in.as_deref() == Some("Projects")), "{end:?}");
    let folders = store.load_folders("c1").unwrap();
    let projects = folders.iter().find(|f| f.path == "Projects").expect("the folder part-way is kept");
    assert!(projects.total < 1000, "{projects:?}");
    assert_eq!(store.load_folder("c1", "Projects").unwrap().messages.len(), projects.total as usize);
    assert_eq!(folders.iter().find(|f| f.path == "INBOX").map(|f| f.total), Some(20));
    assert!(!p.importing("c1"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The account removed while its import runs: the import stops within two seconds, and its data
/// file goes (issue P6: it was written on, and kept, after the account was gone).
#[test]
fn removing_an_account_mid_import_stops_it_and_leaves_no_file() {
    let (dir, pst, store, mut p) = setup("remove", 2000, 0, "r1", 10);
    p.submit(Job::ImportPst { account: "r1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    assert!(store.data_file("r1").is_some(), "the import is writing its data file");
    let asked = Instant::now();
    p.submit(Job::Remove { account: "r1".into() });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(asked.elapsed() < Duration::from_secs(2), "{:?}", asked.elapsed());
    let end = seen.last().unwrap();
    assert_eq!(stopped_as(end), Some(StopHow::Discard), "{end:?}");
    assert!(matches!(end, Event::Imported { error: None, .. }), "{end:?}");
    assert!(store.data_file("r1").is_none());
    assert!(data_files(&dir).is_empty(), "{:?}", data_files(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Thrown away, then MailCraft closes (which pauses every import): it stays thrown away, and
/// nothing of it is left to go on with.
#[test]
fn a_discard_then_closing_stays_a_discard() {
    let (dir, pst, store, mut p) = setup("discard-close", 2000, 0, "d1", 10);
    p.submit(Job::ImportPst { account: "d1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    p.submit(Job::StopImport { account: "d1".into(), how: StopHow::Discard });
    p.shutdown();
    let seen: Vec<Event> = p.poll();
    assert!(seen.iter().any(|e| stopped_as(e) == Some(StopHow::Discard)), "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused)));
    assert!(store.data_file("d1").is_none() && data_files(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import paused has ended: it says so with the folders it finished (not the one it was
/// part-way through), and removing its account then, or stopping it again, is answered at once,
/// as for an account that never imported: thrown away, as far as anything was left of it.
#[test]
fn removing_a_paused_import_completes() {
    let (dir, pst, store, mut p) = setup("paused", 20, 2000, "q1", 30);
    p.submit(Job::ImportPst { account: "q1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    p.submit(Job::StopImport { account: "q1".into(), how: StopHow::Pause });
    let seen =
        events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused));
    assert!(!seen.iter().any(ended), "a pause says nothing but that it stopped: {seen:?}");
    let listed = seen.iter().rev().find_map(|e| match e {
        Event::Folders { folders, .. } => Some(folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>()),
        _ => None,
    });
    assert_eq!(listed, Some(vec!["Deleted Items", "INBOX"]), "the folders finished, said last; not the one part-way");
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("q1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!store.load_folders("q1").unwrap().iter().any(|f| f.path == "Projects"), "the folder it was part-way through isn't shown");
    p.submit(Job::Remove { account: "q1".into() });
    assert!(p.poll().iter().any(|e| stopped_as(e) == Some(StopHow::Discard)), "answered at once");
    p.submit(Job::StopImport { account: "never".into(), how: StopHow::Cancel });
    let answer = p.poll().into_iter().find(|e| matches!(e, Event::Imported { account, done: true, .. } if account == "never"));
    assert_eq!(answer.as_ref().and_then(stopped_as), Some(StopHow::Discard), "{answer:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import thrown away removes the data file it wrote, and nothing later does by the account's
/// name: an account added under the same id once it's over keeps its mail when the pool next
/// lets go of ended imports, and when MailCraft closes.
#[test]
fn a_discarded_import_never_removes_a_later_accounts_file() {
    let (dir, pst, store, mut p) = setup("reuse", 2000, 0, "u1", 10);
    p.submit(Job::ImportPst { account: "u1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    p.submit(Job::Remove { account: "u1".into() });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert_eq!(seen.last().and_then(stopped_as), Some(StopHow::Discard));
    assert!(store.data_file("u1").is_none() && data_files(&dir).is_empty());
    // A new account, given the id again, and its mail.
    let inbox = Folder { account: "u1".into(), path: "INBOX".into(), role: FolderRole::Inbox, ..Folder::default() };
    store.save_folders("u1", std::slice::from_ref(&inbox)).unwrap();
    let file = store.data_file("u1").expect("the new account's data file");
    p.submit(Job::StopImport { account: "other".into(), how: StopHow::Cancel });
    p.shutdown();
    assert!(file.exists(), "the new account's data file stays");
    assert_eq!(store.load_folders("u1").unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import asked for while the account's last one is being thrown away (an account removed,
/// and its id given to the next file opened) waits for that one to be over, then runs to its
/// end: it isn't refused, so the window doesn't wait on it for ever.
#[test]
fn an_import_after_a_discard_runs_once_that_is_over() {
    let (dir, pst, store, mut p) = setup("queued", 2000, 0, "n1", 10);
    p.submit(Job::ImportPst { account: "n1".into(), path: pst.clone(), resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    p.submit(Job::Remove { account: "n1".into() });
    faults().lock().unwrap().remove("n1");
    p.submit(Job::ImportPst { account: "n1".into(), path: pst, resume: false });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(!seen.iter().any(|e| matches!(e, Event::Notice { .. })), "not refused: {seen:?}");
    assert_eq!(seen.last().and_then(stopped_as), Some(StopHow::Discard), "the first ends thrown away");
    let seen = events_until(&p, Duration::from_secs(120), ended);
    let end = seen.last().unwrap();
    assert!(matches!(end, Event::Imported { messages: 2000, error: None, .. }), "{end:?}");
    assert_eq!(stopped_as(end), None);
    assert_eq!(store.load_folder("n1", "INBOX").unwrap().messages.len(), 2000, "the second's data file wasn't the one thrown away");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file looked at before it's imported is planned then: what it holds is told, and its
/// import goes by that plan (a fault planning its Inbox costs nothing) while the file is as it
/// was; changed since, it's planned again.
#[test]
fn a_file_looked_at_is_imported_as_planned_then() {
    let (dir, pst, store, mut p) = setup("looked", 30, 5, "l1", 0);
    p.submit(Job::InspectDataFile { path: pst.clone() });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::DataFileInspected { .. }));
    let Some(Event::DataFileInspected { facts, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!((facts.messages, facts.folders, facts.error.as_deref()), (35, 3, None), "{facts:?}");
    assert!(facts.bytes > 0);
    let fault = Fault::PlanAt(1);
    faults().lock().unwrap().insert("l1".into(), fault);
    p.submit(Job::ImportPst { account: "l1".into(), path: pst.clone(), resume: false });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(matches!(seen.last(), Some(Event::Imported { messages: 35, .. })), "{:?}", seen.last());

    // The file changed since it was looked at: its plan is the file's own again, fault and all.
    let mut bytes = std::fs::read(&pst).unwrap();
    bytes.extend_from_slice(&[0; 512]);
    std::fs::write(&pst, bytes).unwrap();
    faults().lock().unwrap().insert("l2".into(), fault);
    p.submit(Job::ImportPst { account: "l2".into(), path: pst, resume: false });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(matches!(seen.last(), Some(Event::Imported { messages, .. }) if *messages < 35), "{:?}", seen.last());
    assert!(store.load_folders("l2").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// One import at a time into an account: a second is refused, the first goes on.
#[test]
fn a_second_import_into_an_account_is_refused() {
    let (dir, pst, _store, mut p) = setup("twice", 2000, 0, "t1", 10);
    p.submit(Job::ImportPst { account: "t1".into(), path: pst.clone(), resume: false });
    p.submit(Job::ImportPst { account: "t1".into(), path: pst, resume: false });
    assert!(p.poll().iter().any(|e| matches!(e, Event::Notice { account, .. } if account == "t1")));
    assert!(p.importing("t1"));
    p.submit(Job::StopImport { account: "t1".into(), how: StopHow::Discard });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert_eq!(seen.iter().filter(|e| ended(e)).count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Closing waits for an import to pause (what it read is written), but a pool let go of never
/// waits: its import is paused and left to end on its own.
#[test]
fn closing_pauses_imports_and_dropping_never_waits() {
    let (dir, pst, _store, mut p) = setup("close", 2000, 0, "s1", 10);
    p.submit(Job::ImportPst { account: "s1".into(), path: pst.clone(), resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    let asked = Instant::now();
    p.shutdown();
    assert!(asked.elapsed() < IMPORT_STOP_WAIT, "{:?}", asked.elapsed());
    assert!(!p.importing("s1"));
    assert!(p.poll().iter().any(|e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused)));

    slow("s2", 50);
    p.submit(Job::ImportPst { account: "s2".into(), path: pst, resume: false });
    let asked = Instant::now();
    drop(p);
    assert!(asked.elapsed() < Duration::from_secs(1), "{:?}", asked.elapsed());
    // Its thread ends by itself; give it the time before the folder goes.
    std::thread::sleep(Duration::from_secs(2));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The engine lets go of a removed account's data file before the pool hears of the removal,
/// and may give its id to a new account at once: the import thrown away then removes nothing of
/// the new account's (its data file stays, with its folders), and the removed one's file is kept,
/// as a removed account's is.
#[test]
fn a_new_account_given_the_id_while_an_import_is_thrown_away_keeps_its_file() {
    let (dir, pst, store, mut p) = setup("window", 200, 0, "w1", 400);
    p.submit(Job::ImportPst { account: "w1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    let kept = store.remove_account("w1").unwrap().expect("the import's data file, let go of");
    let inbox = Folder { account: "w1".into(), path: "INBOX".into(), role: FolderRole::Inbox, ..Folder::default() };
    store.save_folders("w1", std::slice::from_ref(&inbox)).unwrap();
    let file = store.data_file("w1").expect("the new account's data file");
    assert_ne!(file, kept);
    p.submit(Job::Remove { account: "w1".into() });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert_eq!(seen.last().and_then(stopped_as), Some(StopHow::Discard), "{:?}", seen.last());
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Folders { .. } | Event::FolderChanged { .. })),
        "no folders said of the account once thrown away: {seen:?}"
    );
    p.shutdown();
    assert!(file.exists(), "the new account's data file stays");
    assert_eq!(store.data_file("w1"), Some(file));
    assert!(store.load_folders("w1").unwrap().iter().any(|f| f.path == "INBOX"));
    assert!(kept.exists(), "the removed account's file is kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import queued behind one thrown away, and thrown away in its turn (the account removed
/// again), never begins: another asked for then takes its place and runs to its end, and every
/// import asked for is answered.
#[test]
fn an_import_queued_and_thrown_away_gives_way_to_the_next() {
    let (dir, pst, store, mut p) = setup("requeue", 300, 0, "k1", 10);
    p.submit(Job::ImportPst { account: "k1".into(), path: pst.clone(), resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.work.done > 0));
    p.submit(Job::Remove { account: "k1".into() });
    p.submit(Job::ImportPst { account: "k1".into(), path: pst.clone(), resume: false });
    p.submit(Job::Remove { account: "k1".into() });
    faults().lock().unwrap().remove("k1");
    p.submit(Job::ImportPst { account: "k1".into(), path: pst, resume: false });
    let mut ends = Vec::new();
    let mut seen = Vec::new();
    while ends.len() < 3 {
        let more = events_until(&p, Duration::from_secs(60), ended);
        assert!(more.last().is_some_and(ended), "{more:?}");
        ends.push(more.last().cloned().unwrap());
        seen.extend(more);
    }
    assert!(!seen.iter().any(|e| matches!(e, Event::Notice { .. })), "none refused: {seen:?}");
    assert_eq!(ends.iter().take(2).filter_map(stopped_as).collect::<Vec<_>>(), [StopHow::Discard, StopHow::Discard], "{ends:?}");
    assert!(matches!(&ends[2], Event::Imported { messages: 300, error: None, report: Some(r), .. } if r.stopped.is_none()), "{:?}", ends[2]);
    assert_eq!(store.load_folder("k1", "INBOX").unwrap().messages.len(), 300);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A MailCraft data file whose copy is cancelled, or paused, once part of it is in: none of it
/// is kept (a part of a data file is folder lists naming messages not there, all of them another
/// account's), and a cancel ends as a file that couldn't be opened.
#[test]
fn a_mailcraft_copy_stopped_part_way_keeps_nothing() {
    let (dir, _pst, store, mut p) = setup("copy-stop", 0, 0, "m1", 200);
    let source = big_data_file(&dir);
    let path = source.to_string_lossy().into_owned();

    for (account, how) in [("m1", StopHow::Cancel), ("m2", StopHow::Pause)] {
        slow(account, 200);
        p.submit(Job::ImportPst { account: account.into(), path: path.clone(), resume: false });
        // Its first batch is in once its data file is there.
        let end = Instant::now() + Duration::from_secs(60);
        while store.data_file(account).is_none_or(|f| !f.exists()) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        p.submit(Job::StopImport { account: account.into(), how });
        let seen = if how == StopHow::Pause {
            events_until(
                &p,
                Duration::from_secs(60),
                |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused),
            )
        } else {
            events_until(&p, Duration::from_secs(60), ended)
        };
        if how == StopHow::Cancel {
            let end = seen.last().unwrap();
            assert!(matches!(end, Event::Imported { messages: 0, error: Some(_), .. }), "{end:?}");
            assert_eq!(stopped_as(end), Some(StopHow::Cancel));
        } else {
            assert!(seen.last().is_some_and(|e| matches!(e, Event::ImportProgress { .. })), "{seen:?}");
        }
        assert!(!seen.iter().any(|e| matches!(e, Event::Folders { .. })), "no folders said: {seen:?}");
        assert!(store.load_folders(account).unwrap().is_empty(), "{account}: nothing kept");
        assert!(store.load_folder(account, "zz").unwrap().messages.is_empty());
        assert!(store.data_file(account).is_none(), "{account}: its data file went");
    }
    assert!(source.exists(), "the file copied from stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A MailCraft data file whose copy finds the drive full: as it begins, it waits on the user with
/// nothing copied; part-way, none of it is kept (as when paused) and it waits the same. Gone on
/// with once there's room, it's copied whole.
#[test]
fn a_mailcraft_copy_onto_a_full_drive_waits_with_nothing_kept() {
    let (dir, _pst, store, mut p) = setup("copy-full", 0, 0, "c1", 0);
    let source = big_data_file(&dir);
    let path = source.to_string_lossy().into_owned();
    let blocked = |e: &Event| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Blocked);
    let full = |seen: &[Event], free: u64| matches!(seen.last(), Some(Event::ImportProgress { progress, .. }) if progress.blocked == Some(mailcraft_model::job::Blocked::DiskFull { free }));
    let wait = |p: &Pool| {
        let end = Instant::now() + Duration::from_secs(10);
        while p.importing("c1") && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    // Too full to begin: asked once, nothing copied.
    let drive = give_drive("c1", Filling::new(0, LOW_SPACE - 1));
    p.submit(Job::ImportPst { account: "c1".into(), path: path.clone(), resume: false });
    let seen = events_until(&p, Duration::from_secs(60), |e| blocked(e) || ended(e));
    assert!(full(&seen, LOW_SPACE - 1), "{seen:?}");
    assert_eq!(drive.asks(), 1);
    assert!(store.data_file("c1").is_none_or(|f| !f.exists()), "no data file");
    wait(&p);
    // Room for the data file it fills but not also for the copy it's read from (made on the same
    // drive): looked at, it's said not to fit; imported, it waits at once, nothing copied, rather
    // than filling the drive and throwing the copy away at every Continue.
    let size = std::fs::metadata(&source).unwrap().len();
    let short = NEED_EXTRA + size;
    give_drive(&path, Filling::new(0, short));
    p.submit(Job::InspectDataFile { path: path.clone() });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::DataFileInspected { .. }));
    let Some(Event::DataFileInspected { facts, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!((facts.kind.as_str(), facts.size, facts.free), ("mailcraft", size, Some(short)), "{facts:?}");
    assert!(facts.need > 2 * size + NEED_EXTRA && facts.need > short, "{facts:?}");
    let drive = give_drive("c1", Filling::new(0, short));
    p.submit(Job::ImportPst { account: "c1".into(), path: path.clone(), resume: true });
    let seen = events_until(&p, Duration::from_secs(60), |e| blocked(e) || ended(e));
    assert!(full(&seen, short), "{seen:?}");
    assert_eq!(drive.asks(), 1);
    assert!(store.data_file("c1").is_none_or(|f| !f.exists()), "no data file");
    let tmp = store.data_files_dir().unwrap().join(".import-c1.tmp");
    assert!(!tmp.exists() && !store.root().join(".import-c1.tmp").exists(), "nothing copied");
    wait(&p);
    // Roomy as it begins, full as it goes: none of it kept.
    slow("c1", 100);
    give_drive("c1", Filling::new(2, 300 << 20));
    p.submit(Job::ImportPst { account: "c1".into(), path: path.clone(), resume: true });
    let seen = events_until(&p, Duration::from_secs(60), |e| blocked(e) || ended(e));
    assert!(full(&seen, 300 << 20), "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::Folders { .. })), "no folders said: {seen:?}");
    assert!(store.load_folders("c1").unwrap().is_empty(), "nothing kept");
    assert!(store.data_file("c1").is_none(), "its data file went");
    wait(&p);
    // Room again: copied whole.
    slow("c1", 0);
    give_drive("c1", Filling::new(usize::MAX, 0));
    p.submit(Job::ImportPst { account: "c1".into(), path, resume: true });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(matches!(seen.last(), Some(Event::Imported { messages: 4, error: None, .. })), "{seen:?}");
    assert_eq!(store.load_folder("c1", "zz").unwrap().messages.len(), 4);
    assert!(source.exists(), "the file copied from stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A MailCraft data file of more than one batch, in `dir`: big messages that don't compress, filed
/// after the folder list, so the list comes in with the first batch.
fn big_data_file(dir: &Path) -> PathBuf {
    let old = Store::open_packed(dir.join("old"), dir.join("Old Files")).unwrap();
    let folder = Folder { account: "old".into(), path: "zz".into(), ..Folder::default() };
    old.save_folders("old", std::slice::from_ref(&folder)).unwrap();
    let mut st = old.load_folder("old", "zz").unwrap();
    st.validity = 1;
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    for uid in 1..=4 {
        let key = MessageKey { account: "old".into(), folder: "zz".into(), validity: 1, uid };
        let body: Vec<u8> = (0..12 << 20)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        old.write_body(&key, &body).unwrap();
        st.messages.push(Summary { key, ..Summary::default() });
    }
    old.save_folder("old", "zz", &st).unwrap();
    let source = old.data_file("old").unwrap();
    drop(old);
    source
}

/// Paused as MailCraft closes, an import asked to go on does, from where it stopped: it ends with
/// every message, in two runs. Asked to go on with a file that's gone, it waits on the user,
/// saying why.
#[test]
fn a_paused_import_goes_on_where_it_stopped() {
    let (dir, pst, store, mut p) = setup("go-on", 20, 2000, "g1", 30);
    p.submit(Job::ImportPst { account: "g1".into(), path: pst.clone(), resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    p.submit(Job::StopImport { account: "g1".into(), how: StopHow::Pause });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused));
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("g1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    slow("g1", 0);
    p.submit(Job::ImportPst { account: "g1".into(), path: pst.clone(), resume: true });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    let report = match seen.last() {
        Some(Event::Imported { report: Some(r), error: None, .. }) => r.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!((report.messages, report.runs, report.stopped), (2020, 2, None));
    assert_eq!(store.load_folder("g1", "Projects").unwrap().messages.len(), 2000);
    // Done: asked again, it says so without reading the file, even one that's gone.
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("g1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_file(&pst).unwrap();
    p.submit(Job::ImportPst { account: "g1".into(), path: pst.clone(), resume: true });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    assert!(matches!(seen.last(), Some(Event::Imported { messages: 2020, error: None, .. })), "{seen:?}");

    // Paused, then gone: it waits on the user.
    let (dir2, pst2, _store2, mut p2) = setup("go-on-gone", 20, 2000, "g2", 30);
    p2.submit(Job::ImportPst { account: "g2".into(), path: pst2.clone(), resume: false });
    events_until(&p2, Duration::from_secs(60), |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    p2.submit(Job::StopImport { account: "g2".into(), how: StopHow::Pause });
    events_until(&p2, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused));
    let end = Instant::now() + Duration::from_secs(10);
    while p2.importing("g2") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_file(&pst2).unwrap();
    p2.submit(Job::ImportPst { account: "g2".into(), path: pst2, resume: true });
    let seen =
        events_until(&p2, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Blocked));
    assert!(
        matches!(seen.last(), Some(Event::ImportProgress { progress, .. }) if progress.blocked == Some(mailcraft_model::job::Blocked::SourceMissing)),
        "{seen:?}"
    );
    assert!(!seen.iter().any(ended), "nothing ends: it waits");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// Paused, then kept as it is before its thread has ended (asked for together with the pause,
/// as a keep asked for just after the pause is said may be): the keep follows on that thread.
/// The folder it was part-way through, which the pause kept out of the list, comes into it with
/// every message written before the pause, and the window is told, then that it's kept.
#[test]
fn a_paused_import_kept_lists_its_folder_part_way_through() {
    let (dir, pst, store, mut p) = setup("keep", 20, 2000, "k1", 30);
    p.submit(Job::ImportPst { account: "k1".into(), path: pst, resume: false });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX"));
    p.submit(Job::StopImport { account: "k1".into(), how: StopHow::Pause });
    p.submit(Job::KeepImport { account: "k1".into() });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportKept { .. }));
    let paused = seen.iter().position(|e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused));
    let listed = seen.iter().position(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Projects"));
    assert!(paused.is_some() && paused < listed, "paused first, kept out of the list till then");
    assert!(matches!(seen.last(), Some(Event::ImportKept { error: None, .. })), "{:?}", seen.last());
    let told = seen.iter().find_map(|e| match e {
        Event::Folders { folders, .. } => folders.iter().find(|f| f.path == "Projects").cloned(),
        _ => None,
    });
    let projects = told.expect("the folder part-way through is listed");
    assert!(projects.total > 0 && projects.total < 2000, "{projects:?}");
    assert_eq!(store.load_folder("k1", "Projects").unwrap().messages.len(), projects.total as usize);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Gone on with, an import says where its run began and the time the runs before took: the time
/// left goes by this run's rate, not by the work of the runs before as if done in no time.
#[test]
fn an_import_gone_on_with_says_where_its_run_began() {
    let (dir, pst, _store, mut p) = setup("run-start", 20, 2000, "h1", 5);
    p.submit(Job::ImportPst { account: "h1".into(), path: pst.clone(), resume: false });
    let importing = |e: &Event| match e {
        Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Importing && progress.work.done > 0 => Some(progress.clone()),
        _ => None,
    };
    let first = events_until(&p, Duration::from_secs(60), |e| importing(e).is_some_and(|p| p.work.done >= 400));
    p.submit(Job::StopImport { account: "h1".into(), how: StopHow::Pause });
    events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused));
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("h1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    let said = first.iter().filter_map(importing).map(|p| p.work_done()).max().unwrap();
    assert!(first.iter().filter_map(importing).all(|p| p.run_start_work == 0 && p.elapsed_ms == p.run_ms), "a first run began at nothing");
    slow("h1", 0);
    p.submit(Job::ImportPst { account: "h1".into(), path: pst, resume: true });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    let run: Vec<ImportProgress> = seen.iter().filter_map(importing).collect();
    let start = run.first().map(|p| p.run_start_work).unwrap();
    // It began where the first run stopped: at least as far as that run last said.
    assert!(start >= said, "began at {start}, the first run said {said}");
    for p in &run {
        assert_eq!(p.run_start_work, start);
        assert!(p.work_done() >= start);
        assert!(p.elapsed_ms > p.run_ms, "the first run's time counted: {} of {}", p.elapsed_ms, p.run_ms);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The drive filling up as an import writes: it stops itself, as a pause does, with what came in
/// kept and listed and nothing counted as left out, and waits on the user, saying why. Asked to
/// go on with the drive still full, it waits again, nothing read; with room, it goes on from
/// where it stopped.
#[test]
fn a_full_drive_stops_an_import_with_nothing_lost() {
    let (dir, pst, store, mut p) = setup("full", 20, 2000, "f1", 1);
    // Roomy as it begins, full from its first batch on.
    let drive = give_drive("f1", Filling::new(1, 300 << 20));
    p.submit(Job::ImportPst { account: "f1".into(), path: pst.clone(), resume: false });
    let blocked = |e: &Event| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Blocked);
    let seen = events_until(&p, Duration::from_secs(60), |e| blocked(e) || ended(e));
    let Some(Event::ImportProgress { progress, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!(progress.blocked, Some(mailcraft_model::job::Blocked::DiskFull { free: 300 << 20 }));
    assert!(progress.imported > 0 && progress.imported < 2020, "stopped part-way: {progress:?}");
    assert_eq!(progress.skipped, 0, "nothing counted as left out");
    assert!(drive.asks() >= 2);
    let listed: usize = store.load_folders("f1").unwrap().iter().map(|f| store.load_folder("f1", &f.path).unwrap().messages.len()).sum();
    assert_eq!(u64::try_from(listed).unwrap(), progress.imported, "what came in is listed");
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("f1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Still full: it waits again, nothing read.
    let drive = give_drive("f1", Filling::new(0, LOW_SPACE - 1));
    p.submit(Job::ImportPst { account: "f1".into(), path: pst.clone(), resume: true });
    let seen = events_until(&p, Duration::from_secs(60), |e| blocked(e) || ended(e));
    assert!(
        matches!(seen.last(), Some(Event::ImportProgress { progress, .. }) if progress.blocked == Some(mailcraft_model::job::Blocked::DiskFull { free: LOW_SPACE - 1 })),
        "{seen:?}"
    );
    assert_eq!(drive.asks(), 1);
    let end = Instant::now() + Duration::from_secs(10);
    while p.importing("f1") && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Room again: it goes on, and ends with every message.
    give_drive("f1", Filling::new(usize::MAX, 0));
    p.submit(Job::ImportPst { account: "f1".into(), path: pst, resume: true });
    let seen = events_until(&p, Duration::from_secs(60), ended);
    let report = match seen.last() {
        Some(Event::Imported { report: Some(r), error: None, .. }) => r.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!((report.messages, report.runs, report.skips.len()), (2020, 2, 0), "{report:?}");
    assert_eq!(store.load_folder("f1", "Projects").unwrap().messages.len(), 2000);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file looked at says how big MailCraft's copy will be, the room asked for, and the room
/// there is where the data files go (none when that can't be told).
#[test]
fn a_file_looked_at_says_the_room_for_its_copy() {
    let (dir, pst, store, mut p) = setup("room", 30, 5, "r1", 0);
    let drive = give_drive(&pst, Filling::new(0, 5 << 30));
    p.submit(Job::InspectDataFile { path: pst.clone() });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::DataFileInspected { .. }));
    let Some(Event::DataFileInspected { facts, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!(facts.error, None);
    assert!(facts.estimate > 0 && facts.estimate <= facts.bytes, "{facts:?}");
    assert!(facts.need > facts.estimate, "{facts:?}");
    assert_eq!((facts.free, drive.asks()), (Some(5 << 30), 1));
    assert_eq!(Path::new(&facts.destination), store.data_files_dir().unwrap());
    // Not a data file at all: what can be told still is.
    let junk = dir.join("junk.pst");
    std::fs::write(&junk, b"not a data file").unwrap();
    let junk = junk.to_string_lossy().into_owned();
    p.submit(Job::InspectDataFile { path: junk });
    let seen = events_until(&p, Duration::from_secs(60), |e| matches!(e, Event::DataFileInspected { .. }));
    let Some(Event::DataFileInspected { facts, .. }) = seen.last() else { panic!("{seen:?}") };
    assert!(facts.error.is_some() && !facts.destination.is_empty(), "{facts:?}");
    assert!(facts.free.is_some(), "the drive's own room: {facts:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
