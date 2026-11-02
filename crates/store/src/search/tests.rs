use mailcraft_model::search::IndexDoc;

use super::format::*;
use super::*;

mod maint;
mod memory;

/// The three layouts, each in a folder of its own: memory, the folder layout, data files.
fn layouts(name: &str) -> Vec<(&'static str, PathBuf, Store)> {
    let base = std::env::temp_dir().join(format!("mailcraft-search-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    vec![
        ("memory", base.join("memory"), Store::memory()),
        ("disk", base.join("disk"), Store::open(base.join("disk")).unwrap()),
        ("packed", base.join("packed"), Store::open_packed(base.join("packed").join("app"), base.join("packed").join("Files")).unwrap()),
    ]
}

/// The same store closed and opened again (a memory store: the same one).
fn reopen(kind: &str, dir: &Path, s: Store) -> Store {
    match kind {
        "disk" => Store::open(dir).unwrap(),
        "packed" => {
            drop(s);
            Store::open_packed(dir.join("app"), dir.join("Files")).unwrap()
        }
        _ => s,
    }
}

fn mk(folder: &str, uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: folder.into(), validity: 7, uid }
}

fn text(t: &str, names: &[&str], files: u32) -> IndexDoc {
    IndexDoc {
        text: t.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
        files,
        truncated: false,
        parts_cut: false,
        has_text: !t.is_empty(),
    }
}

fn found(r: &mut Reader, s: &Store, field: Field, prefix: &str) -> Vec<u32> {
    r.word(s, field, prefix).unwrap().ones().collect()
}

/// The index's files the meta names, and those it doesn't.
fn files(s: &Store) -> (BTreeSet<Name>, BTreeSet<Name>) {
    let meta = load_meta(s, "a1").unwrap().unwrap();
    let uses = meta.uses();
    let mut on_disk = BTreeSet::new();
    for sub in SUBDIRS {
        for (n, _) in s.files.list(&dir(s, "a1").join(sub)) {
            on_disk.insert(Name::parse(sub, &n).unwrap());
        }
    }
    let extra = on_disk.difference(&uses).copied().collect();
    (on_disk, extra)
}

/// Written, read back on every layout: words of each field, prefixes, addresses, the files'
/// count, the texts; a doc removed and one moved; a big batch over two doc pages, several
/// lexicon pages and postings pages, scanned a page at a time; and again once the store reopens.
#[test]
fn an_index_reads_back_on_every_layout() {
    for (kind, dir, s) in layouts("round") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let resume = w.add(&mk("INBOX", 1), 11, 900, &text("Your Résumé, from Ana <ana@studio.example>", &["Q3 Plan.pdf"], 1)).unwrap();
        let shoot = w.add(&mk("INBOX", 2), 22, 800, &text("The shoot is on Friday", &[], 0)).unwrap();
        let moved = w.add(&mk("Archive", 3), 33, 700, &text("Strasse names and a plan", &[], 0)).unwrap();
        assert_eq!(w.flush(&s).unwrap(), Some(1), "{kind}");
        assert_eq!(w.flush(&s).unwrap(), None, "{kind}: nothing to write");

        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!((r.generation(), r.live()), (1, 3));
        assert_eq!(found(&mut r, &s, Field::Body, "resume"), [resume], "{kind}: read folded");
        assert_eq!(found(&mut r, &s, Field::Body, "str"), [moved]);
        assert_eq!(found(&mut r, &s, Field::Names, "plan"), [resume], "{kind}: names apart from the text");
        assert_eq!(found(&mut r, &s, Field::Body, "plan"), [moved]);
        assert_eq!(found(&mut r, &s, Field::Address, "ana@studio.example"), [resume]);
        assert_eq!(found(&mut r, &s, Field::Address, "ana@stu"), [resume]);
        assert_eq!(found(&mut r, &s, Field::Address, "studio.example"), [resume]);
        assert!(found(&mut r, &s, Field::Body, "nothing").is_empty());
        assert_eq!(r.doc(&mk("INBOX", 2)), Some(shoot));
        assert_eq!(r.doc(&MessageKey { validity: 8, ..mk("INBOX", 2) }), None, "another validity");
        assert_eq!(r.key(moved), Some(mk("Archive", 3)));
        assert_eq!(
            r.info(resume),
            Some(DocInfo { files: 1, has_text: true, truncated: false, parts_cut: false, no_source: false, msgid_hash: 11, size: 900 })
        );
        let blob = r.text(&s, resume).unwrap().unwrap();
        assert_eq!((blob.text.as_str(), blob.names), ("Your Résumé, from Ana <ana@studio.example>", vec!["Q3 Plan.pdf".to_string()]));

        // A removal and a move: the reader follows.
        w.remove(shoot);
        w.retarget(moved, &mk("INBOX", 4)).unwrap();
        assert_eq!(w.flush(&s).unwrap(), Some(2));
        assert!(r.refresh(&s).unwrap());
        assert!(!r.refresh(&s).unwrap(), "nothing new");
        assert_eq!(r.doc(&mk("INBOX", 2)), None);
        assert!(found(&mut r, &s, Field::Body, "shoot").is_empty(), "{kind}: a removed doc matches nothing");
        assert_eq!(r.text(&s, shoot).unwrap(), None);
        assert_eq!((r.doc(&mk("Archive", 3)), r.doc(&mk("INBOX", 4))), (None, Some(moved)));
        assert_eq!(found(&mut r, &s, Field::Body, "strasse"), [moved], "its words stay");

        // Many: two doc pages, lexicon and postings pages, and texts over several pages.
        let n = DOCS_PER_PAGE + 300;
        let mut first = None;
        for i in 0..n {
            let body = format!("common uniq{i:05}x{} {}", "y".repeat(30), "filler words ".repeat(1 + (i % 7) as usize * 20));
            let d = w.add(&mk("Big", i + 1), u64::from(i), i, &text(&body, &[], 0)).unwrap();
            first.get_or_insert(d);
        }
        assert!(!w.should_flush(), "far under the budget");
        assert_eq!(w.flush(&s).unwrap(), Some(3));
        assert!(r.refresh(&s).unwrap());
        let first = first.unwrap();
        let all: Vec<u32> = (first..first + n).collect();
        assert_eq!(found(&mut r, &s, Field::Body, "common"), all, "{kind}");
        let meta = load_meta(&s, "a1").unwrap().unwrap();
        let seg = meta.segments.last().unwrap();
        assert!(seg.lex_pages > 1 && seg.post_pages >= 1 && meta.doc_pages.len() == 2 && meta.text_pages.len() > 2, "{kind}: {seg:?}");
        // A page at a time: as many steps as it takes, the same docs.
        let (mut cur, mut out, mut steps) = (ScanCursor::default(), Bits::new(r.len()), 0);
        while !r.scan_prefix(&s, Field::Body, "uniq", &mut cur, &mut out, 1).unwrap() {
            steps += 1;
        }
        assert!(steps >= 1, "{kind}: {steps} steps");
        assert_eq!(out.ones().collect::<Vec<_>>(), all);
        assert_eq!(r.text(&s, first + n - 1).unwrap().unwrap().text.split(' ').nth(1), Some(format!("uniq{:05}x{}", n - 1, "y".repeat(30)).as_str()));
        drop((w, r));

        let s = reopen(kind, &dir, s);
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!((r.generation(), r.live()), (3, n + 2), "{kind}");
        assert_eq!(found(&mut r, &s, Field::Names, "q3"), [resume]);
        assert_eq!(found(&mut r, &s, Field::Body, "uniq04395"), [first + 4395]);
        assert_eq!(files(&s).1, BTreeSet::new(), "{kind}: nothing left over");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A crash after a commit's files and before its meta, on every layout: the index is the
/// generation before, and what the commit wrote is removed when the writer next opens.
#[test]
fn a_crash_before_the_meta_leaves_the_generation_before() {
    for (kind, dir, s) in layouts("crash") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let kept = w.add(&mk("INBOX", 1), 1, 1, &text("kept words", &["kept.pdf"], 1)).unwrap();
        let goes = w.add(&mk("INBOX", 2), 2, 2, &text("doomed words", &[], 0)).unwrap();
        w.flush(&s).unwrap();
        w.remove(goes);
        w.add(&mk("INBOX", 3), 3, 3, &text("never committed", &[], 0)).unwrap();
        w.stop_before_meta = true;
        assert!(w.flush(&s).is_err());
        let (_, extra) = files(&s);
        assert!(
            extra.iter().any(|n| matches!(n, Name::SegHead(_))) && extra.iter().any(|n| matches!(n, Name::Docs { generation: 2, .. })),
            "{kind}: {extra:?}"
        );
        drop(w);

        let s = reopen(kind, &dir, s);
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!((r.generation(), r.live()), (1, 2), "{kind}: the generation before");
        assert_eq!(found(&mut r, &s, Field::Body, "doomed"), [goes]);
        assert!(found(&mut r, &s, Field::Body, "never").is_empty());
        let mut w = Writer::open(&s, "a1").unwrap();
        assert_eq!(files(&s).1, BTreeSet::new(), "{kind}: what it wrote is gone");
        w.add(&mk("INBOX", 3), 3, 3, &text("committed now", &[], 0)).unwrap();
        assert_eq!(w.flush(&s).unwrap(), Some(2));
        assert!(r.refresh(&s).unwrap());
        assert_eq!(found(&mut r, &s, Field::Body, "committed").len(), 1);
        assert_eq!(found(&mut r, &s, Field::Names, "kept"), [kept]);
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A batch that fails to write leaves the generation before, and the same changes commit once
/// writing works again (the files of the failed one removed first).
#[test]
fn a_failed_batch_leaves_the_generation_before() {
    let s = Store::failing();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.add(&mk("INBOX", 1), 1, 1, &text("first words", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    w.add(&mk("INBOX", 2), 2, 2, &text("second words", &[], 0)).unwrap();
    s.fail(None, Some("-l0.bin"));
    assert!(w.flush(&s).is_err());
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!((r.generation(), r.live()), (1, 1));
    s.fail(None, None);
    // A flush prepared before a change is refused.
    let stale = w.prepare(&s).unwrap();
    w.add(&mk("INBOX", 3), 3, 3, &text("third words", &[], 0)).unwrap();
    assert!(w.commit(&s, stale).is_err());
    assert_eq!(w.flush(&s).unwrap(), Some(2));
    assert!(r.refresh(&s).unwrap());
    assert_eq!(found(&mut r, &s, Field::Body, "words").len(), 3);
    assert_eq!(files(&s).1, BTreeSet::new());
}

/// Words read with another tokenizer are made again from the texts kept: no message is read,
/// and `attachment:` (the names) still finds what it found.
#[test]
fn a_new_tokenizer_reads_the_words_again_from_the_texts() {
    for (kind, dir, s) in layouts("tokenizer") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let plan = w.add(&mk("INBOX", 1), 1, 1, &text("Budget for the shoot", &["Q3 Plan.pdf"], 1)).unwrap();
        let gone = w.add(&mk("INBOX", 2), 2, 2, &text("Budget gone", &[], 0)).unwrap();
        w.flush(&s).unwrap();
        w.remove(gone);
        w.flush(&s).unwrap();
        drop(w);
        // As an older MailCraft left it.
        let path = meta_path(&s, "a1");
        let mut meta = decode_meta(&s.files.read(&path).unwrap()).unwrap();
        let old_segments = meta.segments.clone();
        meta.tokenizer = 0;
        s.files.write(&path, &encode_meta(&meta)).unwrap();
        assert!(Reader::open(&s, "a1").unwrap().is_none(), "{kind}: not used as it is");

        let reads = s.reads();
        let w = Writer::open(&s, "a1").unwrap();
        assert_eq!(s.reads(), reads, "{kind}: no message read");
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!(r.generation(), 3);
        assert_eq!(found(&mut r, &s, Field::Names, "plan"), [plan], "{kind}: attachment: still matches");
        assert_eq!(found(&mut r, &s, Field::Body, "budget"), [plan], "{kind}: the removed doc's words aren't made again");
        let (on_disk, extra) = files(&s);
        assert_eq!(extra, BTreeSet::new());
        assert!(old_segments.iter().flat_map(SegInfo::names).all(|n| !on_disk.contains(&n)), "{kind}: the old words are gone");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A message removed takes its text out of its text page in the same commit: none of the
/// index's files has it after (its words alone stay until their segment is merged).
#[test]
fn a_removed_message_takes_its_text_with_it() {
    for (kind, dir, s) in layouts("removed") {
        let mut w = Writer::open(&s, "a1").unwrap();
        let secret = w.add(&mk("INBOX", 1), 1, 1, &text("the zebracorn password", &["zebracorn.txt"], 1)).unwrap();
        w.add(&mk("INBOX", 2), 2, 2, &text("ordinary words", &[], 0)).unwrap();
        w.flush(&s).unwrap();
        let has = |s: &Store| {
            let base = super::dir(s, "a1");
            let all = SUBDIRS.iter().flat_map(|sub| s.files.list(&base.join(sub))).map(|(_, p)| p).chain([base.join("meta.bin")]);
            all.map(|p| s.files.read(&p).unwrap()).any(|b| b.windows(22).any(|w| w == b"the zebracorn password"))
        };
        assert!(has(&s), "{kind}");
        w.remove(secret);
        w.flush(&s).unwrap();
        assert!(!has(&s), "{kind}: gone with the commit that removed it");
        let mut r = Reader::open(&s, "a1").unwrap().unwrap();
        assert_eq!(found(&mut r, &s, Field::Body, "ordinary").len(), 1, "{kind}: the page's other text stays");
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A data file from before the index opens as it was, has no index, and takes one.
#[test]
fn a_data_file_from_before_opens() {
    let base = std::env::temp_dir().join(format!("mailcraft-search-old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let open = || Store::open_packed(base.join("app"), base.join("Files")).unwrap();
    let s = open();
    let st = FolderState {
        validity: 7,
        uid_next: 2,
        messages: vec![Summary { key: mk("INBOX", 1), subject: "Hello".into(), ..Default::default() }],
        ..Default::default()
    };
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.write_body(&mk("INBOX", 1), b"Subject: Hello\r\n\r\nhi").unwrap();
    drop(s);

    let s = open();
    assert!(Reader::open(&s, "a1").unwrap().is_none(), "no index yet");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    let mut w = Writer::open(&s, "a1").unwrap();
    w.add(&mk("INBOX", 1), 1, 20, &text("hi", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    drop((w, s));

    let s = open();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    assert_eq!(s.read_body(&mk("INBOX", 1)).unwrap(), b"Subject: Hello\r\n\r\nhi");
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(found(&mut r, &s, Field::Body, "hi"), [0]);
    let _ = std::fs::remove_dir_all(&base);
}

/// An index that doesn't read back is set aside and made again, from past its generation and
/// segment numbers, under a new epoch; nothing of the old one is left.
#[test]
fn a_damaged_index_is_set_aside_and_made_again() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    for g in 0..3 {
        w.add(&mk("INBOX", g + 1), 1, 1, &text("words", &[], 0)).unwrap();
        w.flush(&s).unwrap();
    }
    drop(w);
    let old = load_meta(&s, "a1").unwrap().unwrap();
    let page = Name::Docs { generation: old.doc_pages[0], page: 0 }.path(&dir(&s, "a1"));
    s.files.write(&page, b"MCSD\x01junk").unwrap();
    assert!(Reader::open(&s, "a1").unwrap().is_none(), "set aside");
    assert!(s.files.is_file(&meta_path(&s, "a1").with_extension("bin.bad")));

    let mut w = Writer::open(&s, "a1").unwrap();
    assert_eq!(w.live(), 0, "made again");
    assert_eq!(s.files.list(&dir(&s, "a1").join("s")), Vec::new(), "the old files are gone");
    w.add(&mk("INBOX", 9), 1, 1, &text("fresh", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let new = load_meta(&s, "a1").unwrap().unwrap();
    assert!(new.generation > old.generation && new.next_seg > old.next_seg && new.epoch != old.epoch, "{new:?}");
    assert!(new.segments.iter().all(|seg| seg.id >= old.next_seg));
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(found(&mut r, &s, Field::Body, "fresh"), [0]);
    assert!(found(&mut r, &s, Field::Body, "words").is_empty());

    // A meta that doesn't read: the same, the new one still past the old one's files.
    w.add(&mk("INBOX", 10), 1, 1, &text("again", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    drop(w);
    let old = load_meta(&s, "a1").unwrap().unwrap();
    s.files.write(&meta_path(&s, "a1"), b"MCSM\x09").unwrap();
    assert!(Reader::open(&s, "a1").unwrap().is_none());
    let mut w = Writer::open(&s, "a1").unwrap();
    assert_eq!(w.live(), 0);
    w.add(&mk("INBOX", 11), 1, 1, &text("third", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let new = load_meta(&s, "a1").unwrap().unwrap();
    assert!(new.generation > old.generation && new.next_seg > old.next_seg && new.epoch != old.epoch, "{new:?} after {old:?}");
    assert!(new.segments.iter().all(|seg| seg.id >= old.next_seg));
}

/// A meta claiming billions of docs whose pages aren't there: set aside, nothing allocated for
/// what it claims.
#[test]
fn a_meta_claiming_more_than_is_there_is_set_aside() {
    let s = Store::memory();
    let next_doc = u32::MAX - 1;
    let tokenizer = mailcraft_model::search::TOKENIZER_VERSION;
    let meta =
        Meta { tokenizer, generation: 1, next_doc, live: next_doc, doc_pages: vec![1; next_doc.div_ceil(DOCS_PER_PAGE) as usize], ..Meta::default() };
    s.files.write(&meta_path(&s, "a1"), &encode_meta(&meta)).unwrap();
    assert!(Reader::open(&s, "a1").unwrap().is_none());
    assert!(s.files.is_file(&meta_path(&s, "a1").with_extension("bin.bad")));
    s.files.write(&meta_path(&s, "a1"), &encode_meta(&meta)).unwrap();
    assert_eq!(Writer::open(&s, "a1").unwrap().live(), 0, "made again");
}

/// One writer an account at a time.
#[test]
fn an_index_has_one_writer() {
    let s = Store::memory();
    let w = Writer::open(&s, "a1").unwrap();
    assert!(matches!(Writer::open(&s.clone(), "a1"), Err(StoreError::Busy { .. })));
    assert!(Writer::open(&s, "a2").is_ok(), "another account's");
    drop(w);
    assert!(Writer::open(&s, "a1").is_ok());
}

/// The doc table names a folder in 16 bits: past what it can name, the index is to be made
/// again (an error that says it's damaged), never a number that wraps.
#[test]
fn folders_past_what_a_doc_can_name_mean_making_it_again() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let doc = IndexDoc::default();
    for f in 0..MAX_FOLDERS {
        w.add(&mk(&format!("f{f}"), 1), 0, 0, &doc).unwrap();
    }
    let e = w.add(&mk("one more", 1), 0, 0, &doc).unwrap_err();
    assert!(is_damaged(&e), "{e}");
    assert_eq!(w.add(&mk("f7", 2), 0, 0, &doc).unwrap(), u32::try_from(MAX_FOLDERS).unwrap(), "a folder it has still takes mail");
}

/// Addresses and their domains are words of their own.
#[test]
fn addresses_are_words_whole() {
    let mut out = BTreeSet::new();
    address_words("Write to Ana <Ana.Lee@Studio.Example> or ops@mail.studio.example. Not @ alone, me@local or a@b", &mut out);
    let want = ["ana.lee@studio.example", "mail.studio.example", "ops@mail.studio.example", "studio.example"];
    assert_eq!(out.iter().map(String::as_str).collect::<Vec<_>>(), want);
}

/// Every file of a real index, cut short at every length and with bytes changed, reads as an
/// error or as something checked, never a panic.
#[test]
fn damaged_files_are_errors() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    for i in 0..40 {
        w.add(&mk("INBOX", i + 1), 1, 1, &text(&format!("common word{i} ana@studio.example"), &["a.pdf"], 1)).unwrap();
    }
    w.flush(&s).unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    let seg = meta.segments[0];
    let check = |n: Name, b: &[u8]| {
        let _ = match n {
            Name::Docs { .. } => decode_docs(b, 40, 1, 1).map(|_| ()),
            Name::Text { .. } => decode_text_page(b, 40).map(|t| t.iter().for_each(|(_, b)| drop(decode_blob(b)))),
            Name::SegHead(_) => decode_head(b, &seg).map(|_| ()),
            Name::SegLex(..) => decode_lex(b, 40, seg.post_pages).map(|_| ()),
            Name::SegPost(..) => decode_post_page(b).map(|p| drop(decode_postings(p, 40, 40))),
        };
    };
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut all: Vec<(Option<Name>, Vec<u8>)> = meta.uses().into_iter().map(|n| (Some(n), s.files.read(&n.path(&dir(&s, "a1"))).unwrap())).collect();
    all.push((None, encode_meta(&meta)));
    for (n, good) in all {
        for cut in 0..good.len() {
            let b = &good[..cut];
            match n {
                Some(n) => check(n, b),
                None => assert!(decode_meta(b).is_err()),
            }
        }
        for _ in 0..2000 {
            let mut b = good.clone();
            for _ in 0..1 + x % 4 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let i = (x % b.len() as u64) as usize;
                b[i] = (x >> 32) as u8;
            }
            match n {
                Some(n) => check(n, &b),
                None => drop(decode_meta(&b)),
            }
        }
    }
}

/// The fuzz target's seed (`fuzz/seeds/search_index/index`) is a whole index as
/// [`hostile_index`] reads one: it opens, finds its words, takes a change. `MAILCRAFT_WRITE_SEED=1`
/// writes it again first.
#[test]
fn the_fuzz_seed_is_an_index() {
    let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/search_index/index");
    if std::env::var_os("MAILCRAFT_WRITE_SEED").is_some() {
        let s = Store::memory();
        let mut w = Writer::open(&s, "a1").unwrap();
        for i in 0..12 {
            w.add(&mk("INBOX", i + 1), u64::from(i), 100, &text(&format!("common word{i} from ana@studio.example"), &["Q3 Plan.pdf"], 1)).unwrap();
        }
        w.remove(3);
        w.flush(&s).unwrap();
        let base = dir(&s, "a1");
        let meta = load_meta(&s, "a1").unwrap().unwrap();
        let mut bytes = s.files.read(&base.join("meta.bin")).unwrap();
        for n in meta.uses() {
            bytes.extend(s.files.read(&n.path(&base)).unwrap());
        }
        std::fs::write(&seed, bytes).unwrap();
    }
    let s = hostile_index(&std::fs::read(&seed).unwrap());
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(found(&mut r, &s, Field::Body, "common").len(), 11);
    assert_eq!(found(&mut r, &s, Field::Address, "ana@").len(), 11);
    exercise_index(&s);
    assert!(r.refresh(&s).unwrap());
    assert_eq!(found(&mut r, &s, Field::Body, "common").len(), 11, "one in, one out");
}

/// A folder name longer than the meta keeps is refused, the message not indexed: the index
/// still reads back and keeps what it had (it was set aside and made again forever).
#[test]
fn a_folder_name_too_long_to_keep_is_refused() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let kept = w.add(&mk("INBOX", 1), 1, 1, &text("kept words", &[], 0)).unwrap();
    let long = "x".repeat(MAX_NAME_BYTES + 1);
    let e = w.add(&mk(&long, 1), 1, 1, &text("never", &[], 0)).unwrap_err();
    assert!(!is_damaged(&e) && !is_gone(&e), "{e}");
    assert!(w.retarget(kept, &mk(&long, 2)).is_err());
    assert_eq!(w.flush(&s).unwrap(), Some(1));
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!((r.live(), r.key(kept)), (1, Some(mk("INBOX", 1))));
    assert_eq!(found(&mut r, &s, Field::Body, "kept"), [kept]);
    w.add(&mk(&"y".repeat(MAX_NAME_BYTES), 1), 1, 1, &text("longest", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(r.refresh(&s).unwrap(), "the longest name kept reads back");
}

/// Flush after flush never names more segments than a meta may: at the limit the words are
/// made again from the texts, into few segments, and everything still reads back. (The limit
/// lowered from [`MAX_SEGMENTS`]: 4,100 flushes took 30 s.)
#[test]
fn segments_never_pass_what_a_meta_names() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.max_segments = 40;
    let n = 100;
    for i in 0..n {
        w.add(&mk("INBOX", i + 1), 1, 1, &text(&format!("common word{i}"), &[], 0)).unwrap();
        w.flush(&s).unwrap();
    }
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert!(meta.segments.len() <= 40 && meta.next_seg > 80, "{} segments of {}", meta.segments.len(), meta.next_seg);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.live(), n);
    assert_eq!(found(&mut r, &s, Field::Body, "common").len(), n as usize);
    assert_eq!(found(&mut r, &s, Field::Body, &format!("word{}", n - 1)), [n - 1]);
    drop(w);
    assert_eq!(files(&s).1, BTreeSet::new(), "the old segments are gone");
}

/// A text longer than a text page keeps is cut (marked truncated) and reads back; the index
/// and the other docs stay.
#[test]
fn a_text_too_long_for_a_page_is_cut() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let small = w.add(&mk("INBOX", 2), 1, 1, &text("keepme", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let big = w.add(&mk("INBOX", 1), 1, 1, &text(&"word \u{e9} ".repeat(1_500_000), &["a.pdf"], 1)).unwrap();
    let after = w.add(&mk("INBOX", 3), 1, 1, &text("after", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.live(), 3);
    assert_eq!((found(&mut r, &s, Field::Body, "keepme"), found(&mut r, &s, Field::Body, "after")), (vec![small], vec![after]));
    assert!(r.info(big).unwrap().truncated && !r.info(small).unwrap().truncated);
    let blob = r.text(&s, big).unwrap().unwrap();
    assert!(blob.text.len() <= MAX_BLOB_TEXT && blob.text.starts_with("word \u{e9}") && blob.names == ["a.pdf"]);
    assert_text_pages(&s);
}

/// Every text page holds at most [`TEXT_PAGE`], or one text alone that still goes in one
/// background transaction.
fn assert_text_pages(s: &Store) {
    let meta = load_meta(s, "a1").unwrap().unwrap();
    for (p, t) in meta.text_pages.iter().enumerate().filter(|(_, t)| t.generation != 0) {
        let page = s.files.read(&Name::Text { generation: t.generation, page: p as u32 }.path(&dir(s, "a1"))).unwrap();
        let texts = decode_text_page(&page, meta.next_doc).unwrap().len();
        let bytes = t.bytes as usize;
        assert!(bytes <= TEXT_PAGE || (texts == 1 && bytes <= crate::BACKGROUND_COMMIT), "page {p}: {bytes} bytes, {texts} texts");
    }
}

/// A text longer than a page has a page of its own: it never joins the texts of a page that has
/// some, nor do they join its.
#[test]
fn a_long_text_has_a_page_of_its_own() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let small = w.add(&mk("INBOX", 1), 1, 1, &text(&"short ".repeat(1000), &[], 0)).unwrap();
    let long = w.add(&mk("INBOX", 2), 1, 1, &text(&"long ".repeat(800_000), &[], 0)).unwrap();
    let after = w.add(&mk("INBOX", 3), 1, 1, &text("after", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert_text_pages(&s);
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    assert_eq!(meta.text_pages.len(), 3, "{:?}", meta.text_pages);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.text(&s, long).unwrap().unwrap().text.len(), MAX_BLOB_TEXT);
    assert!(r.info(long).unwrap().truncated);
    assert!(r.text(&s, small).unwrap().is_some() && r.text(&s, after).unwrap().is_some());
}

/// Removals count toward what a flush holds: the text pages losing a text are read and written
/// again whole, so a change that only removes still flushes before it holds too much.
#[test]
fn removals_count_toward_what_a_flush_holds() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.in_flight = 1 << 20;
    let mut docs = Vec::new();
    for i in 0..128u32 {
        let t: String = (0..2000).map(|n| format!("w{i}x{n} ")).collect();
        docs.push(w.add(&mk("INBOX", i + 1), 1, 1, &text(&t, &[], 0)).unwrap());
        if w.should_flush() {
            w.flush(&s).unwrap();
        }
    }
    w.flush(&s).unwrap();
    let pages = load_meta(&s, "a1").unwrap().unwrap().text_pages.len();
    assert!(pages >= 8, "{pages} text pages");
    let mut flushes = 0;
    for d in docs.iter().step_by(2) {
        w.remove(*d);
        if w.should_flush() {
            let f = w.prepare(&s).unwrap();
            assert!(f.bytes() <= w.in_flight, "a flush of {} bytes", f.bytes());
            w.commit(&s, f).unwrap();
            flushes += 1;
        }
    }
    assert!(flushes >= 2, "removals alone flushed {flushes} times");
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.live(), 64);
    assert!(r.text(&s, docs[0]).unwrap().is_none() && r.text(&s, docs[1]).unwrap().is_some());
}

/// Words a commit began to make again (at the segments' limit) and didn't finish are made by the
/// next flush, with nothing new to write: the index isn't left without them.
#[test]
fn words_left_to_make_again_are_made_by_the_next_flush() {
    let s = Store::failing();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.max_segments = 1;
    w.add(&mk("INBOX", 1), 1, 1, &text("first", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    w.add(&mk("INBOX", 2), 2, 2, &text("second", &[], 0)).unwrap();
    s.fail(None, Some("-h.bin"));
    assert_eq!(w.flush(&s).unwrap(), Some(2), "the generation is in");
    assert!(Reader::open(&s, "a1").unwrap().is_none(), "its words not made yet");
    s.fail(None, None);
    assert_eq!(w.flush(&s).unwrap(), Some(3));
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!((found(&mut r, &s, Field::Body, "first"), found(&mut r, &s, Field::Body, "second")), (vec![0], vec![1]));
    assert_eq!(w.flush(&s).unwrap(), None);
    assert_eq!(files(&s).1, BTreeSet::new());
}

/// A message added at a folder and uid another doc holds under an older validity replaces it:
/// it is another message now, and the old one leaves the index.
#[test]
fn a_new_validity_replaces_the_old_doc() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let old = w.add(&mk("INBOX", 1), 1, 1, &text("hello old", &[], 0)).unwrap();
    let new = w.add(&MessageKey { validity: 8, ..mk("INBOX", 1) }, 2, 2, &text("hello new", &[], 0)).unwrap();
    let moved = w.add(&mk("Archive", 5), 3, 3, &text("hello moved", &[], 0)).unwrap();
    w.retarget(moved, &MessageKey { validity: 9, ..mk("INBOX", 1) }).unwrap();
    w.flush(&s).unwrap();
    assert_eq!(w.live(), 1);
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    assert_eq!(r.live(), 1);
    assert_eq!(found(&mut r, &s, Field::Body, "hello"), [moved]);
    assert_eq!((r.key(old), r.key(new)), (None, None));
    assert_eq!(r.key(moved), Some(MessageKey { validity: 9, ..mk("INBOX", 1) }));
}

/// A text page the writer's meta names and isn't there is damage (to be made again), not an
/// error every flush and every open repeats.
#[test]
fn a_missing_text_page_is_damage_to_the_writer() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let d = w.add(&mk("INBOX", 1), 1, 1, &text("words", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    s.files.remove_file(&Name::Text { generation: meta.text_pages[0].generation, page: 0 }.path(&dir(&s, "a1"))).unwrap();
    w.remove(d);
    let e = w.flush(&s).unwrap_err();
    assert!(is_damaged(&e), "{e}");
    drop(w);
    // An older tokenizer's: its words are made again from the texts, and that one isn't there.
    let mut meta = load_meta(&s, "a1").unwrap().unwrap();
    meta.tokenizer = 0;
    s.files.write(&meta_path(&s, "a1"), &encode_meta(&meta)).unwrap();
    assert_eq!(Writer::open(&s, "a1").unwrap().live(), 0, "set aside and made again");
}

/// A page the same meta still names and isn't there is damage; while the writer is held that's
/// the reader's error, and the meta is left for the writer's holder to set aside.
#[test]
fn a_reader_leaves_a_held_index_to_its_writer() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.add(&mk("INBOX", 1), 1, 1, &text("words", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    s.files.remove_file(&Name::Docs { generation: meta.doc_pages[0], page: 0 }.path(&dir(&s, "a1"))).unwrap();
    let e = Reader::open(&s, "a1").unwrap_err();
    assert!(is_damaged(&e), "{e}");
    assert!(s.files.is_file(&meta_path(&s, "a1")), "not set aside under its writer");
    drop(w);
    assert!(Reader::open(&s, "a1").unwrap().is_none(), "set aside once let go");
    assert!(s.files.is_file(&meta_path(&s, "a1").with_extension("bin.bad")));
}

/// A refresh that fails part way leaves the reader at the generation it had, whole.
#[test]
fn a_failed_refresh_leaves_the_reader_as_it_was() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let a = w.add(&mk("INBOX", 1), 1, 1, &text("alpha", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    w.remove(a);
    w.add(&mk("Other", 2), 2, 2, &text("beta", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    s.files.remove_file(&Name::SegHead(meta.segments.last().unwrap().id).path(&dir(&s, "a1"))).unwrap();
    assert!(r.refresh(&s).is_err());
    assert_eq!((r.generation(), r.live(), r.len()), (1, 1, 1));
    assert_eq!((r.doc(&mk("INBOX", 1)), r.key(a)), (Some(a), Some(mk("INBOX", 1))));
    assert_eq!(found(&mut r, &s, Field::Body, "alpha"), [a]);
}

/// An index commit is the data file's last, and sealed: a crash after it costs no repair, and
/// nothing is left waiting for a seal.
#[test]
fn an_index_commit_seals_its_data_file() {
    let (_, dir, s) = layouts("sealed").pop().unwrap();
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let mut w = Writer::open(&s, "a1").unwrap();
    let d = w.add(&mk("INBOX", 1), 1, 1, &text("words", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(!packs.seal_due("a1"), "sealed with its meta");
    w.remove(d);
    w.add(&mk("INBOX", 2), 1, 1, &text("more", &[], 0)).unwrap();
    w.flush(&s).unwrap();
    assert!(!packs.seal_due("a1"), "the files it no longer uses removed in the sealed commit");
    assert_eq!(files(&s).1, BTreeSet::new());
    drop((w, s));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Words made ready on another thread ([`Ready`]) and filed by the writer give the index that
/// adding the docs themselves gives: the same words, texts, files and flags, and the same bytes
/// held for the flush.
#[test]
fn words_made_ready_elsewhere_are_the_same_index() {
    let docs = [
        text("Alpha beta beta ana@studio.example", &["Plan.PDF", "beta.txt"], 2),
        IndexDoc { truncated: true, ..text("gamma", &[], 0) },
        text("", &["only-a-name.zip"], 1),
        text(&"long ".repeat(400_000), &[], 0),
    ];
    let (direct, ready) = (Store::memory(), Store::memory());
    let mut a = Writer::open(&direct, "a1").unwrap();
    let mut b = Writer::open(&ready, "a1").unwrap();
    let made: Vec<Ready> = std::thread::scope(|t| t.spawn(|| docs.iter().map(Ready::of).collect()).join().unwrap());
    for (uid, (doc, r)) in (1..).zip(docs.iter().zip(made)) {
        assert_eq!(a.add(&mk("INBOX", uid), u64::from(uid), 9, doc).unwrap(), b.add_ready(&mk("INBOX", uid), u64::from(uid), 9, r).unwrap());
    }
    assert_eq!(a.pending_bytes(), b.pending_bytes());
    a.flush(&direct).unwrap();
    b.flush(&ready).unwrap();
    let (mut ra, mut rb) = (Reader::open(&direct, "a1").unwrap().unwrap(), Reader::open(&ready, "a1").unwrap().unwrap());
    for (field, prefix) in
        [(Field::Body, "beta"), (Field::Body, "long"), (Field::Body, "g"), (Field::Names, "plan"), (Field::Address, "studio.example")]
    {
        let want = found(&mut ra, &direct, field, prefix);
        assert!(!want.is_empty(), "{prefix}");
        assert_eq!(found(&mut rb, &ready, field, prefix), want, "{prefix}");
    }
    for d in 0..4 {
        assert_eq!(rb.info(d), ra.info(d));
        assert_eq!(rb.text(&ready, d).unwrap(), ra.text(&direct, d).unwrap());
    }
}

/// A writer told to hold less writes sooner (a background pass's holds a quarter of an
/// import's), and none holds more than an import's.
#[test]
fn a_writer_held_to_less_writes_sooner() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    w.hold_at_most(IN_FLIGHT_BYTES * 4);
    assert_eq!(w.in_flight, IN_FLIGHT_BYTES);
    w.hold_at_most(64 << 10);
    let mut uid = 0;
    while !w.should_flush() {
        uid += 1;
        w.add(&mk("INBOX", uid), 0, 100, &text(&format!("word{uid} other{uid} more{uid}"), &[], 0)).unwrap();
        assert!(uid < 10_000, "never due");
    }
    assert!(uid < 2_000, "{uid} messages held");
}
