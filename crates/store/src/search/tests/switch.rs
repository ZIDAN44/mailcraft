//! Every way a writer makes a generation switches to it with its meta, written last: a crash
//! before it (a write of one of its files, or of its meta, failing) leaves the generation before,
//! read as before, and what it wrote is removed when the writer next opens.

use mailcraft_model::search::TOKENIZER_VERSION;

use super::*;

/// A way a writer makes a generation: what it needs first, the step that makes it (on the
/// writer given, or one opened), and how its meta differs from the one before.
struct Way {
    name: &'static str,
    setup: fn(&mut Writer, &Store),
    step: fn(Option<Writer>, &Store) -> Result<Writer>,
    made: fn(&Meta, &Meta) -> bool,
    /// Whether it writes files before its meta (one that writes only a meta has none to leave).
    writes_files: bool,
}

fn docs(w: &mut Writer, s: &Store, from: u32, n: u32) {
    for uid in from..from + n {
        w.add(&mk("INBOX", uid), u64::from(uid), uid, &text(&format!("common u{uid}"), &[&format!("f{uid}.pdf")], 1)).unwrap();
    }
    w.flush(s).unwrap();
}

fn on(w: Option<Writer>, s: &Store) -> Writer {
    w.unwrap_or_else(|| Writer::open(s, "a1").unwrap())
}

const WAYS: [Way; 5] = [
    Way {
        name: "flush",
        setup: |w, s| docs(w, s, 1, 10),
        step: |w, s| {
            let mut w = on(w, s);
            w.remove(0);
            w.add(&mk("INBOX", 50), 50, 50, &text("common later", &[], 0))?;
            w.flush(s).map(|_| w)
        },
        made: |b, a| a.next_doc == b.next_doc + 1 && a.segments.len() == 2 && a.live == b.live,
        writes_files: true,
    },
    Way {
        name: "since",
        setup: |w, s| {
            docs(w, s, 1, 10);
            w.remove(3);
            w.flush(s).unwrap();
        },
        step: |w, s| {
            let mut w = on(w, s);
            w.merge_step(s, 0, 1000).map(|_| w)
        },
        made: |b, a| (b.dead_since, a.dead_since) == (0, 1000) && a.segments == b.segments,
        writes_files: false,
    },
    Way {
        name: "merge",
        setup: |w, s| {
            for i in 0..8 {
                docs(w, s, 1 + i * 5, 5);
            }
            // Its words left out by the merge: its doc page written again.
            w.remove(7);
            w.flush(s).unwrap();
        },
        step: |w, s| {
            let mut w = on(w, s);
            w.merge_step(s, 0, 1000).map(|_| w)
        },
        made: |b, a| (b.segments.len(), a.segments.len()) == (8, 1),
        writes_files: true,
    },
    Way {
        name: "renumber",
        setup: |w, s| {
            docs(w, s, 1, 20);
            for doc in 0..16 {
                w.remove(doc);
            }
            w.flush(s).unwrap();
        },
        step: |w, s| {
            let mut w = on(w, s);
            w.renumber_slack = 4;
            w.merge_step(s, 0, 1000).map(|_| w)
        },
        made: |b, a| a.epoch != b.epoch && (b.next_doc, a.next_doc, a.live) == (20, 4, 4),
        writes_files: true,
    },
    Way {
        name: "rebuild",
        setup: |w, s| {
            docs(w, s, 1, 10);
            // As an older MailCraft left it: its words made again as the writer opens.
            let path = meta_path(s, "a1");
            let mut meta = decode_meta(&s.files.read(&path).unwrap()).unwrap();
            meta.tokenizer = 0;
            s.files.write(&path, &encode_meta(&meta)).unwrap();
        },
        step: |w, s| {
            drop(w);
            Writer::open(s, "a1")
        },
        made: |b, a| (b.tokenizer, a.tokenizer) == (0, TOKENIZER_VERSION) && a.segments.iter().all(|n| b.segments.iter().all(|o| n.id > o.id)),
        writes_files: true,
    },
];

/// Where a step is stopped (the endings of the files whose writes fail): at its meta (every file
/// of it written), at its first file, at the new generation's first doc page or first text page
/// (each a crash between its files and its meta, wherever they are written).
fn faults(next: u64) -> [String; 4] {
    let any = "0.bin|1.bin|2.bin|3.bin|4.bin|5.bin|6.bin|7.bin|8.bin|9.bin|h.bin";
    ["meta.bin".into(), any.into(), format!("docs/{next}-0.bin"), format!("t/{next}-0.bin")]
}

/// Whether a fault's endings name a file.
fn named(fault: &str, path: &std::path::Path) -> bool {
    let path = path.display().to_string().replace('\\', "/");
    fault.split('|').any(|end| path.ends_with(end))
}

/// What a reader finds: the generation and every doc's key with `common`; `None`: no index.
fn read(s: &Store) -> Option<(u64, Vec<MessageKey>)> {
    let mut r = Reader::open(s, "a1").unwrap()?;
    let keys = found(&mut r, s, Field::Body, "common").into_iter().filter_map(|d| r.key(d)).collect();
    Some((r.generation(), keys))
}

#[test]
fn every_writer_path_switches_generation_with_its_meta() {
    for (p, f) in WAYS.iter().flat_map(|p| (0..4).map(move |f| (p, f))) {
        let s = Store::failing();
        let mut w = Writer::open(&s, "a1").unwrap();
        (p.setup)(&mut w, &s);
        let before = load_meta(&s, "a1").unwrap().unwrap();
        let reads = read(&s);
        let at = &faults(before.generation + 1)[f];
        let name = format!("{} stopped at {at}", p.name);
        assert_eq!(files(&s).1, BTreeSet::new(), "{name}: nothing left over before");

        s.fail(None, Some(at));
        let stopped = (p.step)(Some(w), &s);
        s.fail(None, None);
        if let Ok(w) = stopped {
            // Nothing it writes fails so: the generation it made has no such file new.
            let after = load_meta(&s, "a1").unwrap().unwrap();
            assert!(f > 0 && after.generation == before.generation + 1, "{name}");
            let new = after.uses().difference(&before.uses()).map(|n| n.path(&dir(&s, "a1"))).collect::<Vec<_>>();
            assert!(new.iter().all(|p| !named(at, p)), "{name}: {new:?}");
            drop(w);
            continue;
        }
        assert_eq!(load_meta(&s, "a1").unwrap().unwrap(), before, "{name}: the generation before");
        assert_eq!(read(&s), reads, "{name}: read as before");
        if f == 0 {
            assert_eq!(!files(&s).1.is_empty(), p.writes_files, "{name}: what it wrote is there");
        }

        let w = (p.step)(None, &s).unwrap();
        assert_eq!(files(&s).1, BTreeSet::new(), "{name}: what it wrote before is gone");
        let after = load_meta(&s, "a1").unwrap().unwrap();
        assert_eq!(after.generation, before.generation + 1, "{name}: done again, one generation on");
        assert!((p.made)(&before, &after), "{name}: {before:?} then {after:?}");
        assert!(read(&s).is_some_and(|(g, keys)| g == after.generation && !keys.is_empty()), "{name}: and read");
        drop(w);
    }
}
