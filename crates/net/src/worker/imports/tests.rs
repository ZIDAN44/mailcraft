//! Imports as the pool runs them: one at a time into an account, stopped as asked, and never
//! holding MailCraft up as it closes.

use std::path::PathBuf;
use std::sync::OnceLock;

use mailcraft_model::job::{ImportPhase, StopHow};
use pst_rw::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

use super::*;
use crate::pst::Fault;

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
