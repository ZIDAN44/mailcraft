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
