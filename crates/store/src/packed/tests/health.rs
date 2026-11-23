//! A data file's health: the walk of what it keeps against the database's own count, `META` read
//! value by value, files from before it, and what a compaction leaves.

use redb::ReadableDatabase;

use super::*;
use crate::tests::random::Rng;

/// What the database itself has in use: its allocated pages (walking every one).
fn allocated(s: &Store) -> u64 {
    let db = packs(s).db("a1", false).unwrap().unwrap();
    let tx = db.begin_write().unwrap();
    let stats = tx.stats().unwrap();
    stats.allocated_pages() * stats.page_size() as u64
}

/// What compacting the data file gives back, as Compact now does it for a file this size (a copy
/// into a new file): the file's size before, less after.
fn given_back(s: &Store) -> u64 {
    let r = s.copy_data_file("a1", &mut |_| true, &mut || true).unwrap().unwrap();
    r.before.saturating_sub(r.after)
}

/// What the card says can be given back is what compacting gives back, to 2% of the file (issue
/// #221: it said what the database had free, which a file this size gets back at its next write).
fn as_promised(s: &Store) {
    let promised = s.data_file_health("a1").unwrap().unwrap();
    let given = given_back(s);
    eprintln!("promised {} of {}; a copy gave back {given}", promised.reclaimable, promised.file_bytes);
    assert!(
        promised.reclaimable.abs_diff(given) as f64 <= 0.02 * promised.file_bytes as f64,
        "{} promised, {given} given back",
        promised.reclaimable
    );
    let after = s.data_file_health("a1").unwrap().unwrap();
    assert!(!after.can_compact(), "nothing more offered: {after:?}");
}

/// What the pieces of a file take is counted as cutting it would cut it, without cutting it.
#[test]
fn what_pieces_take_is_counted_without_cutting() {
    let cut = |len: usize| -> u64 {
        super::super::piece_sizes(len)
            .into_iter()
            .map(|p| ((p + super::super::PAGE_SLACK).div_ceil(super::super::PAGE) * super::super::PAGE) as u64)
            .sum()
    };
    let mut rng = Rng(0x5eed);
    let lens = (0..2000).map(|i| if i < 200 { i * 337 } else { rng.below(5 << 20) as usize });
    for len in lens.chain([4095, 4096, 3584, 3585, 31744, 64512, 65024, 65025, 65536 * 3, 65024 + 64512]) {
        assert_eq!(super::super::pieced_footprint(len as u64), cut(len), "{len}");
    }
    // A crafted note's length is only counted.
    assert!(super::super::pieced_footprint(u64::MAX) > 0);
}

/// On a data file written over many commits, with files of every size written over and removed
/// at random, the walk's count of what is kept comes within 2% of the pages the database has in
/// use (by its own walk of every page), and what it says can be given back is what compacting
/// gives back.
#[test]
fn the_walk_comes_within_two_percent_of_the_database() {
    let t = temp("health-walk");
    let s = packed_store(&t);
    let mut rng = Rng(0x9e37_79b9);
    let source = |rng: &mut Rng, uid: u32| -> Vec<u8> {
        let n = match rng.below(4) {
            0 => 200 + rng.below(3000),
            1 => 4000 + rng.below(60_000),
            _ => 60_000 + rng.below(400_000),
        } as usize;
        if rng.below(3) == 0 { text(n, u64::from(uid)) } else { noise(n, rng.below(u64::MAX)) }
    };
    let mut kept: Vec<u32> = Vec::new();
    for round in 0..40u32 {
        let mut batch = Vec::new();
        for i in 0..20 {
            let uid = round * 20 + i;
            batch.push((inbox(uid), Store::prepare(true, source(&mut rng, uid))));
            kept.push(uid);
        }
        s.write_prepared(batch, rng.below(2) == 0).unwrap();
        // Small changes between: a document written over, a message written again or removed.
        for _ in 0..10 {
            match rng.below(3) {
                0 => s.save_account_doc("a1", &format!("doc{}.json", rng.below(30)), &vec![rng.below(1000); rng.below(200) as usize]).unwrap(),
                1 if !kept.is_empty() => {
                    let uid = kept[rng.below(kept.len() as u64) as usize];
                    s.write_body(&inbox(uid), &source(&mut rng, uid)).unwrap();
                }
                _ if !kept.is_empty() => s.remove_body(&inbox(kept.swap_remove(rng.below(kept.len() as u64) as usize))),
                _ => {}
            }
        }
    }
    let health = s.data_file_health("a1").unwrap().unwrap();
    let used = allocated(&s);
    let off = health.live_bytes.abs_diff(used) as f64 / used as f64;
    eprintln!("walk {} bytes, database {used} ({:.2}% off); file {}", health.live_bytes, off * 100.0, health.file_bytes);
    assert!(off <= 0.02, "the walk is {:.2}% off the database's count", off * 100.0);
    assert_eq!(health.files, packs(&s).keys("a1", "").unwrap().len() as u64);
    assert!(health.recompressed, "a file this build made");

    // Half of it removed: what's said to be there to give back is what compacting gives back.
    for uid in kept.iter().step_by(2) {
        s.remove_body(&inbox(*uid));
    }
    let after = s.data_file_health("a1").unwrap().unwrap();
    assert!(after.live_bytes < health.live_bytes);
    as_promised(&s);
    drop(s);
}

/// A data file of small messages only (each kept whole, in the pages of the list of files): a
/// value over half a page has a page to itself, and the pages are as full as they were left. The
/// walk counts those pages as the database does, so it comes within 2% of it there too, and what
/// it says can be given back after messages are removed at random is what compacting gives back.
#[test]
fn the_walk_counts_small_messages_as_the_pages_they_take() {
    let t = temp("health-small");
    let s = packed_store(&t);
    let mut rng = Rng(0x51ed_270b);
    let mut kept: Vec<u32> = Vec::new();
    for round in 0..40u32 {
        let batch = (0..500u32)
            .map(|i| {
                let uid = round * 500 + i;
                kept.push(uid);
                (inbox(uid), Store::prepare(true, noise(1_000 + rng.below(2_500) as usize, u64::from(uid) + 1)))
            })
            .collect();
        s.write_prepared(batch, rng.below(2) == 0).unwrap();
    }
    let health = s.data_file_health("a1").unwrap().unwrap();
    let used = allocated(&s);
    let off = health.live_bytes.abs_diff(used) as f64 / used as f64;
    eprintln!("walk {} bytes, database {used} ({:.2}% off); file {}", health.live_bytes, off * 100.0, health.file_bytes);
    assert!(off <= 0.02, "the walk is {:.2}% off the database's count", off * 100.0);

    for _ in 0..3_000 {
        let i = rng.below(kept.len() as u64) as usize;
        s.remove_body(&inbox(kept.swap_remove(i)));
    }
    let after = s.data_file_health("a1").unwrap().unwrap();
    let used = allocated(&s);
    let off = after.live_bytes.abs_diff(used) as f64 / used as f64;
    eprintln!("walk {} against {used} ({:.2}% off)", after.live_bytes, off * 100.0);
    assert!(off <= 0.02, "the walk is {:.2}% off the database's count", off * 100.0);
    as_promised(&s);
    drop(s);
}

/// A data file made new says as it's made that everything in it is kept as this build keeps
/// it; one from before (no `META`) doesn't, and writing to it doesn't make it say so.
#[test]
fn a_new_data_file_says_how_it_is_kept() {
    let t = temp("health-new");
    let s = packed_store(&t);
    s.write_body(&inbox(1), &text(50_000, 1)).unwrap();
    let meta = {
        let db = packs(&s).db("a1", false).unwrap().unwrap();
        super::super::read_meta(&db.begin_read().unwrap()).unwrap()
    };
    assert_eq!(meta.deflate, super::super::DEFLATE_NOW, "written as it was made");
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompressed && health.recompress_gain == 0 && !health.recompress_stopped);
    assert!(!health.can_compact() && !health.needs_compacting());
    drop(s);
}

/// A data file made new says how it's kept on the disk at once, not in its first write: that write
/// not waited for to the disk (an import's batch), then a write the full drive refuses, which lets
/// the file go back to what was on the disk and opens it again, leaves it saying so all the same
/// (it said nothing, and read as one from before for good).
#[test]
fn a_new_data_file_says_how_it_is_kept_before_anything_reaches_the_disk() {
    use std::sync::atomic::Ordering;
    let t = temp("health-new-undurable");
    let s = packed_store(&t);
    s.write_prepared(vec![(inbox(1), Store::prepare(true, text(5_000, 1)))], false).unwrap();
    let p = packs(&s);
    let reopened = p.reopened("a1");
    p.full.store(true, Ordering::Relaxed);
    s.write_body(&inbox(2), &noise(80 << 20, 2)).unwrap_err();
    p.full.store(false, Ordering::Relaxed);
    s.write_body(&inbox(3), &text(5_000, 3)).unwrap();
    assert!(p.reopened("a1") > reopened, "let go of and opened again");
    let health = s.data_file_health("a1").unwrap().unwrap();
    assert!(health.recompressed, "{health:?}");
    drop(s);
}

/// A data file as a build before this one left it: no `META`, base64 attachments kept raw (that
/// build's fast deflate kept them at 0.995 of their size), pictures raw (they don't compress) and
/// text compressed. Its health says what compressing it again would save, from what was sampled
/// compressed for real: the base64, not the pictures, which a check of the mark alone counted, and
/// in proportion to the files a sample is drawn from: not the search index's (packed already, and
/// never compressed again), however big.
#[test]
fn an_old_data_file_without_meta() {
    let t = temp("health-old");
    let (app, files) = (t.join("app"), t.join("Files"));
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(app.join("datafiles.json"), br#"{"a1": "old.mailcraft"}"#).unwrap();
    let (mut base64_bytes, mut picture_bytes, mut index_bytes) = (0u64, 0u64, 0u64);
    {
        let db = redb::Database::create(files.join("old.mailcraft")).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut tables = super::super::Tables {
                files: tx.open_table(super::super::FILES).unwrap(),
                pieces: tx.open_table(super::super::PIECES).unwrap(),
                touched: None,
            };
            for i in 0..3840u32 {
                let key = format!("INBOX/1/{i}.eml");
                match i % 3 {
                    // Kept raw as that build kept it: put as kept, not compressed now.
                    0 => {
                        let b = base64(&noise(2_000 + (i as usize % 7) * 1_500, u64::from(i)));
                        base64_bytes += b.len() as u64;
                        tables.put_kept(&key, &b).unwrap();
                    }
                    1 => {
                        let b = noise(3_000 + (i as usize % 5) * 2_000, u64::from(i) + 9);
                        picture_bytes += b.len() as u64;
                        tables.put_kept(&key, &b).unwrap();
                    }
                    _ => tables.put(&key, &text(5_000, u64::from(i))).unwrap(),
                }
            }
            // As much again in the search index.
            for i in 0..40u32 {
                let b = noise(512 << 10, u64::from(i) + 77);
                index_bytes += b.len() as u64;
                tables.put_kept(&format!("search/a1/seg{i}"), &b).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let h = s.data_file_health("a1").unwrap().unwrap();
    eprintln!("{h:?}; base64 {base64_bytes}, pictures {picture_bytes}, index {index_bytes}");
    assert!(!h.recompressed && !h.recompress_stopped && h.files == 3880);
    assert!(index_bytes >= h.kept_bytes / 2, "the index is half of what's kept");
    // Most of what's kept is base64 that would compress, and it'd save about a quarter of it; the
    // pictures, kept raw too, aren't counted.
    let mail = (h.kept_bytes - index_bytes) as f64;
    let (raw_share, unmarked) = (base64_bytes as f64 / mail, (base64_bytes + picture_bytes) as f64 / mail);
    assert!((f64::from(h.raw_fraction) - raw_share).abs() < 0.1, "raw {} against {raw_share}", h.raw_fraction);
    assert!(f64::from(h.raw_fraction) < unmarked - 0.1, "raw {} counts the pictures ({unmarked})", h.raw_fraction);
    let gain = h.recompress_gain as f64 / base64_bytes as f64;
    assert!((0.15..0.35).contains(&gain), "would save {gain} of the base64");
    // Written to by this build, it still has its old files: it says nothing new.
    s.write_body(&inbox(1), &text(40_000, 1)).unwrap();
    assert!(!s.data_file_health("a1").unwrap().unwrap().recompressed);
    drop(s);
}

/// `META` is hostile like the rest of the file: each value that can't be read as what it should
/// be is as if it weren't there, and a `META` that isn't a table of its kind is as if there were
/// none. A cursor that reads says a recompress was stopped part way.
#[test]
fn meta_is_read_value_by_value() {
    use redb::TableDefinition;
    let t = temp("health-meta");
    let read = |f: &dyn Fn(&redb::WriteTransaction)| {
        let path = t.join(format!("m{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = redb::Database::create(&path).unwrap();
        let tx = db.begin_write().unwrap();
        f(&tx);
        tx.commit().unwrap();
        let meta = super::super::read_meta(&db.begin_read().unwrap()).unwrap();
        drop(db);
        let _ = std::fs::remove_file(&path);
        meta
    };
    std::fs::create_dir_all(&t).unwrap();
    let put = |tx: &redb::WriteTransaction, values: &[(&str, &[u8])]| {
        let mut m = tx.open_table(super::super::META).unwrap();
        for (k, v) in values {
            m.insert(*k, *v).unwrap();
        }
    };
    // None at all.
    assert_eq!(read(&|_| {}), super::super::Meta::default());
    // Too short, out of range, not text, too long.
    let long = vec![b'a'; 5000];
    let junk = read(&|tx| {
        put(tx, &[("deflate", &[2, 0, 0]), ("compact.overhead", &u64::MAX.to_le_bytes()), ("recompress.cursor", &[0xff, 0xfe, 0x00])]);
    });
    assert_eq!(junk, super::super::Meta::default());
    assert_eq!(read(&|tx| put(tx, &[("recompress.cursor", &long)])).cursor, None);
    assert_eq!(read(&|tx| put(tx, &[("recompress.cursor", b"")])).cursor, None);
    // Good values, one with bytes after it (a later layout may add some).
    let good = read(&|tx| {
        put(
            tx,
            &[
                ("version", &7u64.to_le_bytes()),
                ("deflate", &[2, 0, 0, 0, 0, 0, 0, 0, 9]),
                ("compact.overhead", &31_000u64.to_le_bytes()),
                ("recompress.cursor", b"INBOX/1/7.eml"),
            ],
        )
    });
    assert_eq!(good, super::super::Meta { deflate: 2, overhead: Some(31_000), cursor: Some("INBOX/1/7.eml".into()), ..Default::default() });
    // A table of another kind under the name.
    let other = read(&|tx| {
        let def: TableDefinition<u64, u64> = TableDefinition::new("meta");
        tx.open_table(def).unwrap().insert(1, 2).unwrap();
    });
    assert_eq!(other, super::super::Meta::default());

    // Through the store: the stopped recompress shows, and is offered.
    let s = packed_store(&t);
    s.write_body(&inbox(1), &text(5000, 1)).unwrap();
    {
        let db = packs(&s).db("a1", false).unwrap().unwrap();
        let tx = db.begin_write().unwrap();
        super::super::write_meta(&tx, &[(super::super::CURSOR, b"INBOX/1/1.eml")]).unwrap();
        tx.commit().unwrap();
    }
    let h = s.data_file_health("a1").unwrap().unwrap();
    assert!(h.recompress_stopped && h.can_compact());
    drop(s);
}

/// What a compacted file has over what it keeps is the least it can have: just after, nothing
/// shows as there to give back, and what's removed after shows as much as compacting gives back.
#[test]
fn a_compaction_sets_the_baseline() {
    let t = temp("health-compacted");
    let s = packed_store(&t);
    for round in 0..10u32 {
        let batch =
            (0..20).map(|i| (inbox(round * 20 + i), Store::prepare(true, noise(30_000 + i as usize * 7_000, u64::from(round * 20 + i))))).collect();
        s.write_prepared(batch, true).unwrap();
    }
    for uid in (0..200).step_by(2) {
        s.remove_body(&inbox(uid));
    }
    drop(s);
    let file = t.join("Files").join("zoe@studio.example.mailcraft");
    {
        let mut db = redb::Database::create(&file).unwrap();
        db.compact().unwrap();
    }
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let before = s.data_file_health("a1").unwrap().unwrap();
    s.data_file_compacted("a1").unwrap();
    let after = s.data_file_health("a1").unwrap().unwrap();
    eprintln!("compacted: {before:?} then {after:?}");
    assert!(after.reclaimable * 100 <= after.file_bytes, "{} to give back just after compacting", after.reclaimable);
    // A quarter of what's left removed: what shows is what compacting gives back.
    for uid in (1..200).step_by(4) {
        s.remove_body(&inbox(uid));
    }
    as_promised(&s);
    // What a file is counted against: copied (under 4 GiB kept), the first size its growth from
    // the pages it's grown to first gives over what it keeps and its tree's 2%, and its header
    // page; compacted in place, what it keeps and what the last compaction measured it had over
    // that (2% until one has), to the end of its region.
    let settled = crate::packed::health::settled;
    let copy = |doublings: u32| super::super::shrink::COPY_GROWN_PAGES * 4096 * (1 << doublings) + 4096;
    assert_eq!(settled(915_000_000, Some(500_000)), copy(10), "a copy: what it had is no matter");
    assert_eq!(settled(1_000_000, None), copy(0), "a small one: as it's grown first");
    assert_eq!(settled(3 << 30, None), copy(12));
    // In place, to the end of the region (4 GiB) it ends in: 6 GiB and 5% in two regions, 7.9
    // GiB and 2% in three.
    let big: u64 = 6 << 30;
    assert_eq!(settled(big, Some(50_000)), 8 << 30);
    assert_eq!(settled(8 << 30, Some(0)), 8 << 30, "a region's end is its own");
    assert_eq!(settled((79 << 30) / 10, None), 12 << 30);
    assert_eq!(settled(u64::MAX, Some(1_000_000)), u64::MAX, "saturated, not wrapped");
    // Thresholds.
    let h = |reclaimable: u64, file: u64| crate::DataFileHealth { reclaimable, file_bytes: file, recompressed: true, ..Default::default() };
    assert!(!h(63 << 20, 1 << 30).can_compact() && h(64 << 20, 1 << 30).can_compact());
    assert!(!h(2 << 30, 20 << 30).needs_compacting() && h(4 << 30, 20 << 30).needs_compacting() && !h(900 << 20, 2 << 30).needs_compacting());
    let raw = crate::DataFileHealth { raw_fraction: 0.3, recompress_gain: 2 << 30, ..Default::default() };
    assert!(raw.needs_compacting() && raw.can_compact());
    assert!(!crate::DataFileHealth { recompressed: true, ..raw }.needs_compacting());
    drop(s);
}

/// A real data file's health, timed (counts and timings told, not held to a mark): `MC_DATA`
/// names a copy of a data folder (opening a data file may write to it, so never the original),
/// its data files in "MailCraft Files". Cold the first time only on a copy not yet read (copied
/// unbuffered, or after a standby purge); the second is warm, and finds the same.
#[test]
#[ignore = "needs a copy of a real data folder in MC_DATA"]
fn a_real_data_files_health_measuring_is_timed() {
    let Some(root) = std::env::var_os("MC_DATA").map(std::path::PathBuf::from) else { return };
    let t = std::time::Instant::now();
    let s = Store::open_packed(&root, root.join("MailCraft Files")).unwrap();
    eprintln!("opened in {:?}", t.elapsed());
    for a in s.load_accounts().unwrap() {
        let mut runs = Vec::new();
        for run in ["cold", "warm"] {
            let t = std::time::Instant::now();
            let h = s.data_file_health(&a.id).unwrap();
            eprintln!("{run}: {:?} {h:?}", t.elapsed());
            runs.push(h);
        }
        assert_eq!(runs[0], runs[1], "{}: the same file, the same health", a.id);
    }
}
