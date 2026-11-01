//! Compressing again what a data file from before keeps raw: everything reads back the same, a
//! file written meanwhile is left as written, a run stopped or killed goes on from where it got
//! to, a write from the window waits little for it, and hostile files don't stop it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use mailcraft_model::job::progress::Work;
use redb::ReadableDatabase;

use super::commits::child;
use super::*;
use crate::packed::compact::{Account, Step, Target};
use crate::packed::{Packs, Tables};

/// What a data file from before holds: path → bytes as kept.
type Kept = Vec<(String, Vec<u8>)>;

/// A data store at `t` whose account `a1` has a data file as a build before this one left it: no
/// `META`, and `kept` as it was kept (raw or compressed, whole or in pieces).
fn old_store(t: &std::path::Path, kept: &Kept) -> Store {
    let (app, files) = (t.join("app"), t.join("Files"));
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(app.join("datafiles.json"), br#"{"a1": "old.mailcraft"}"#).unwrap();
    {
        let db = redb::Database::create(files.join("old.mailcraft")).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut tables = Tables { files: tx.open_table(super::super::FILES).unwrap(), pieces: tx.open_table(super::super::PIECES).unwrap() };
            for (k, v) in kept {
                tables.put_kept(k, v).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s
}

fn packs(s: &Store) -> &Packs {
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    packs
}

/// Mail text of about `n` bytes (compresses well).
fn text(n: usize, seed: u64) -> Vec<u8> {
    format!("Subject: note {seed}\r\n\r\n").bytes().chain(b"the figures for the quarter are in ".iter().copied().cycle().take(n)).collect()
}

/// Every file of `a1` as it reads back (`None`: it doesn't).
fn read_back(s: &Store) -> BTreeMap<String, Option<Vec<u8>>> {
    let p = packs(s);
    p.keys("a1", "").unwrap().into_iter().map(|(k, _)| (k.clone(), p.get("a1", &k).ok().flatten())).collect()
}

/// Whether the file at `key` is kept compressed (its first bytes, or its first piece's, say so).
fn kept_compressed(s: &Store, key: &str) -> bool {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    let tx = db.begin_read().unwrap();
    let files = tx.open_table(super::super::FILES).unwrap();
    let v = files.get(key).unwrap().unwrap().value().to_vec();
    if super::super::pieced_len(&v).is_none() {
        return v.starts_with(super::super::DEFLATED);
    }
    let pieces = tx.open_table(super::super::PIECES).unwrap();
    pieces.get((key, 0)).unwrap().unwrap().value().starts_with(super::super::DEFLATED)
}

/// A data file from before: base64 attachments kept raw (whole and in pieces, one of them bigger
/// than a batch), pictures kept raw (they don't compress), text compressed, small files, the
/// search index's pages, and a file in pieces whose pieces are gone. Also which keys are the base64
/// ones, and the pictures.
fn an_old_file(base64_files: u32) -> (Kept, Vec<String>, Vec<String>) {
    let (mut kept, mut b64, mut pictures) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..base64_files {
        let key = format!("INBOX/1/{i}.eml");
        let n = match i % 4 {
            0 => 2_000,
            1 => 30_000,
            2 => 120_000,
            _ => 9_000,
        } + (i as usize % 7) * 500;
        kept.push((key.clone(), base64(&noise(n, u64::from(i) + 1))));
        b64.push(key);
    }
    for i in 0..base64_files / 4 {
        let key = format!("Pictures/1/{i}.eml");
        kept.push((key.clone(), noise(5_000 + i as usize * 3_000, u64::from(i) + 7_000)));
        pictures.push(key);
    }
    for i in 0..base64_files / 4 {
        kept.push((format!("Notes/1/{i}.eml"), super::super::pack(&text(6_000, u64::from(i))).into_owned()));
        kept.push((format!("Tiny/1/{i}.eml"), b"Subject: x\r\n\r\nshort".to_vec()));
    }
    // One bigger than a batch: a batch of its own.
    let big = "Big/1/1.eml".to_string();
    kept.push((big.clone(), base64(&noise(3 << 20, 99))));
    b64.push(big);
    for i in 0..8u32 {
        kept.push((format!("search/a1/seg{i}"), base64(&noise(40_000, u64::from(i) + 500))));
    }
    (kept, b64, pictures)
}

/// A note for a file in pieces, with none of them there.
fn damaged(s: &Store, key: &str) {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    let tx = db.begin_write().unwrap();
    {
        let mut note = b"\0MailCraft:pieces\0".to_vec();
        note.extend_from_slice(&(200_000u64).to_le_bytes());
        tx.open_table(super::super::FILES).unwrap().insert(key, note.as_slice()).unwrap();
    }
    tx.commit().unwrap();
}

/// A data file from before compressed again: the base64 is kept compressed now, in about three
/// quarters of the room; the pictures, the index's pages and what was compressed already are left
/// as they are; a damaged file doesn't stop it; every file reads back the same. Batches hold at
/// most a background commit (but for the file bigger than that, alone), and its progress goes up
/// to the whole. Then the file says it's all kept as this build keeps it, and a second run has
/// nothing to do. One run at a time: another asking meanwhile is refused.
#[test]
fn an_old_data_file_is_compressed_again_and_reads_back_the_same() {
    let t = temp("recompress");
    let (kept, b64, pictures) = an_old_file(240);
    let s = old_store(&t, &kept);
    damaged(&s, "Damaged/1/1.eml");
    let before = read_back(&s);
    assert_eq!(before.get("Damaged/1/1.eml"), Some(&None));
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(!health.recompressed && health.raw_fraction > 0.5, "{health:?}");
    let (mut heard, mut refused) = (Vec::<Work>::new(), None);
    let stop = AtomicBool::new(false);
    let r = s
        .recompress_data_file("a1", &stop, &mut |w| {
            if refused.is_none() {
                refused = Some(s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).is_err());
            }
            heard.push(w);
        })
        .unwrap()
        .unwrap();
    eprintln!("{r:?}");
    assert_eq!(refused, Some(true), "a second run at once");
    assert!(r.finished);
    assert_eq!((r.files, r.stayed_raw, r.changed), (b64.len() as u64, pictures.len() as u64, 0));
    let b64_bytes: u64 = kept.iter().filter(|(k, _)| b64.contains(k)).map(|(_, v)| v.len() as u64).sum();
    assert_eq!(r.before, b64_bytes);
    assert!(r.after * 100 <= r.before * 80, "kept in {} of {}", r.after, r.before);
    assert_eq!(r.saved_in_all, r.before - r.after);
    assert_eq!(read_back(&s), before, "everything reads back the same");
    assert!(b64.iter().all(|k| kept_compressed(&s, k)));
    assert!(pictures.iter().chain(&["search/a1/seg0".to_string()]).all(|k| !kept_compressed(&s, k)), "left as they were");

    // Progress: up to the whole, never back.
    let last = *heard.last().unwrap();
    assert!(heard.windows(2).all(|w| w[0].done <= w[1].done && w[0].bytes_done <= w[1].bytes_done));
    assert_eq!((last.done, last.bytes_done), (last.total, last.bytes_total));
    assert_eq!(last.total, before.keys().filter(|k| !k.starts_with("search/")).count() as u64);
    assert!(heard.len() > 5, "{} batches", heard.len() - 1);
    // Each batch's write holds a background commit at most, but the big file's alone.
    let biggest = packs(&s).seals.undurable("a1").2;
    let big = super::super::pack(&kept.iter().find(|(k, _)| k == "Big/1/1.eml").unwrap().1).len();
    assert!(biggest <= crate::BACKGROUND_COMMIT.max(big), "a batch held {biggest}");

    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompressed && !health.recompress_stopped, "{health:?}");
    let again = s.recompress_data_file("a1", &stop, &mut |_| {}).unwrap().unwrap();
    assert!(again.finished && again.files == 0 && again.saved_in_all == r.saved_in_all, "{again:?}");
    drop(s);

    // A data file this build made has nothing to do.
    let t2 = temp("recompress-new");
    let s = Store::open_packed(t2.join("app"), t2.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&super::commits::inbox(1), &base64(&noise(50_000, 1))).unwrap();
    let r = s.recompress_data_file("a1", &stop, &mut |_| {}).unwrap().unwrap();
    assert!(r.finished && r.files == 0);
    assert_eq!(s.recompress_data_file("nobody", &stop, &mut |_| {}).unwrap(), None);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
    let _ = std::fs::remove_dir_all(&t2);
}

/// A run whose first batch has files written again (whole and in pieces), and one removed,
/// between reading them and writing them back compressed: those are left as written (none
/// brought back), and the others in the batch are compressed all the same.
struct Meddling<'a> {
    inner: Account<'a>,
    done: AtomicBool,
}

impl Target for Meddling<'_> {
    fn db(&self) -> std::io::Result<std::sync::Arc<redb::Database>> {
        self.inner.db()
    }

    fn commit(&self, step: Step, f: &mut dyn FnMut(&mut Tables<'_>, &redb::WriteTransaction) -> std::io::Result<()>) -> std::io::Result<()> {
        if matches!(step, Step::Batch(_)) && !self.done.swap(true, Ordering::Relaxed) {
            self.inner.packs.change(self.inner.account, |t| {
                t.put("A/1.eml", b"Subject: written again\r\n\r\nnew")?;
                t.put_kept("A/2.eml", &base64(&noise(40_000, 77)))?;
                t.take_kept("A/3.eml").map(drop)
            })?;
        }
        self.inner.commit(step, f)
    }
}

#[test]
fn a_file_written_while_it_is_compressed_is_left_as_written() {
    let t = temp("recompress-race");
    let mut kept: Kept = vec![
        ("A/1.eml".into(), base64(&noise(2_000, 1))),
        ("A/2.eml".into(), base64(&noise(30_000, 2))),
        ("A/3.eml".into(), base64(&noise(9_000, 3))),
        ("A/4.eml".into(), base64(&noise(20_000, 4))),
    ];
    kept.extend((0..40u32).map(|i| (format!("B/1/{i}.eml"), base64(&noise(60_000, u64::from(i) + 10)))));
    let s = old_store(&t, &kept);
    let mut before = read_back(&s);
    let p = packs(&s);
    let target = Meddling { inner: Account { packs: p, account: "a1", reopened: p.reopened("a1") }, done: AtomicBool::new(false) };
    let r = super::super::compact::recompress(&target, &AtomicBool::new(false), &mut |_| {}).unwrap();
    eprintln!("{r:?}");
    assert!(r.finished);
    assert_eq!((r.changed, r.files), (3, kept.len() as u64 - 3));
    before.insert("A/1.eml".into(), Some(b"Subject: written again\r\n\r\nnew".to_vec()));
    before.insert("A/2.eml".into(), Some(base64(&noise(40_000, 77))));
    before.remove("A/3.eml");
    assert_eq!(read_back(&s), before);
    assert!(!kept_compressed(&s, "A/2.eml"), "the file as written meanwhile");
    assert!(kept_compressed(&s, "A/4.eml") && kept_compressed(&s, "B/1/39.eml"));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A run stopped after its first batch says so (the file's health shows it, and offers to go on),
/// with what it did on the disk; the next run goes on from where it stopped (its progress starts
/// there), and between them every file was compressed once.
#[test]
fn a_stopped_recompress_goes_on_from_where_it_stopped() {
    let t = temp("recompress-stop");
    let (kept, b64, _) = an_old_file(160);
    let s = old_store(&t, &kept);
    let before = read_back(&s);
    let stop = AtomicBool::new(false);
    let mut heard = 0;
    let first = s
        .recompress_data_file("a1", &stop, &mut |_| {
            heard += 1;
            stop.store(heard >= 2, Ordering::Relaxed);
        })
        .unwrap()
        .unwrap();
    eprintln!("stopped: {first:?}");
    assert!(!first.finished && first.files > 0 && first.files < b64.len() as u64);
    assert!(!packs(&s).seal_due("a1"), "what it did is on the disk, sealed");
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompress_stopped && !health.recompressed && health.can_compact(), "{health:?}");
    assert_eq!(read_back(&s), before);
    drop(s);

    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let mut starts = None;
    let rest = s
        .recompress_data_file("a1", &AtomicBool::new(false), &mut |w| {
            starts.get_or_insert(w);
        })
        .unwrap()
        .unwrap();
    eprintln!("went on: {rest:?} from {starts:?}");
    let starts = starts.unwrap();
    assert!(starts.done > 0 && starts.bytes_done > 0 && starts.done < starts.total);
    assert!(rest.finished);
    assert_eq!(first.files + rest.files, b64.len() as u64, "each compressed once");
    assert_eq!(rest.saved_in_all, first.before - first.after + rest.before - rest.after);
    assert_eq!(read_back(&s), before);
    assert!(b64.iter().all(|k| kept_compressed(&s, k)));
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompressed && !health.recompress_stopped);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// Killed part way through (after a different batch each time, its write not yet on the disk):
/// every file reads back the same when the store opens again, and a run then finishes the job.
#[test]
fn a_recompress_killed_part_way_loses_nothing() {
    for batches in [1u32, 3, 6] {
        let t = temp(&format!("recompress-kill-{batches}"));
        let (kept, b64, _) = an_old_file(200);
        let before = {
            let s = old_store(&t, &kept);
            read_back(&s)
        };
        let env = [("MC_CHILD_DIR", t.display().to_string()), ("MC_CHILD_BATCHES", batches.to_string())];
        let out = child("packed::tests::compact::killed_during_recompress", &env).wait_with_output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            !out.status.success() && said.contains("killed"),
            "{batches}: the child was to be killed: {said}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        assert_eq!(read_back(&s), before, "{batches}: after the kill");
        let r = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
        assert!(r.finished, "{batches}: {r:?}");
        assert_eq!(read_back(&s), before, "{batches}: once finished");
        assert!(b64.iter().all(|k| kept_compressed(&s, k)), "{batches}");
        drop(s);
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// The child of [`a_recompress_killed_part_way_loses_nothing`].
#[test]
#[ignore = "run by a_recompress_killed_part_way_loses_nothing"]
fn killed_during_recompress() {
    let Some(dir) = std::env::var_os("MC_CHILD_DIR").map(std::path::PathBuf::from) else { return };
    let batches: u32 = std::env::var("MC_CHILD_BATCHES").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let s = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    let heard = AtomicU32::new(0);
    let _ = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {
        // Heard as it starts, then after each batch.
        if heard.fetch_add(1, Ordering::Relaxed) == batches {
            println!("killed");
            std::process::abort();
        }
    });
}

/// A write from the window (a message marked read) waits little for a recompress running beside
/// it: each batch holds the file only to write what it compressed away from it. A measurement, as
/// the one for a background writer: alone, p99 4 to 6 ms; with every test of the workspace running
/// beside it, 80 ms.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn a_write_from_the_window_waits_little_for_a_recompress() {
    let t = temp("recompress-latency");
    let kept: Kept = (0..600u32).map(|i| (format!("INBOX/1/{i}.eml"), base64(&noise(90_000, u64::from(i) + 1)))).collect();
    let s = old_store(&t, &kept);
    s.write_body(&super::commits::inbox(1), b"Subject: x\r\n\r\nx").unwrap();
    let running = {
        let s = s.clone();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let r = s.recompress_data_file("a1", &AtomicBool::new(false), &mut |_| {}).unwrap().unwrap();
            (r, t0.elapsed())
        })
    };
    let p = packs(&s);
    p.seals.waits.lock().unwrap().clear();
    let mut writes = 0u32;
    while !running.is_finished() || writes < 10 {
        s.write_body(&super::commits::inbox(1), format!("Subject: x\r\n\r\nread {writes}").as_bytes()).unwrap();
        writes += 1;
        std::thread::sleep(Duration::from_millis(3));
    }
    let (r, took) = running.join().unwrap();
    let mut waits: Vec<f64> = p.seals.waits.lock().unwrap().drain(..).map(|d| d.as_secs_f64() * 1000.0).collect();
    waits.sort_by(f64::total_cmp);
    let p99 = waits.get(((waits.len() as f64 - 1.0) * 0.99).round() as usize).copied().unwrap_or(0.0);
    eprintln!("recompressed {} files in {took:?}; {writes} writes from the window waited at most {p99:.1} ms (p99)", r.files);
    assert!(r.finished && r.files == 600);
    assert!(p99 <= 60.0, "a write from the window waited {p99:.1} ms for the recompress (p99)");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A fuzz input of records (`fuzz_db`): a value in `FILES`, `PIECES` or `META`.
fn record(out: &mut Vec<u8>, kind: u8, key: &str, piece: Option<u8>, value: &[u8], times: u8) {
    out.push(kind);
    out.push(key.len() as u8);
    out.extend_from_slice(key.as_bytes());
    if let Some(n) = piece {
        out.push(n);
    }
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.push(times);
    out.extend_from_slice(value);
}

/// The fuzz target's input that every kind of file is in (its seed, `fuzz/seeds/data_file`).
fn fuzz_seed() -> Vec<u8> {
    let mut i = Vec::new();
    let line = b"QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0\r\n";
    record(&mut i, b'F', "INBOX/1/1.eml", None, line, 40);
    let mut note = b"\0MailCraft:pieces\0".to_vec();
    note.extend_from_slice(&(line.len() as u64 * 100).to_le_bytes());
    record(&mut i, b'F', "INBOX/1/2.eml", None, &note, 1);
    record(&mut i, b'P', "INBOX/1/2.eml", Some(0), line, 50);
    record(&mut i, b'P', "INBOX/1/2.eml", Some(1), line, 50);
    record(&mut i, b'F', "INBOX/1/3.eml", None, b"\0MailCraft:pieces\0\xff\xff\xff\xff\0\0\0\0", 1);
    record(&mut i, b'F', "INBOX/1/4.eml", None, b"\0MailCraft:deflate\0\x03\0", 1);
    record(&mut i, b'F', "search/a1/seg1", None, line, 30);
    record(&mut i, b'F', "Sent/1/1.eml", None, &noise(3000, 5), 1);
    record(&mut i, b'M', "recompress.cursor", None, b"INBOX/1/1.eml", 1);
    record(&mut i, b'M', "deflate", None, &1u64.to_le_bytes(), 1);
    i
}

/// The fuzz target's hook, on its seed (every file reads back the same once compressed again), and
/// on that seed damaged every way a few hundred times (never a panic).
#[test]
fn the_fuzz_targets_data_files_read_back_the_same() {
    let seed = fuzz_seed();
    assert!(crate::fuzz_data_file(&seed));
    let written = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/data_file/kinds");
    assert_eq!(std::fs::read(&written).ok().as_deref(), Some(seed.as_slice()), "fuzz/seeds/data_file/kinds is the seed made here");
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for round in 0..300 {
        let mut input = seed.clone();
        for _ in 0..1 + round % 8 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let at = (x >> 8) as usize % input.len();
            input[at] = (x >> 40) as u8;
        }
        assert!(crate::fuzz_data_file(&input), "round {round}");
    }
}
