//! The search index kept up by the pool.

use std::sync::atomic::{AtomicBool, AtomicUsize};

use mailcraft_model::search::IndexDoc;
use mailcraft_pst::write;
use mailcraft_store::search;

use super::*;
use crate::fakes::measure::percentile;

mod index_bench;
mod index_damaged;

/// The search index kept up by the pool: an account synced, then asked for, is searchable.
#[test]
fn synced_mail_is_indexed_and_found() {
    let st = crate::fake::State::new();
    {
        let mut s = st.lock().unwrap();
        s.add("INBOX", "hello", &[]);
        s.add("INBOX", "second", &["\\Seen"]);
    }
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::store("indexed");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = imap_account("a1", port);
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Asked twice in a row: one pass.
    pool.submit(Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] });
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    let Some(Event::Indexed { work, generation, failed, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!((work.done, work.total, *failed), (2, 2, 0));
    let mut r = search::Reader::open(&store, "a1").unwrap().unwrap();
    assert_eq!(r.generation(), *generation);
    let found: Vec<u32> = r.word(&store, search::Field::Body, "hello").unwrap().ones().filter_map(|d| r.key(d)).map(|k| k.uid).collect();
    assert_eq!(found, [1]);
    // No second pass was waiting.
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        assert!(!matches!(pool.wait(Duration::from_millis(100)), Some(Event::Indexed { .. })), "a second pass ran");
    }
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A pass as the indexer runs it, with this extraction; how it ended.
fn index_with(s: &Store, stop: &AtomicBool, readers: usize, extract: &(dyn Fn(&[u8]) -> IndexDoc + Sync)) -> Event {
    let mut last = None;
    indexing::pass(s, "a1", &[], stop, readers, extract, &mut |e| last = Some(e)).unwrap();
    last.unwrap()
}

/// The messages the index has `word` in; none when there's no index yet (a pass stopped before
/// it added the first message writes none).
fn found(s: &Store, word: &str) -> Vec<u32> {
    let Some(mut r) = search::Reader::open(s, "a1").unwrap() else { return Vec::new() };
    r.word(s, search::Field::Body, word).unwrap().ones().filter_map(|d| r.key(d)).map(|k| k.uid).collect()
}

/// A message moved here (its source written again under its new folder and uid) keeps its doc:
/// the next pass reads no message.
#[test]
fn a_local_move_is_followed_without_reading() {
    let s = indexed_mail(3);
    let ended = index_with(&s, &AtomicBool::new(false), 2, &mailcraft_mime::index_doc);
    assert!(matches!(ended, Event::Indexed { work, finished: true, .. } if work.done == 3));
    let mut folders = s.load_folders("a1").unwrap();
    folders.push(Folder { account: "a1".into(), path: "Archive".into(), selectable: true, ..Default::default() });
    s.save_folders("a1", &folders).unwrap();
    // Filed there as a local move does it: under a new number, its source with it.
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 2 };
    let mut moved = s.load_folder("a1", "INBOX").unwrap().find(2).unwrap().clone();
    moved.key = MessageKey { account: "a1".into(), folder: "Archive".into(), validity: 1, uid: 1 };
    s.write_body(&moved.key, &s.read_body(&key).unwrap()).unwrap();
    let archive = mailcraft_store::FolderState { validity: 1, uid_next: 2, messages: vec![moved.into()], ..Default::default() };
    s.save_folder("a1", "Archive", &archive).unwrap();
    s.apply_op("a1", &Op::Move { keys: vec![key], to: "Archive".into() }).unwrap();
    let reads = s.reads();
    let ended = index_with(&s, &AtomicBool::new(false), 2, &mailcraft_mime::index_doc);
    assert_eq!(s.reads(), reads, "no message read");
    assert!(matches!(ended, Event::Indexed { work, finished: true, .. } if work.total == 0), "{ended:?}");
    let mut r = search::Reader::open(&s, "a1").unwrap().unwrap();
    let keys: Vec<MessageKey> = r.word(&s, search::Field::Body, "word2").unwrap().ones().filter_map(|d| r.key(d)).collect();
    assert_eq!(keys.iter().map(|k| k.folder.as_str()).collect::<Vec<_>>(), ["Archive"]);
}

/// The sources a pass reads at once are bounded by their size, not only by the threads: big ones
/// (their summaries say 30 MB) are read one at a time on four threads, and every one is indexed.
#[test]
fn big_sources_are_read_one_at_a_time() {
    let s = Store::memory();
    fill(&s, 12);
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    for m in st.messages.iter_mut().map(std::sync::Arc::make_mut) {
        m.size = 30 << 20;
    }
    s.save_folder("a1", "INBOX", &st).unwrap();
    let (now, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let extract = |raw: &[u8]| {
        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(n, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(15));
        now.fetch_sub(1, Ordering::SeqCst);
        mailcraft_mime::index_doc(raw)
    };
    let ended = index_with(&s, &AtomicBool::new(false), 4, &extract);
    assert!(matches!(ended, Event::Indexed { work, finished: true, .. } if work.done == 12), "{ended:?}");
    assert_eq!(most.load(Ordering::SeqCst), 1, "one big source at a time");
    assert_eq!(found(&s, "common").len(), 12);
}

/// Stopped partway, a pass keeps what it added, and the index reads back; the next pass adds the
/// rest and nothing twice.
#[test]
fn a_pass_stopped_partway_leaves_a_consistent_index() {
    let s = indexed_mail(40);
    let stop = AtomicBool::new(false);
    let read = AtomicUsize::new(0);
    let stopping = |raw: &[u8]| {
        if read.fetch_add(1, Ordering::Relaxed) == 9 {
            stop.store(true, Ordering::Relaxed);
        }
        mailcraft_mime::index_doc(raw)
    };
    let ended = index_with(&s, &stop, 1, &stopping);
    let Event::Indexed { work, finished: true, .. } = ended else { panic!("{ended:?}") };
    // What was read and not yet added when it stopped is let go: none of it when the reader got
    // to its tenth before the pass added the first (a busy machine).
    assert!((0..=10).contains(&work.done) && work.total == 40, "{work:?}");
    assert_eq!(found(&s, "common").len() as u64, work.done, "what it added is there");
    let ended = index_with(&s, &AtomicBool::new(false), 4, &mailcraft_mime::index_doc);
    assert!(matches!(ended, Event::Indexed { work: w, finished: true, .. } if w.total == 40 - work.done), "{ended:?}");
    assert_eq!(found(&s, "common").len(), 40);
    assert_eq!(found(&s, "word40"), [40], "once");
}

/// An account removed while its pass runs (the store letting go of it before the pass hears the
/// stop) isn't written again: the pass's last write once put its index back, making the removed
/// account a new data file beside the one it kept.
#[test]
fn a_pass_writes_nothing_for_an_account_removed_meanwhile() {
    let t = std::env::temp_dir().join(format!("mailcraft-index-removed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[AccountConfig { id: "a1".into(), ..Default::default() }]).unwrap();
    fill(&s, 20);
    let files = || {
        let mut names: Vec<String> = std::fs::read_dir(t.join("Files")).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
        names.sort();
        names
    };
    let before = files();
    let stop = AtomicBool::new(false);
    let read = AtomicUsize::new(0);
    let removing = |raw: &[u8]| {
        // Removed as the session removes it, the stop coming after.
        if read.fetch_add(1, Ordering::Relaxed) == 5 {
            s.remove_account("a1").unwrap();
            stop.store(true, Ordering::Relaxed);
        }
        mailcraft_mime::index_doc(raw)
    };
    let _ = indexing::pass(&s, "a1", &[], &stop, 1, &removing, &mut |_| {});
    assert_eq!(s.data_file("a1"), None, "a removed account got a data file back");
    assert_eq!(files(), before);
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}

/// A message whose text can't be taken out (the extraction panics) is counted, kept with no
/// words, and doesn't stop the others.
#[test]
fn a_message_that_breaks_the_extraction_is_counted() {
    let s = indexed_mail(5);
    let breaking = |raw: &[u8]| {
        assert!(!String::from_utf8_lossy(raw).contains("word3 "), "a message that breaks it");
        mailcraft_mime::index_doc(raw)
    };
    let ended = index_with(&s, &AtomicBool::new(false), 3, &breaking);
    assert!(matches!(ended, Event::Indexed { work, failed: 1, finished: true, .. } if work.done == 5), "{ended:?}");
    assert_eq!(found(&s, "common").len(), 4);
    let r = search::Reader::open(&s, "a1").unwrap().unwrap();
    let doc = r.doc(&MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 3 }).unwrap();
    assert!(r.info(doc).unwrap().truncated, "kept as something left out");
    // Not read again by the next pass.
    let reads = s.reads();
    index_with(&s, &AtomicBool::new(false), 3, &breaking);
    assert_eq!(s.reads(), reads);
}

/// No pass runs for an account being imported into; it runs once the import ends.
#[test]
fn no_pass_runs_during_an_import() {
    let s = indexed_mail(2);
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    let importing = pool.indexer.importing("a1");
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        assert!(!matches!(pool.wait(Duration::from_millis(100)), Some(Event::Indexed { .. })), "a pass ran during the import");
    }
    drop(importing);
    until(&pool, "both indexed", |e| matches!(e, Event::Indexed { work, finished: true, .. } if work.done == 2));
}

/// A data file's messages go into the index as the import writes them: the pass asked for at
/// its end has nothing to add, and reads nothing.
#[test]
fn an_imported_file_leaves_nothing_to_index() {
    use mailcraft_pst::write::NewMessage;
    let t = std::env::temp_dir().join(format!("mailcraft-worker-import-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let file = t.join("in.pst");
    let message = NewMessage {
        subject: "Plan",
        sender_name: "Ana",
        sender_email: "ana@studio.example",
        recipients: &[],
        body: "the rehearsal moved to thursday",
        html_body: None,
        message_id: "<plan@studio.example>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    write::create_with(&file, &["Inbox"], &message, &[]).unwrap();
    let store = Store::memory();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::ImportPst { account: "p1".into(), path: file.to_string_lossy().into_owned(), resume: false });
    until(&pool, "the import", |e| matches!(e, Event::Imported { messages: 1, done: true, error: None, .. }));
    let reads = store.reads();
    pool.submit(Job::Index { account: "p1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    assert!(matches!(seen.last(), Some(Event::Indexed { work, failed: 0, .. }) if work.total == 0), "{seen:?}");
    assert_eq!(store.reads(), reads, "no message read");
    let mut r = search::Reader::open(&store, "p1").unwrap().unwrap();
    assert_eq!(r.word(&store, search::Field::Body, "rehearsal").unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&t);
}

/// Summaries that said a message carries files where its source shows only a picture in its
/// text (an old `.pst` import's), or none where it carries one, are put right by the pool's
/// pass: in the folder's list, and told to the window as one event that isn't merged away.
#[test]
fn a_pass_puts_right_what_summaries_say_of_files() {
    let s = Store::memory();
    wrong_about_files(&s);
    let before = s.load_folder("a1", "INBOX").unwrap();
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let mut known = Vec::new();
    until(&pool, "the index finished", |e| {
        if let Event::AttachmentsKnown { folder, with, without, .. } = e {
            known.push((folder.clone(), with.clone(), without.clone()));
        }
        matches!(e, Event::Indexed { finished: true, .. })
    });
    assert_eq!(known, [("INBOX".to_string(), vec![2], vec![1])]);
    let st = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!((st.find(1).unwrap().has_attachments, st.find(2).unwrap().has_attachments), (false, true));
    // A save that loaded the list before writes it back as it was (the very bytes the pass was
    // planned on): the folder wasn't marked, so the next pass puts it right again.
    s.save_folder("a1", "INBOX", &before).unwrap();
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    assert_eq!(seen.iter().filter(|e| matches!(e, Event::AttachmentsKnown { .. })).count(), 1, "{seen:?}");
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(1).unwrap().has_attachments);
    // The next pass finds them right: nothing to tell.
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    assert!(!seen.iter().any(|e| matches!(e, Event::AttachmentsKnown { .. })), "{seen:?}");
}

/// Summaries that couldn't be put right (the list's write failed) leave their folder unmarked:
/// the next pass, its list unchanged, finds them again from the index and puts them right.
#[test]
fn summaries_not_put_right_are_tried_again() {
    let s = Store::failing();
    wrong_about_files(&s);
    s.fail(None, Some("state.json"));
    let mut pool = Pool::new(s.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    assert!(!seen.iter().any(|e| matches!(e, Event::AttachmentsKnown { .. })), "{seen:?}");
    assert!(s.load_folder("a1", "INBOX").unwrap().find(1).unwrap().has_attachments, "not written");
    s.fail(None, None);
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, "the index finished", |e| matches!(e, Event::Indexed { finished: true, .. }));
    let known: Vec<_> = seen.iter().filter(|e| matches!(e, Event::AttachmentsKnown { .. })).collect();
    assert_eq!(known.len(), 1, "{seen:?}");
    let st = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!((st.find(1).unwrap().has_attachments, st.find(2).unwrap().has_attachments), (false, true));
}

/// INBOX's two messages, their summaries wrong about files: 1 shows only a picture in its text
/// and is said to carry one; 2 carries a PDF and is said not to.
fn wrong_about_files(s: &Store) {
    let picture = concat!(
        "Subject: pic\r\nMessage-ID: <1@x>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"b\"\r\n\r\n",
        "--b\r\nContent-Type: text/html\r\n\r\n<p>see <img src=\"cid:pic1\"></p>\r\n",
        "--b\r\nContent-Type: image/png; name=\"image001.png\"\r\nContent-Disposition: inline; filename=\"image001.png\"\r\n",
        "Content-ID: <pic1>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n",
    );
    let file = concat!(
        "Subject: plan\r\nMessage-ID: <2@x>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n",
        "--b\r\nContent-Type: text/plain\r\n\r\nthe plan\r\n",
        "--b\r\nContent-Type: application/pdf; name=\"plan.pdf\"\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--b--\r\n",
    );
    let summary = |uid: u32, files: bool| mailcraft_model::Summary {
        key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid },
        message_id: format!("{uid}@x"),
        size: 40,
        date: i64::from(uid),
        has_attachments: files,
        ..Default::default()
    };
    let inbox = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.save_folders("a1", &[inbox]).unwrap();
    let messages = vec![summary(2, false), summary(1, true)];
    s.save_folder(
        "a1",
        "INBOX",
        &mailcraft_store::FolderState { validity: 7, uid_next: 3, messages: messages.into_iter().map(Into::into).collect(), ..Default::default() },
    )
    .unwrap();
    for (uid, raw) in [(1, picture), (2, file)] {
        s.write_body(&MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid }, raw.as_bytes()).unwrap();
    }
}

/// A pass's end isn't hidden by the next pass starting before the window looks: both are
/// heard, the end first; a later report of the next only replaces the one before it.
#[test]
fn a_pass_end_outlives_the_next_pass_starting() {
    let indexed = |done, finished| Event::Indexed {
        account: "a1".into(),
        work: mailcraft_model::job::Work { done, ..Default::default() },
        generation: 1,
        failed: 0,
        finished,
    };
    let latest = indexing::Latest::default();
    latest.put("a1", indexed(1, false));
    latest.put("a1", indexed(5, true));
    latest.put("a1", indexed(0, false));
    latest.put("a1", indexed(2, false));
    latest.put("b2", indexed(3, true));
    let heard: Vec<(u64, bool)> = latest
        .take()
        .into_iter()
        .filter_map(|e| match e {
            Event::Indexed { work, finished, .. } => Some((work.done, finished)),
            _ => None,
        })
        .collect();
    assert_eq!(heard, [(5, true), (2, false), (3, true)]);
    latest.put("a1", indexed(5, true));
    latest.put("a1", indexed(0, false));
    assert!(matches!(latest.pop(), Some(Event::Indexed { finished: true, .. })));
    assert!(matches!(latest.pop(), Some(Event::Indexed { finished: false, .. })));
    assert!(latest.pop().is_none());
}

/// Asks for an account are one: folders add up, none (every folder) takes them all, and the
/// latest time to start is kept.
#[test]
fn asks_for_an_account_are_one() {
    let mut p = indexing::Pending::new();
    let at = Instant::now();
    indexing::want(&mut p, "a1".into(), vec!["INBOX".into()], at);
    indexing::want(&mut p, "a1".into(), vec!["Sent".into()], at + Duration::from_secs(1));
    assert_eq!(p.get("a1").map(|(f, w)| (f.clone().unwrap_or_default().len(), *w)), Some((2, at + Duration::from_secs(1))));
    indexing::want(&mut p, "a1".into(), Vec::new(), at);
    indexing::want(&mut p, "a1".into(), vec!["Drafts".into()], at);
    assert_eq!(p.get("a1").map(|(f, w)| (f.is_none(), *w)), Some((true, at + Duration::from_secs(1))));
}

/// A pass gives way to the read marks: with a mark handed over while it reads (its plan waited
/// for those before), it writes nothing of the index until the mark is in (a second at most), so
/// a mark never waits behind a run of the pass's writes.
#[test]
fn a_pass_gives_way_to_read_marks() {
    let s = indexed_mail(30);
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 1 };
    // The first message taken apart holds the pass there until the mark has been handed over.
    let (reading, read) = (std::sync::Mutex::new(None::<mpsc::Sender<()>>), std::sync::Mutex::new(None::<mpsc::Receiver<()>>));
    let (tx, started) = mpsc::channel();
    let (go, rx) = mpsc::channel();
    *reading.lock().unwrap() = Some(tx);
    *read.lock().unwrap() = Some(rx);
    let extract = |raw: &[u8]| {
        if let Some(tx) = reading.lock().unwrap().take() {
            tx.send(()).unwrap();
            read.lock().unwrap().take().unwrap().recv().unwrap();
        }
        mailcraft_mime::index_doc(raw)
    };
    let stop = AtomicBool::new(false);
    std::thread::scope(|sc| {
        let pass = sc.spawn(|| indexing::pass(&s, "a1", &[], &stop, 1, &extract, &mut |_| {}));
        started.recv().unwrap();
        let marks = s.pause_marks();
        let seen = mailcraft_model::Flags { seen: true, ..Default::default() };
        s.mark_later("a1", vec![mailcraft_store::FolderMarks { folder: "INBOX".into(), validity: 7, flags: vec![(key.uid, seen)] }], Vec::new());
        go.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!pass.is_finished(), "the pass wrote its index with a read mark waiting");
        assert!(s.marks_waiting());
        drop(marks);
        assert!(pass.join().unwrap().is_ok());
    });
    assert!(!s.marks_waiting());
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 1 && m.flags.seen));
    assert!(matches!(index_with(&s, &stop, 2, &mailcraft_mime::index_doc), Event::Indexed { work, .. } if work.total == 0), "all indexed");
}

/// A mark-read from the window while a pass writes a data file's index, the way the window makes
/// it (handed to the store's thread for marks, timed until it's on disk), waits for one of the
/// pass's short transactions at most: compared with the same mark-reads alone. Measured in release
/// on 32 threads: 20,000 messages indexed in 9.8 s; a mark-read's median 3.4 ms alone and 3.6 ms
/// during the pass, p99 3.7 and 11.7 ms, within the plan's 60 ms (made in the window's frame, as
/// this measured before, they took 49 and 59 ms by median, 108 and 128 ms at p99). A measurement,
/// as the store's (timings on a busy machine swing too far to hold them in a test): run alone
/// with `--ignored --nocapture`; `index_bench` measures the same on a real data file.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn a_mark_read_waits_little_for_an_index_pass() {
    let t = std::env::temp_dir().join(format!("mailcraft-index-latency-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[AccountConfig { id: "a1".into(), ..Default::default() }]).unwrap();
    let n = 20_000u32;
    let messages: Vec<mailcraft_model::Summary> = (1..=n)
        .map(|uid| mailcraft_model::Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid },
            message_id: format!("m{uid}@x"),
            size: 6000,
            date: i64::from(uid),
            ..Default::default()
        })
        .collect();
    s.save_folders("a1", &[Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    s.save_folder(
        "a1",
        "INBOX",
        &mailcraft_store::FolderState {
            validity: 7,
            uid_next: n + 1,
            messages: messages.into_iter().map(Into::into).collect(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut rng = crate::fakes::random::Rng::new(0x9E37_79B9_7F4A_7C15);
    for uid in 1..=n {
        let words: String = (0..800).map(|_| format!("w{} ", rng.next_u64() % 50_000)).collect();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        s.write_body(&key, format!("Subject: m\r\nMessage-ID: <m{uid}@x>\r\n\r\n{words}\r\n").as_bytes()).unwrap();
    }
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 1 };
    let mark = |on: bool| {
        let flags = mailcraft_model::Flags { seen: on, ..Default::default() };
        let t0 = Instant::now();
        s.mark_later("a1", vec![mailcraft_store::FolderMarks { folder: "INBOX".into(), validity: 7, flags: vec![(key.uid, flags)] }], Vec::new());
        assert!(s.flush_marks());
        t0.elapsed().as_secs_f64() * 1000.0
    };
    let at = |took: &mut Vec<f64>, q: usize| {
        took.sort_by(f64::total_cmp);
        percentile(took, q).unwrap_or(0.0)
    };
    let mut alone: Vec<f64> = (0..50).map(|i| mark(i % 2 == 0)).collect();
    let (alone50, alone99) = (at(&mut alone, 50), at(&mut alone, 99));
    let stop = Arc::new(AtomicBool::new(false));
    let began = Instant::now();
    let pass = {
        let (s, stop) = (s.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut reports = 0u32;
            indexing::pass(&s, "a1", &[], &stop, indexing::readers(), &mailcraft_mime::index_doc, &mut |_| reports += 1).unwrap();
            reports
        })
    };
    let mut took = Vec::new();
    while !pass.is_finished() {
        took.push(mark(took.len() % 2 == 0));
        std::thread::sleep(Duration::from_millis(20));
    }
    let reports = pass.join().unwrap();
    let whole = began.elapsed();
    let (median, p99) = (at(&mut took, 50), at(&mut took, 99));
    eprintln!(
        "{n} messages indexed in {whole:?} ({reports} reports); mark-reads alone: median {alone50:.1} ms, p99 {alone99:.1} ms; {} during the pass: median {median:.1} ms, p99 {p99:.1} ms",
        took.len()
    );
    assert!(p99 <= 60.0, "a mark-read took {p99:.1} ms (p99) during the pass, {alone99:.1} ms alone");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}
