//! A drive that fills up as a data file is written: the write fails saying the drive is full,
//! and once there's room again the file is written to as before, nothing written before lost.

use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use redb::Database;

use super::super::{disk, from_disk, growth};
use super::commits::inbox;
use super::*;

/// `n` bytes that don't compress (a data file keeps what it's given compressed).
fn noise(n: usize, mut x: u64) -> Vec<u8> {
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x.to_le_bytes()[0]
        })
        .collect()
}

/// The drive full as a write grows the data file: the write fails as one for want of room (the
/// disk's own error, not the database's word for it); with room again, the file is opened again
/// and written to, and what was written before the full drive is all there.
#[test]
fn a_full_drive_fails_a_write_as_such_and_room_again_writes() {
    let t = temp("full-drive");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let first = noise(1 << 20, 7);
    s.write_body(&inbox(1), &first).unwrap();
    s.seal("a1").unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs.full.store(true, Ordering::Relaxed);
    let big = noise(64 << 20, 11);
    let e = s.write_body(&inbox(2), &big).unwrap_err();
    assert!(matches!(&e, crate::StoreError::Io { source, .. } if source.kind() == io::ErrorKind::StorageFull), "{e}");
    // Still full: refused the same way, not as a database that gave up after a failed write.
    let e = s.write_body(&inbox(3), &big).unwrap_err();
    assert!(matches!(&e, crate::StoreError::Io { source, .. } if source.kind() == io::ErrorKind::StorageFull), "{e}");
    packs.full.store(false, Ordering::Relaxed);
    s.write_body(&inbox(2), &big).unwrap();
    assert_eq!(s.read_body(&inbox(1)).as_deref(), Some(first.as_slice()));
    assert_eq!(s.read_body(&inbox(2)).map(|b| b.len()), Some(big.len()));
    assert_eq!(s.read_body(&inbox(3)), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// How far a data file can grow in one step, as the database grows it: doubling while small,
/// then to the end of its region, or past the next when its own is nearly full.
#[test]
fn a_data_file_grows_at_most_by_a_region_or_two() {
    const GIB: u64 = 1 << 30;
    assert_eq!(growth(0), 64 << 20);
    assert_eq!(growth(GIB), GIB);
    assert_eq!(growth(4 * GIB - 1), 4 * GIB - 1);
    assert_eq!(growth(4 * GIB), 4 * GIB);
    assert_eq!(growth(38 * GIB), 2 * GIB);
    assert_eq!(growth(39 * GIB + GIB / 2), GIB / 2 + 4 * GIB);
    assert!(growth(u64::MAX) <= 8 * GIB);
}

/// A write refused because an earlier one failed at the disk (on another thread) is one to open
/// the data file again for, as the failed one is; one refused over what was written isn't.
#[test]
fn a_database_refusing_after_a_failed_write_is_opened_again() {
    assert!(from_disk(&disk(redb::Error::PreviousIo)));
    assert!(from_disk(&disk(redb::Error::Io(io::Error::from(io::ErrorKind::StorageFull)))));
    assert!(!from_disk(&disk(redb::Error::ValueTooLarge(1))));
}

/// A data file let go of after a failed write while another thread still holds it (a writer
/// waiting its turn, a reader): the next use waits for that one to let go, then opens it again,
/// and it isn't said to be open in another copy of MailCraft while it can't. A second failure on
/// the old file doesn't let go of the one opened since. The file is counted as let go of once,
/// and a checkpoint counting on what came before is refused with nothing written.
#[test]
fn a_data_file_let_go_after_a_failed_write_waits_for_its_holders() {
    let t = temp("full-held");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&inbox(1), b"one").unwrap();
    s.seal("a1").unwrap();
    assert_eq!(s.data_file_reopened("a1"), 0);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let held = packs.db("a1", false).unwrap().unwrap();
    packs.full.store(true, Ordering::Relaxed);
    let big = noise(64 << 20, 5);
    s.write_body(&inbox(2), &big).unwrap_err();
    packs.full.store(false, Ordering::Relaxed);
    assert_eq!(s.data_file_reopened("a1"), 1);
    // Held all along: said as what it is, not as another copy's.
    let e = s.write_body(&inbox(3), b"x").unwrap_err().to_string();
    assert!(e.contains("opened again") && !e.contains("another copy"), "{e}");
    // The old file failing again on another thread lets go of nothing more.
    assert!(!packs.let_go_failed("a1", &held));
    // Let go of a moment later: waited for, then opened again and written to.
    std::thread::scope(|sc| {
        sc.spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(held);
        });
        s.write_body(&inbox(3), b"three").unwrap();
    });
    assert_eq!(s.read_body(&inbox(1)).as_deref(), Some(&b"one"[..]));
    let now = packs.db("a1", false).unwrap().unwrap();
    let stale = Arc::new(Database::create(t.join("stale.redb")).unwrap());
    assert!(!packs.let_go_failed("a1", &stale), "only the file that failed is let go of");
    assert!(Arc::ptr_eq(&now, &packs.db("a1", false).unwrap().unwrap()));
    assert_eq!(s.data_file_reopened("a1"), 1);
    // A checkpoint counting on the file before it was let go of: refused, nothing written.
    let doc = || vec![("import.json", br#"{"next":7}"#.to_vec())];
    s.write_batch("a1", Vec::new(), doc(), 0).unwrap_err();
    assert_eq!(s.load_account_doc::<Option<serde_json::Value>>("a1", "import.json").unwrap(), None);
    s.write_batch("a1", Vec::new(), doc(), 1).unwrap();
    assert!(s.load_account_doc::<Option<serde_json::Value>>("a1", "import.json").unwrap().is_some());
    drop((now, stale, s));
    let _ = std::fs::remove_dir_all(&t);
}
