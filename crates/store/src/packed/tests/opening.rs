//! Opening the data files on a thread of their own, with a check after a crash told as it goes
//! and holding up only its own account.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::commits::{child, inbox, repairs_on_open};
use super::*;
use crate::OpenStage;

/// A store whose `a1` (zoe's file) was written to by a process that then crashed, so it is checked
/// when it next opens; `a2` (ana's file, opened before it by name) was closed properly.
fn crashed(name: &str) -> std::path::PathBuf {
    let t = temp(name);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "ana@studio.example")]).unwrap();
    s.write_body(&inbox(99), b"Subject: first\r\n\r\nx").unwrap();
    s.write_body(&MessageKey { account: "a2".into(), ..inbox(5) }, b"Subject: other\r\n\r\ny").unwrap();
    drop(s);
    let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_MODE", "now".to_string())];
    let out = child("packed::tests::commits::killed_after_a_write", &env).wait_with_output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stdout).contains("written"), "the child was to write, then be killed");
    t
}

/// Until `f` holds, for up to 30 s.
fn until(f: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(30);
    while !f() {
        if Instant::now() > end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

/// The store opens with nothing opened (no data file, no folder from before brought in); its data
/// files then open together, the one not closed properly checked and told.
#[test]
fn data_files_open_later_and_a_check_is_told() {
    let t = crashed("open-later");
    // A folder from before, still to come in.
    let legacy = Store::open(t.join("app")).unwrap();
    let k = MessageKey { account: "a3".into(), folder: "INBOX".into(), validity: 3, uid: 1 };
    let mut accounts = legacy.load_accounts().unwrap();
    accounts.push(account("a3", "lee@studio.example"));
    legacy.save_accounts(&accounts).unwrap();
    legacy.write_body(&k, b"Subject: old\r\n\r\nz").unwrap();
    drop(legacy);

    let s = Store::packed(t.join("app"), t.join("Files")).unwrap();
    let progress = s.open_progress();
    assert_eq!(progress.stage(), OpenStage::Opening);
    assert_eq!(progress.files(), (0, 0));
    assert!(t.join("app").join("a3").is_dir(), "nothing brought in yet");
    s.open_data_files().unwrap();
    assert_eq!(progress.stage(), OpenStage::Done);
    assert_eq!(progress.files(), (3, 3), "every account's data file opened");
    assert_eq!(progress.checked(), 1, "the one written as the process crashed was checked");
    assert!(progress.problems().is_empty(), "{:?}", progress.problems());
    assert!(!t.join("app").join("a3").exists(), "the folder from before came in");
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: old\r\n\r\nz");
    assert_eq!(s.read_body(&inbox(1)).unwrap(), b"Subject: kept\r\n\r\nx", "nothing lost");
    // Opened again (the checked one now closed properly): nothing to check.
    drop(s);
    let s = Store::packed(t.join("app"), t.join("Files")).unwrap();
    s.open_data_files().unwrap();
    assert_eq!((s.open_progress().checked(), s.open_progress().files()), (0, (3, 3)));
    // Other stores have nothing to open.
    assert_eq!(Store::memory().open_progress().stage(), OpenStage::Done);
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// A check names the file it is of, whatever was being opened, and counts once however often the
/// database says how far it is. (Which file was checked can't be seen once opening ends: the
/// last file opened is named then.)
#[test]
fn a_check_names_its_file() {
    let p = crate::OpenProgress::default();
    p.opening("ana@studio.example.mailcraft");
    p.checking("zoe@studio.example.mailcraft", 0.25);
    assert_eq!((p.stage(), p.file(), p.permille(), p.checked()), (OpenStage::Checking, "zoe@studio.example.mailcraft".to_string(), 250, 1));
    p.checking("zoe@studio.example.mailcraft", 0.5);
    assert_eq!((p.permille(), p.checked()), (500, 1));
    p.opened();
    assert_eq!(p.stage(), OpenStage::Opening);
}

/// Each check is told and counted, data files opened one by one as they're needed included (the
/// command line's way, where none is counted as opened): the first one's name stayed, and later
/// checks went untold.
#[test]
fn every_check_is_told() {
    let p = crate::OpenProgress::default();
    p.checking("ana@studio.example.mailcraft", 0.5);
    p.checked_one();
    p.checking("zoe@studio.example.mailcraft", 0.1);
    assert_eq!((p.stage(), p.file(), p.checked()), (OpenStage::Checking, "zoe@studio.example.mailcraft".to_string(), 2));
}

/// A data file checked as it opens is sealed then: the process ending without closing it (a window
/// closed as it booted, a crash later) leaves nothing to check again next time, where it was
/// checked at every start until something wrote to it or sealed it. The lazy way of opening (the
/// command line's) ends a check too.
#[test]
fn a_checked_data_file_is_not_checked_again() {
    let t = crashed("checked-once");
    let s = Store::packed(t.join("app"), t.join("Files")).unwrap();
    let file = s.data_file("a1").unwrap();
    drop(s);
    let env = [("MC_CHILD_DIR", t.display().to_string())];
    let out = child("packed::tests::opening::opened_then_killed", &env).wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success() && said.contains("checked 1"), "the child was to check zoe's file, then be killed: {said}");
    assert_eq!(repairs_on_open(&t, &file), (0, true), "checked again");
    let _ = std::fs::remove_dir_all(&t);

    // Opened when needed, not all together: the check ends there too.
    let t = crashed("checked-lazily");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    assert!(s.read_body(&inbox(1)).is_some());
    let progress = s.open_progress();
    assert_eq!((progress.checked(), progress.stage()), (1, OpenStage::Opening), "the check didn't end");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// The child of [`a_checked_data_file_is_not_checked_again`]: opens the data files, then is killed
/// (as a process exit leaves them: nothing dropped).
#[test]
#[ignore = "run by a_checked_data_file_is_not_checked_again"]
fn opened_then_killed() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let s = Store::packed(dir.join("app"), dir.join("Files")).unwrap();
    s.open_data_files().unwrap();
    println!("checked {}", s.open_progress().checked());
    std::process::abort();
}

/// A data file being checked holds up only its own account: another account's file is read
/// meanwhile, and a second reader of the one being checked waits for that check rather than
/// starting another. (Opening held one lock for every account, so all of them waited.)
#[test]
fn a_check_holds_up_only_its_own_account() {
    let t = crashed("check-alone");
    let s = Store::packed(t.join("app"), t.join("Files")).unwrap();
    let progress = s.open_progress();
    progress.hold_checks(true);
    let opening = {
        let s = s.clone();
        std::thread::spawn(move || s.open_data_files())
    };
    assert!(until(|| progress.stage() == OpenStage::Checking), "zoe's file wasn't checked");
    assert_eq!(progress.file(), "zoe@studio.example.mailcraft");

    let (tx, rx) = mpsc::channel();
    let other = s.clone();
    std::thread::spawn(move || {
        let _ = tx.send(other.read_body(&MessageKey { account: "a2".into(), ..inbox(5) }));
    });
    let read = rx.recv_timeout(Duration::from_secs(10));
    assert_eq!(read.ok().flatten().as_deref(), Some(&b"Subject: other\r\n\r\ny"[..]), "ana's file waited for zoe's check");

    let (tx, rx) = mpsc::channel();
    let same = s.clone();
    std::thread::spawn(move || {
        let _ = tx.send(same.read_body(&inbox(1)));
    });
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "zoe's file was read before its check ended");
    progress.hold_checks(false);
    let read = rx.recv_timeout(Duration::from_secs(30));
    assert_eq!(read.ok().flatten().as_deref(), Some(&b"Subject: kept\r\n\r\nx"[..]));
    opening.join().unwrap().unwrap();
    assert_eq!((progress.checked(), progress.stage()), (1, OpenStage::Done), "checked once");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// A data file that can't be opened (another MailCraft has it) is told, and the others still open.
#[test]
fn a_data_file_that_cant_be_opened_is_told() {
    let t = temp("open-problem");
    let first = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    first.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "ana@studio.example")]).unwrap();
    first.write_body(&MessageKey { account: "a2".into(), ..inbox(5) }, b"Subject: other\r\n\r\ny").unwrap();
    drop(first);
    let holder = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    holder.write_body(&inbox(1), b"Subject: held\r\n\r\nx").unwrap();

    let s = Store::packed(t.join("app"), t.join("Files")).unwrap();
    s.open_data_files().unwrap();
    let progress = s.open_progress();
    assert_eq!((progress.stage(), progress.files()), (OpenStage::Done, (2, 2)));
    let problems = progress.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].0, "zoe@studio.example.mailcraft");
    assert!(problems[0].1.contains("another copy of MailCraft"), "{problems:?}");
    assert!(s.read_body(&MessageKey { account: "a2".into(), ..inbox(5) }).is_some(), "the other opened");
    drop((s, holder));
    let _ = std::fs::remove_dir_all(t);
}
