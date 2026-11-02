//! A data file made smaller: compacted in place, with the account waiting for it and an error
//! leaving it to be opened again; or copied into a new file that takes its place, with what's
//! written meanwhile, stopped at any batch, and a copy a crash left removed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::compact::{Kept, kept_compressed, old_store, packs, read_back};
use super::*;
use crate::packed::shrink::{copy_path, is_copy};

/// A data store at `t` whose account `a1` keeps 40 messages of 64 KiB that don't compress, of
/// which 30 are then deleted: room to give back.
fn with_deleted_mail(t: &std::path::Path) -> (Store, Vec<MessageKey>) {
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    for uid in 1..=40 {
        s.write_body(&key(uid), &noise(64 << 10, u64::from(uid))).unwrap();
    }
    for uid in 11..=40 {
        packs(&s).remove_file(&s.body_path(&key(uid))).unwrap();
    }
    (s, (1..=10).map(key).collect())
}

#[test]
fn deleted_mails_room_goes_back_to_the_drive_in_place() {
    let t = temp("shrink-in-place");
    let (s, kept) = with_deleted_mail(&t);
    let file = s.data_file("a1").unwrap();
    let before = read_back(&s);
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.reclaimable > 1 << 20 && health.pace == 0, "{health:?}");

    let shrunk = s.compact_data_file("a1").unwrap().unwrap();
    assert!(shrunk.after < shrunk.before && !shrunk.copied, "{shrunk:?}");
    assert_eq!(std::fs::metadata(&file).unwrap().len(), shrunk.after);
    assert_eq!(read_back(&s), before, "every file reads back the same");
    for k in &kept {
        assert_eq!(s.read_body(k).unwrap(), noise(64 << 10, u64::from(k.uid)));
    }
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.reclaimable < 64 << 10, "nothing left to give back: {health:?}");
    assert!(health.pace >= mailcraft_model::job::PACE_ONE / 20, "how long it took is kept: {health:?}");
    assert!(!packs(&s).seal_due("a1"), "sealed: a crash after it reopens at once");
    // Written to again, as ever.
    s.write_body(&kept[0], b"Subject: again\r\n\r\nx").unwrap();
    assert_eq!(s.read_body(&kept[0]).unwrap(), b"Subject: again\r\n\r\nx");
    assert!(s.compact_data_file("nobody").unwrap().is_none(), "an account with no data file");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_compaction_that_fails_leaves_the_file_to_be_opened_again() {
    let t = temp("shrink-fails");
    let (s, kept) = with_deleted_mail(&t);
    let before = read_back(&s);
    let p = packs(&s);
    let open_before = p.db("a1", false).unwrap().unwrap();
    *crate::packed::Packs::lock(&p.compact_hook) = Some(Box::new(|| Err(std::io::Error::other("the drive went away"))));
    drop(open_before);
    let e = s.compact_data_file("a1").unwrap_err();
    assert!(e.to_string().contains("the drive went away"), "{e}");
    assert!(!crate::packed::Packs::lock(&p.open).dbs.contains_key("a1"), "closed, not given back");
    *crate::packed::Packs::lock(&p.compact_hook) = None;
    assert_eq!(read_back(&s), before, "opened again as it was");
    assert_eq!(s.read_body(&kept[1]).unwrap(), noise(64 << 10, 2));
    assert!(s.compact_data_file("a1").unwrap().is_some(), "and compacts next time");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn the_account_waits_for_a_compaction_in_place() {
    let t = temp("shrink-waits");
    let (s, kept) = with_deleted_mail(&t);
    let p = packs(&s);
    let (started, start) = mpsc::channel();
    let started = std::sync::Mutex::new(started);
    *crate::packed::Packs::lock(&p.compact_hook) = Some(Box::new(move || {
        let _ = started.lock().unwrap().send(());
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    }));
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        // Readers going on throughout: the compaction still has the file to itself.
        for k in kept.iter().take(4) {
            let (s, stop) = (&s, &stop);
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    // Until it's removed.
                    assert!(s.read_body(k).is_some() || stop.load(Ordering::Relaxed));
                }
            });
        }
        let compacting = scope.spawn(|| {
            let shrunk = s.compact_data_file("a1").unwrap().unwrap();
            (shrunk, Instant::now())
        });
        start.recv().unwrap();
        // A read and a removal asked meanwhile wait for it.
        assert!(s.read_body(&kept[5]).is_some());
        let read_at = Instant::now();
        stop.store(true, Ordering::Relaxed);
        let gone = s.remove_account("a1").unwrap();
        let removed_at = Instant::now();
        let (shrunk, done_at) = compacting.join().unwrap();
        assert!(shrunk.after < shrunk.before);
        assert!(read_at >= done_at && removed_at >= done_at, "they waited for it");
        assert!(gone.is_some_and(|f| f.exists()), "removed after, its data file kept");
    });
    let _ = std::fs::remove_dir_all(t);
}

/// What a data file from before (attachments kept raw) holds: base64 that compresses, raw.
fn bloated() -> Kept {
    (0..24u64).map(|i| (format!("{:016x}/1/{i}.eml", 7), base64(&noise(300 << 10, i)))).collect()
}

#[test]
fn a_copy_takes_the_files_place_with_what_was_written_meanwhile() {
    let t = temp("shrink-copy");
    let kept = bloated();
    let s = old_store(&t, &kept);
    let file = s.data_file("a1").unwrap();
    let before_len = std::fs::metadata(&file).unwrap().len();
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(!health.recompressed && health.recompress_gain > 1 << 20, "{health:?}");
    let mut want = read_back(&s);
    let at = |k: &str| s.root().join("a1").join(k);
    let mut asked = 0;
    let shrunk = s
        .copy_data_file("a1", &mut |w| {
            asked += 1;
            assert!(w.total == 24 && w.done <= w.total, "{w:?}");
            if asked == 2 {
                // Written while it's copied: one it has copied, one it hasn't yet, a new one and
                // one taken out.
                packs(&s).write(&at(&kept[0].0), b"changed early").unwrap();
                packs(&s).write(&at(&kept[23].0), b"changed late").unwrap();
                packs(&s).write(&at("new.json"), b"{}").unwrap();
                packs(&s).remove_file(&at(&kept[1].0)).unwrap();
            }
            true
        })
        .unwrap()
        .unwrap();
    assert!(asked > 2, "in batches: {asked}");
    assert!(shrunk.copied && !shrunk.cancelled && shrunk.after < before_len, "{shrunk:?}");
    want.insert(kept[0].0.clone(), Some(b"changed early".to_vec()));
    want.insert(kept[23].0.clone(), Some(b"changed late".to_vec()));
    want.insert("new.json".into(), Some(b"{}".to_vec()));
    want.remove(&kept[1].0);
    assert_eq!(read_back(&s), want, "every file as it is now");
    assert!(kept_compressed(&s, &kept[5].0), "compressed again on the way");
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompressed && !health.recompress_stopped, "{health:?}");
    assert!(!copy_path(&file).exists(), "the copy took its place");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_stopped_copy_leaves_the_file_as_it_was() {
    let t = temp("shrink-copy-stopped");
    let s = old_store(&t, &bloated());
    let file = s.data_file("a1").unwrap();
    let (len, want) = (std::fs::metadata(&file).unwrap().len(), read_back(&s));
    let mut asked = 0;
    let shrunk = s
        .copy_data_file("a1", &mut |_| {
            asked += 1;
            asked < 3
        })
        .unwrap()
        .unwrap();
    assert!(shrunk.cancelled && !shrunk.copied && shrunk.after == shrunk.before, "{shrunk:?}");
    assert!(!copy_path(&file).exists(), "the copy is thrown away");
    assert_eq!((std::fs::metadata(&file).unwrap().len(), read_back(&s)), (len, want));
    // Writes after it aren't noted for a copy any more.
    assert!(!packs(&s).copying("a1"));
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn a_copy_a_crash_left_is_removed_as_the_files_open() {
    assert!(is_copy("zoe@studio.example.mailcraft.compacting") && is_copy("a.mailcraft.compacting"));
    assert!(!is_copy("zoe.mailcraft") && !is_copy("notes.compacting") && !is_copy("mailcraft.compacting"));
    let t = temp("shrink-crash-copy");
    let s = old_store(&t, &bloated());
    let file = s.data_file("a1").unwrap();
    std::fs::write(copy_path(&file), b"half a copy").unwrap();
    std::fs::write(t.join("Files").join("other.compacting"), b"not ours").unwrap();
    drop(s);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.open_data_files().unwrap();
    assert!(!copy_path(&file).exists(), "removed");
    assert!(t.join("Files").join("other.compacting").exists(), "only a data file's copy");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}
