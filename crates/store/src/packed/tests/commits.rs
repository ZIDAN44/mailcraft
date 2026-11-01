//! How a data file commits: sources prepared away from the writer, background batches capped,
//! durability, and quick-repair (what a crash costs the next open).

use std::time::{Duration, Instant};

use redb::{ReadableDatabase, ReadableTable};

use super::*;

/// This test binary again, running only `test` (an ignored one that does nothing unless `env`
/// is set): a process that can be killed part-way. What it prints starts on the test harness's
/// own line.
pub(super) fn child(test: &str, env: &[(&str, String)]) -> std::process::Child {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--ignored", "--exact", test, "--nocapture", "--test-threads=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().unwrap()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// (median, mean, 90th percentile) in ms.
fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    (at(0.5), v.iter().sum::<f64>() / v.len().max(1) as f64, at(0.9))
}

pub(super) fn inbox(uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid }
}

/// A data store with account `a1`, at `t`.
fn packed_store(t: &std::path::Path) -> Store {
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s
}

/// Sources of every kind: text, a big text one, noise that doesn't compress (one in pieces), a
/// tiny one, and one that is only just too small to compress.
fn sources(seed: u64) -> Vec<Vec<u8>> {
    let text =
        |n: usize| b"Subject: hello\r\n\r\n".iter().copied().chain(b"the quick brown fox ".iter().copied().cycle().take(n)).collect::<Vec<u8>>();
    vec![
        text(40_000),
        text(3 << 20),
        noise(1_500_000, seed),
        noise(super::super::WHOLE_MAX + 1, seed + 1),
        b"Subject: x\r\n\r\ny".to_vec(),
        text(1000),
    ]
}

/// Sources prepared away from the writer read back as written, through batches committed with
/// and without waiting for the disk, in a data file, on disk and in memory; prepared for a data
/// file, what compresses is kept compressed.
#[test]
fn prepared_sources_read_back_as_written() {
    let t = temp("prepared");
    let disk = t.join("disk");
    for (name, s) in [("packed", packed_store(&t)), ("disk", Store::open(&disk).unwrap()), ("memory", Store::memory())] {
        let packs = s.packs("a1");
        assert_eq!(packs, name == "packed", "{name}");
        for (round, durable) in [(0u32, false), (1, true)] {
            let raw = sources(u64::from(round) + 7);
            let prepared: Vec<(MessageKey, crate::Prepared)> =
                raw.iter().enumerate().map(|(i, r)| (inbox(round * 100 + i as u32), Store::prepare(packs, r.clone()))).collect();
            for (p, r) in prepared.iter().zip(&raw) {
                assert_eq!(p.1.raw_len(), r.len());
            }
            if packs {
                assert!(prepared[1].1.kept_len() * 10 < raw[1].len(), "text is kept compressed");
                assert_eq!(prepared[2].1.kept_len(), raw[2].len(), "noise is kept as it is");
            } else {
                assert!(prepared.iter().zip(&raw).all(|(p, r)| p.1.kept_len() == r.len()), "{name}: kept as it is");
            }
            s.write_prepared(prepared, durable).unwrap();
            for (i, r) in raw.iter().enumerate() {
                assert_eq!(s.read_body(&inbox(round * 100 + i as u32)).as_ref(), Some(r), "{name} round {round} source {i}");
            }
        }
    }
    // And after opening again.
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    assert_eq!(s.read_body(&inbox(2)), Some(sources(7)[2].clone()));
    assert_eq!(s.read_body(&inbox(101)), Some(sources(8)[1].clone()));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A file kept in pieces written over (by a smaller one in pieces, then a whole one) leaves none
/// of its old pieces behind, now that pieces are looked for only where a file was.
#[test]
fn overwriting_a_file_in_pieces_leaves_no_old_pieces() {
    let t = temp("overwrite-pieces");
    let s = packed_store(&t);
    let pieces_of = |s: &Store, key: &str| -> usize {
        let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
        let db = packs.db("a1", false).unwrap().unwrap();
        let tx = db.begin_read().unwrap();
        let table = tx.open_table(crate::packed::PIECES).unwrap();
        table.range((key, 0)..=(key, u32::MAX)).unwrap().count()
    };
    let key = s.body_path(&inbox(1));
    let inside = key.strip_prefix(t.join("app").join("a1")).unwrap().to_str().unwrap().replace('\\', "/");
    let big = noise(3_000_000, 3);
    s.write_prepared(vec![(inbox(1), Store::prepare(true, big.clone()))], true).unwrap();
    let before = pieces_of(&s, &inside);
    assert_eq!(before, super::super::piece_sizes(big.len()).len());
    let smaller = noise(200_000, 4);
    s.write_prepared(vec![(inbox(1), Store::prepare(true, smaller.clone()))], false).unwrap();
    assert_eq!(pieces_of(&s, &inside), super::super::piece_sizes(smaller.len()).len());
    assert!(pieces_of(&s, &inside) < before);
    assert_eq!(s.read_body(&inbox(1)), Some(smaller));
    s.write_body(&inbox(1), b"Subject: small now\r\n\r\nx").unwrap();
    assert_eq!(pieces_of(&s, &inside), 0);
    assert_eq!(s.read_body(&inbox(1)).unwrap(), b"Subject: small now\r\n\r\nx");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A batch that didn't wait for the disk is made durable by the next write that does, and by the
/// data file closing; then it's all there when opened again (on disk and in memory, where every
/// write is as durable as the store is, the same calls keep everything).
#[test]
fn a_batch_not_waited_for_is_kept_by_the_next_durable_write() {
    let t = temp("not-durable");
    let raw = sources(11);
    {
        let s = packed_store(&t);
        s.write_prepared(raw.iter().enumerate().map(|(i, r)| (inbox(i as u32), Store::prepare(true, r.clone()))).collect(), false).unwrap();
        s.save_folder("a1", "INBOX", &crate::FolderState { validity: 1, uid_next: 7, ..Default::default() }).unwrap();
        s.write_prepared(vec![(inbox(50), Store::prepare(true, raw[0].clone()))], false).unwrap();
    }
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    for (i, r) in raw.iter().enumerate() {
        assert_eq!(s.read_body(&inbox(i as u32)).as_ref(), Some(r), "source {i}");
    }
    assert_eq!(s.read_body(&inbox(50)).as_ref(), Some(&raw[0]), "made durable by closing");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().uid_next, 7);
    drop(s);
    let disk = Store::open(t.join("disk")).unwrap();
    disk.write_prepared(vec![(inbox(1), Store::prepare(false, raw[1].clone()))], false).unwrap();
    disk.seal("a1").unwrap();
    assert_eq!(Store::open(t.join("disk")).unwrap().read_body(&inbox(1)).as_ref(), Some(&raw[1]));
    let mem = Store::memory();
    mem.write_prepared(vec![(inbox(1), Store::prepare(false, raw[1].clone()))], false).unwrap();
    mem.seal("a1").unwrap();
    assert_eq!(mem.read_body(&inbox(1)).as_ref(), Some(&raw[1]));
    let _ = std::fs::remove_dir_all(&t);
}

/// The sources of batch `n` of [`killed_during_write_bodies`]: one in pieces among text ones.
fn crash_batch(n: u32) -> Vec<(MessageKey, Vec<u8>)> {
    (0..12)
        .map(|i| {
            let raw = if i == 5 {
                noise(300_000, u64::from(n) * 31 + 1)
            } else {
                format!("Subject: {n}/{i}\r\n\r\n{}", "words ".repeat(400 + i as usize)).into_bytes()
            };
            (inbox(n * 100 + i), raw)
        })
        .collect()
}

/// The process aborted from another thread while it writes sources (at a different moment for
/// each seed): every batch whose write had returned reads back whole when the store opens again.
#[test]
fn a_crash_during_write_bodies_keeps_what_was_committed() {
    for seed in 1..=4u64 {
        let t = temp(&format!("crash-{seed}"));
        drop(packed_store(&t));
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_SEED", seed.to_string())];
        let out = child("packed::tests::commits::killed_during_write_bodies", &env).wait_with_output().unwrap();
        assert!(!out.status.success(), "seed {seed}: the child was to be killed");
        let done: Vec<u32> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split("done ").nth(1)?.trim().parse().ok()).collect();
        assert!(
            !done.is_empty(),
            "seed {seed}: nothing was written before the kill ({:?}): {}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        for n in &done {
            for (k, raw) in crash_batch(*n) {
                assert_eq!(s.read_body(&k).as_ref(), Some(&raw), "seed {seed}, batch {n}, {k:?}");
            }
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// The child of [`a_crash_during_write_bodies_keeps_what_was_committed`].
#[test]
#[ignore = "run by a_crash_during_write_bodies_keeps_what_was_committed"]
fn killed_during_write_bodies() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let seed: u64 = std::env::var("MC_CHILD_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    for n in 0..u32::MAX {
        s.write_bodies(&crash_batch(n)).unwrap();
        println!("done {n}");
        // The clock starts once the file is open and written: the kill lands among the writes.
        if n == 0 {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(7 + seed * 23));
                std::process::abort();
            });
        }
    }
}

/// How many times opening `file` had to repair it, and whether `inbox(1)` then reads back.
pub(super) fn repairs_on_open(t: &std::path::Path, file: &std::path::Path) -> (u32, bool) {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let seen = calls.clone();
    let db = redb::Database::builder()
        .set_repair_callback(move |_| {
            seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .create(file)
        .unwrap();
    drop(db);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    (calls.load(std::sync::atomic::Ordering::Relaxed), s.read_body(&inbox(1)).is_some())
}

/// A data file is sealed once its writes go quiet, where a writer asks (an import's end), and as
/// MailCraft closes, which also seals any write after it (a thread still syncing as the process
/// ends, with the store never dropped): a crash after that reopens with no repair. A crash right
/// after a plain write needs one (each commit isn't sealed: that tripled a small one's time on a
/// big file), and still loses nothing.
#[test]
fn a_data_file_is_sealed_once_quiet_and_when_asked() {
    for (mode, sealed) in [("now", false), ("quiet", true), ("asked", true), ("closing", true), ("closed", true)] {
        let t = temp(&format!("seal-{mode}"));
        let s = packed_store(&t);
        s.write_body(&inbox(99), b"Subject: first\r\n\r\nx").unwrap();
        let file = s.data_file("a1").unwrap();
        drop(s);
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_MODE", mode.to_string())];
        let out = child("packed::tests::commits::killed_after_a_write", &env).wait_with_output().unwrap();
        assert!(!out.status.success() && String::from_utf8_lossy(&out.stdout).contains("written"), "{mode}: the child was to write, then be killed");
        let (repairs, there) = repairs_on_open(&t, &file);
        assert!(there, "{mode}: the write was lost");
        assert_eq!(repairs == 0, sealed, "{mode}: {repairs} repairs");
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// The child of [`a_data_file_is_sealed_once_quiet_and_when_asked`]: one write, then killed at once
/// (`now`), once the file has been quiet a while (`quiet`), after asking for a seal (`asked`), or
/// after the closing seal (`closing`); or the closing seal, then the write, then killed (`closed`).
#[test]
#[ignore = "run by a_data_file_is_sealed_once_quiet_and_when_asked"]
fn killed_after_a_write() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let mode = std::env::var("MC_CHILD_MODE").unwrap_or_default();
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    // The process ending with a thread still writing: what it writes after the closing seal.
    if mode == "closed" {
        s.seal_all().unwrap();
    }
    s.write_body(&inbox(1), b"Subject: kept\r\n\r\nx").unwrap();
    match mode.as_str() {
        "quiet" => std::thread::sleep(super::super::commit::QUIET + Duration::from_millis(1500)),
        "asked" => s.seal("a1").unwrap(),
        "closing" => s.seal_all().unwrap(),
        _ => {}
    }
    println!("written");
    std::process::abort();
}

/// An import's checkpoint goes in with its batch, in one commit that waits for the disk and is
/// sealed: the batches before it are on the disk with it, and nothing waits for a seal.
#[test]
fn a_checkpoint_and_its_batch_are_one_sealed_commit() {
    let t = temp("checkpoint");
    let s = packed_store(&t);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let mut uid = 1000u32;
    s.write_prepared(background_batch(&mut uid), false).unwrap();
    assert!(packs.seals.undurable("a1").0 > 0 && packs.seals.waiting_for("a1"));
    let batch = background_batch(&mut uid);
    let keys: Vec<MessageKey> = batch.iter().map(|(k, _)| k.clone()).collect();
    s.write_batch("a1", batch, vec![("import.json", br#"{"next":7}"#.to_vec())], 0).unwrap();
    assert_eq!(packs.seals.undurable("a1").0, 0, "the batches before it are on the disk");
    assert!(!packs.seals.waiting_for("a1"), "and it's sealed");
    assert!(keys.iter().all(|k| s.has_body(k)));
    assert_eq!(s.load_account_doc::<serde_json::Value>("a1", "import.json").unwrap(), serde_json::json!({"next": 7}));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A sealed commit (a search index's meta) clears only the writes before it: one that came in as
/// soon as it let go of the file still waits for its seal.
#[test]
fn a_sealed_commit_leaves_a_later_write_waiting_for_its_seal() {
    let t = temp("sealed-race");
    let s = packed_store(&t);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let at = Instant::now();
    packs.seals.written_at("a1", at.checked_sub(Duration::from_millis(5)).unwrap_or(at));
    packs.seals.sealed("a1", at);
    assert!(!packs.seals.waiting_for("a1"), "a write before the seal waits no more");
    packs.seals.written_at("a1", at + Duration::from_millis(5));
    packs.seals.sealed("a1", at);
    assert!(packs.seals.waiting_for("a1"), "a write after the seal began lost its seal");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// An account removed lets go of its data file for good: a seal under way ends first (it held the
/// file open, so it couldn't be opened again or deleted), and none is left waiting for it.
#[test]
fn a_data_file_let_go_is_not_held_by_a_seal() {
    let t = temp("let-go");
    let s = packed_store(&t);
    s.write_prepared(vec![(inbox(1), Store::prepare(true, b"Subject: x\r\n\r\nx".to_vec()))], false).unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    assert!(packs.seals.waiting_for("a1"));
    packs.seals.pretend_sealing(Some("a1"));
    let gone = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let remover = {
        let (s, gone) = (s.clone(), gone.clone());
        std::thread::spawn(move || {
            let file = s.remove_account("a1").unwrap();
            gone.store(true, std::sync::atomic::Ordering::SeqCst);
            file
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!gone.load(std::sync::atomic::Ordering::SeqCst), "let go of while a seal held it");
    packs.seals.pretend_sealing(None);
    let file = remover.join().unwrap().expect("the data file is kept");
    assert!(!packs.seals.waiting_for("a1"), "a seal still waits for a file let go of");
    drop(redb::Database::create(&file).expect("nothing holds it open"));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Batch `n` of [`killed_after_batches`]: 2 MiB that doesn't compress.
fn undurable_batch(n: u32) -> Vec<(MessageKey, Vec<u8>)> {
    (0..8).map(|i| (inbox(n * 100 + i), noise(256 << 10, u64::from(n * 100 + i) + 3))).collect()
}

/// A long run of batches that don't wait for the disk doesn't go on forever: the one that takes
/// it past the cap is made durable, so a crash after it keeps everything up to it (and a write
/// from the window isn't left writing out the whole run).
#[test]
fn a_long_run_of_batches_is_made_durable() {
    let t = temp("undurable-run");
    drop(packed_store(&t));
    let out = child("packed::tests::commits::killed_after_batches", &[("MC_CHILD_DIR", t.display().to_string())]).wait_with_output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stdout).contains("written"), "the child was to write, then be killed");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let cap = super::super::commit::UNDURABLE_MAX as u32 / (2 << 20);
    for n in 0..=cap {
        for (k, raw) in undurable_batch(n) {
            assert_eq!(s.read_body(&k).as_ref(), Some(&raw), "batch {n} of a run past the cap was lost");
        }
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// The child of [`a_long_run_of_batches_is_made_durable`]: 40 MiB in batches not waited for, then
/// killed at once.
#[test]
#[ignore = "run by a_long_run_of_batches_is_made_durable"]
fn killed_after_batches() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    for n in 0..20 {
        let batch = undurable_batch(n).into_iter().map(|(k, raw)| (k, Store::prepare(true, raw))).collect();
        s.write_prepared(batch, false).unwrap();
    }
    println!("written");
    std::process::abort();
}

/// Only writes from the window are timed for the writer: a background batch, made durable or
/// not, isn't one of them.
#[test]
fn only_writes_from_the_window_are_timed_for_the_writer() {
    let t = temp("timed");
    let s = packed_store(&t);
    let writer = {
        let s = s.clone();
        std::thread::spawn(move || {
            let mut uid = 1000u32;
            for _ in 0..super::super::commit::UNDURABLE_MAX / crate::BACKGROUND_COMMIT + 3 {
                s.write_prepared(background_batch(&mut uid), false).unwrap();
            }
        })
    };
    for i in 0..20u32 {
        s.write_body(&inbox(1), format!("Subject: x\r\n\r\nread {i}").as_bytes()).unwrap();
    }
    writer.join().unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    assert_eq!(packs.seals.waits.lock().unwrap().len(), 20, "only the writes from the window are timed");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// What a data file has waiting for the disk is counted exactly: each batch adds what it keeps, a
/// durable write or a seal writes it all out. Against several writers at once, sealing and
/// writing from the window between them, it never passes the cap: the count and the look at it
/// are made with the file's writer held, so a batch can't be lost from it, and two can't both
/// take it past the cap.
#[test]
fn what_waits_for_the_disk_is_counted_and_capped() {
    let t = temp("undurable-count");
    let s = packed_store(&t);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let mut uid = 1000u32;
    s.write_body(&inbox(0), b"Subject: x\r\n\r\nx").unwrap();
    assert_eq!(packs.seals.undurable("a1").0, 0);
    let mut kept = 0;
    for _ in 0..3 {
        let batch = background_batch(&mut uid);
        kept += batch.iter().map(|(_, p)| p.kept_len()).sum::<usize>();
        s.write_prepared(batch, false).unwrap();
        assert_eq!(packs.seals.undurable("a1").0, kept);
    }
    s.seal("a1").unwrap();
    assert_eq!(packs.seals.undurable("a1").0, 0, "a seal writes out what was waiting");
    s.write_prepared(background_batch(&mut uid), false).unwrap();
    s.write_body(&inbox(0), b"Subject: x\r\n\r\ny").unwrap();
    assert_eq!(packs.seals.undurable("a1").0, 0, "a write from the window writes out what was waiting");

    let writers: Vec<_> = (0..4u32)
        .map(|n| {
            let s = s.clone();
            std::thread::spawn(move || {
                let mut uid = 10_000 * (n + 1);
                for _ in 0..8 {
                    s.write_prepared(background_batch(&mut uid), false).unwrap();
                }
            })
        })
        .collect();
    for i in 0..10u32 {
        s.write_body(&inbox(1), format!("Subject: x\r\n\r\nread {i}").as_bytes()).unwrap();
        if i % 3 == 0 {
            s.seal("a1").unwrap();
        }
    }
    for w in writers {
        w.join().unwrap();
    }
    let (now, peak, _) = packs.seals.undurable("a1");
    assert!(peak <= super::super::commit::UNDURABLE_MAX, "{peak} bytes waited for the disk");
    assert!(now <= peak);
    s.seal("a1").unwrap();
    assert_eq!(packs.seals.undurable("a1").0, 0);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Bringing in a MailCraft data file writes as an import does: in batches of at most
/// [`crate::BACKGROUND_COMMIT`] that don't wait for the disk (it was one durable transaction of
/// up to 32 MiB, compressed in it), and sealed at the end, so nothing is left waiting.
#[test]
fn a_data_file_brought_in_goes_in_short_batches() {
    let t = temp("bring-in-batches");
    let old = packed_store(&t);
    let raw: Vec<(MessageKey, Vec<u8>)> = (0..24u32).map(|i| (inbox(i), noise(700 << 10, u64::from(i) + 11))).collect();
    old.write_bodies(&raw).unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);
    let s = Store::open_packed(t.join("new"), t.join("New Files")).unwrap();
    s.save_accounts(&[account("b2", "ana@example.org")]).unwrap();
    s.import_data_file("b2", &file).unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let (now, peak, biggest) = packs.seals.undurable("b2");
    assert_eq!(now, 0, "sealed at the end");
    assert!(biggest > 0 && biggest <= crate::BACKGROUND_COMMIT, "a batch of {biggest} bytes");
    assert!(peak <= super::super::commit::UNDURABLE_MAX);
    for (k, r) in &raw {
        let mine = MessageKey { account: "b2".into(), ..k.clone() };
        assert_eq!(s.read_body(&mine).as_ref(), Some(r));
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Writes from the window (a message marked read: a small durable commit) while another thread
/// writes capped batches as an import does, as fast as it can. The wait for the writer is what
/// the store decides, and stays short. The whole write also waits for the disk, which that writer
/// keeps busy: measured on a busy machine, its p99 was 45 to 230 ms (7 to 11 ms median) whether
/// the batches waited for the disk or not, and 13 ms with no writer, which misses the 60 ms aimed
/// at. A measurement, not run with the rest: timings on a busy machine or a shared runner swing
/// too far for a test to hold them (run alone, the wait's p99 was 6 to 31 ms; beside the other
/// tests it reached 61). What doesn't depend on the clock is tested by
/// [`only_writes_from_the_window_are_timed_for_the_writer`] and
/// [`what_waits_for_the_disk_is_counted_and_capped`].
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn a_write_from_the_window_waits_little_for_a_background_writer() {
    const { assert!(crate::BACKGROUND_COMMIT <= crate::WRITE_BUFFER / 4 && crate::BACKGROUND_COMMIT <= 8 << 20) };
    let t = temp("latency");
    let s = packed_store(&t);
    s.write_body(&inbox(0), b"Subject: x\r\n\r\nx").unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (s, stop) = (s.clone(), stop.clone());
        std::thread::spawn(move || {
            let (mut uid, mut batches) = (1000u32, 0u32);
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                s.write_prepared(background_batch(&mut uid), false).unwrap();
                batches += 1;
            }
            batches
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs.seals.waits.lock().unwrap().clear();
    let mut whole = Vec::new();
    for i in 0..100u32 {
        let t0 = Instant::now();
        s.write_body(&inbox(1), format!("Subject: x\r\n\r\nread {i}").as_bytes()).unwrap();
        whole.push(ms(t0.elapsed()));
        std::thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let batches = writer.join().unwrap();
    let waits: Vec<f64> = packs.seals.waits.lock().unwrap().drain(..).map(ms).collect();
    assert_eq!(waits.len(), 100, "only the writes from the window are timed");
    let (wait, write) = (spread(waits.clone()), spread(whole.clone()));
    let (wait99, write99) = (p99(waits), p99(whole));
    eprintln!(
        "{batches} background batches; waits for the writer: median {:.1} ms, p99 {wait99:.1} ms; whole writes: median {:.1} ms, p99 {write99:.1} ms",
        wait.0, write.0
    );
    assert!(batches >= 5, "the background writer wrote {batches} batches");
    assert!(wait99 <= 60.0, "a write from the window waited {wait99:.1} ms for the writer (p99)");
    assert!(write99 <= 250.0, "a write from the window took {write99:.1} ms (p99)");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// What a write from the window costs when background batches as far as the cap haven't gone to
/// the disk yet (it writes them out with its own): 10 to 30 ms more than with none, measured. A
/// measurement, as the one above.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn a_write_from_the_window_writes_out_little_of_a_backlog() {
    let t = temp("backlog");
    let s = packed_store(&t);
    s.write_body(&inbox(0), b"Subject: x\r\n\r\nx").unwrap();
    let per = crate::BACKGROUND_COMMIT;
    let (mut uid, mut alone, mut after) = (1000u32, Vec::new(), Vec::new());
    for i in 0..12u32 {
        let t0 = Instant::now();
        s.write_body(&inbox(1), format!("Subject: x\r\n\r\nalone {i}").as_bytes()).unwrap();
        alone.push(ms(t0.elapsed()));
        // Up to the cap, not over it: none of these is made durable itself.
        for _ in 0..super::super::commit::UNDURABLE_MAX / per {
            s.write_prepared(background_batch(&mut uid), false).unwrap();
        }
        let t0 = Instant::now();
        s.write_body(&inbox(1), format!("Subject: x\r\n\r\nafter {i}").as_bytes()).unwrap();
        after.push(ms(t0.elapsed()));
    }
    let (alone, after) = (spread(alone), spread(after));
    eprintln!(
        "a write from the window: median {:.1} ms alone, {:.1} ms after {} MiB of batches",
        alone.0,
        after.0,
        super::super::commit::UNDURABLE_MAX >> 20
    );
    assert!(after.0 <= 60.0, "a write after the batches took {:.1} ms (median)", after.0);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A background batch of [`crate::BACKGROUND_COMMIT`]: noise, kept as it is (what each batch holds
/// the file for is storing it).
fn background_batch(uid: &mut u32) -> Vec<(MessageKey, crate::Prepared)> {
    let one = noise(100 << 10, 5);
    (0..crate::BACKGROUND_COMMIT / one.len())
        .map(|_| {
            *uid += 1;
            (inbox(*uid), Store::prepare(true, one.clone()))
        })
        .collect()
}

/// The 99th percentile of `v`.
fn p99(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get((v.len() * 99 / 100).saturating_sub(1)).copied().unwrap_or(0.0)
}

/// One commit of `items` (as kept) into `db`, timed.
fn timed_commit(db: &redb::Database, quick: bool, items: &[(String, Vec<u8>)]) -> Duration {
    let t = Instant::now();
    let mut tx = db.begin_write().unwrap();
    tx.set_quick_repair(quick);
    {
        let mut tables =
            crate::packed::Tables { files: tx.open_table(crate::packed::FILES).unwrap(), pieces: tx.open_table(crate::packed::PIECES).unwrap() };
        for (k, v) in items {
            tables.put_kept(k, v).unwrap();
        }
    }
    tx.commit().unwrap();
    t.elapsed()
}

/// Take out what the measurements wrote (quick-repair on, so the file is left as it was found).
fn tidy(db: &redb::Database) {
    let mut tx = db.begin_write().unwrap();
    tx.set_quick_repair(true);
    {
        let mut tables =
            crate::packed::Tables { files: tx.open_table(crate::packed::FILES).unwrap(), pieces: tx.open_table(crate::packed::PIECES).unwrap() };
        let keys: Vec<String> =
            tables.files.range("zz-commit-measure/".."zz-commit-measure0").unwrap().map(|r| r.unwrap().0.value().to_string()).collect();
        for k in keys {
            tables.take_kept(&k).unwrap();
        }
    }
    tx.commit().unwrap();
}

/// A 16 MiB commit as an import makes one: 160 sources of 100 KiB that don't compress.
fn batch(round: usize) -> Vec<(String, Vec<u8>)> {
    (0..160).map(|i| (format!("zz-commit-measure/{round}-{i}"), noise(100 << 10, (round * 1000 + i) as u64 + 1))).collect()
}

/// What quick-repair costs a commit on a big data file: a one-key commit and a 16 MiB one, each
/// with and without it, taken in turns. The commit policy is decided from these numbers. Run on a
/// copy (it writes, then takes out what it wrote): `MC_QUICK_FILE=<copy>.mailcraft cargo test
/// -p mailcraft-store --release quick_repair_cost -- --ignored --nocapture`.
#[test]
#[ignore = "needs a copy of a real data file in MC_QUICK_FILE"]
fn quick_repair_cost() {
    let Some(path) = std::env::var_os("MC_QUICK_FILE") else { return };
    let t = Instant::now();
    let db = redb::Database::builder().set_cache_size(crate::CACHE).create(&path).unwrap();
    eprintln!("opened in {:.0} ms", ms(t.elapsed()));
    let one = |round: usize| vec![(format!("zz-commit-measure/one-{round}"), b"{\"seen\":true}".to_vec())];
    for r in 0..3 {
        timed_commit(&db, false, &one(r));
        timed_commit(&db, true, &one(r));
    }
    let (mut plain, mut quick) = (Vec::new(), Vec::new());
    for r in 0..60 {
        // In turns, each going first half the time.
        let (a, b) = if r % 2 == 0 { (false, true) } else { (true, false) };
        for q in [a, b] {
            let d = ms(timed_commit(&db, q, &one(r)));
            if q { quick.push(d) } else { plain.push(d) }
        }
    }
    let (p, q) = (spread(plain), spread(quick));
    eprintln!("one key   plain median {:.2} mean {:.2} p90 {:.2} | quick median {:.2} mean {:.2} p90 {:.2} ms", p.0, p.1, p.2, q.0, q.1, q.2);
    eprintln!("one key   quick-repair adds {:.2} ms ({:.1}%) by median, {:.2} ms by mean", q.0 - p.0, (q.0 - p.0) / p.0 * 100.0, q.1 - p.1);
    let (mut plain, mut quick) = (Vec::new(), Vec::new());
    for r in 0..12 {
        let (a, b) = if r % 2 == 0 { (false, true) } else { (true, false) };
        for (i, q) in [a, b].into_iter().enumerate() {
            let items = batch(r * 2 + i);
            let d = ms(timed_commit(&db, q, &items));
            if q { quick.push(d) } else { plain.push(d) }
        }
    }
    let (p, q) = (spread(plain), spread(quick));
    eprintln!("16 MiB    plain median {:.1} mean {:.1} p90 {:.1} | quick median {:.1} mean {:.1} p90 {:.1} ms", p.0, p.1, p.2, q.0, q.1, q.2);
    eprintln!("16 MiB    quick-repair adds {:.1} ms ({:.1}%) by median", q.0 - p.0, (q.0 - p.0) / p.0 * 100.0);
    tidy(&db);
}

/// The process killed while it commits 16 MiB batches, then the file opened again: how long that
/// takes and whether redb had to repair it. `MC_QUICK_FILE` as for [`quick_repair_cost`];
/// `MC_QUICK_KILL_PLAIN=1` kills it during plain commits instead (a full repair: slow on a big file).
#[test]
#[ignore = "needs a copy of a real data file in MC_QUICK_FILE"]
fn quick_repair_reopen_after_kill() {
    let Some(path) = std::env::var_os("MC_QUICK_FILE") else { return };
    let quick = std::env::var_os("MC_QUICK_KILL_PLAIN").is_none();
    let path = std::path::PathBuf::from(path);
    let env = [("MC_KILL_FILE", path.display().to_string()), ("MC_KILL_QUICK", if quick { "1" } else { "0" }.to_string())];
    let out = child("packed::tests::commits::killed_while_committing", &env).wait_with_output().unwrap();
    assert!(!out.status.success(), "the child was to be killed");
    let committed = String::from_utf8_lossy(&out.stdout).matches("committed ").count();
    let repairs = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let seen = repairs.clone();
    let t = Instant::now();
    let db = redb::Database::builder().set_cache_size(crate::CACHE).set_repair_callback(move |_| seen.set(seen.get() + 1)).create(&path).unwrap();
    eprintln!(
        "quick {quick}: {committed} batches committed before the kill; reopened in {:.0} ms, {} repair callbacks",
        ms(t.elapsed()),
        repairs.get()
    );
    tidy(&db);
}

/// The child of [`quick_repair_reopen_after_kill`]: commits 16 MiB batches until a second thread
/// aborts the process part-way through one.
#[test]
#[ignore = "run by quick_repair_reopen_after_kill"]
fn killed_while_committing() {
    let Some(path) = std::env::var_os("MC_KILL_FILE") else { return };
    let quick = std::env::var("MC_KILL_QUICK").as_deref() == Ok("1");
    let db = redb::Database::builder().set_cache_size(crate::CACHE).create(path).unwrap();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        std::process::abort();
    });
    for round in 0..usize::MAX {
        timed_commit(&db, quick, &batch(1000 + round));
        println!("committed {round}");
    }
}
