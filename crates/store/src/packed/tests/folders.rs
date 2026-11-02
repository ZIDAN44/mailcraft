//! The folders lease: stores on the folder layout share it, and data files bring its folders in
//! only once none has them open, in this process or another, alive or crashed.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;

/// Every file under `dir`, by its path there, with its bytes.
pub(super) fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.strip_prefix(dir).unwrap().to_path_buf(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn inbox(uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid }
}

/// A folder layout at `app` with account `a1` and one message in it.
fn layout(app: &Path) -> Store {
    let s = Store::open(app).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&inbox(1), b"Subject: one\r\n\r\nbody").unwrap();
    s
}

/// Two stores on one folder layout both open and write it (the same root opened twice, or the
/// app's and a test's); data files wait for both.
#[test]
fn stores_on_one_folder_layout_share_it() {
    let t = temp("folders-shared");
    let app = t.join("app");
    let first = layout(&app);
    let second = Store::open(&app).unwrap();
    second.write_body(&inbox(2), b"Subject: two\r\n\r\nbody").unwrap();
    assert_eq!(first.read_body(&inbox(2)).unwrap(), b"Subject: two\r\n\r\nbody");
    drop(first);
    assert!(Store::open_packed(&app, t.join("Files")).is_err(), "the second still has it open");
    drop(second);
    let s = Store::open_packed(&app, t.join("Files")).unwrap();
    assert_eq!(s.read_body(&inbox(2)).unwrap(), b"Subject: two\r\n\r\nbody");
    assert!(!app.join("a1").exists());
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// The app holds its folder layout alone: a second window's store on it, alone or not, is told
/// it's open in another copy, and so are data files bringing it in; let go, it opens again.
#[test]
fn the_apps_folder_layout_is_its_alone() {
    let t = temp("folders-alone");
    let app = t.join("app");
    drop(layout(&app));
    let first = Store::open_alone(&app).unwrap();
    for refused in [Store::open_alone(&app).unwrap_err(), Store::open(&app).unwrap_err(), Store::open_packed(&app, t.join("Files")).unwrap_err()] {
        assert!(refused.to_string().contains("open in another copy of MailCraft"), "{refused}");
    }
    assert_eq!(first.read_body(&inbox(1)).unwrap(), b"Subject: one\r\n\r\nbody");
    let clone = first.clone();
    drop(first);
    assert!(Store::open(&app).is_err(), "held while a clone lives");
    drop(clone);
    // Nor can the app have it alone while a store shares it.
    let shared = Store::open(&app).unwrap();
    assert!(Store::open_alone(&app).is_err());
    drop(shared);
    let again = Store::open_alone(&app).unwrap();
    assert_eq!(again.read_body(&inbox(1)).unwrap(), b"Subject: one\r\n\r\nbody");
    drop(again);
    let _ = std::fs::remove_dir_all(t);
}

/// The leases' folder is never brought into a data file, nor taken for an account's, even one
/// whose name would be the same but for its dot.
#[test]
fn the_leases_folder_stays_out_of_data_files() {
    let t = temp("folders-locks");
    let app = t.join("app");
    let s = layout(&app);
    s.save_accounts(&[account("a1", "zoe@studio.example"), account(".locks", "ana@example.org")]).unwrap();
    drop(s);
    assert!(app.join(".locks").join("folders.lock").is_file());
    let s = Store::open_packed(&app, t.join("Files")).unwrap();
    assert!(app.join(".locks").join("folders.lock").is_file(), "left where it is");
    assert!(!app.join("a1").exists(), "the account's folder came in");
    let files: Vec<String> = std::fs::read_dir(t.join("Files")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["zoe@studio.example.mailcraft"]);
    assert_eq!(s.load_accounts().unwrap().len(), 2, "the accounts are accounts.json's");
    assert_eq!(s.read_body(&inbox(1)).unwrap(), b"Subject: one\r\n\r\nbody");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// What the child below is told: the folder it opens.
const CHILD_DIR: &str = "MAILCRAFT_TEST_FOLDERS_CHILD";

/// Another process has the folder layout open: nothing is brought in, and its folder is as it
/// was. Killed (no drop runs, as in a crash), the system lets its lease go and the folders come in.
#[test]
fn another_process_on_the_folder_layout_keeps_its_folders() {
    let t = temp("folders-process");
    let app = t.join("app");
    drop(layout(&app));
    let mut child = commits::child("packed::tests::folders::child_holds_the_folders", &[(CHILD_DIR, app.display().to_string())]);
    let out = child.stdout.take().unwrap();
    let ready = std::io::BufReader::new(out).lines().map_while(|l| l.ok()).any(|l| l.contains("folders-held"));
    assert!(ready, "the child got as far as opening the folders");
    let before = tree(&app.join("a1"));
    let refused = Store::open_packed(&app, t.join("Files")).unwrap_err();
    assert!(refused.to_string().contains("open in another copy of MailCraft"), "{refused}");
    assert_eq!(tree(&app.join("a1")), before, "the folder and its files untouched");

    child.kill().unwrap();
    let _ = child.wait();
    // The system lets go of a dead process's locks a moment after it ends (Windows).
    let deadline = Instant::now() + Duration::from_secs(10);
    let s = loop {
        match Store::open_packed(&app, t.join("Files")) {
            Ok(s) => break s,
            Err(e) if Instant::now() < deadline && e.to_string().contains("open in another copy") => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("the dead process's lease wasn't let go: {e}"),
        }
    };
    assert!(!app.join("a1").exists(), "the folder came in");
    assert_eq!(s.read_body(&inbox(2)).unwrap(), b"Subject: child\r\n\r\nbody", "with what the child wrote");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// The other process of [`another_process_on_the_folder_layout_keeps_its_folders`]: opens the
/// folder layout, writes to it, says so, and keeps it open until it's killed.
#[test]
#[ignore = "run by another_process_on_the_folder_layout_keeps_its_folders"]
fn child_holds_the_folders() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else { return };
    let s = Store::open(PathBuf::from(dir)).unwrap();
    s.write_body(&inbox(2), b"Subject: child\r\n\r\nbody").unwrap();
    println!("folders-held");
    // Killed long before this ends; should the parent fail first, it goes on its own.
    std::thread::sleep(Duration::from_secs(60));
    drop(s);
}
