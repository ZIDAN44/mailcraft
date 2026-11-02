//! A data file's bigger cache while its account takes a bulk write (an import), and letting go of
//! it after: opened again with the cache wanted once nobody holds the file, never while someone
//! does.

use std::sync::Arc;

use super::commits::inbox;
use super::*;
use crate::{BULK_CACHE, CACHE};

fn packs(s: &Store) -> &Arc<crate::packed::Packs> {
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs
}

fn store(t: &std::path::Path) -> Store {
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s
}

fn source(i: u32) -> Vec<u8> {
    format!("Subject: {i}\r\n\r\n{}", "the quick brown fox ".repeat(200)).into_bytes()
}

/// An import's cache comes with its bulk write and goes with it: the file is opened again with
/// it while idle, what was written (batches not waited for included) is all there after, and
/// the file isn't left waiting for a seal by the close.
#[test]
fn bulk_cache_is_let_go_after_the_import() {
    let t = temp("bulk-let-go");
    let s = store(&t);
    s.write_body(&inbox(1), &source(1)).unwrap();
    assert_eq!(s.data_file_cache("a1"), Some(CACHE));

    s.bulk_begin("a1", BULK_CACHE);
    assert_eq!(s.data_file_cache("a1"), None, "closed while nobody held it, to be opened with the bigger cache");
    let batch: Vec<_> = (2..40).map(|i| (inbox(i), Store::prepare(true, source(i)))).collect();
    s.write_prepared(batch, false).unwrap();
    assert_eq!(s.data_file_cache("a1"), Some(BULK_CACHE));

    s.bulk_end("a1");
    assert_eq!(s.data_file_cache("a1"), None, "let go of at once");
    assert!(!packs(&s).seal_due("a1"), "closed cleanly: nothing waits for a seal");
    for i in 1..40 {
        assert_eq!(s.read_body(&inbox(i)), Some(source(i)), "source {i}");
    }
    assert_eq!(s.data_file_cache("a1"), Some(CACHE), "opened again with the usual cache");
    drop(s);
    // And after opening again: the batches not waited for were made durable by the close.
    let s = store(&t);
    assert_eq!(s.read_body(&inbox(39)), Some(source(39)));
    assert_eq!(packs(&s).progress.checked(), 0);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A file someone holds isn't closed under them: the bulk write starts with the cache it has, and
/// the file changes cache the first time it's asked for once let go of; likewise at the end.
#[test]
fn a_held_data_file_keeps_its_cache_until_let_go() {
    let t = temp("bulk-held");
    let s = store(&t);
    s.write_body(&inbox(1), &source(1)).unwrap();
    let held = packs(&s).db("a1", false).unwrap().unwrap();
    s.bulk_begin("a1", BULK_CACHE);
    assert_eq!(s.data_file_cache("a1"), Some(CACHE), "held: kept open as it is");
    s.write_body(&inbox(2), &source(2)).unwrap();
    assert_eq!(s.data_file_cache("a1"), Some(CACHE));
    drop(held);
    s.write_body(&inbox(3), &source(3)).unwrap();
    assert_eq!(s.data_file_cache("a1"), Some(BULK_CACHE), "opened again once nobody held it");

    let held = packs(&s).db("a1", false).unwrap().unwrap();
    s.bulk_end("a1");
    assert_eq!(s.data_file_cache("a1"), Some(BULK_CACHE), "held at the end: shrinking waits");
    assert_eq!(s.read_body(&inbox(2)), Some(source(2)));
    drop(held);
    assert_eq!(s.read_body(&inbox(3)), Some(source(3)));
    assert_eq!(s.data_file_cache("a1"), Some(CACHE), "shrunk on the next use");
    for i in 1..4 {
        assert_eq!(s.read_body(&inbox(i)), Some(source(i)));
    }
    // A cache no bigger than the usual one changes nothing.
    s.bulk_begin("a1", CACHE / 2);
    assert_eq!(s.data_file_cache("a1"), Some(CACHE));
    s.bulk_end("a1");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A bulk write ended by a panic leaves the file open: closed then, it would write nothing and
/// need its full check when opened next. The next ask gives it its usual cache, sealed first.
#[test]
fn a_panic_ends_the_bulk_write_without_closing_the_file() {
    struct Ends<'a>(&'a Store);
    impl Drop for Ends<'_> {
        fn drop(&mut self) {
            self.0.bulk_end("a1");
        }
    }
    let t = temp("bulk-panic");
    let s = store(&t);
    s.write_body(&inbox(1), &source(1)).unwrap();
    s.bulk_begin("a1", BULK_CACHE);
    let batch: Vec<_> = (2..40).map(|i| (inbox(i), Store::prepare(true, source(i)))).collect();
    s.write_prepared(batch, false).unwrap();
    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ends = Ends(&s);
        panic!("an import fails");
    }));
    assert!(ended.is_err());
    assert_eq!(s.data_file_cache("a1"), Some(BULK_CACHE), "left open by the panic");
    assert_eq!(s.read_body(&inbox(39)), Some(source(39)));
    assert_eq!(s.data_file_cache("a1"), Some(CACHE), "the bulk write is over: shrunk on the next use");
    assert!(!packs(&s).seal_due("a1"));
    drop(s);
    let s = store(&t);
    for i in 1..40 {
        assert_eq!(s.read_body(&inbox(i)), Some(source(i)), "source {i}");
    }
    assert_eq!(packs(&s).progress.checked(), 0, "closed cleanly: no check");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Bulk writes beginning and ending while other threads write and read the file (the sealing
/// thread among them): nothing fails (no "open in another copy"), nothing is lost.
#[test]
fn changing_the_cache_under_writers_loses_nothing() {
    let t = temp("bulk-race");
    let s = Arc::new(store(&t));
    s.write_body(&inbox(1), &source(1)).unwrap();
    let writers: Vec<_> = (0..4u32)
        .map(|w| {
            let s = s.clone();
            std::thread::spawn(move || {
                for i in 0..60u32 {
                    let uid = 1000 + w * 100 + i;
                    if i % 2 == 0 {
                        s.write_body(&inbox(uid), &source(uid)).unwrap();
                    } else {
                        s.write_prepared(vec![(inbox(uid), Store::prepare(true, source(uid)))], false).unwrap();
                    }
                    assert_eq!(s.read_body(&inbox(uid)), Some(source(uid)), "writer {w}, {i}");
                }
            })
        })
        .collect();
    for _ in 0..40 {
        s.bulk_begin("a1", BULK_CACHE);
        let _ = s.read_body(&inbox(1));
        s.bulk_end("a1");
    }
    for w in writers {
        w.join().unwrap();
    }
    s.bulk_end("a1");
    let _ = s.read_body(&inbox(1));
    assert_eq!(s.data_file_cache("a1"), Some(CACHE));
    for w in 0..4u32 {
        for i in 0..60u32 {
            let uid = 1000 + w * 100 + i;
            assert_eq!(s.read_body(&inbox(uid)), Some(source(uid)));
        }
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// While an account takes a bulk write (an import), a commit that waits for the disk is sealed: a
/// write from the window, and a batch past what may wait for the disk. Killed part-way, the data
/// file reopens with no repair, back to its last durable commit (a 37 GB one took 55 s to check
/// whole); without the bulk write, the same commits leave it to be checked.
#[test]
fn a_bulk_write_s_durable_commits_are_sealed() {
    for (mode, sealed) in [("bulk", true), ("plain", false)] {
        let t = temp(&format!("bulk-sealed-{mode}"));
        let s = store(&t);
        s.write_body(&inbox(99), &source(99)).unwrap();
        let file = s.data_file("a1").unwrap();
        drop(s);
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_MODE", mode.to_string())];
        let out = super::commits::child("packed::tests::bulk::killed_in_a_bulk_write", &env).wait_with_output().unwrap();
        assert!(!out.status.success() && String::from_utf8_lossy(&out.stdout).contains("written"), "{mode}: the child was to write, then be killed");
        let (repairs, there) = super::commits::repairs_on_open(&t, &file);
        assert!(there, "{mode}: a durable write was lost");
        assert_eq!(repairs == 0, sealed, "{mode}: {repairs} repairs");
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// The child of [`a_bulk_write_s_durable_commits_are_sealed`]: in a bulk write (`bulk`) or not, a
/// write from the window, a run of batches past the cap, one more batch, then killed at once.
#[test]
#[ignore = "run by a_bulk_write_s_durable_commits_are_sealed"]
fn killed_in_a_bulk_write() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    if std::env::var("MC_CHILD_MODE").is_ok_and(|m| m == "bulk") {
        s.bulk_begin("a1", BULK_CACHE);
    }
    s.write_body(&inbox(1), &source(1)).unwrap();
    let past = super::super::commit::UNDURABLE_MAX / (256 << 10) + 2;
    for n in 0..u32::try_from(past).unwrap() {
        let uid = 100 + n;
        s.write_prepared(vec![(inbox(uid), Store::prepare(true, noise(256 << 10, u64::from(uid))))], false).unwrap();
    }
    println!("written");
    std::process::abort();
}
