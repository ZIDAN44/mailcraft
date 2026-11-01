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
