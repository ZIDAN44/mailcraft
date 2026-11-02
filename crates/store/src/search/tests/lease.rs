//! One writer an account's index across processes: the lease at `<root>/.locks`, taken by a
//! second store on the same folder (as another copy of MailCraft would), by another process,
//! and after a holder that crashed.

use std::io::BufRead;
use std::time::{Duration, Instant};

use super::*;

/// A folder of its own for a test.
fn base(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("mailcraft-lease-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    base
}

/// A store on the folder layout or data files at `dir`: a second one on the same `dir` is
/// another copy of MailCraft's.
fn on(kind: &str, dir: &Path) -> Store {
    match kind {
        "disk" => Store::open(dir).unwrap(),
        _ => Store::open_packed(dir.join("app"), dir.join("Files")).unwrap(),
    }
}

fn busy(r: Result<Writer>) -> bool {
    matches!(r, Err(StoreError::Busy { ref path }) if path == "a1/search")
}

/// Two stores on one folder: the second's writer is refused while the first's holds the index,
/// and opens once it's let go, reading what the first committed (its clean-up took none of it).
/// A reader on the second follows the first's commits meanwhile, and setting the index aside
/// from the second leaves it to its writer.
#[test]
fn a_second_store_waits_for_the_first_writer() {
    let root = base("two");
    for kind in ["disk", "packed"] {
        let dir = root.join(kind);
        let first = on(kind, &dir);
        let mut w = Writer::open(&first, "a1").unwrap();
        let one = w.add(&mk("INBOX", 1), 1, 1, &text("first words", &[], 0)).unwrap();
        w.flush(&first).unwrap();

        let second = on(kind, &dir);
        assert!(busy(Writer::open(&second, "a1")), "{kind}");
        assert!(Writer::open(&second, "a2").is_ok(), "{kind}: another account's");
        assert!(!held_here(&second, "a1"), "{kind}: held elsewhere");
        if kind == "disk" {
            // A data file is open in one copy at a time: only the folder layout's is shared.
            let mut r = Reader::open(&second, "a1").unwrap().unwrap();
            assert_eq!(found(&mut r, &second, Field::Body, "first"), [one]);
            let two = w.add(&mk("INBOX", 2), 2, 2, &text("second words", &[], 0)).unwrap();
            w.flush(&first).unwrap();
            assert!(r.refresh(&second).unwrap(), "the first's commit read");
            assert_eq!(found(&mut r, &second, Field::Body, "second"), [two]);
            set_aside(&second, "a1").unwrap();
            assert!(second.files.is_file(&meta_path(&second, "a1")), "left to its writer");
        }
        let (committed, _) = files(&first);
        drop(w);
        // The first copy closed (its data file let go of too).
        drop(first);

        let mut w = Writer::open(&second, "a1").unwrap();
        let (now, extra) = files(&second);
        assert_eq!((now, extra), (committed, BTreeSet::new()), "{kind}: nothing committed removed");
        let mut r = Reader::open(&second, "a1").unwrap().unwrap();
        assert_eq!(found(&mut r, &second, Field::Body, "first"), [one], "{kind}");
        w.add(&mk("INBOX", 3), 3, 3, &text("third words", &[], 0)).unwrap();
        w.flush(&second).unwrap();
        assert!(r.refresh(&second).unwrap());
        assert_eq!(found(&mut r, &second, Field::Body, "third").len(), 1, "{kind}");
        drop(w);
        // The lease is let go with the writer, here too.
        drop(second);
        assert!(Writer::open(&on(kind, &dir), "a1").is_ok(), "{kind}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A lease file left behind (a copy that crashed, its lock let go by the system) is taken at
/// once; its contents are never read.
#[test]
fn a_lease_left_behind_is_taken_at_once() {
    let dir = base("left");
    let s = Store::open(&dir).unwrap();
    let path = s.search_lease("a1").unwrap();
    assert_eq!(path, dir.join(".locks").join("search-a1.lock"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"whatever a crash left").unwrap();
    let w = Writer::open(&s, "a1").unwrap();
    drop(w);
    assert!(path.is_file(), "a lease's file is never deleted");
    assert!(Writer::open(&s, "a1").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing to keep out where the index is this store's alone: in memory, and an online-only
/// account's (held in memory too). No lease file is made for them (the folder layout's own
/// folders lease is all `.locks` holds).
#[test]
fn an_index_in_memory_takes_no_lease() {
    let s = Store::memory();
    assert_eq!(s.search_lease("a1"), None);
    assert!(Writer::open(&s, "a1").is_ok());
    assert_eq!(Store::failing().search_lease("a1"), None);

    let dir = base("online");
    let s = Store::open(&dir).unwrap();
    s.set_online_only("a1", true).unwrap();
    assert_eq!(s.search_lease("a1"), None);
    let w = Writer::open(&s, "a1").unwrap();
    drop(w);
    let leases: Vec<_> = std::fs::read_dir(dir.join(".locks")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(leases, ["folders.lock"]);
    // Another account's is on disk: it has one.
    assert!(s.search_lease("a2").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `.locks` is never an account's folder: an account's is made of letters, digits, `-` and `_`,
/// and data files bring in only the folders `accounts.json` names, so the leases stay where
/// they are when the folder layout becomes data files.
#[test]
fn the_leases_folder_is_never_an_account() {
    for id in [".locks", "../.locks", "x/.locks", ".locks/search-a1.lock"] {
        assert!(!safe_segment(id).starts_with('.'), "{id}");
    }
    let dir = base("bring");
    let s = Store::open(&dir).unwrap();
    s.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    drop(Writer::open(&s, "a1").unwrap());
    drop(s);
    let lease = dir.join(".locks").join("search-a1.lock");
    assert!(lease.is_file());
    let packed = Store::open_packed(&dir, dir.join("Files")).unwrap();
    assert!(lease.is_file(), "left where it was");
    let names: Vec<String> = std::fs::read_dir(dir.join("Files")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.contains("locks")), "{names:?}");
    assert!(Writer::open(&packed, "a1").is_ok());
    drop(packed);
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the child below is told: the folder it opens.
const CHILD_DIR: &str = "MAILCRAFT_TEST_LEASE_CHILD";

/// Another process holds the index: refused here, and its commits read; killed (no drop runs,
/// as in a crash), its lease is let go by the system and the writer here opens, with what it
/// committed and without what it left half written.
#[test]
fn another_process_holds_the_index_until_it_dies() {
    let dir = base("process");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "search::tests::lease::child_holds_the_index", "--nocapture", "--test-threads=1"])
        .env(CHILD_DIR, &dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    let mut lines = std::io::BufReader::new(out).lines();
    let ready = lines.by_ref().map_while(|l| l.ok()).any(|l| l.contains("lease-held"));
    assert!(ready, "the child got as far as holding the index");

    let s = Store::open(&dir).unwrap();
    assert!(busy(Writer::open(&s, "a1")), "held by the other process");
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!((r.generation(), r.live()), (1, 2), "its commit read");
    assert_eq!(found(&mut r, &s, Field::Body, "child").len(), 2);
    set_aside(&s, "a1").unwrap();
    assert!(s.files.is_file(&meta_path(&s, "a1")), "left to its writer");
    let (_, half) = files(&s);
    assert!(!half.is_empty(), "the child left a commit half written");

    child.kill().unwrap();
    let _ = child.wait();
    // The system lets go of a dead process's locks a moment after it ends (Windows).
    let deadline = Instant::now() + Duration::from_secs(10);
    let w = loop {
        match Writer::open(&s, "a1") {
            Ok(w) => break w,
            Err(e) if Instant::now() < deadline && matches!(e, StoreError::Busy { .. }) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("the dead process's lease wasn't let go: {e}"),
        }
    };
    assert_eq!(files(&s).1, BTreeSet::new(), "its half-written commit removed");
    assert!(!r.refresh(&s).unwrap(), "the same generation");
    assert_eq!(found(&mut r, &s, Field::Body, "child").len(), 2);
    assert!(found(&mut r, &s, Field::Body, "never").is_empty());
    drop(w);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other process of [`another_process_holds_the_index_until_it_dies`]: commits two docs,
/// writes a third's files without its meta, says so, and holds the index until it's killed.
#[test]
#[ignore = "run by another_process_holds_the_index_until_it_dies"]
fn child_holds_the_index() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else { return };
    let s = Store::open(PathBuf::from(dir)).unwrap();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.add(&mk("INBOX", 1), 1, 1, &text("child words", &[], 0)).unwrap();
    w.add(&mk("INBOX", 2), 2, 2, &text("child more", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    w.add(&mk("INBOX", 3), 3, 3, &text("never committed", &[], 0)).unwrap();
    w.stop_before_meta = true;
    assert!(w.flush(&s).is_err());
    println!("lease-held");
    // Killed long before this ends; should the parent fail first, it goes on its own.
    std::thread::sleep(Duration::from_secs(60));
}
