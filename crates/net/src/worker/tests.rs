use std::sync::atomic::{AtomicBool, AtomicUsize};

use mailcraft_model::search::IndexDoc;
use mailcraft_store::search;

use super::*;

/// Sent Items asked for after each of several messages is read once.
#[test]
fn a_folder_waiting_to_be_read_isnt_queued_twice() {
    let mut q = VecDeque::new();
    let sent = Job::SyncFolder { account: "a".into(), folder: "Sent".into() };
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::SyncFolder { account: "a".into(), folder: "INBOX".into() });
    push_job(&mut q, sent.clone());
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    push_job(&mut q, Job::GetVacation { account: "a".into() });
    assert_eq!(q.len(), 4, "only the folder read is merged");
    // A calendar sync waiting already: the second would send the first one's new items again
    // before its results were merged (found testing issue #30 against Radicale).
    push_job(&mut q, Job::SyncDav { account: "a".into() });
    push_job(&mut q, Job::SyncDav { account: "a".into() });
    push_job(&mut q, Job::SyncDav { account: "b".into() });
    assert_eq!(q.len(), 6);
}

#[test]
fn memory_credentials() {
    let m = MemoryCredentials::default();
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    assert_eq!(m.get(&a), None);
    m.set(&a, "pw").unwrap();
    assert_eq!(m.get(&a).as_deref(), Some("pw"));
    m.delete(&a);
    assert_eq!(m.get(&a), None);
}

/// Microsoft can take a token and then refuse the mailbox ("User is authenticated but not
/// connected"). The address the browser signed in as is tried; otherwise it's a refusal that
/// says what to do, not a sign-in to ask for again.
#[test]
fn a_mailbox_microsoft_wont_open_says_why() {
    let st = crate::fake::State::new();
    st.lock().unwrap().not_connected.push("zoe@hotmail.example".into());
    let port = crate::fake::serve_tcp(&st);
    let store = crate::fake::tests_store("closed-mailbox");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut acc = AccountConfig {
        id: "a1".into(),
        email: "zoe@hotmail.example".into(),
        imap: mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None },
        oauth: Some("microsoft".into()),
        ..Default::default()
    };
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(e.to_string().starts_with(mailcraft_model::MICROSOFT_IMAP_CLOSED) && e.to_string().contains("IMAP"), "{e:?}");
    // The browser signed in as the mailbox's own address: that one opens it.
    acc.oauth_user = Some("me@studio.example".into());
    let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
    assert!(w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).is_ok());
    // Signed in as another account that can't open it either: named in the refusal.
    acc.oauth_user = Some("someone@else.example".into());
    let w = Worker::new(acc, Auth::shared("refresh"), store, pool.sink.clone(), SyncOptions::default());
    let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
    assert!(matches!(&e, NetError::Refused(m) if m.contains("signed in as someone@else.example")), "{e:?}");
    // A token that's simply refused is still a sign-in to ask for again.
    let w = Worker::new(
        AccountConfig { oauth_user: None, ..w.acc.clone() },
        Auth::shared("refresh"),
        crate::fake::tests_store("closed-mailbox-2"),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    assert!(matches!(w.imap_with_token("zoe@hotmail.example", "expired"), Err(NetError::Auth(_))));
}

/// A MailCraft data file (`.mailcraft`) comes in through the same job as a `.pst`.
#[test]
fn a_mailcraft_data_file_is_imported() {
    let t = std::env::temp_dir().join(format!("mailcraft-worker-datafile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 3 };
    let mut st = old.load_folder("a1", "INBOX").unwrap();
    st.messages.push(mailcraft_model::Summary { key: k, subject: "Kept".into(), ..Default::default() });
    old.save_folder("a1", "INBOX", &st).unwrap();
    let mut pim = mailcraft_model::pim::Pim::default();
    pim.notes.push(mailcraft_model::pim::StoredNote::default());
    old.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);
    let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // Looked at first: what the file system says of it.
    let path = file.to_string_lossy().into_owned();
    p.submit(Job::InspectDataFile { path: path.clone() });
    p.submit(Job::InspectDataFile { path: t.join("gone.pst").to_string_lossy().into_owned() });
    let end = Instant::now() + Duration::from_secs(20);
    let mut facts = Vec::new();
    while Instant::now() < end && facts.len() < 2 {
        facts.extend(p.wait(Duration::from_millis(100)).into_iter().filter_map(|e| match e {
            Event::DataFileInspected { facts, .. } => Some(facts),
            _ => None,
        }));
    }
    let size = std::fs::metadata(&file).unwrap().len();
    assert!(facts.iter().any(|f| f.path == path && f.kind == "mailcraft" && f.size == size && f.modified > 0 && f.error.is_none()), "{facts:?}");
    assert!(facts.iter().any(|f| f.path.ends_with("gone.pst") && f.kind == "pst" && f.error.is_some()), "{facts:?}");
    p.submit(Job::ImportPst { account: "p1".into(), path, resume: false });
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end && !seen.iter().any(|e| matches!(e, Event::Imported { done: true, .. })) {
        seen.extend(p.wait(Duration::from_millis(100)));
    }
    assert!(seen.iter().any(|e| matches!(e, Event::Imported { messages: 1, done: true, error: None, .. })), "{seen:?}");
    assert!(
        seen.iter().any(|e| matches!(e, Event::Imported { report: Some(r), .. } if r.messages == 1 && r.folders == 1 && (r.contacts, r.notes) == (0, 1) && r.runs == 1 && r.skips.is_empty() && !r.data_file.is_empty())),
        "with what it came to"
    );
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 1)));
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages[0].key.account, "p1");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn pool_without_password_asks_for_one() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pool-{}", std::process::id()));
    let store = Store::open(&dir).unwrap();
    let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
    let a = AccountConfig { id: "a1".into(), ..Default::default() };
    p.submit(Job::Start { account: a, password: None });
    assert!(matches!(p.poll().as_slice(), [Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }]));
    p.submit(Job::SyncAll { account: "a1".into() });
    assert!(matches!(p.poll().as_slice(), [Event::Notice { .. }]));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    p.submit(Job::FetchBody { key: key.clone() });
    assert!(matches!(p.poll().as_slice(), [Event::BodyFailed { .. }]));
    p.store.write_body(&key, b"Subject: cached\r\n\r\nx").unwrap();
    p.submit(Job::FetchBody { key });
    assert!(matches!(p.poll().as_slice(), [Event::Body { .. }]));
    let _ = std::fs::remove_dir_all(dir);
}

/// A pending-drafts list that can't be read isn't saved over with just the newest draft: the
/// drafts waiting in it stay.
#[test]
fn a_draft_kept_offline_doesnt_write_over_a_list_that_cant_be_read() {
    let store = Store::failing();
    keep_draft(&store, "a1", "Drafts", None, b"Subject: first\r\n\r\nOne\r\n".to_vec()).unwrap();
    store.fail(Some(PENDING_DRAFTS), None);
    assert!(keep_draft(&store, "a1", "Drafts", None, b"Subject: second\r\n\r\nTwo\r\n".to_vec()).is_err());
    store.fail(None, None);
    let kept: Vec<PendingDraft> = store.load_account_doc("a1", PENDING_DRAFTS).unwrap();
    assert_eq!(kept.iter().map(|d| d.raw.clone()).collect::<Vec<_>>(), [b"Subject: first\r\n\r\nOne\r\n".to_vec()]);
}

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
    let store = crate::fake::tests_store("indexed");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: mailcraft_model::ServerConfig { host: "127.0.0.1".into(), port, security: mailcraft_model::Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Asked twice in a row: one pass.
    pool.submit(Job::Index { account: "a1".into(), folders: vec!["INBOX".into()] });
    pool.submit(Job::Index { account: "a1".into(), folders: Vec::new() });
    let seen = until(&pool, |e| matches!(e, Event::Indexed { finished: true, .. }));
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
    let _ = std::fs::remove_dir_all(store.root());
}

/// Events until one `f` takes, all of them.
fn until(pool: &Pool, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let done = f(&e);
            seen.push(e);
            if done {
                return seen;
            }
        }
    }
    panic!("never came: {seen:?}")
}

/// An account of `n` messages in its Inbox, each with its source: `word{uid}` and `common`.
fn indexed_mail(n: u32) -> Store {
    let s = Store::memory();
    fill(&s, n);
    s
}

/// `n` messages in `s`'s account `a1`, as [`indexed_mail`] has them.
fn fill(s: &Store, n: u32) {
    let messages = (1..=n)
        .map(|uid| mailcraft_model::Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid },
            message_id: format!("m{uid}@x"),
            size: 40,
            date: i64::from(uid),
            ..Default::default()
        })
        .collect();
    let inbox = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.save_folders("a1", &[inbox]).unwrap();
    s.save_folder("a1", "INBOX", &mailcraft_store::FolderState { validity: 7, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    for uid in 1..=n {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        s.write_body(&key, format!("Subject: m\r\nMessage-ID: <m{uid}@x>\r\n\r\nword{uid} common\r\n").as_bytes()).unwrap();
    }
}

/// A pass as the indexer runs it, with this extraction; how it ended.
fn index_with(s: &Store, stop: &AtomicBool, readers: usize, extract: &(dyn Fn(&[u8]) -> IndexDoc + Sync)) -> Event {
    let mut last = None;
    indexing::pass(s, "a1", &[], stop, readers, extract, &mut |e| last = Some(e)).unwrap();
    last.unwrap()
}

fn found(s: &Store, word: &str) -> Vec<u32> {
    let mut r = search::Reader::open(s, "a1").unwrap().unwrap();
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
    let archive = mailcraft_store::FolderState { validity: 1, uid_next: 2, messages: vec![moved], ..Default::default() };
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
    // What was read and not yet added when it stopped is let go.
    assert!((1..=10).contains(&work.done) && work.total == 40, "{work:?}");
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
    until(&pool, |e| matches!(e, Event::Indexed { work, finished: true, .. } if work.done == 2));
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

/// A mark-read from the window while a pass writes a data file's index waits for one of the
/// pass's short transactions at most: compared with the same mark-reads alone (which write a
/// page of a 20,000-message list). Measured in release on 32 threads: 20,000 messages indexed in
/// 9.5 s; a mark-read's median 49 ms alone and 59 ms during the pass, p99 108 and 128 ms. The
/// plan's 60 ms for a mark-read during a pass is not met (the list's own page costs most of it):
/// what this holds is the wait the pass adds. A measurement, as the store's (timings on a busy
/// machine swing too far to hold them in a test): run alone with `--ignored --nocapture`.
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
    s.save_folder("a1", "INBOX", &mailcraft_store::FolderState { validity: 7, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    for uid in 1..=n {
        let words: String = (0..800)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                format!("w{} ", x % 50_000)
            })
            .collect();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid };
        s.write_body(&key, format!("Subject: m\r\nMessage-ID: <m{uid}@x>\r\n\r\n{words}\r\n").as_bytes()).unwrap();
    }
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: 1 };
    let mark = |on: bool| {
        let t0 = Instant::now();
        s.apply_op("a1", &Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on }).unwrap();
        t0.elapsed().as_secs_f64() * 1000.0
    };
    let at = |took: &mut Vec<f64>, q: usize| {
        took.sort_by(f64::total_cmp);
        took.get((took.len() * q / 100).min(took.len().saturating_sub(1))).copied().unwrap_or(0.0)
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
    assert!(p99 - alone99 <= 60.0, "a mark-read took {p99:.1} ms (p99) during the pass, {alone99:.1} ms alone");
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}
